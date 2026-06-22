//! GI-Utils: Game input automation utility.
//!
//! Uses the Interception driver for kernel-level keyboard/mouse input
//! injection. Press F12 to exit.

#![allow(dead_code)] // Phase 1: infrastructure will be used in Phase 3

mod engine;
mod functions;
mod interception;
mod scan_code;
mod utils;

use engine::{KeyMonitor, TriggerMode};
use engine::bindings::KeyId;
use functions::auto_clicker::AutoClicker;
use scan_code::ScanCode;
use std::sync::Arc;

fn main() {
    // Internal logging (tracing → stderr, debug builds only)
    #[cfg(debug_assertions)]
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    println!("══════════════════════════════════════════");
    println!("  GI-Utils v0.1.0");
    println!("  Game Input Automation Utility (Rust)");
    println!("══════════════════════════════════════════");
    println!();

    // Set real-time priority + CPU affinity BEFORE TSC calibration.
    // Real-time priority prevents the OS from preempting the busy-wait
    // calibration loop, which would skew the measured TSC frequency.
    print!("Setting real-time priority... ");
    if let Err(e) = utils::affinity::init_tool_affinity() {
        eprintln!("FAILED: {}", e);
        eprintln!("Timing accuracy may be degraded.");
    } else {
        println!("done.");
    }
    println!();

    // Calibrate TSC frequency: 20 × 100ms samples (~2 seconds)
    println!("Calibrating TSC frequency...");
    utils::delay::calibrate_tsc_frequency();
    println!();

    // Create the main monitor
    let monitor = KeyMonitor::new();
    let send_ctx = monitor.send_context();
    let bindings = monitor.bindings();

    // ── Register functions ──────────────────────────────────

    // F13: Auto clicker
    let auto_clicker = Arc::new(AutoClicker::new(send_ctx.clone()));
    bindings.register(KeyId::new(ScanCode::F13, false), TriggerMode::WhileHeld, auto_clicker);

    // TODO: Register more functions as they are ported
    // bindings.register(KeyId::new(SC_F14, false), quick_pickup);
    // bindings.register(KeyId::new(SC_F15, false), dragon_spin);
    // ...

    println!("Registered functions:");
    println!("  F13 = Auto Clicker (hold to activate)");
    println!();
    println!("Monitoring started. Press F12 to exit.");
    println!();

    // Run the main event loop (blocks until F12)
    monitor.run();

    println!();

    // Restore all processes to full CPU affinity on exit
    if let Err(e) = utils::affinity::restore_all_affinity() {
        eprintln!("Warning: failed to restore affinity: {}", e);
    }

    println!("GI-Utils exiting. Goodbye!");
}
