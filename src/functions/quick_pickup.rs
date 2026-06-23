//! 快速拾取 — 反复按 F 键 + 滚轮下拉。
//! 用于原神快速收集掉落物。

use crate::engine::event::{EventSequence, InputEvent, ScrollDir};
use crate::engine::function::KeyFunction;
use crate::interception::SendContext;
use crate::key::Key;
use crate::utils::delay;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct 快速拾取 {
    sequence: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 快速拾取 {
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
