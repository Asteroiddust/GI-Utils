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
pub mod spam_key;
pub mod stop;
pub mod thread_sampler;

use crate::engine::event::InputEvent;
use crate::interception::SendContext;
use crate::utils::delay;
use std::sync::atomic::AtomicBool;

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

// ═══════════════════════════════════════════════════════════════════
// 敲击节奏引擎 — 连点器 / SpamKey 共用（2026-09 抽取：此前两份复制
// 各自维护同一语义，属项目出现过的"两处复制 → 修复漂移"风险面）
// ═══════════════════════════════════════════════════════════════════

/// 一个敲击周期的发送计划（纯决策 — 可单测）。
///
/// `interval` / `hold` 均来自参数槽（GUI 直写下周期生效）。取值已在 spec
/// 范围内：非有限值由上游拦截（`profile::apply_params` 显式拒绝 + GUI
/// DragValue 按 range 钳制），故此处不做 NaN 兜底。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ClickPlan {
    /// `hold <= 0`：down+up 合并为一次驱动级原子批（v1 语义），随后等满 interval。
    AtomicBatch,
    /// `hold > 0`：down → 等 `hold` → up → 等 `rest` 补足（v2 语义）。
    Split { hold: f64, rest: f64 },
}

/// 节奏参数 → 本周期动作计划（纯函数）。`hold` 钳到 `[0, interval]`：
/// 负值/未配置 → 0（原子批）；超过 interval → 钳到 interval（此时
/// `rest = 0`，调用方跳过补足等待 — 周期即按下时长）。
pub(crate) fn click_plan(interval_ms: f64, hold_ms: f64) -> ClickPlan {
    let hold = hold_ms.clamp(0.0, interval_ms);
    if hold <= 0.0 {
        ClickPlan::AtomicBatch
    } else {
        ClickPlan::Split {
            hold,
            rest: interval_ms - hold,
        }
    }
}

/// 执行一个周期的敲击 — 连点器与 SpamKey 的**唯一**实现（v1/v2 归并语义
/// 此前在两侧各有一份复制，修复只能靠人肉对齐）。
///
/// `down`/`up` 由调用方按目标设备构造（鼠标左键 / 任意键盘键）；参数每
/// 周期读取；停止响应由 `delay_ms_interruptible` 的 ~100us 检查点承担。
pub(crate) fn click_rhythm(
    send_ctx: &SendContext,
    interval_ms: f64,
    hold_ms: f64,
    stop: &AtomicBool,
    down: InputEvent,
    up: InputEvent,
) {
    match click_plan(interval_ms, hold_ms) {
        ClickPlan::AtomicBatch => {
            // 原子批：down+up 一次 IOCTL（栈数组，零分配）— 点击时长最短，
            // 且不受其他功能线程并发发送插入
            send_ctx.send_events(&[down, up]);
            delay::delay_ms_interruptible(interval_ms, stop);
        }
        ClickPlan::Split { hold, rest } => {
            send_ctx.send_event(&down);
            delay::delay_ms_interruptible(hold, stop);
            send_ctx.send_event(&up);
            if rest > 0.0 {
                delay::delay_ms_interruptible(rest, stop);
            }
        }
    }
}

// 节奏决策单测 — 纯逻辑，不触碰驱动（SendContext::create 不在此层调用）。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_plan_zero_or_negative_hold_is_atomic_batch() {
        assert_eq!(click_plan(10.0, 0.0), ClickPlan::AtomicBatch);
        // 负值（未配置/手写错误）钳到 0 → 原子批（不分拆成按下/松开）
        assert_eq!(click_plan(10.0, -5.0), ClickPlan::AtomicBatch);
    }

    #[test]
    fn click_plan_split_tops_up_rest_after_release() {
        // interval 10 / hold 4 → 松开后补足 6（周期总长恒为 interval）
        assert_eq!(
            click_plan(10.0, 4.0),
            ClickPlan::Split {
                hold: 4.0,
                rest: 6.0
            }
        );
    }

    #[test]
    fn click_plan_hold_clamped_to_interval_without_rest() {
        // hold > interval → 钳到 interval，rest = 0（调用方跳过补足等待）
        assert_eq!(
            click_plan(10.0, 25.0),
            ClickPlan::Split {
                hold: 10.0,
                rest: 0.0
            }
        );
        // 边界：hold == interval → rest 恰为 0，`rest > 0.0` 守卫跳过补足等待
        assert_eq!(
            click_plan(10.0, 10.0),
            ClickPlan::Split {
                hold: 10.0,
                rest: 0.0
            }
        );
    }
}
