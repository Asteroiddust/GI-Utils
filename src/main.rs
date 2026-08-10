//! GI-Utils: 游戏输入自动化工具 — Game input automation utility.
//!
//! 使用 Interception 驱动进行内核级键盘/鼠标输入注入。按 F12 退出。
//! Uses the Interception driver for kernel-level keyboard/mouse input injection.
//! Press F12 to exit.

use gi_utils::engine::Engine;
use gi_utils::{config, engine, functions, utils};
use std::sync::Arc;

/// 按终端显示宽度填充字符串（CJK 字符计 2 列，ASCII 计 1 列）。
fn pad_wide(s: &str, width: usize) -> String {
    let dw: usize = s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum();
    let pad = width.saturating_sub(dw);
    format!("{}{}", s, " ".repeat(pad))
}

/// ── Table layout ────────────────────────────────────────

const KEY_W: usize = 16;
const FUNC_W: usize = 12;
const MODE_W: usize = 6;

fn repeat(c: char, n: usize) -> String {
    std::iter::repeat(c).take(n).collect()
}

fn hr(left: &str, tee: &str, right: &str, fill: char) -> String {
    format!(
        "{}{}{}{}{}{}{}",
        left,
        repeat(fill, KEY_W),
        tee,
        repeat(fill, FUNC_W),
        tee,
        repeat(fill, MODE_W),
        right,
    )
}

fn row(key: &str, func: &str, mode: &str) -> String {
    format!(
        "\u{2502}{}\u{2502}{}\u{2502}{}\u{2502}",
        pad_wide(key, KEY_W),
        pad_wide(func, FUNC_W),
        pad_wide(mode, MODE_W),
    )
}

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
    for line in utils::init() {
        println!("{}", line);
    }

    let engine = Engine::new();
    let send_ctx = engine.send_context();
    let bindings = engine.bindings();

    // ── Register bindings ───────────────────────────────────
    // 停止功能需要引擎的 stop_flag，特殊处理
    // Stop function — needs engine's stop_flag, special-cased
    let stop_func = Arc::new(functions::stop::停止退出::new(engine.stop_flag()));

    println!("Registered functions:");
    println!("{}", hr("\u{250c}", "\u{252c}", "\u{2510}", '\u{2500}'));
    println!("{}", row("Key", "Function", "Mode"));
    println!("{}", hr("\u{251c}", "\u{253c}", "\u{2524}", '\u{2500}'));
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
            "{}",
            row(
                binding.key.name(),
                &binding.func,
                &format!("{:?}", binding.mode)
            ),
        );
    }
    println!("{}", hr("\u{2514}", "\u{2534}", "\u{2518}", '\u{2500}'));
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
