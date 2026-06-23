//! 双玛头 — 复杂鼠标按键序列 + S 键。
//! 用于原神双玛头操作。WhileHeld 模式。

use crate::engine::event::{EventSequence, InputEvent};
use crate::engine::function::KeyFunction;
use crate::interception::InterceptionContext;
use crate::scan_code::ScanCode;
use crate::utils::delay;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct 双玛头 {
    click_once: EventSequence,
    main_loop: EventSequence,
    send_ctx: Arc<InterceptionContext>,
}

impl 双玛头 {
    pub fn new(send_ctx: Arc<InterceptionContext>) -> Self {
        let mut click_once = EventSequence::new();
        click_once.left_click().sleep(40.0);

        let mut main_loop = EventSequence::new();
        // C++ 原版每轮 while 循环内连跑 5 遍序列。
        // 序列是静态的，在构造时展开为 5 份，执行时就是单层 for。
        for _ in 0..5 {
            main_loop
                .left_down()              //  1: L↓
                .sleep(180.0)             //     hold L 180ms
                .right_click()            //  2-3: R↓R↑
                .sleep(160.0)             //
                .left_up()                //  4: L↑
                .sleep(40.0)              //
                .left_down()              //  5: L↓
                .sleep(180.0)             //     hold L 180ms
                .right_click()            //  6-7: R↓R↑
                .press(ScanCode::S)       //  8: S↓
                .sleep(750.0)             //     hold S 750ms
                .release(ScanCode::S)     //  9: S↑
                .sleep(350.0)             //
                .left_up()                // 10: L↑
                .sleep(540.0);
        }

        Self { click_once, main_loop, send_ctx }
    }
}

impl KeyFunction for 双玛头 {
    fn execute(&self, running: Arc<AtomicBool>) {
        // ── on activate ──
        for event in self.click_once.events() {
            self.send_ctx.send_event(event);
            if let InputEvent::Sleep { ms } = event {
                delay::delay_ms(*ms);
            }
        }

        // ── main loop ──
        while running.load(Ordering::Acquire) {
            for event in self.main_loop.events() {
                self.send_ctx.send_event(event);
                if let InputEvent::Sleep { ms } = event {
                    delay::delay_ms_interruptible(*ms, &running);
                }
            }
        }
    }
}
