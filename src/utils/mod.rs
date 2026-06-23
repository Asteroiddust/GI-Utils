//! 系统初始化工具集 — System initialization utilities.
//!
//! Orchestrates startup tasks in the correct dependency order:
//! DPI awareness → process priority/CPU affinity → TSC calibration.
//! Each submodule provides a focused capability for the engine loop.

/// 高精度延时 — High-precision TSC-based delay and calibration.
pub mod delay;

/// 蜂鸣反馈 — System beep for audio feedback (sync and async).
pub mod beep;

/// CPU 亲和性与优先级 — Core affinity and process priority management.
pub mod affinity;

/// 屏幕像素与光标 — Pixel color capture and cursor position via GDI.
pub mod screen;

/// 系统初始化 — Mirrors the original C++ `Utils::Initialize()`.
///
/// Sets DPI awareness, real-time CPU priority/core affinity, and calibrates
/// the TSC (Time Stamp Counter) for high-precision timing. Must be called
/// once at startup, before any timing-critical or GDI work.
pub fn init() {
    // 1. DPI awareness — must precede any GDI screen operations.
    //    Without this, GetPixel reads virtualized coordinates on high-DPI
    //    displays while GetCursorPos returns physical coordinates.
    print!("Setting DPI awareness... ");
    unsafe {
        windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    println!("done.");

    // 2. Real-time priority + CPU affinity — must precede TSC calibration.
    //    Prevents the OS from preempting the busy-wait calibration loop,
    //    which would skew the measured TSC frequency.
    print!("Setting real-time priority... ");
    if let Err(e) = affinity::configure_self() {
        eprintln!("FAILED: {}", e);
        eprintln!("Timing accuracy may be degraded.");
    } else {
        println!("done.");
    }
    println!();

    // 3. TSC frequency calibration — 20 × 100ms samples (~2 seconds).
    //    Must occur after priority is elevated for accurate timing.
    println!("Calibrating TSC frequency...");
    delay::calibrate_tsc_frequency();
    println!();
}
