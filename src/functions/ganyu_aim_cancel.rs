//! 甘雨走A — 鼠标左右键连点 + R 键取消射箭后摇。
//! 用于原神甘雨走A输出手法。Once 模式，单次执行。

use crate::engine::event::{EventSequence, InputEvent};
use crate::engine::function::KeyFunction;
use crate::interception::InterceptionContext;
use crate::key::Key;
use crate::utils::delay;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub struct 甘雨走A {
    sequence: EventSequence,
    send_ctx: Arc<InterceptionContext>,
}

impl 甘雨走A {
    pub fn new(send_ctx: Arc<InterceptionContext>) -> Self {
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
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        let events = self.sequence.events();
        for event in events {
            self.send_ctx.send_event(event);
            if let InputEvent::Sleep { ms } = event {
                delay::delay_ms(*ms);
            }
        }
    }
}
