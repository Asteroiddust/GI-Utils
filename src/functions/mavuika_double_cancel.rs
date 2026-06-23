//! 双玛头 (Mavuika Double Cancel) — 复杂鼠标按键序列 + S 键。
//! 用于原神玛薇卡双坠操作。Loop 模式，按住循环。

use crate::engine::event::{EventSequence, InputEvent};
use crate::engine::function::KeyFunction;
use crate::interception::ffi::{INTERCEPTION_KEY_DOWN, INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN,
    INTERCEPTION_MOUSE_LEFT_BUTTON_UP, INTERCEPTION_KEY_UP};
use crate::interception::SendContext;
use crate::key::Key;
use crate::utils::delay;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 双玛头 (Mavuika Double Cancel) 功能 — Loop 模式。
///
/// 执行复杂的鼠标按键序列（左键按/放 + 右键点击）+ S 键，用于原神玛薇卡双坠操作。
/// 每次循环展开 5 轮序列，每轮包含左键 hold、右键点击、S 键 hold 等步骤。
pub struct 双玛头 {
    click_once: EventSequence,
    main_loop: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 双玛头 {
    /// 创建 `双玛头` 实例。
    ///
    /// 构建两个 `EventSequence`：
    /// - `click_once`: 激活时先执行一次 left_click + 40ms
    /// - `main_loop`: C++ 原版每轮 while 内跑 5 遍序列，此处构造时展开为 5 份，
    ///   每份含 10 步（左键 down/up、右键 click、S press/release 等时间控制）
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
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
                .press(Key::S)       //  8: S↓
                .sleep(750.0)             //     hold S 750ms
                .release(Key::S)     //  9: S↑
                .sleep(350.0)             //
                .left_up()                // 10: L↑
                .sleep(540.0);
        }

        Self { click_once, main_loop, send_ctx }
    }
}

impl KeyFunction for 双玛头 {
    /// 执行双玛头操作：初始点击 → 主循环。
    ///
    /// 初始段发送 left_click + 40ms (不可中断)。
    /// 主循环发送展开的 5 轮序列，跟踪鼠标左键和 S 键状态，
    /// 提前停止时自动释放粘滞键。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        // ── on activate ──
        for event in self.click_once.events() {
            self.send_ctx.send_event(event);
            if let InputEvent::Sleep { ms } = event {
                delay::delay_ms(*ms);
            }
        }

        // ── main loop ──
        while !stop_requested.load(Ordering::Acquire) {
            let mut lbtn_held = false;
            let mut s_held = false;

            for event in self.main_loop.events() {
                // Track held state for cleanup on early exit
                match event {
                    InputEvent::Mouse { state, .. } => {
                        if *state == INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN { lbtn_held = true; }
                        else if *state == INTERCEPTION_MOUSE_LEFT_BUTTON_UP { lbtn_held = false; }
                    }
                    InputEvent::Keyboard { state, .. } => {
                        if *state == INTERCEPTION_KEY_DOWN { s_held = true; }
                        else if *state == INTERCEPTION_KEY_UP { s_held = false; }
                    }
                    _ => {}
                }

                self.send_ctx.send_event(event);

                if let InputEvent::Sleep { ms } = event {
                    delay::delay_ms_interruptible(*ms, &stop_requested);
                    if stop_requested.load(Ordering::Acquire) {
                        if lbtn_held { self.send_ctx.send_event(&InputEvent::left_up()); }
                        if s_held { self.send_ctx.send_event(&InputEvent::release(Key::S)); }
                        return;
                    }
                }
            }
        }
    }
}
