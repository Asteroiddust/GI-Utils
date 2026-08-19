//! 鬼畜走路 (Ghost Walk) — WASD 交错短按产生鬼畜移动效果。
//! 基于时间戳引擎的滚动调度器（RollingKeys）：按下由节奏器按绝对节奏实时
//! 产生、释放动态排程，对应 C++ 原版 next_press_time + scheduled_releases。
//! Loop 模式，按住循环。

use crate::engine::bindings::KeyFunction;
use crate::engine::timeline::{RollingKeys, RollingPlayer};
use crate::interception::SendContext;
use crate::key::Key;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// 鬼畜走路 (Ghost Walk) 功能 — Loop 模式。
///
/// 滚动调度：每 50ms 按下下一个键（W→A→S→D 轮转），每个键按 1ms 后释放
/// （短按，与 C++ 原版一致）。按下序列按绝对节奏无限均匀推进，无累计漂移；
/// 释放事件在按下时动态排程（按下时刻 + 按住时长）。
pub struct 鬼畜走路 {
    roller: RollingPlayer,
}

impl 鬼畜走路 {
    /// 创建 `鬼畜走路` 实例。
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        Self {
            roller: RollingKeys::new()
                .keys(vec![Key::W, Key::A, Key::S, Key::D])
                .interval(50.0)
                .duration(1.0)
                .into_player(send_ctx),
        }
    }
}

impl KeyFunction for 鬼畜走路 {
    /// 无限滚动播放。stop 时 play() 内部已补发所有挂起释放，无需手动清理。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        self.roller.play(&stop_requested);
    }
}
