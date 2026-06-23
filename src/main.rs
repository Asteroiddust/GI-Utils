//! GI-Utils: 游戏输入自动化工具 — Game input automation utility.
//!
//! 使用 Interception 驱动进行内核级键盘/鼠标输入注入。按 F12 退出。
//! Uses the Interception driver for kernel-level keyboard/mouse input injection.
//! Press F12 to exit.

#![allow(dead_code)] // 第一阶段：基础设施将在第三阶段使用。Phase 1 infra unused in Phase 2.

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
    // 内部日志（debug 构建时 tracing 输出到 stderr）
    // Internal logging (tracing to stderr, debug builds only)
    #[cfg(debug_assertions)]
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    println!("══════════════════════════════════════════");
    println!("  GI-Utils v1.0.0");
    println!("  Game Input Automation Utility (Rust)");
    println!("══════════════════════════════════════════");
    println!();

    // ── Load config ─────────────────────────────────────────
    // 配置必须最先加载——配置损坏时快速失败，无需等待 TSC 校准。
    // Must come first — fail-fast if config is broken, no need to
    // wait through TSC calibration.
    let config = config::load().expect("Failed to load config");

    // 系统初始化：DPI → 优先级/亲和性 → TSC 校准
    // System init: DPI → priority/affinity → TSC calibration
    utils::init();

    let engine = Engine::new();
    let send_ctx = engine.send_context();
    let bindings = engine.bindings();

    // ── Register bindings ───────────────────────────────────
    // 停止功能需要引擎的 stop_flag，特殊处理
    // Stop function — needs engine's stop_flag, special-cased
    let stop_func = Arc::new(functions::stop::停止退出::new(engine.stop_flag()));

    println!("Registered functions:");
    for binding in &config {
        let func: Arc<dyn engine::function::KeyFunction> = if binding.func == "停止退出" {
            stop_func.clone()
        } else {
            config::create_function(&binding.func, send_ctx.clone()).unwrap_or_else(|e| {
                panic!(
                    "config error: binding '{}' -> '{}': {}",
                    binding.key.name(),
                    binding.func,
                    e
                )
            })
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

    // 运行主事件循环（阻塞直到 F12 按下）
    // Run the main event loop (blocks until F12)
    engine.run();

    // 退出蜂鸣：375 Hz, 300 ms（与原 C++ 版一致）
    // Exit beep: 375 Hz, 300 ms (same as original C++)
    utils::beep::beep(375, 300);

    // 退出时恢复所有进程的完整 CPU 亲和性
    // Restore all processes to full CPU affinity on exit
    if let Err(e) = utils::affinity::restore_all_affinity() {
        eprintln!("Warning: failed to restore affinity: {}", e);
    }

    println!("GI-Utils exiting. Goodbye!");
}
