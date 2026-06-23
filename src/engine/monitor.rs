//! KeyMonitor — the main input event loop.
//!
//! Creates Interception contexts, sets up filters, and runs the
//! blocking event loop that receives, forwards, and dispatches input events.

use crate::engine::bindings::KeyBindings;
use crate::interception::ffi::*;
use crate::interception::InterceptionContext;
use crate::key::Key;
use crate::scan_code::ScanCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The main input monitor. Separate recv context for receiving,
/// shared send context for forwarding events.
pub struct KeyMonitor {
    recv_ctx: InterceptionContext,
    send_ctx: Arc<InterceptionContext>,
    bindings: KeyBindings,
    stop_requested: Arc<AtomicBool>,
    /// When true, prints every keystroke to stdout.
    verbose: bool,
}

impl KeyMonitor {
    /// Create a new monitor with separate recv/send contexts.
    pub fn new() -> Self {
        Self::with_verbose(false)
    }

    /// Create a monitor that prints every keystroke to stdout.
    pub fn verbose() -> Self {
        Self::with_verbose(true)
    }

    fn with_verbose(verbose: bool) -> Self {
        let recv_ctx = InterceptionContext::create();
        let send_ctx = Arc::new(InterceptionContext::create());

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
    pub fn send_context(&self) -> Arc<InterceptionContext> {
        self.send_ctx.clone()
    }

    /// Request the monitor to stop from another thread.
    pub fn stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
    }

    /// Run the main event loop. Blocks until F12 is pressed or [`stop`] is called.
    ///
    /// Consumes the monitor — should only be called once.
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

                // 3. F12 exit
                if ks.code == Key::F12.code.raw() && ks.state == INTERCEPTION_KEY_DOWN {
                    self.stop_requested.store(true, Ordering::Release);
                    break;
                }

                // 4. Parse into a fully disambiguated Key
                let is_e0      = (ks.state & INTERCEPTION_KEY_E0) != 0;
                let is_pressing = (ks.state & INTERCEPTION_KEY_UP) == 0;
                let is_e1      = (ks.state & INTERCEPTION_KEY_E1) != 0;
                let key = Key { code: ScanCode(ks.code), is_e0 };

                // 5. Verbose display
                if self.verbose {
                    print_keystroke(device, key, is_pressing, is_e1, ks.information);
                }

                // 6. Dispatch
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

fn print_keystroke(device: i32, key: Key, pressing: bool, e1: bool, info: u32) {
    let dir = if pressing { "\u{2193}" } else { "\u{2191}" };
    let tags = match (key.is_e0, e1) {
        (false, false) => "",
        (true,  false) => " E0",
        (false, true)  => " E1",
        (true,  true)  => " E0 E1",
    };
    // Determine device type: 1-10 = keyboard, 11-20 = mouse
    let dev_type = if device <= 10 { "KBD" } else { "MSE" };
    println!(
        "[{}] {:<3} #{:<2} {:<16} {:>4}  code={:#04X}  state={:#04X}  info={:#08X}",
        dir, dev_type, device, tags, key.name(), key.code.raw(),
        if pressing { 0x00u16 } else { 0x01u16 }, info
    );
}

