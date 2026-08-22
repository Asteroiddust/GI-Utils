//! 连点器 v1 / v2 / 动态参数版 — 按住绑定键时快速连点鼠标左键。
//! Loop 模式，按住循环。
//!
//! - `连点器`（动态参数版）：`interval_ms` / `hold_ms` 可 GUI 实时调整
//!   （live 直写，下周期生效）— v1/v2 语义归并：hold=0 即 v1 的驱动级
//!   原子点击对，hold>0 即 v2 的按下/松开节奏。
//! - `连点器v1`：left_click + sleep 10ms（原版参数，静态版保留兼容）
//! - `连点器v2`：left_down 8ms / left_up 8ms（调参定稿，静态版保留兼容）

use crate::engine::bindings::{KeyFunction, ParamSpec, ParamValues};
use crate::engine::event::{EventSequence, InputEvent};
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

// ═══════════════════════════════════════════════════════════════════
// 连点器 v1 — 原版参数
// ═══════════════════════════════════════════════════════════════════

/// 连点器 v1 功能 — Loop 模式。
///
/// 按住绑定键时以 10ms 周期快速重复发送鼠标左键点击事件。
pub struct 连点器v1 {
    sequence: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 连点器v1 {
    /// 创建 `连点器v1` 实例。
    ///
    /// 构建一个 `EventSequence`：每次迭代执行一次鼠标左键点击，然后 sleep 10ms。
    /// 循环由 `KeyFunction::execute` 中的 `while` 控制。
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        let mut sequence = EventSequence::new();
        sequence.left_click().sleep(10.0);
        Self { sequence, send_ctx }
    }
}

impl KeyFunction for 连点器v1 {
    /// 执行连点循环。
    ///
    /// 反复播放 left_click → sleep 10ms；[down, up] 点击对经
    /// `EventSequence::play` 合并为一次 IOCTL_WRITE（驱动级原子送达），
    /// 每次 sleep 后检查 `stop_requested`。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        while !stop_requested.load(Ordering::Acquire) {
            self.sequence.play(&self.send_ctx, Some(&stop_requested));
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// 连点器 v2 — v1 的复制版，独立调参
// ═══════════════════════════════════════════════════════════════════

/// 连点器 v2 功能 — Loop 模式。v1 的复制版，调参请改本 struct。
///
/// 按住绑定键时以 16ms 周期快速重复发送鼠标左键点击事件（按下 8ms / 松开 8ms）。
pub struct 连点器v2 {
    sequence: EventSequence,
    send_ctx: Arc<SendContext>,
}

impl 连点器v2 {
    /// 创建 `连点器v2` 实例。
    ///
    /// 构建一个 `EventSequence`：左键按下 → sleep 8ms → 松开 → sleep 8ms
    /// （16ms 周期，按下时长独立可调）。循环由 `KeyFunction::execute` 中的
    /// `while` 控制。
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        let mut sequence = EventSequence::new();
        // hold_left(8ms) = left_down → sleep(8) → left_up（与手写三连等价，
        // 复用 event.rs 现成包装糖）
        sequence.hold_left(8.0).sleep(8.0);
        Self { sequence, send_ctx }
    }
}

impl KeyFunction for 连点器v2 {
    /// 执行连点循环。
    ///
    /// 反复播放 left_down → sleep 8ms → left_up → sleep 8ms
    /// （每个事件都被 Sleep 隔开，无连续段 — 合并对 v2 是 no-op），
    /// 每次 sleep 后检查 `stop_requested`。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        while !stop_requested.load(Ordering::Acquire) {
            self.sequence.play(&self.send_ctx, Some(&stop_requested));
        }
    }
}
