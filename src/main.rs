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
use functions::auto_clicker::连点器;
use functions::ganyu_aim_cancel::甘雨走A;
use functions::ghost_walk::鬼畜走路;
use functions::mavuika_jump::火神跳喷;
use functions::quick_pickup::快速拾取;
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

    // F13: 连点器
    let auto_clicker = Arc::new(连点器::new(send_ctx.clone()));
    bindings.register(KeyId::new(ScanCode::F13, false), TriggerMode::WhileHeld, auto_clicker);

    // F14: 快速拾取 (F + 滚轮下拉)
    let quick_pickup = Arc::new(快速拾取::new(send_ctx.clone()));
    bindings.register(KeyId::new(ScanCode::F14, false), TriggerMode::WhileHeld, quick_pickup);

    // F15: 鬼畜走路 (WASD 交错按键)
    let ghost_walk = Arc::new(鬼畜走路::new(send_ctx.clone()));
    bindings.register(KeyId::new(ScanCode::F15, false), TriggerMode::WhileHeld, ghost_walk);

    // F16: 火神跳喷 (初始跳 + 循环连跳)
    let fire_jump = Arc::new(火神跳喷::new(send_ctx.clone()));
    bindings.register(KeyId::new(ScanCode::F16, false), TriggerMode::WhileHeld, fire_jump);

    // F17: 甘雨走A (射箭后摇取消)
    let ganyu_aim = Arc::new(甘雨走A::new(send_ctx.clone()));
    bindings.register(KeyId::new(ScanCode::F17, false), TriggerMode::Once, ganyu_aim);

    // TODO: 继续注册移植的功能
    // bindings.register(KeyId::new(ScanCode::F18, false), 双玛头);
    // ...

    println!("Registered functions:");
    println!("  F13 = 连点器  (按住循环)");
    println!("  F14 = 快速拾取  (按住循环)");
    println!("  F15 = 鬼畜走路  (按住循环)");
    println!("  F16 = 火神跳喷  (按住循环)");
    println!("  F17 = 甘雨走A  (单次执行)");
    println!();
    println!("Monitoring started. Press F12 to exit.");
    println!();

    // Run the main event loop (blocks until F12)
    monitor.run();

    // Exit beep: 375 Hz, 300 ms (same as original C++)
    utils::beep::beep(375, 300);

    // Restore all processes to full CPU affinity on exit
    if let Err(e) = utils::affinity::restore_all_affinity() {
        eprintln!("Warning: failed to restore affinity: {}", e);
    }

    println!("GI-Utils exiting. Goodbye!");
}
