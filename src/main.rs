//! GI-Utils: Game input automation utility.
//!
//! Uses the Interception driver for kernel-level keyboard/mouse input
//! injection. Press F12 to exit.

#![allow(dead_code)] // Phase 1: infrastructure will be used in Phase 3

mod config;
mod engine;
mod functions;
mod interception;
mod key;
mod scan_code;
mod utils;

use engine::Engine;
use std::sync::Arc;

fn main() {
    // Internal logging (tracing → stderr, debug builds only)
    #[cfg(debug_assertions)]
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    // ── Load config ─────────────────────────────────────────
    // Must come first — fail-fast if config is broken, no need to
    // wait through TSC calibration.
    let config = config::load().expect("Failed to load config");

    println!("══════════════════════════════════════════");
    println!("  GI-Utils v1.0.0");
    println!("  Game Input Automation Utility (Rust)");
    println!("══════════════════════════════════════════");
    println!();

    // Set real-time priority + CPU affinity BEFORE TSC calibration.
    // Real-time priority prevents the OS from preempting the busy-wait
    // calibration loop, which would skew the measured TSC frequency.
    print!("Setting real-time priority... ");
    if let Err(e) = utils::affinity::configure_self() {
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

    let engine = Engine::new();
    let send_ctx = engine.send_context();
    let bindings = engine.bindings();

    // ── Register bindings ───────────────────────────────────

    // Stop function — needs engine's stop_flag, special-cased
    let stop_func = Arc::new(functions::stop::停止退出::new(engine.stop_flag()));

    println!("Registered functions:");
    for binding in &config {
        let func: Arc<dyn engine::function::KeyFunction> =
            if binding.func == "停止退出" {
                stop_func.clone()
            } else {
                config::create_function(&binding.func, send_ctx.clone())
                    .unwrap_or_else(|e| panic!(
                        "config error: binding '{}' → '{}': {}",
                        binding.key.name(), binding.func, e
                    ))
            };
        bindings.register(binding.key, binding.mode, func);
        println!(
            "  {} = {}  ({:?})",
            binding.key.name(),
            binding.func,
            binding.mode
        );
    }
    println!();
    println!("Engine running.");
    println!();

    // Run the main event loop (blocks until F12)
    engine.run();

    // Exit beep: 375 Hz, 300 ms (same as original C++)
    utils::beep::beep(375, 300);

    // Restore all processes to full CPU affinity on exit
    if let Err(e) = utils::affinity::restore_all_affinity() {
        eprintln!("Warning: failed to restore affinity: {}", e);
    }

    println!("GI-Utils exiting. Goodbye!");
}
