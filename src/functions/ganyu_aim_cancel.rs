//! 甘雨走A — 鼠标左右键连点 + R 键取消射箭后摇 (aim cancel)。
//! 用于原神甘雨走 A 输出手法。Once 模式，单次执行。

use crate::engine::event::EventSequence;
use crate::engine::function::KeyFunction;
use crate::interception::SendContext;
use crate::key::Key;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// 甘雨走 A 功能 — Once 模式。
///
/// 执行鼠标左键点击 → sleep 50ms → 鼠标右键点击 → sleep 30ms → press R → release R。
/// 用于原神甘雨走 A 射箭后摇取消 (aim cancel)。
pub struct 甘雨走A {
    sequence: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 甘雨走A {
    /// 创建 `甘雨走A` 实例。
    ///
    /// 构建 `EventSequence`：left_click → 50ms → right_click → 30ms → press R → release R。
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        let mut sequence = EventSequence::new();
        sequence
            .left_click()
            .sleep(50.0)
            .right_click()
            .sleep(30.0)
            .press(Key::R)
            .release(Key::R);
        Self { sequence, send_ctx }
    }
}

impl KeyFunction for 甘雨走A {
    /// 执行甘雨走 A 序列（单次，不循环）。
    ///
    /// 顺序播放 left_click → 50ms → right_click → 30ms → press R → release R；
    /// 各点击对与 R 对经 play 合并为一次 IOCTL_WRITE（6 次发送 → 3 次）。
    /// `stop` 传 None — 延时不可中断（Once 模式无需在 sleep 中响应停止，
    /// 对齐原 `delay_ms` 语义）。
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        self.sequence.play(&self.send_ctx, None);
    }
}
