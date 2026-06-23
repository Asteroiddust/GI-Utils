//! Audio feedback via system beep.
//!
//! Uses the Windows `Beep` function from kernel32.
//! On modern Windows (10+), this routes through the sound card
//! rather than the PC speaker.

use std::thread;

// Direct FFI — avoids windows-rs version churn for this trivial API.
extern "system" {
    fn Beep(dwFreq: u32, dwDuration: u32) -> i32;
}

/// Play a beep synchronously. Blocks until the sound finishes.
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

/// Play a beep asynchronously on a separate thread.
/// Returns immediately; the beep plays in the background.
///
/// Intended for infrequent use (startup, exit, one-off feedback).
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
