//! High-precision delay using x86_64 TSC (Time Stamp Counter).
//!
//! Replicates the TSC-based delay from the original C++ Utils::delay().
//! On modern x86_64 CPUs with invariant TSC, this provides sub-microsecond
//! precision without syscall overhead.
//!
//! TSC frequency is calibrated automatically at startup (20 × 100ms samples).
//! Falls back to a hardcoded default (9800X3D base clock) if calibration is skipped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

// ── Safe TSC wrapper ──────────────────────────────────────────

/// Read the CPU's Time Stamp Counter.
/// Safe wrapper around the `RDTSC` instruction.
#[inline(always)]
fn read_tsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Hint to the CPU that we are in a spin-wait loop.
#[inline(always)]
fn cpu_relax() {
    core::arch::x86_64::_mm_pause();
}

// ── Frequency storage ─────────────────────────────────────────

/// TSC frequency in Hz. Set by `calibrate_tsc_frequency()` at startup.
/// Falls back to 9800X3D base clock if calibration was somehow skipped.
static TSC_FREQ: OnceLock<f64> = OnceLock::new();

/// Set the TSC frequency (called once by calibration).
pub(crate) fn init_tsc_freq(freq_hz: f64) {
    if TSC_FREQ.set(freq_hz).is_err() {
        // Already calibrated — ignore repeat call.
        // The first calibration result is kept.
    }
}

/// Get the current TSC frequency.
fn tsc_freq() -> f64 {
    *TSC_FREQ.get_or_init(|| {
        // Safe default: 9800X3D base clock (spread spectrum off)
        4_699_909_550.0
    })
}

/// Check interval in TSC cycles (~100μs for interruptible delay).
/// Computed once from the calibrated frequency.
static CHECK_INTERVAL: OnceLock<u64> = OnceLock::new();

fn check_interval() -> u64 {
    *CHECK_INTERVAL.get_or_init(|| {
        (tsc_freq() / 10_000.0) as u64
    })
}

// ── Public API ────────────────────────────────────────────────

/// High-precision busy-wait delay in milliseconds.
///
/// Uses `RDTSC` + `PAUSE` for minimal latency. On invariant TSC CPUs,
/// accuracy is within a few nanoseconds.
///
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

/// Like [`delay_ms`], but returns early if `running` becomes false.
///
/// Checks the flag every ~100μs — imperceptible latency for human
/// input, negligible overhead for the hot path.
///
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

/// Calibrate TSC frequency with default parameters: 20 samples × 100ms.
/// Returns the calibrated frequency in Hz.
/// Called automatically at startup. Takes ~2 seconds total.
pub fn calibrate_tsc_frequency() -> f64 {
    calibrate(20, 100.0)
}

/// Measure TSC frequency using `sample_count` samples of `duration_ms` each.
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
            eprintln!("\r  sample {:>2}/{}: invalid — skipping", i + 1, sample_count);
        }
    }
    println!(); // final newline after \r overwrites

    let n = rates.len();
    if n == 0 {
        // All samples invalid — use hardcoded fallback.
        // Should never happen on modern invariant-TSC CPUs.
        eprintln!("  -> all samples invalid, using default frequency");
        return tsc_freq();
    }

    rates.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());

    let median = if n % 2 == 0 {
        (rates[n / 2 - 1] + rates[n / 2]) / 2.0
    } else {
        rates[n / 2]
    };

    init_tsc_freq(median);
    println!(
        "  -> calibrated: {:.0} Hz (median of {} x {}ms samples)",
        median, sample_count, duration_ms
    );
    median
}
