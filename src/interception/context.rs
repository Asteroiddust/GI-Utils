//! RAII wrapper around Interception send/receive contexts.
//!
//! Creating a context allocates driver resources. Dropping frees them.
//!
//! # Thread Safety
//!
//! Per the Interception docs, one context can safely be shared across
//! threads for sending. Each receiving thread should use its own context.

use crate::engine::event::InputEvent;
use crate::interception::ffi;
use crate::interception::ffi::*;

pub struct InterceptionContext {
    raw: ffi::InterceptionContext,
}

impl InterceptionContext {
    pub fn create() -> Self {
        let raw = unsafe { interception_create_context() };
        if raw.is_null() {
            panic!(
                "Failed to create Interception context. \
                 Is the interception driver installed? \
                 (https://github.com/oblitum/Interception)"
            );
        }
        Self { raw }
    }

    /// Raw pointer for FFI calls that don't have a safe wrapper yet.
    pub fn raw(&self) -> ffi::InterceptionContext {
        self.raw
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

    // ── Send ─────────────────────────────────────────────────

    /// Forward a raw stroke to a device (used by the monitor for pass-through).
    pub fn send_stroke(&self, device: ffi::InterceptionDevice, stroke: &InterceptionStroke) {
        unsafe {
            interception_send(self.raw, device, stroke as *const InterceptionStroke, 1);
        }
    }

    /// Send an [`InputEvent`] to a device. Sleep events are no-ops.
    ///
    /// This is the primary send API for function threads. It encapsulates
    /// all `unsafe` FFI — callers don't need to touch raw pointers.
    pub fn send_event(&self, device: ffi::InterceptionDevice, event: &InputEvent) {
        match event {
            InputEvent::Sleep { .. } => {}
            _ => {
                let mut buf: InterceptionStroke = [0u8; STROKE_SIZE];
                event.write_to_buffer(&mut buf);
                unsafe {
                    interception_send(self.raw, device, &buf as *const InterceptionStroke, 1);
                }
            }
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

unsafe impl Send for InterceptionContext {}
unsafe impl Sync for InterceptionContext {}
