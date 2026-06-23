//! 火神跳喷 — 初始跳跃后循环空格连跳。
//! 用于原神火神跳喷移动。WhileHeld 模式，按住循环。

use crate::engine::event::{EventSequence, InputEvent};
use crate::engine::function::KeyFunction;
use crate::interception::InterceptionContext;
use crate::key::Key;
use crate::utils::delay;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct 火神跳喷 {
    initial_jump: EventSequence,
    loop_seq: EventSequence,
    send_ctx: Arc<InterceptionContext>,
}

impl 火神跳喷 {
    pub fn new(send_ctx: Arc<InterceptionContext>) -> Self {
        let mut initial_jump = EventSequence::new();
        initial_jump.tap(Key::SPACE).sleep(120.0);

        let mut loop_seq = EventSequence::new();
        loop_seq.tap(Key::SPACE).sleep(10.0);

        Self { initial_jump, loop_seq, send_ctx }
    }
}

impl KeyFunction for 火神跳喷 {
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        // ── on activate: initial jump ──
        for event in self.initial_jump.events() {
            self.send_ctx.send_event(event);
            if let InputEvent::Sleep { ms } = event {
                delay::delay_ms(*ms);
            }
        }

        // ── while held: loop jump ──
        while !stop_requested.load(Ordering::Acquire) {
            for event in self.loop_seq.events() {
                self.send_ctx.send_event(event);
                if let InputEvent::Sleep { ms } = event {
                    delay::delay_ms_interruptible(*ms, &stop_requested);
                }
            }
        }
    }
}
