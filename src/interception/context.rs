//! RAII wrappers around Interception contexts.
//!
//! Creating a context allocates driver resources. Dropping frees them.
//!
//! # Thread Safety
//!
//! Per the Interception docs, one context can safely be shared across
//! threads for **sending**. Each **receiving** thread should use its own context.
//!
//! [`SendContext`] enforces this at the type level — it only exposes send
//! methods and implements `Send + Sync`.

use crate::engine::event::InputEvent;
use crate::interception::ffi;
use crate::interception::ffi::*;

// ═══════════════════════════════════════════════════════════════════
// Internal helpers
// ═══════════════════════════════════════════════════════════════════

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

/// Receive-only Interception context. Owns the raw driver handle.
///
/// Must be used from a single thread at a time (receiving is not
/// thread-safe). Implements `Send` (safe to move), but NOT `Sync`.
pub struct InterceptionContext {
    raw: ffi::InterceptionContext,
}

impl InterceptionContext {
    pub fn create() -> Self {
        Self { raw: create_raw() }
    }

    // ── Receive ──────────────────────────────────────────────

    /// Block until input arrives. Returns the device with pending data.
    pub fn wait(&self) -> ffi::InterceptionDevice {
        unsafe { interception_wait(self.raw) }
    }

    /// Wait with a timeout. Returns 0 on timeout.
    pub fn wait_timeout(&self, ms: u32) -> ffi::InterceptionDevice {
        unsafe { interception_wait_with_timeout(self.raw, ms) }
    }

    /// Set a device filter.
    pub fn set_filter(&self, predicate: InterceptionPredicate, filter: InterceptionFilter) {
        unsafe { interception_set_filter(self.raw, predicate, filter); }
    }

    /// Receive a raw stroke from a device.
    pub fn receive(
        &self,
        device: ffi::InterceptionDevice,
        stroke: &mut InterceptionStroke,
    ) -> i32 {
        unsafe { interception_receive(self.raw, device, stroke as *mut InterceptionStroke, 1) }
    }

    // ── Send (used for event forwarding from recv thread) ────

    /// Forward a raw stroke to a device.
    /// Must only be called from the same thread as `receive`.
    pub fn send_stroke(&self, device: ffi::InterceptionDevice, stroke: &InterceptionStroke) {
        unsafe {
            interception_send(self.raw, device, stroke as *const InterceptionStroke, 1);
        }
    }
}

impl Drop for InterceptionContext {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { interception_destroy_context(self.raw); }
        }
    }
}

// Safe to move between threads. NOT Sync — receiving is single-thread.
unsafe impl Send for InterceptionContext {}

// ═══════════════════════════════════════════════════════════════════
// SendContext — send-only, thread-safe to share
// ═══════════════════════════════════════════════════════════════════

/// Send-only Interception context. Thread-safe to share across threads.
///
/// Exposes only `send_event()` and `send_stroke()`. The Interception driver
/// explicitly supports concurrent sends through one handle.
pub struct SendContext {
    raw: ffi::InterceptionContext,
}

impl SendContext {
    pub fn create() -> Self {
        Self { raw: create_raw() }
    }

    /// Send an [`InputEvent`], routing to the correct device automatically.
    pub fn send_event(&self, event: &InputEvent) {
        send_event_impl(self.raw, event)
    }

    /// Forward a raw stroke to a specific device.
    pub fn send_stroke(&self, device: ffi::InterceptionDevice, stroke: &InterceptionStroke) {
        unsafe {
            interception_send(self.raw, device, stroke as *const InterceptionStroke, 1);
        }
    }
}

impl Drop for SendContext {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { interception_destroy_context(self.raw); }
        }
    }
}

// Safe: only send methods are exposed, driver supports concurrent sends.
unsafe impl Send for SendContext {}
unsafe impl Sync for SendContext {}
