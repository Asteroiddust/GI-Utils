//! 连点器（时间轴） — 绝对时刻节奏的鼠标左键连点（Timestamp 范式）。
//!
//! 与 Serial 版[连点器](crate::functions::auto_clicker::连点器)并行存在，互不影响：
//! - 旧版：每周期 `down → up → delay(interval)`，周期耗时含发送与调度开销；
//! - 本版：维护**下一次按下的绝对 TSC 截止时刻**（`next_down += interval`），
//!   `hold_ms` 只影响释放排程，不进入按下间隔的计算 —— 每次按下之间的间隔
//!   严格锚定在 `interval_ms` 网格上（发送系统调用抖动为 µs 级）。
//!
//! # 调度语义（与 `RollingKeys` 一致）
//!
//! - 第一次按下在激活时立即发生；之后第 k 次按下的**计划时刻** = 起点 + k×interval。
//! - 若某次按下被系统延迟但未越过下一次截止时刻，网格保持不变，后续按下自动追回相位。
//! - 若延迟越过下一次截止时刻（卡顿）：错过的拍子即弃，以当前时刻重新锚定，
//!   **不突发追拍**（同一时刻连发多次点击对游戏手感最差）。
//! - `hold_ms` 在运行时钳制到 `[0, interval_ms]`；等于 interval 时先释放再按下。
//!   停止时若仍有按下未释放，立即补发 release（与全局 panic 兜底互补）。
//!
//! # 触发模式
//!
//! 推荐 **Toggle**（按下热键开始，再按一次停止）或 **Loop**（按住连点）。
//! Once 模式下 `execute` 会持续运行直到进程退出（引擎不会为该模式置 stop），
//! 不要用于本功能。
//!
//! # 动态参数
//!
//! 每次按下前读取参数快照；等待期间每 ≤1ms 复检一次 **interval**，缩短间隔会在
//! ≤1ms 内重新锚定下一拍（不沿用旧间隔的远期截止时刻）。参数由 GUI 直写原子槽，
//! 不重启线程。

use crate::engine::bindings::{KeyFunction, ParamSpec, ParamValues};
use crate::engine::event::InputEvent;
use crate::interception::SendContext;
use crate::utils::delay;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 参数声明 — 索引即槽位（interval=0，hold=1）。
const SPEC: &[ParamSpec] = &[
    ParamSpec::float("interval_ms", 10.0, 1.0, 10_000.0, 0.5),
    ParamSpec::float("hold_ms", 0.0, 0.0, 10_000.0, 0.5),
];

/// 时间轴连点器 — Timestamp 范式的鼠标左键连点。
///
/// 计划时刻直接由 TSC 绝对时刻推进（非 `sleep(interval)` 串行延时），
/// 因此按下间隔不受发送耗时累积影响。
pub struct 连点器时间轴 {
    send_ctx: Arc<SendContext>,
    params: Arc<ParamValues>,
}

impl 连点器时间轴 {
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        Self {
            send_ctx,
            params: Arc::new(ParamValues::new(SPEC)),
        }
    }
}

impl KeyFunction for 连点器时间轴 {
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        // 下一次按下截止时刻（绝对 TSC）。首次立即按下。
        let mut next_down = delay::tsc_now();
        // 上一次排程使用的间隔（ticks）— 参数变更时重新锚定下一拍，
        // 避免缩短 interval 后仍等待旧间隔留下的远期截止时刻。
        let mut scheduled_interval_ticks = 0u64;
        // 按住释放的绝对截止时刻（hold=0 时不排队，按下后立即释放）。
        let mut pending_up: Option<u64> = None;

