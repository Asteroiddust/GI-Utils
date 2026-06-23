//! Engine — the main event loop.
//!
//! Creates Interception contexts, sets up filters, and runs the
//! blocking event loop that receives, forwards, and dispatches input events.

pub mod bindings;
pub mod event;
pub mod function;

pub use bindings::TriggerMode;

use bindings::KeyBindings;
use crate::interception::ffi::*;
use crate::interception::{InterceptionContext, SendContext};
use crate::key::Key;
use crate::scan_code::ScanCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The application engine. Separate recv context for receiving,
/// shared send context for forwarding events.
pub struct Engine {
    recv_ctx: InterceptionContext,
    send_ctx: Arc<SendContext>,
    bindings: KeyBindings,
    stop_requested: Arc<AtomicBool>,
    /// When true, prints every keystroke to stdout.
    verbose: bool,
}

impl Engine {
    /// Create a new engine with separate recv/send contexts.
    pub fn new() -> Self {
        Self::with_verbose(false)
    }

    /// Create an engine that prints every keystroke to stdout.
    pub fn verbose() -> Self {
        Self::with_verbose(true)
    }

    fn with_verbose(verbose: bool) -> Self {
        let recv_ctx = InterceptionContext::create();
        let send_ctx = Arc::new(SendContext::create());

        // Set keyboard filter on the receive context: capture all key events
        recv_ctx.set_filter(
            interception_is_keyboard as InterceptionPredicate,
            INTERCEPTION_FILTER_KEY_ALL,
        );

        Self {
            recv_ctx,
            send_ctx,
            bindings: KeyBindings::new(),
            stop_requested: Arc::new(AtomicBool::new(false)),
            verbose,
        }
    }

    /// Get a reference to the key bindings (for registering functions).
    pub fn bindings(&self) -> &KeyBindings {
        &self.bindings
    }

    /// Get the shared send context (for functions that need to send input).
    pub fn send_context(&self) -> Arc<SendContext> {
        self.send_ctx.clone()
    }

    /// Request the engine to stop from another thread.
    pub fn stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
    }

    /// Get a clone of the stop flag for use by the stop function.
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop_requested.clone()
    }

    /// Run the main event loop. Blocks until [`stop`] is called.
    pub fn run(&self) {
        let send_ctx = &self.send_ctx;

        while !self.stop_requested.load(Ordering::Acquire) {
            let device = self.recv_ctx.wait();
            let mut stroke_buf: InterceptionStroke = [0u8; STROKE_SIZE];

            while self.recv_ctx.receive(device, &mut stroke_buf) > 0 {
                // 1. Forward raw event
                send_ctx.send_stroke(device, &stroke_buf);

                // 2. Parse the key stroke
                let ks = crate::interception::strokes::read_key_stroke(&stroke_buf);

                // 3. Parse into a fully disambiguated Key
                let is_e0      = (ks.state & INTERCEPTION_KEY_E0) != 0;
                let is_pressing = (ks.state & INTERCEPTION_KEY_UP) == 0;
                let is_e1      = (ks.state & INTERCEPTION_KEY_E1) != 0;
                let key = Key { code: ScanCode(ks.code), is_e0 };

                // 4. Verbose display
                if self.verbose {
                    print_keystroke(device, key, ks.state, is_e1, ks.information);
                }

                // 5. Dispatch
                if is_pressing {
                    self.bindings.process_key_down(key);
                } else {
                    self.bindings.process_key_up(key);
                }
            }
        }
    }
}

// ── Keystroke display ────────────────────────────────────────

fn print_keystroke(device: i32, key: Key, state: u16, e1: bool, info: u32) {
    let pressing = (state & INTERCEPTION_KEY_UP) == 0;
    let dir = if pressing { "\u{2193}" } else { "\u{2191}" };
    let tags = match (key.is_e0, e1) {
        (false, false) => "",
        (true,  false) => " E0",
        (false, true)  => " E1",
        (true,  true)  => " E0 E1",
    };
    let dev_type = if device <= INTERCEPTION_MAX_KEYBOARD { "KBD" } else { "MSE" };
    println!(
        "[{}] {:<3} #{:<2} {:<16} {:>4}  code={:#04X}  state={:#04X}  info={:#08X}",
        dir, dev_type, device, tags, key.name(), key.code.raw(),
        state, info
    );
}
