//! 快速拾取 — 反复按 F 键 + 滚轮下拉。
//! 用于原神快速收集掉落物。

use crate::engine::event::{EventSequence, InputEvent, ScrollDir};
use crate::engine::function::KeyFunction;
use crate::interception::InterceptionContext;
use crate::scan_code::ScanCode;
use crate::utils::delay;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct 快速拾取 {
    sequence: EventSequence,
    send_ctx: Arc<InterceptionContext>,
}

impl 快速拾取 {
    pub fn new(send_ctx: Arc<InterceptionContext>) -> Self {
        let mut sequence = EventSequence::new();
        sequence
            .tap(ScanCode::F)
            .sleep(10.0)
            .wheel(ScrollDir::DOWN)
            .sleep(10.0);
        Self { sequence, send_ctx }
    }
}

impl KeyFunction for 快速拾取 {
    fn execute(&self, running: Arc<AtomicBool>) {
        let events = self.sequence.events();

        while running.load(Ordering::Acquire) {
            for event in events {
                self.send_ctx.send_event(event);
                if let InputEvent::Sleep { ms } = event {
                    delay::delay_ms_interruptible(*ms, &running);
                }
            }
        }
    }
}
