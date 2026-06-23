//! Input event types and event sequences.
//!
//! Uses Rust enums instead of the C++ tag+union pattern:
//! no more `is_keyboard` flag + `reinterpret_cast`.
//!
//! ## Philosophy
//!
//! Every primitive action (`press`, `left_down`, `wheel`, …) sends instantly
//! with **no built-in delay**. Timing is controlled by the orthogonal `sleep`
//! primitive, inserted between actions. Common patterns like tap or hold are
//! composed by the caller from these primitives.
//!
//! ```
//! // Tap a key (press, hold 50ms, release)
//! seq.press(ScanCode::F).sleep(50.0).release(ScanCode::F);
//!
//! // Hold a key for 500ms
//! seq.press(ScanCode::W).sleep(500.0).release(ScanCode::W);
//!
//! // Click then scroll
//! seq.left_click().sleep(80.0).wheel(ScrollDir::DOWN).sleep(5.0);
//! ```

use crate::interception::ffi::*;
use crate::key::Key;
use crate::scan_code::ScanCode;

// ═══════════════════════════════════════════════════════════════
// Scroll direction
// ═══════════════════════════════════════════════════════════════

/// One notch of the mouse wheel in Windows (standard WHEEL_DELTA).
pub const WHEEL_DELTA: i16 = 120;

/// Semantic direction constants for `wheel()`.
/// Positive = scroll up/right, negative = scroll down/left.
pub struct ScrollDir;
impl ScrollDir {
    pub const UP: i16 = WHEEL_DELTA;
    pub const DOWN: i16 = -WHEEL_DELTA;
}

// ═══════════════════════════════════════════════════════════════
// InputEvent
// ═══════════════════════════════════════════════════════════════

/// A single input action: keyboard stroke, mouse stroke, or a delay.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    /// Press or release a key.
    Keyboard {
        code: ScanCode,
        state: u16,
    },
    /// Mouse button, wheel move, or cursor move.
    Mouse {
        state: u16,
        flags: u16,
        rolling: i16,
        x: i32,
        y: i32,
    },
    /// Pure delay — pause the sequence for `ms` milliseconds.
    Sleep {
        ms: f64,
    },
}

// ── Constructors ──────────────────────────────────────────────

impl InputEvent {
    pub fn press(key: impl Into<Key>) -> Self {
        let key = key.into();
        InputEvent::Keyboard { code: key.code, state: key.down_state() }
    }

    pub fn release(key: impl Into<Key>) -> Self {
        let key = key.into();
        InputEvent::Keyboard { code: key.code, state: key.up_state() }
    }

    pub fn left_down() -> Self {
        InputEvent::Mouse {
            state: INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN,
            flags: 0, rolling: 0, x: 0, y: 0,
        }
    }

    pub fn left_up() -> Self {
        InputEvent::Mouse {
            state: INTERCEPTION_MOUSE_LEFT_BUTTON_UP,
            flags: 0, rolling: 0, x: 0, y: 0,
        }
    }

    pub fn right_down() -> Self {
        InputEvent::Mouse {
            state: INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN,
            flags: 0, rolling: 0, x: 0, y: 0,
        }
    }

    pub fn right_up() -> Self {
        InputEvent::Mouse {
            state: INTERCEPTION_MOUSE_RIGHT_BUTTON_UP,
            flags: 0, rolling: 0, x: 0, y: 0,
        }
    }

    /// `delta` is one or more WHEEL_DELTA units.
    /// Positive = up/right, negative = down/left.
    pub fn wheel(delta: i16) -> Self {
        InputEvent::Mouse {
            state: INTERCEPTION_MOUSE_WHEEL,
            flags: 0, rolling: delta, x: 0, y: 0,
        }
    }

    pub fn move_relative(dx: i32, dy: i32) -> Self {
        InputEvent::Mouse {
            state: 0,
            flags: INTERCEPTION_MOUSE_MOVE_RELATIVE,
            rolling: 0, x: dx, y: dy,
        }
    }

    pub fn move_absolute(x: i32, y: i32) -> Self {
        InputEvent::Mouse {
            state: 0,
            flags: INTERCEPTION_MOUSE_MOVE_ABSOLUTE,
            rolling: 0, x, y,
        }
    }

    pub fn sleep(ms: f64) -> Self {
        InputEvent::Sleep { ms }
    }

