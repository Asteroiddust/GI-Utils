//! 快速拾取 — 反复按 F 键 + 鼠标滚轮下拉。
//! 用于原神快速收集掉落物。Loop 模式，按住循环。

use crate::engine::event::{EventSequence, InputEvent, ScrollDir};
use crate::engine::function::KeyFunction;
use crate::interception::SendContext;
use crate::key::Key;
use crate::utils::delay;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 快速拾取功能 — Loop 模式。
///
/// 按住绑定键时反复执行 F 键按下 + 鼠标滚轮下拉，用于原神快速收集掉落物。
pub struct 快速拾取 {
    sequence: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 快速拾取 {
    /// 创建 `快速拾取` 实例。
    ///
    /// 构建 `EventSequence`：tap F → sleep 10ms → wheel DOWN → sleep 10ms，循环执行。
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        let mut sequence = EventSequence::new();
        sequence
            .tap(Key::F)
            .sleep(10.0)
            .wheel(ScrollDir::DOWN)
            .sleep(10.0);
        Self { sequence, send_ctx }
    }
}

impl KeyFunction for 快速拾取 {
    /// 执行快速拾取循环。
    ///
    /// 反复发送 tap F → sleep 10ms → wheel DOWN → sleep 10ms，每次 sleep 后检查 `stop_requested`。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        let events = self.sequence.events();

        while !stop_requested.load(Ordering::Acquire) {
            for event in events {
                self.send_ctx.send_event(event);
                if let InputEvent::Sleep { ms } = event {
                    delay::delay_ms_interruptible(*ms, &stop_requested);
                }
            }
        }
    }
}
