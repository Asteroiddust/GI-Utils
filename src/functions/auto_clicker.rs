//! Auto-clicker: rapidly clicks the left mouse button while active.

use crate::engine::event::{EventSequence, InputEvent};
use crate::engine::function::KeyFunction;
use crate::interception::{interception_mouse, InterceptionContext};
use crate::utils::delay;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct AutoClicker {
    sequence: EventSequence,
    send_ctx: Arc<InterceptionContext>,
}

impl AutoClicker {
    pub fn new(send_ctx: Arc<InterceptionContext>) -> Self {
        let mut sequence = EventSequence::new();
        sequence.left_click().sleep(10.0);
        Self { sequence, send_ctx }
    }
}

impl KeyFunction for AutoClicker {
    fn execute(&self, running: Arc<AtomicBool>) {
        let events = self.sequence.events();
        let device = interception_mouse(0);

        while running.load(Ordering::Acquire) {
            for event in events {
                self.send_ctx.send_event(device, event);
                if let InputEvent::Sleep { ms } = event {
                    delay::delay_ms(*ms);
                }
            }
        }
    }
}
