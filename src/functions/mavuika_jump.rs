//! 火神跳喷 (Mavuika Jump) — 初始跳跃后循环空格连跳。
//! 用于原神火神跳喷移动。Loop 模式，按住循环。

use crate::engine::event::{EventSequence, InputEvent};
use crate::engine::function::KeyFunction;
use crate::interception::SendContext;
use crate::key::Key;
use crate::utils::delay;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 火神跳喷 (Mavuika Jump) 功能 — Loop 模式。
///
/// 激活时先执行一次不可中断的初始跳跃 (tap SPACE + 120ms)，
/// 然后以 10ms 间隔循环 tap SPACE，实现火神跳喷移动。
pub struct 火神跳喷 {
    initial_jump: EventSequence,
    loop_seq: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 火神跳喷 {
    /// 创建 `火神跳喷` 实例。
    ///
    /// 构建两个 `EventSequence`：
    /// - `initial_jump`: tap SPACE + 120ms (不可中断，保证首次跳跃成功)
    /// - `loop_seq`: tap SPACE + 10ms (循环段，响应 stop_requested)
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        let mut initial_jump = EventSequence::new();
        initial_jump.tap(Key::SPACE).sleep(120.0);

        let mut loop_seq = EventSequence::new();
        loop_seq.tap(Key::SPACE).sleep(10.0);

        Self {
            initial_jump,
            loop_seq,
            send_ctx,
        }
    }
}

impl KeyFunction for 火神跳喷 {
    /// 执行火神跳喷：先初始跳跃 (不可中断) → 循环跳跃 (可中断)。
    ///
    /// 初始段使用 `delay_ms` 保证首次跳跃不被中途打断；
    /// 循环段使用 `delay_ms_interruptible` 以实现即时停止响应。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        // ── on activate: initial jump (non-cancellable, matches C++) ──
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
