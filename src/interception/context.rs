//! Interception 上下文的类型化封装
//!
//! 底层实现见 [`crate::interception::protocol`]（原生 Rust 移植，替代原
//! interception.lib FFI）。
//!
//! # 线程安全 (Thread Safety)
//!
//! 根据 Interception 文档，一个上下文可在多个线程间安全共享 **发送** 操作。
//! 每个 **接收** 线程应使用自己的上下文。
//!
//! [`SendContext`] 在类型层面强制执行此约束 — 它只暴露发送方法，
//! 且实现了 `Send + Sync`。
//! [`InterceptionContext`] 实现了 `Send` 但 **不** 实现 `Sync`。

use crate::engine::event::InputEvent;
use crate::interception::pressed;
use crate::interception::protocol::{self, Device, InterceptionKeyStroke, KeyboardDevice};
use std::marker::PhantomData;

/// 创建原始上下文，失败 panic（驱动未安装）— 与旧 create_raw 语义一致。
fn create_raw() -> protocol::Context {
    protocol::Context::create().unwrap_or_else(|e| {
        panic!(
            "Failed to create Interception context ({e}). \
             Is the interception driver installed? \
             (https://github.com/oblitum/Interception)"
        )
    })
}

// ═══════════════════════════════════════════════════════════════════
// InterceptionContext — receive-only, single-thread
// ═══════════════════════════════════════════════════════════════════

/// 只接收的 Interception 上下文。
///
/// 必须在单线程中使用（接收操作非线程安全）。
/// 实现了 `Send`（可跨线程移动），但 **不** 实现 `Sync`。
///
/// Must be used from a single thread at a time (receiving is not
/// thread-safe). Implements `Send` (safe to move), but NOT `Sync`.
pub struct InterceptionContext {
    raw: protocol::Context,
    /// `*mut ()` 为 !Send + !Sync：阻断 Sync 自动实现（接收仅限单线程）；
    /// Send 由下方显式 unsafe impl 恢复（可跨线程移动）。
    _not_sync: PhantomData<*mut ()>,
}

unsafe impl Send for InterceptionContext {}

impl InterceptionContext {
    /// 创建新的 Interception 接收上下文（独立打开全部 20 个设备）。
    pub fn create() -> Self {
        Self {
            raw: create_raw(),
            _not_sync: PhantomData,
        }
    }

    // ── Receive ──────────────────────────────────────────────

    /// 阻塞等待输入到达。返回有待处理数据的设备；失败返回 `None`
    /// （对齐 C 版返回 0 哨兵）。
    pub fn wait(&self) -> Option<Device> {
        self.raw.wait()
    }

    /// 带超时的等待（毫秒）。超时时返回 `None`。
    pub fn wait_timeout(&self, ms: u32) -> Option<Device> {
        self.raw.wait_with_timeout(ms)
    }

    /// 设置设备过滤器（谓词为 Rust 闭包，替代 C 版 extern "C" 函数指针）。
    ///
    /// 返回 `Err` 表示至少一个目标设备的 SET_FILTER 失败；启动路径必须处理
    /// （否则会“启动成功但捕获不到任何键”）。
    pub fn set_filter(
        &self,
        predicate: impl Fn(Device) -> bool,
        filter: u16,
    ) -> windows::core::Result<()> {
        self.raw.set_filter(predicate, filter)
    }

    /// 从设备批量接收键盘输入，返回实际读到的前缀切片（防误用 API —
    /// 遍历返回值即遍历真实条目，缓冲尾部陈旧数据不可见）。
    ///
    /// 鼠标接收不在此暴露：引擎过滤器仅设键盘（鼠标事件 pass-through
    /// 不进队列）— 鼠标分支为防御性死代码已删（review）；底层协议 API
    /// 仍在 protocol.rs 完整保留。
    pub fn receive_keyboard<'a>(
        &self,
        device: KeyboardDevice,
        out: &'a mut [InterceptionKeyStroke],
    ) -> &'a mut [InterceptionKeyStroke] {
        self.raw.receive_keyboard(device, out)
    }
}

// ═══════════════════════════════════════════════════════════════════
// SendContext — send-only, thread-safe to share
// ═══════════════════════════════════════════════════════════════════

