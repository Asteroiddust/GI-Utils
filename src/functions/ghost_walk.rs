//! 鬼畜走路 — WASD 交错按键，产生鬼畜移动效果。
//! 用于原神鬼畜移动。WhileHeld 模式，按住循环。

use crate::engine::event::{EventSequence, InputEvent};
use crate::engine::function::KeyFunction;
use crate::interception::InterceptionContext;
use crate::key::Key;
use crate::utils::delay;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct 鬼畜走路 {
    sequence: EventSequence,
    send_ctx: Arc<InterceptionContext>,
}

impl 鬼畜走路 {
    pub fn new(send_ctx: Arc<InterceptionContext>) -> Self {
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
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        let events = self.sequence.events();

        while !stop_requested.load(Ordering::Acquire) {
            let mut held = None;

            for event in events {
                // Track key state for cleanup on early stop
                if let InputEvent::Keyboard { code, state } = event {
                    if *state == crate::interception::ffi::INTERCEPTION_KEY_DOWN {
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