    /// Write this event into a raw Interception stroke buffer.
    /// Sleep events are no-ops (nothing to send).
    pub fn write_to_buffer(&self, buffer: &mut InterceptionStroke) {
        match self {
            InputEvent::Keyboard { code, state } => {
                let ks = InterceptionKeyStroke::new(code.raw(), *state);
                crate::interception::strokes::write_key_stroke(buffer, &ks);
            }
            InputEvent::Mouse { state, flags, rolling, x, y } => {
                let ms = InterceptionMouseStroke::new(*state, *flags, *rolling, *x, *y);
                crate::interception::strokes::write_mouse_stroke(buffer, &ms);
            }
            InputEvent::Sleep { .. } => {}
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// EventSequence
// ═══════════════════════════════════════════════════════════════

/// An ordered sequence of composable input primitives.
///
/// All action methods send instantly with **no built-in delay**.
/// Insert [`sleep`](Self::sleep) between actions to control timing.
///
/// # Example
///
/// ```
/// let mut seq = EventSequence::new();
/// seq.press(ScanCode::F)
///    .sleep(50.0)
///    .release(ScanCode::F)
///    .sleep(30.0)
///    .left_click()
///    .sleep(10.0)
///    .wheel(ScrollDir::DOWN);
/// ```
#[derive(Debug, Clone, Default)]
pub struct EventSequence {
    events: Vec<InputEvent>,
}

impl EventSequence {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    // ── Accessors ──────────────────────────────────────────

    pub fn len(&self) -> usize { self.events.len() }
    pub fn is_empty(&self) -> bool { self.events.is_empty() }
    pub fn events(&self) -> &[InputEvent] { &self.events }
    pub fn clear(&mut self) { self.events.clear(); }

    // ── Raw push ───────────────────────────────────────────

    /// Append any `InputEvent`. Returns `&mut Self` for chaining.
    pub fn push(&mut self, event: InputEvent) -> &mut Self {
        self.events.push(event);
        self
    }

    // ── Keyboard primitives ────────────────────────────────

    /// Press a key down. No delay.
    pub fn press(&mut self, key: impl Into<Key>) -> &mut Self {
        self.push(InputEvent::press(key))
    }

    /// Release a key. No delay.
    pub fn release(&mut self, key: impl Into<Key>) -> &mut Self {
        self.push(InputEvent::release(key))
    }

    /// Hold a key: press → sleep(`duration_ms`) → release.
    pub fn hold(&mut self, key: impl Into<Key>, duration_ms: f64) -> &mut Self {
        let k = key.into();
        self.press(k).sleep(duration_ms).release(k)
    }

    /// Tap a key: press then immediately release (no delay between).
    pub fn tap(&mut self, key: impl Into<Key>) -> &mut Self {
        let k = key.into();
        self.press(k).release(k)
    }

    // ── Mouse button primitives ────────────────────────────

    pub fn left_down(&mut self) -> &mut Self {
        self.push(InputEvent::left_down())
    }

    pub fn left_up(&mut self) -> &mut Self {
        self.push(InputEvent::left_up())
    }

    /// Left click: down then immediately up (no delay between).
    pub fn left_click(&mut self) -> &mut Self {
        self.left_down().left_up()
    }

    /// Hold left button: down → sleep(`duration_ms`) → up.
    pub fn hold_left(&mut self, duration_ms: f64) -> &mut Self {
        self.left_down().sleep(duration_ms).left_up()
    }

    pub fn right_down(&mut self) -> &mut Self {
        self.push(InputEvent::right_down())
    }

    pub fn right_up(&mut self) -> &mut Self {
        self.push(InputEvent::right_up())
    }

    /// Right click: down then immediately up (no delay between).
    pub fn right_click(&mut self) -> &mut Self {
        self.right_down().right_up()
    }

    /// Hold right button: down → sleep(`duration_ms`) → up.
    pub fn hold_right(&mut self, duration_ms: f64) -> &mut Self {
        self.right_down().sleep(duration_ms).right_up()
    }

    // ── Mouse movement primitives ──────────────────────────

    pub fn move_rel(&mut self, dx: i32, dy: i32) -> &mut Self {
        self.push(InputEvent::move_relative(dx, dy))
    }

    pub fn move_abs(&mut self, x: i32, y: i32) -> &mut Self {
        self.push(InputEvent::move_absolute(x, y))
    }

    /// Scroll the mouse wheel once.
    /// Use [`ScrollDir::UP`] / [`ScrollDir::DOWN`] or any multiple of [`WHEEL_DELTA`].
    pub fn wheel(&mut self, delta: i16) -> &mut Self {
        self.push(InputEvent::wheel(delta))
    }

    /// Scroll the mouse wheel by `delta` × `times` notches in a single event.
    ///
    /// Interception sends the accumulated wheel delta directly; no need
    /// for multiple events. `scroll(ScrollDir::DOWN, 7)` equals one event
    /// with `rolling = -840`.
    pub fn scroll(&mut self, delta: i16, times: usize) -> &mut Self {
        self.wheel(delta.saturating_mul(times as i16))
    }

    // ── Timing primitive ───────────────────────────────────

    /// Pause the sequence for `ms` milliseconds.
    pub fn sleep(&mut self, ms: f64) -> &mut Self {
        self.push(InputEvent::sleep(ms))
    }
}
