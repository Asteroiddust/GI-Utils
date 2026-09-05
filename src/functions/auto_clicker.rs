//! 连点器 — 按住绑定键时快速连点鼠标左键。Loop 模式，按住循环。
//!
//! 动态参数版（2026-08-22 起唯一版本，v1/v2 已归并删除）：
//! `interval_ms` / `hold_ms` 可 GUI 实时调整（live 直写，下周期生效）—
//! hold=0 即 v1 的驱动级原子点击对，hold>0 即 v2 的按下/松开节奏。

use crate::engine::bindings::{KeyFunction, ParamSpec, ParamValues};
use crate::engine::event::InputEvent;
use crate::interception::SendContext;
use crate::utils::delay;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// ═══════════════════════════════════════════════════════════════════
// 连点器（动态参数版）— v1/v2 归并
// ═══════════════════════════════════════════════════════════════════

/// 连点器参数声明 — 索引即槽位（interval=0，hold=1）。
const CLICKER_SPEC: &[ParamSpec] = &[
    ParamSpec::float("interval_ms", 10.0, 1.0, 10_000.0, 0.5),
    ParamSpec::float("hold_ms", 0.0, 0.0, 10_000.0, 0.5),
];

/// 连点器（动态参数版）— Loop 模式。
///
/// 每周期读参数快照：`interval_ms` 为点击周期；`hold_ms` 为按下时长
/// （0 = down+up 合并为一次驱动级原子批 — 最短点击且不受并发发送插入；
/// >0 = down → hold → up → 补足 interval）。GUI 修改下周期生效，
/// 不停线程。
pub struct 连点器 {
    send_ctx: Arc<SendContext>,
    params: Arc<ParamValues>,
}

impl 连点器 {
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        Self {
            send_ctx,
            params: Arc::new(ParamValues::new(CLICKER_SPEC)),
        }
    }
}

impl KeyFunction for 连点器 {
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        while !stop_requested.load(Ordering::Acquire) {
            let interval = self.params.get_f64(0);
            let hold = self.params.get_f64(1).clamp(0.0, interval);
            if hold <= 0.0 {
                // v1 语义：点击对一次 IOCTL（栈数组，零分配）
                self.send_ctx
                    .send_events(&[InputEvent::left_down(), InputEvent::left_up()]);
                delay::delay_ms_interruptible(interval, &stop_requested);
            } else {
                // v2 语义：独立按下/松开节奏
                self.send_ctx.send_event(&InputEvent::left_down());
                delay::delay_ms_interruptible(hold, &stop_requested);
                self.send_ctx.send_event(&InputEvent::left_up());
                let rest = interval - hold;
                if rest > 0.0 {
                    delay::delay_ms_interruptible(rest, &stop_requested);
                }
            }
        }
    }

    fn parameters(&self) -> &[ParamSpec] {
        CLICKER_SPEC
    }

    fn param_store(&self) -> Option<Arc<ParamValues>> {
        Some(self.params.clone())
    }
}
