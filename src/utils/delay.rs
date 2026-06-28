//! 高精度延时 — High-precision delay using x86_64 TSC (Time Stamp Counter).
//!
//! Replicates the TSC-based delay from the original C++ `Utils::delay()`.
//! 在现代 x86_64 CPU 上使用恒定 TSC，无需 syscall 即可达到亚微秒级精度。
//! On modern x86_64 CPUs with invariant TSC, this provides sub-microsecond
//! precision without syscall overhead.
//!
//! TSC frequency is calibrated automatically at startup (20 × 100ms samples).
//! 若校准被跳过，将回退到硬编码默认值（9800X3D 基础频率）。
//! Falls back to a hardcoded default (9800X3D base clock) if calibration is skipped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

// ── Safe TSC wrapper ──────────────────────────────────────────

/// 读取 CPU 时间戳计数器 — Read the CPU's Time Stamp Counter.
/// Safe wrapper around the `RDTSC` instruction.
#[inline(always)]
fn read_tsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// 自旋等待提示 — Hint to the CPU that we are in a spin-wait loop.
#[inline(always)]
fn cpu_relax() {
    core::arch::x86_64::_mm_pause();
}

// ── Frequency storage ─────────────────────────────────────────

/// TSC 频率（Hz）— TSC frequency in Hertz.
/// Set by `calibrate_tsc_frequency()` at startup.
/// 若校准被跳过，回退到 9800X3D 基础频率。
/// Falls back to 9800X3D base clock if calibration was somehow skipped.
static TSC_FREQ: OnceLock<f64> = OnceLock::new();

/// 写入校准后的 TSC 频率 — Store the calibrated TSC frequency.
/// Called once by calibration at startup. 重复调用会被静默忽略。
pub(crate) fn init_tsc_freq(freq_hz: f64) {
    if TSC_FREQ.set(freq_hz).is_err() {
        // Already calibrated — ignore repeat call.
        // The first calibration result is kept.
    }
}

/// 获取当前 TSC 频率 — Get the current TSC frequency.
fn tsc_freq() -> f64 {
    *TSC_FREQ.get_or_init(|| {
        // Safe default: 9800X3D base clock (spread spectrum off)
        4_699_909_550.0
    })
}

/// 中断检查间隔（TSC 周期数）— Check interval for interruptible delay (~100us).
/// Computed once from the calibrated frequency.
static CHECK_INTERVAL: OnceLock<u64> = OnceLock::new();

fn check_interval() -> u64 {
    *CHECK_INTERVAL.get_or_init(|| (tsc_freq() / 10_000.0) as u64)
}

// ── Public API ────────────────────────────────────────────────

/// 高精度忙等待延时（毫秒）— High-precision busy-wait delay in milliseconds.
///
/// 使用 `RDTSC` + `PAUSE` 实现最低延迟。在恒定 TSC CPU 上精度可达数纳秒。
/// Uses `RDTSC` + `PAUSE` for minimal latency. On invariant TSC CPUs,
/// accuracy is within a few nanoseconds.
///
/// 当 `ms <= 0.0` 时立即返回。
/// Returns immediately if `ms <= 0.0`.
pub fn delay_ms(ms: f64) {
    if ms <= 0.0 {
        return;
    }
    let freq = tsc_freq();
    let target = read_tsc() + (ms * freq / 1000.0) as u64;
    while read_tsc() < target {
        cpu_relax();
    }
}

/// 可中断的高精度延时 — Like [`delay_ms`], but returns early if `running` becomes false.
///
/// 每 ~100us 检查一次标志位——对人机输入延迟无感知，对热路径开销可忽略。
/// Checks the flag every ~100us — imperceptible latency for human
/// input, negligible overhead for the hot path.
///
/// 精度与 [`delay_ms`] 相同：TSC 目标固定，额外的 `RDTSC` 和标志检查不改变退出时刻。
/// Precision is identical to [`delay_ms`]: the TSC target is fixed,
/// and neither the extra `RDTSC` nor the flag check changes the
/// exit moment.
pub fn delay_ms_interruptible(ms: f64, stop_requested: &AtomicBool) {
    if ms <= 0.0 {
        return;
    }
    let freq = tsc_freq();
    let target = read_tsc() + (ms * freq / 1000.0) as u64;

    let interval = check_interval();
    let mut next_check = read_tsc().wrapping_add(interval);

    while read_tsc() < target {
        if read_tsc() >= next_check {
            if stop_requested.load(Ordering::Acquire) {
                return;
            }
            next_check = read_tsc().wrapping_add(interval);
        }
        cpu_relax();
    }
}

// ── Calibration ──────────────────────────────────────────────

/// 校准 TSC 频率 — Calibrate TSC frequency with default parameters.
///
/// 20 samples x 100ms，总计约 2 秒。返回校准后的频率（Hz）。启动时自动执行。
/// 20 samples x 100ms. Returns the calibrated frequency in Hz.
/// Called automatically at startup. Takes ~2 seconds total.
pub fn calibrate_tsc_frequency() -> f64 {
    calibrate(20, 100.0)
}

/// 测量 TSC 频率 — Measure TSC frequency using `sample_count` samples of `duration_ms` each.
/// 返回中位频率（Hz）。
/// Returns the median frequency in Hz.
fn calibrate(sample_count: usize, duration_ms: f64) -> f64 {
    use std::time::Instant;

    let dur = std::time::Duration::from_secs_f64(duration_ms / 1000.0);
    let mut rates: Vec<f64> = Vec::with_capacity(sample_count);

    use std::io::{self, Write};

    for i in 0..sample_count {
        let start = Instant::now();
        let tsc_start = read_tsc();

        while start.elapsed() < dur {
            cpu_relax();
        }

        let tsc_end = read_tsc();
        let elapsed = start.elapsed().as_secs_f64();
        let rate = (tsc_end - tsc_start) as f64 / elapsed;
        if rate.is_finite() {
            print!("\r  sample {:>2}/{}: {:.0} Hz", i + 1, sample_count, rate);
            io::stdout().flush().ok();
            rates.push(rate);
        } else {
            eprintln!(
                "\r  sample {:>2}/{}: invalid — skipping",
                i + 1,
                sample_count
            );
        }
    }

    let n = rates.len();
    if n == 0 {
        // 所有样本无效 — 使用硬编码回退。
        // All samples invalid — use hardcoded fallback.
        // Should never happen on modern invariant-TSC CPUs.
        eprintln!("\r  -> all samples invalid, using default frequency");
        return tsc_freq();
    }

    rates.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());

    let median = if n % 2 == 0 {
        (rates[n / 2 - 1] + rates[n / 2]) / 2.0
    } else {
        rates[n / 2]
    };

    init_tsc_freq(median);
    print!(
        "\r  -> calibrated: {:.0} Hz (median of {} x {}ms samples)",
        median, sample_count, duration_ms
    );
    io::stdout().flush().ok();
    println!(); // final newline
    median
}