        loop {
            // 0. 停止优先：先补发未释放的按下，再退出（防卡鼠标左键）。
            if stop_requested.load(Ordering::Acquire) {
                if pending_up.take().is_some() {
                    self.send_ctx.send_event(&InputEvent::left_up());
                }
                return;
            }

            let now = delay::tsc_now();

            // 1. 到期释放优先于按下（同刻先 up 后 down，与时间轴排序一致）。
            if let Some(due) = pending_up {
                if due <= now {
                    self.send_ctx.send_event(&InputEvent::left_up());
                    pending_up = None;
                    continue;
                }
            }

            // 2. 到期按下：读取参数 → 必要时先补发上一拍 release → 发送 down
            //    → 排程 up → 推进绝对网格。
            if now >= next_down {
                let interval = read_interval(&self.params);
                let hold = read_hold(&self.params, interval);
                let interval_ticks = delay::ms_to_ticks(interval);

                // 关键顺序：hold == interval（或钳到 interval）时，上一拍 release
                // 与下一拍按下几乎同刻，且 release 排程在 down 发送之后（晚一个
                // 发送耗时）。若不先补发，pending_up 会被下一拍覆盖 → 鼠标卡住。
                let plan = plan_press(
                    next_down,
                    now,
                    interval_ticks,
                    scheduled_interval_ticks,
                    pending_up.is_some(),
                );
                if plan.flush_pending {
                    pending_up = None;
                    self.send_ctx.send_event(&InputEvent::left_up());
                }

                self.send_ctx.send_event(&InputEvent::left_down());
                let hold_ticks = delay::ms_to_ticks(hold);
                if hold_ticks == 0 {
                    self.send_ctx.send_event(&InputEvent::left_up());
                } else {
                    pending_up = Some(delay::tsc_now().saturating_add(hold_ticks));
                }

                next_down = plan.next_down;
                scheduled_interval_ticks = plan.scheduled_interval;
                continue;
            }

            // 3. 参数复检（≤1ms 一次）：interval 缩短时立即重锚下一拍，
            //    避免沿用旧间隔留下的远期截止时刻（如 10s → 10ms 等 10s）。
            let interval_ticks_now = delay::ms_to_ticks(read_interval(&self.params));
            if interval_ticks_now != scheduled_interval_ticks {
                next_down = now.saturating_add(interval_ticks_now);
                scheduled_interval_ticks = interval_ticks_now;
                continue;
            }

            // 4. 忙等到最近的截止时刻（下一次按下 / 本次释放 / 参数复检窗口）。
            let target = match pending_up {
                Some(due) => next_down.min(due),
                None => next_down,
            };
            let wake = target.min(now.saturating_add(delay::ms_to_ticks(PARAM_POLL_MS)));
            delay::wait_until_interruptible(wake, &stop_requested);
        }
    }

    fn parameters(&self) -> &[ParamSpec] {
        SPEC
    }

    fn param_store(&self) -> Option<Arc<ParamValues>> {
        Some(self.params.clone())
    }
}

// ═══════════════════════════════════════════════════════════════════
// 纯调度决策 — 可单测（不依赖驱动 / TSC）
// ═══════════════════════════════════════════════════════════════════

/// 等待期间参数复检窗口（毫秒）：缩短 interval 后最迟这么久重锚下一拍。
const PARAM_POLL_MS: f64 = 1.0;

/// 一次按下的决策结果（纯函数输出，便于单测顺序与网格逻辑）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PressPlan {
    /// 发 down 前必须先补发上一拍 release（pending_up 存在时恒为 true）。
    flush_pending: bool,
    next_down: u64,
    scheduled_interval: u64,
}

/// 计划一次按下：`flush_pending` 保证「先 release 后 press」的顺序（hold ==
/// interval 时上一拍 up 与下一拍 down 同刻，若不先发会覆盖 pending_up）。
#[inline]
fn plan_press(
    next_down: u64,
    now: u64,
    interval_ticks: u64,
    scheduled_interval_ticks: u64,
    has_pending_up: bool,
) -> PressPlan {
    PressPlan {
        flush_pending: has_pending_up,
        next_down: next_down_after_press(next_down, now, interval_ticks, scheduled_interval_ticks),
        scheduled_interval: interval_ticks,
    }
}

/// 推进绝对网格：正常情况下 `prev + interval`；若该截止时刻已被当前时刻
/// 越过（卡顿/发送过慢），从当前时刻重新锚定 `now + interval`，丢弃错过拍。
#[inline]
fn advance_next_down(prev: u64, now: u64, interval: u64) -> u64 {
    let next = prev.saturating_add(interval);
    if next <= now {
        now.saturating_add(interval)
    } else {
        next
    }
}

/// 一次按下后的下一拍：间隔未变时沿用绝对网格（追回相位/丢错过拍）；
/// 参数间隔变更时从本次按下重新锚定，避免旧间隔的远期截止时刻拖慢新参数。
#[inline]
fn next_down_after_press(prev: u64, now: u64, interval: u64, scheduled_interval: u64) -> u64 {
    if interval == scheduled_interval {
        advance_next_down(prev, now, interval)
    } else {
        now.saturating_add(interval)
    }
}

