//! 连点器 — 按住绑定键时快速连点鼠标左键。
//! Loop 模式，按住循环。

use crate::engine::event::{EventSequence, InputEvent};
use crate::engine::function::KeyFunction;
use crate::interception::SendContext;
use crate::utils::delay;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 连点器功能 — Loop 模式。
///
/// 按住绑定键时以 10ms 间隔快速重复发送鼠标左键点击事件。
pub struct 连点器 {
    sequence: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 连点器 {
    /// 创建 `连点器` 实例。
    ///
    /// 构建一个 `EventSequence`：每次迭代执行一次鼠标左键点击，然后 sleep 10ms。
    /// 循环由 `KeyFunction::execute` 中的 `while` 控制。
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        let mut sequence = EventSequence::new();
        sequence.left_click().sleep(10.0);
        Self { sequence, send_ctx }
    }
}

impl KeyFunction for 连点器 {
    /// 执行连点循环。
    ///
    /// 反复发送 left_click → sleep 10ms，每次 sleep 后检查 `stop_requested`。
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
