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

/// 跨进程线程信息采样 — NT 快照 + ToolHelp + 逐线程句柄查询
/// （Process Explorer Threads 页自动化，线程 pinning 决策数据）。
pub mod thread_info;

/// 热线程 pinning — 按进程名策略把游戏最重线程钉上金银核
/// （候选域过滤 + Top-N + 新鲜度重映射，见 CLAUDE.md 设计决策表）。
pub mod thread_pin;

/// 全局日志收集 — Global log collection via tracing subscriber (GUI log panel).
pub mod log_collector;

/// 系统初始化 — Mirrors the original C++ `Utils::Initialize()`.
///
/// Sets DPI awareness, real-time CPU priority/core affinity, and calibrates
/// the TSC (Time Stamp Counter) for high-precision timing. Must be called
/// once at startup, before any timing-critical or GDI work.
///
/// 返回初始化日志行。GUI 无控制台，println 输出会被静默丢弃 —
/// 调用方注入日志面板。
pub fn init() -> Vec<String> {
    let mut lines: Vec<String> = Vec::with_capacity(8);

    // 1. DPI awareness — must precede any GDI screen operations.
    //    Without this, GetPixel reads virtualized coordinates on high-DPI
    //    displays while GetCursorPos returns physical coordinates.
    lines.push("Setting DPI awareness...".into());
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    lines.push("done.".into());

    // 2. Real-time priority + CPU affinity — must precede TSC calibration.
    //    Prevents the OS from preempting the busy-wait calibration loop,
    //    which would skew the measured TSC frequency.
    lines.push("Setting real-time priority...".into());
    if let Err(e) = affinity::configure_self() {
        lines.push(format!("FAILED: {}", e));
        lines.push("Timing accuracy may be degraded.".into());
    } else {
        lines.push("done.".into());
    }

    // 3. TSC frequency calibration — 20 × 100ms samples (~2 seconds).
    //    Must occur after priority is elevated for accurate timing.
    lines.push("Calibrating TSC frequency...".into());
    let (_freq, cal_lines) = delay::calibrate_tsc_frequency();
    lines.extend(cal_lines);

    lines
}
