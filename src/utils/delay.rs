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

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

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
    // 饱和加法：极端 ms 值下 read_tsc() + ticks 回绕会变成 ~0ms 的瞬间返回
    // （review 4.1）；saturating 保证目标是远期时刻而非已过期时刻。
    let target = read_tsc().saturating_add((ms * freq / 1000.0) as u64);
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
///
/// 实现为 [`wait_until_interruptible`] 的包装（同一公式、同一检查节奏）。
pub fn delay_ms_interruptible(ms: f64, stop_requested: &AtomicBool) {
    if ms <= 0.0 {
        return;
    }
    wait_until_interruptible(tsc_now() + ms_to_ticks(ms), stop_requested);
}

/// 当前 TSC 值 — 绝对时刻调度的时钟源。
/// Current TSC value — the clock source for absolute-time scheduling
/// (monotonic with nanosecond-level resolution on invariant-TSC CPUs).
#[inline(always)]
pub fn tsc_now() -> u64 {
    read_tsc()
}

/// 毫秒 → TSC 周期数（基于已校准频率）。`ms <= 0` 归零。
/// Convert milliseconds to TSC ticks using the calibrated frequency.
#[inline(always)]
pub fn ms_to_ticks(ms: f64) -> u64 {
    if ms <= 0.0 {
        0
    } else {
        (ms * tsc_freq() / 1000.0) as u64
    }
}

/// 忙等直到 TSC 达到绝对时刻 `target_ticks`，每 ~100us 检查停止标志。
/// Busy-wait until the TSC reaches the absolute moment `target_ticks`,
/// checking the stop flag every ~100us.
///
/// 与 [`delay_ms_interruptible`] 精度与响应节奏相同，区别是目标以绝对 TSC
/// 时刻给出——时间轴执行器对齐整个时间线只需换算一次。
/// Same precision and stop-response cadence as [`delay_ms_interruptible`],
/// but the target is an absolute TSC moment — the timeline player
/// converts its schedule to absolute ticks once at playback start.
pub fn wait_until_interruptible(target_ticks: u64, stop_requested: &AtomicBool) {
    let interval = check_interval();
    let mut next_check = read_tsc().wrapping_add(interval);

    while read_tsc() < target_ticks {
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
/// 20 samples x 100ms，总计约 2 秒。返回校准后的频率（Hz）与进度日志行。
/// 启动时自动执行。日志行供调用方输出（GUI 注入日志面板）。
pub fn calibrate_tsc_frequency() -> (f64, Vec<String>) {
    calibrate(20, 100.0)
}

/// 测量 TSC 频率 — Measure TSC frequency using `sample_count` samples of `duration_ms` each.
/// 返回中位频率（Hz）与逐样本日志行。
fn calibrate(sample_count: usize, duration_ms: f64) -> (f64, Vec<String>) {
    use std::time::Instant;

    let dur = std::time::Duration::from_secs_f64(duration_ms / 1000.0);
    let mut rates: Vec<f64> = Vec::with_capacity(sample_count);
    let mut lines: Vec<String> = Vec::with_capacity(sample_count + 2);

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
            lines.push(format!(
                "  sample {:>2}/{}: {:.0} Hz",
                i + 1,
                sample_count,
                rate
            ));
            rates.push(rate);
        } else {
            lines.push(format!(
                "  sample {:>2}/{}: invalid — skipping",
                i + 1,
                sample_count
            ));
        }
    }

    let n = rates.len();
    if n == 0 {
        // 所有样本无效 — 使用硬编码回退。
        // All samples invalid — use hardcoded fallback.
        // Should never happen on modern invariant-TSC CPUs.
        lines.push("  -> all samples invalid, using default frequency".into());
        return (tsc_freq(), lines);
    }

    rates.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());

    let median = if n % 2 == 0 {
        (rates[n / 2 - 1] + rates[n / 2]) / 2.0
    } else {
        rates[n / 2]
    };

    init_tsc_freq(median);
    lines.push(format!(
        "  -> calibrated: {:.0} Hz (median of {} x {}ms samples)",
        median, sample_count, duration_ms
    ));
    (median, lines)
}

// ── Tests ─────────────────────────────────────────────────────
// 全部瞬时完成，不等待真实时间；不触碰驱动（SendContext::create）。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_to_ticks_clamps_nonpositive() {
        assert_eq!(ms_to_ticks(0.0), 0);
        assert_eq!(ms_to_ticks(-3.0), 0);
        assert!(ms_to_ticks(1.0) > 0);
    }

    #[test]
    fn wait_until_past_target_returns_immediately() {
        // 目标已到期（= 现在）— 忙等循环体不执行，立即返回
        wait_until_interruptible(tsc_now(), &AtomicBool::new(false));
    }

    #[test]
    fn wait_until_pre_set_stop_returns_early() {
        // stop 预置 true + 目标在未来 — 首个检查点即返回
        let future = tsc_now() + ms_to_ticks(1000.0);
        wait_until_interruptible(future, &AtomicBool::new(true));
    }
}
