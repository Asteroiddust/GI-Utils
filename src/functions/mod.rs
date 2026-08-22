//! 功能模块 — 具体的游戏自动化功能实现。
//!
//! 每个子模块定义一个 struct 并实现 `KeyFunction` trait，
//! 通过 `TriggerMode` (Once / Loop / Toggle) 控制执行方式。

pub mod auto_clicker;
pub mod ganyu_aim_cancel;
pub mod ghost_walk;
pub mod mavuika_double_cancel;
pub mod mavuika_jump;
pub mod mouse_color;
pub mod optimize_game;
pub mod quick_pickup;
pub mod stop;
pub mod thread_sampler;

/// 已登记游戏进程名单 — 「优化游戏」找窗（名单优先序）与「线程采样」共用。
/// 与窗口类支持面的关系：名单**先于**窗口类查找（覆盖窗口类不明的游戏，
/// 如 Endfield）；窗口类（UnityWndClass/UnrealWindow）作未登记游戏的兜底。
/// 新游戏在此追加（找窗自动覆盖，pinning 策略另见 thread_pin::STRATEGIES）。
pub const GAME_PROCESS_NAMES: &[&str] = &[
    "YuanShen.exe",              // 原神（国服）
    "GenshinImpact.exe",         // 原神（国际服）
    "StarRail.exe",              // 崩铁
    "ZenlessZoneZero.exe",       // 绝区零
    "Client-Win64-Shipping.exe", // 鸣潮
    "b1.exe",                    // 黑神话：悟空
    "Endfield.exe",              // 明日方舟：终末地
];
