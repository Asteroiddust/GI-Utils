//! 鬼畜走路 (Ghost Walk) — WASD 交错按键产生鬼畜移动效果。
//! 用于原神鬼畜移动。Loop 模式，按住循环。

use crate::engine::event::{EventSequence, InputEvent};
use crate::engine::function::KeyFunction;
use crate::interception::ffi::INTERCEPTION_KEY_DOWN;
use crate::interception::SendContext;
use crate::key::Key;
use crate::utils::delay;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 鬼畜走路 (Ghost Walk) 功能 — Loop 模式。
///
/// 以 W → A → S → D 顺序交替短按 1ms + 间隔 49ms，产生鬼畜移动效果。
/// 用于原神鬼畜移动。
pub struct 鬼畜走路 {
    sequence: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 鬼畜走路 {
    /// 创建 `鬼畜走路` 实例。
    ///
    /// 构建 `EventSequence`：W hold 1ms + 49ms → A hold 1ms + 49ms → S hold 1ms + 49ms → D hold 1ms + 49ms。
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        let mut sequence = EventSequence::new();
        // W: 1ms press → release → 49ms gap → next key
        sequence
            .hold(Key::W, 1.0).sleep(49.0)
            .hold(Key::A, 1.0).sleep(49.0)
            .hold(Key::S, 1.0).sleep(49.0)
            .hold(Key::D, 1.0).sleep(49.0);
        Self { sequence, send_ctx }
    }
}

impl KeyFunction for 鬼畜走路 {
    /// 执行鬼畜走路循环。
    ///
    /// 按 W→A→S→D 顺序交替发送短按事件。跟踪当前按下的键，
    /// 提前停止时自动释放粘滞键保证输入状态清洁。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        let events = self.sequence.events();

        while !stop_requested.load(Ordering::Acquire) {
            let mut held = None;

            for event in events {
                // Track key state for cleanup on early stop
                if let InputEvent::Keyboard { code, state } = event {
                    if *state == INTERCEPTION_KEY_DOWN {
                        held = Some(*code);
                    } else {
                        held = None;
                    }
                }

                self.send_ctx.send_event(event);

                if let InputEvent::Sleep { ms } = event {
                    delay::delay_ms_interruptible(*ms, &stop_requested);
                    if stop_requested.load(Ordering::Acquire) {
                        // Release stuck key before exiting
                        if let Some(key) = held {
                            self.send_ctx.send_event(&InputEvent::release(key));
                        }
                        return;
                    }
                }
            }
        }
    }
}
