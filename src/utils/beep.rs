//! 蜂鸣音反馈 — Audio feedback via system beep.
//!
//! 使用 Windows kernel32 的 `Beep` 函数。
//! Uses the Windows `Beep` function from kernel32.
//! 在 Windows 10+ 上，蜂鸣通过声卡而非 PC 喇叭输出。
//! On modern Windows (10+), this routes through the sound card
//! rather than the PC speaker.

use std::thread;

// Direct FFI — avoids windows-rs version churn for this trivial API.
extern "system" {
    fn Beep(dwFreq: u32, dwDuration: u32) -> i32;
}

/// 同步蜂鸣 — Play a beep synchronously. Blocks until the sound finishes.
///
/// 频率范围 37–32767 Hz，超出范围时输出警告并不执行蜂鸣。
/// Frequency must be in 37–32767 Hz range; warns and exits early if out of range.
pub fn beep(frequency: u32, duration_ms: u32) {
    if frequency < 37 || frequency > 32767 {
        eprintln!(
            "Warning: invalid beep frequency {} Hz (valid range: 37–32767)",
            frequency
        );
        return;
    }
    let ok = unsafe { Beep(frequency, duration_ms) };
    if ok == 0 {
        // Beep can fail if the sound card is busy or the system has no beep device.
        // Not actionable — just ignore.
    }
}

/// 异步蜂鸣 — Play a beep asynchronously on a separate thread.
/// Returns immediately; the beep plays in the background.
///
/// 预期用于低频使用场景（启动、退出、一次性反馈）。
/// Intended for infrequent use (startup, exit, one-off feedback).
/// 每次调用会创建 OS 线程；请勿在紧循环中调用。
/// Each call spawns an OS thread; do not call in tight loops.
pub fn beep_async(frequency: u32, duration_ms: u32) {
    if frequency < 37 || frequency > 32767 {
        eprintln!(
            "Warning: invalid beep frequency {} Hz (valid range: 37–32767)",
            frequency
        );
        return;
    }
    thread::spawn(move || {
        unsafe { Beep(frequency, duration_ms); }
    });
}
