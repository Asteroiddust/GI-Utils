//! 连点器 v1 / v2 — 按住绑定键时快速连点鼠标左键。
//! Loop 模式，按住循环。
//!
//! 两个独立版本同文件维护，调参互不影响：
//! - `连点器v1`：left_click + sleep 10ms（原版参数）
//! - `连点器v2`：left_down 8ms / left_up 8ms（调参定稿）

use crate::engine::event::EventSequence;
use crate::engine::function::KeyFunction;
use crate::interception::SendContext;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// ═══════════════════════════════════════════════════════════════════
// 连点器 v1 — 原版参数
// ═══════════════════════════════════════════════════════════════════

/// 连点器 v1 功能 — Loop 模式。
///
/// 按住绑定键时以 10ms 周期快速重复发送鼠标左键点击事件。
pub struct 连点器v1 {
    sequence: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 连点器v1 {
    /// 创建 `连点器v1` 实例。
    ///
    /// 构建一个 `EventSequence`：每次迭代执行一次鼠标左键点击，然后 sleep 10ms。
    /// 循环由 `KeyFunction::execute` 中的 `while` 控制。
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        let mut sequence = EventSequence::new();
        sequence.left_click().sleep(10.0);
        Self { sequence, send_ctx }
    }
}

impl KeyFunction for 连点器v1 {
    /// 执行连点循环。
    ///
    /// 反复播放 left_click → sleep 10ms；[down, up] 点击对经
    /// `EventSequence::play` 合并为一次 IOCTL_WRITE（驱动级原子送达），
    /// 每次 sleep 后检查 `stop_requested`。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        while !stop_requested.load(Ordering::Acquire) {
            self.sequence.play(&self.send_ctx, Some(&stop_requested));
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// 连点器 v2 — v1 的复制版，独立调参
// ═══════════════════════════════════════════════════════════════════

/// 连点器 v2 功能 — Loop 模式。v1 的复制版，调参请改本 struct。
///
/// 按住绑定键时以 16ms 周期快速重复发送鼠标左键点击事件（按下 8ms / 松开 8ms）。
pub struct 连点器v2 {
    sequence: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 连点器v2 {
    /// 创建 `连点器v2` 实例。
    ///
    /// 构建一个 `EventSequence`：左键按下 → sleep 8ms → 松开 → sleep 8ms
    /// （16ms 周期，按下时长独立可调）。循环由 `KeyFunction::execute` 中的
    /// `while` 控制。
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        let mut sequence = EventSequence::new();
        // hold_left(8ms) = left_down → sleep(8) → left_up（与手写三连等价，
        // 复用 event.rs 现成包装糖）
        sequence.hold_left(8.0).sleep(8.0);
        Self { sequence, send_ctx }
    }
}

impl KeyFunction for 连点器v2 {
    /// 执行连点循环。
    ///
    /// 反复播放 left_down → sleep 8ms → left_up → sleep 8ms
    /// （每个事件都被 Sleep 隔开，无连续段 — 合并对 v2 是 no-op），
    /// 每次 sleep 后检查 `stop_requested`。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        while !stop_requested.load(Ordering::Acquire) {
            self.sequence.play(&self.send_ctx, Some(&stop_requested));
        }
    }
}