/// `interval_ms` 读取与防御：非有限值回落默认 10ms；钳到参数面板范围。
#[inline]
fn read_interval(params: &ParamValues) -> f64 {
    let v = params.get_f64(0);
    if v.is_finite() {
        v.clamp(1.0, 10_000.0)
    } else {
        10.0
    }
}

/// `hold_ms` 读取与防御：非有限值回落 0；钳到 `[0, interval]`。
#[inline]
fn read_hold(params: &ParamValues, interval: f64) -> f64 {
    clamp_hold(params.get_f64(1), interval)
}

#[inline]
fn clamp_hold(hold: f64, interval: f64) -> f64 {
    if hold.is_finite() {
        hold.clamp(0.0, interval)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_keeps_grid_when_on_time() {
        // 上一拍 1000，间隔 100 → 下一拍 1100（当前 1050 未越过）
        assert_eq!(advance_next_down(1000, 1050, 100), 1100);
        // 刚好到达截止时刻：本次在 1100 触发后，下一拍锚定 1200
        assert_eq!(advance_next_down(1000, 1100, 100), 1200);
    }

    #[test]
    fn advance_reanchors_after_stall_without_burst() {
        // 当前 1500 已越过 1100 → 从 1500 重新锚定 1600（只补一拍，不连发）
        assert_eq!(advance_next_down(1000, 1500, 100), 1600);
        // 更长的卡顿同样只锚定到未来一拍
        assert_eq!(advance_next_down(1000, 10_000, 100), 10_100);
    }

    #[test]
    fn advance_saturates_instead_of_overflow() {
        assert_eq!(advance_next_down(u64::MAX - 1, 0, 10), u64::MAX);
    }

    #[test]
    fn interval_change_reanchors_next_beat() {
        // 旧间隔 1000 的下次截止 2000；改成 100 后从当前 1100 重新锚定 1200，
        // 不再等到 2000。
        assert_eq!(next_down_after_press(2000, 1100, 100, 1000), 1200);
        // 间隔未变：保持网格（1200）
        assert_eq!(next_down_after_press(1100, 1100, 100, 100), 1200);
    }

    #[test]
    fn press_plan_flushes_pending_up_before_next_down() {
        // hold == interval：上一拍 release 尚未到期（晚一个发送耗时）时，
        // 下一拍按下必须先把 pending release 补发掉（否则 pending_up 被覆盖）。
        let plan = plan_press(1000, 1000, 100, 100, true);
        assert!(plan.flush_pending);
        assert_eq!(plan.next_down, 1100);
        assert_eq!(plan.scheduled_interval, 100);

        // 正常情况（无 pending）不需要补发。
        assert!(!plan_press(1000, 1000, 100, 100, false).flush_pending);

        // 同一次按下同时发生参数变更：从当前时刻重锚。
        let changed = plan_press(10_000, 1000, 10, 100, false);
        assert_eq!(changed.next_down, 1010);
        assert_eq!(changed.scheduled_interval, 10);
    }

    #[test]
    fn param_readers_guard_non_finite_values() {
        let params = ParamValues::new(SPEC);
        params.set_f64(0, f64::NAN);
        assert_eq!(read_interval(&params), 10.0);
        params.set_f64(0, 0.0);
        assert_eq!(read_interval(&params), 1.0);
        params.set_f64(0, 1.0e9);
        assert_eq!(read_interval(&params), 10_000.0);

        params.set_f64(1, f64::INFINITY);
        assert_eq!(read_hold(&params, 10.0), 0.0);
        params.set_f64(1, 25.0);
        assert_eq!(read_hold(&params, 10.0), 10.0);
        params.set_f64(1, 4.0);
        assert_eq!(read_hold(&params, 10.0), 4.0);
    }

    #[test]
    fn hold_is_clamped_to_interval() {
        assert_eq!(clamp_hold(0.0, 10.0), 0.0);
        assert_eq!(clamp_hold(4.0, 10.0), 4.0);
        assert_eq!(clamp_hold(25.0, 10.0), 10.0);
        assert_eq!(clamp_hold(-5.0, 10.0), 0.0);
        assert_eq!(clamp_hold(f64::NAN, 10.0), 0.0);
    }
}
