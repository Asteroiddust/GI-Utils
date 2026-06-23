//! Interception 上下文的 RAII 封装
//!
//! 创建上下文会分配驱动资源，释放时自动销毁。
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
use crate::interception::ffi;
use crate::interception::ffi::*;

// ═══════════════════════════════════════════════════════════════════
// Internal helpers
// ═══════════════════════════════════════════════════════════════════

/// 创建原始 Interception 上下文，失败时 panic（驱动未安装）。
fn create_raw() -> ffi::InterceptionContext {
    let raw = unsafe { interception_create_context() };
    if raw.is_null() {
        panic!(
            "Failed to create Interception context. \
             Is the interception driver installed? \
             (https://github.com/oblitum/Interception)"
        );
    }
    raw
}

/// 发送事件的内部实现，`Sleep` 事件自动跳过。
fn send_event_impl(raw: ffi::InterceptionContext, event: &InputEvent) {
    match event {
        InputEvent::Sleep { .. } => {}
        _ => {
            let mut buf: InterceptionStroke = [0u8; STROKE_SIZE];
            event.write_to_buffer(&mut buf);
            let device = match event {
                InputEvent::Keyboard { .. } => interception_keyboard(0),
                InputEvent::Mouse { .. } => interception_mouse(0),
                InputEvent::Sleep { .. } => return,
            };
            unsafe {
                interception_send(raw, device, &buf as *const InterceptionStroke, 1);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// InterceptionContext — receive-only, single-thread
// ═══════════════════════════════════════════════════════════════════

/// 只接收的 Interception 上下文，拥有原始驱动句柄。
///
/// 必须在单线程中使用（接收操作非线程安全）。
/// 实现了 `Send`（可跨线程移动），但 **不** 实现 `Sync`。
///
/// Must be used from a single thread at a time (receiving is not
/// thread-safe). Implements `Send` (safe to move), but NOT `Sync`.
pub struct InterceptionContext {
    raw: ffi::InterceptionContext,
}

impl InterceptionContext {
    /// 创建新的 Interception 接收上下文。
    pub fn create() -> Self {
        Self { raw: create_raw() }
    }

    // ── Receive ──────────────────────────────────────────────

    /// 阻塞等待输入到达。返回有待处理数据的设备 ID。
    pub fn wait(&self) -> ffi::InterceptionDevice {
        unsafe { interception_wait(self.raw) }
    }

    /// 带超时的等待（毫秒）。超时时返回 0。
    pub fn wait_timeout(&self, ms: u32) -> ffi::InterceptionDevice {
        unsafe { interception_wait_with_timeout(self.raw, ms) }
    }

    /// 设置设备过滤器。
    pub fn set_filter(&self, predicate: InterceptionPredicate, filter: InterceptionFilter) {
        unsafe {
            interception_set_filter(self.raw, predicate, filter);
        }
    }

    /// 从设备接收一个原始输入数据包。
    pub fn receive(&self, device: ffi::InterceptionDevice, stroke: &mut InterceptionStroke) -> i32 {
        unsafe { interception_receive(self.raw, device, stroke as *mut InterceptionStroke, 1) }
    }

    // ── Send (used for event forwarding from recv thread) ────

    /// 向设备转发一个原始输入数据包。
    ///
    /// **必须**在调用 `receive` 的同一个线程中调用。
    pub fn send_stroke(&self, device: ffi::InterceptionDevice, stroke: &InterceptionStroke) {
        unsafe {
            interception_send(self.raw, device, stroke as *const InterceptionStroke, 1);
        }
    }
}

/// 销毁上下文，释放驱动资源。
impl Drop for InterceptionContext {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                interception_destroy_context(self.raw);
            }
        }
    }
}

/// 安全跨线程移动（接收操作仍需单线程使用）。
unsafe impl Send for InterceptionContext {}

// ═══════════════════════════════════════════════════════════════════
// SendContext — send-only, thread-safe to share
// ═══════════════════════════════════════════════════════════════════

/// 只发送的 Interception 上下文，可在线程间安全共享。
///
/// 仅暴露 `send_event()` 和 `send_stroke()` 方法。
/// Interception 驱动明确支持通过同一个句柄并发发送。
///
/// Thread-safe to share across threads. Exposes only send
/// methods — the driver supports concurrent sends through one handle.
pub struct SendContext {
    raw: ffi::InterceptionContext,
}

impl SendContext {
    /// 创建新的 Interception 发送上下文。
    pub fn create() -> Self {
        Self { raw: create_raw() }
    }

    /// 发送一个 [`InputEvent`]，自动路由到正确的设备。
    pub fn send_event(&self, event: &InputEvent) {
        send_event_impl(self.raw, event)
    }

    /// 向指定设备发送一个原始输入数据包。
    pub fn send_stroke(&self, device: ffi::InterceptionDevice, stroke: &InterceptionStroke) {
        unsafe {
            interception_send(self.raw, device, stroke as *const InterceptionStroke, 1);
        }
    }
}

/// 销毁上下文，释放驱动资源。
impl Drop for SendContext {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                interception_destroy_context(self.raw);
            }
        }
    }
}

/// 安全：仅暴露发送方法，驱动支持并发发送。
unsafe impl Send for SendContext {}
/// 安全：仅暴露发送方法，驱动支持并发发送。
unsafe impl Sync for SendContext {}