/// 只发送的 Interception 上下文，可在线程间安全共享。
///
/// 仅暴露 `send_event()`/`send_events()` 与 `forward_*` 方法。
/// Interception 驱动明确支持通过同一个句柄并发发送。
///
/// 发送是尽力而为：失败不向上传播，由 protocol 层 `warn!` 落日志即失败
/// 信号。底层 send_* 返回的实际写入数只在 UP 事件上消费（写成功才清除
/// panic 安全登记的挂起位，见 [`pressed`]）；DOWN 失败不重试（驱动级故障
/// 靠日志诊断即可）。
///
/// Thread-safe to share across threads. Exposes only send
/// methods — the driver supports concurrent sends through one handle.
pub struct SendContext {
    raw: protocol::Context,
}

impl SendContext {
    /// 创建新的 Interception 发送上下文（独立第二个上下文 —
    /// 双上下文架构，与历史行为一致）。
    pub fn create() -> Self {
        Self { raw: create_raw() }
    }

    /// 发送一个 [`InputEvent`]，自动路由到正确的设备（键盘 0 / 鼠标 0）。
    ///
    /// 同时做 panic 安全登记：DOWN 先登记，UP 写成功才清除（见 [`pressed`]）—
    /// panic hook 退出前据此补发挂起合成键。
    pub fn send_event(&self, event: &InputEvent) {
        self.send_one(event);
    }

    /// 单个事件的共用发送实现（send_event / send_events 唯一出口）：
    /// before_send → 发送（消费实写数）→ after_send。
    fn send_one(&self, event: &InputEvent) {
        pressed::before_send(event);
        let written = self.send_raw(event);
        pressed::after_send(event, written);
    }

    /// 发送单个事件但**不更新挂起登记**（panic / 正常关机补发专用）。
    ///
    /// 补发必须走本路径：若走 `send_event`，release 会经 `after_send` 清位 —
    /// 可能误清「快照之后其它线程并发按下」的同键位（补发丢位根因）。返回
    /// 实写条数（`Sleep` 返回 0）；调用方据此决定失败位是否重新登记。
    pub(crate) fn send_untracked(&self, event: &InputEvent) -> usize {
        self.send_raw(event)
    }

    /// 设备路由 + 驱动发送（**合成事件唯一出口**；`forward_keyboard` 是物理
    /// 转发旁路，不在此列），返回实写条数。
    fn send_raw(&self, event: &InputEvent) -> usize {
        match event {
            InputEvent::Keyboard { .. } => {
                let stroke = event.to_key_stroke().expect("Keyboard 分支");
                self.raw
                    .send_keyboard(protocol::keyboard(0), std::slice::from_ref(&stroke))
            }
            InputEvent::Mouse { .. } => {
                let stroke = event.to_mouse_stroke().expect("Mouse 分支");
                self.raw
                    .send_mouse(protocol::mouse(0), std::slice::from_ref(&stroke))
            }
            InputEvent::Sleep { .. } => 0,
        }
    }

    /// 引擎转发用：整批转发接收到的键盘 stroke（一次 IOCTL_WRITE，
    /// 与批量接收对称 — review：逐条转发浪费读侧省下的系统调用，
    /// 且更易与功能线程的发送交错）。
    ///
    /// **不做 panic 登记**：转发的是真实按键（OS 已有物理状态），panic 补发
    /// release 会与物理按住状态冲突。保持 `pub`：物理转发是引擎专用旁路
    /// （非合成事件），不属于 `send_raw` 的合成事件出口。
    pub fn forward_keyboard(&self, device: KeyboardDevice, strokes: &[InterceptionKeyStroke]) {
        self.raw.send_keyboard(device, strokes);
    }

    /// 发送事件序列 — **逐事件发送**（no-slicing 为最终形态，2026-10-23
    /// 作者确认）：每个事件独立一次 DeviceIoControl，不存在"连续同类事件
    /// 合并为一次 IOCTL_WRITE"的切片机制。
    ///
    /// 语义：不同功能线程的并发发送可以交错（无驱动级原子送达），点击对
    /// down/up 之间的间隔不保证 ≈0。Sleep 事件在此层无操作（时间语义由
    /// 调用方的 delay 原语承担）。历史切片实现见 `f621bf3^`
    /// （`segments` + `send_segment`），已删除，不再恢复。
    pub fn send_events(&self, events: &[InputEvent]) {
        for event in events {
            self.send_one(event);
        }
    }
}

// Sync 声明在此层而非 protocol::Context：SendContext 仅暴露发送方法，
// 驱动文档明确支持通过同一上下文并发发送（protocol::Context 不标 Sync —
// 其 receive 方法若被共享会并发瓜分驱动队列，review）。
unsafe impl Sync for SendContext {}

// 切段机制已移除（no-slicing 为最终形态，2026-10-23 作者确认）— SendContext 单测随实现删除。
