//! Interception 上下文的类型化封装
//!
//! 底层实现见 [`crate::interception::native`]（原生 Rust 移植，替代原
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
use crate::interception::native::{
    self, Device, InterceptionKeyStroke, InterceptionMouseStroke, KeyboardDevice, MouseDevice,
};
use std::marker::PhantomData;

/// 创建原始上下文，失败 panic（驱动未安装）— 与旧 create_raw 语义一致。
fn create_raw() -> native::Context {
    native::Context::create().unwrap_or_else(|e| {
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
    raw: native::Context,
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

    /// 阻塞等待输入到达。返回有待处理数据的设备。
    pub fn wait(&self) -> Device {
        self.raw.wait()
    }

    /// 带超时的等待（毫秒）。超时时返回 `None`。
    pub fn wait_timeout(&self, ms: u32) -> Option<Device> {
        self.raw.wait_with_timeout(ms)
    }

    /// 设置设备过滤器（谓词为 Rust 闭包，替代 C 版 extern "C" 函数指针）。
    pub fn set_filter(&self, predicate: impl Fn(Device) -> bool, filter: u16) {
        self.raw.set_filter(predicate, filter);
    }

    /// 从设备批量接收键盘输入，返回实际读取条数（一次 IOCTL 最多
    /// `out` 长度条 — 引擎以批缓冲调用，突发输入一次取回）。
    pub fn receive_keyboard(&self, device: KeyboardDevice, out: &mut [InterceptionKeyStroke]) -> usize {
        self.raw.receive_keyboard(device, out)
    }

    /// 从设备批量接收鼠标输入，返回实际读取条数。
    pub fn receive_mouse(&self, device: MouseDevice, out: &mut [InterceptionMouseStroke]) -> usize {
        self.raw.receive_mouse(device, out)
    }
}

// ═══════════════════════════════════════════════════════════════════
// SendContext — send-only, thread-safe to share
// ═══════════════════════════════════════════════════════════════════

/// 只发送的 Interception 上下文，可在线程间安全共享。
///
/// 仅暴露 `send_event()` 与 `forward_*` 方法。
/// Interception 驱动明确支持通过同一个句柄并发发送。
///
/// Thread-safe to share across threads. Exposes only send
/// methods — the driver supports concurrent sends through one handle.
pub struct SendContext {
    raw: native::Context,
}

impl SendContext {
    /// 创建新的 Interception 发送上下文（独立第二个上下文 —
    /// 双上下文架构，与历史行为一致）。
    pub fn create() -> Self {
        Self { raw: create_raw() }
    }

    /// 发送一个 [`InputEvent`]，自动路由到正确的设备（键盘 0 / 鼠标 0）。
    pub fn send_event(&self, event: &InputEvent) {
        match event {
            InputEvent::Keyboard { code, state } => {
                let stroke = InterceptionKeyStroke {
                    code: code.raw(),
                    state: *state,
                    information: 0,
                };
                self.raw
                    .send_keyboard(native::keyboard(0), std::slice::from_ref(&stroke));
            }
            InputEvent::Mouse {
                state,
                flags,
                rolling,
                x,
                y,
            } => {
                let stroke = InterceptionMouseStroke {
                    state: *state,
                    flags: *flags,
                    rolling: *rolling,
                    x: *x,
                    y: *y,
                    information: 0,
                };
                self.raw
                    .send_mouse(native::mouse(0), std::slice::from_ref(&stroke));
            }
            InputEvent::Sleep { .. } => {}
        }
    }

    /// 引擎转发用：原样转发接收到的键盘 stroke。
    pub fn forward_keyboard(&self, device: KeyboardDevice, strokes: &[InterceptionKeyStroke]) {
        self.raw.send_keyboard(device, strokes);
    }

    /// 引擎转发用：原样转发接收到的鼠标 stroke。
    pub fn forward_mouse(&self, device: MouseDevice, strokes: &[InterceptionMouseStroke]) {
        self.raw.send_mouse(device, strokes);
    }
}
