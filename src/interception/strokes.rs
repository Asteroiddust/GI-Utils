//! Conversion between typed Interception structs and raw byte buffers.
//!
//! Interception uses a flat byte buffer (`InterceptionStroke = [u8; 20]`)
//! for both keyboard and mouse strokes. The buffer has alignment 1 while
//! the typed structs have alignment 4, so we use unaligned reads/writes.

use super::ffi::*;
use std::mem::size_of;
use std::ptr;

// Compile-time size check.
const _: () = assert!(size_of::<InterceptionKeyStroke>() <= STROKE_SIZE);
const _: () = assert!(size_of::<InterceptionMouseStroke>() <= STROKE_SIZE);

// ── Conversion ────────────────────────────────────────────────

/// Write a keyboard stroke into a raw buffer.
pub fn write_key_stroke(buffer: &mut InterceptionStroke, ks: &InterceptionKeyStroke) {
    let ptr = buffer.as_mut_ptr() as *mut InterceptionKeyStroke;
    unsafe { ptr::write_unaligned(ptr, *ks); }
}

/// Write a mouse stroke into a raw buffer.
pub fn write_mouse_stroke(buffer: &mut InterceptionStroke, ms: &InterceptionMouseStroke) {
    let ptr = buffer.as_mut_ptr() as *mut InterceptionMouseStroke;
    unsafe { ptr::write_unaligned(ptr, *ms); }
}

/// Read a keyboard stroke from a raw buffer.
pub fn read_key_stroke(buffer: &InterceptionStroke) -> InterceptionKeyStroke {
    unsafe { ptr::read_unaligned(buffer.as_ptr() as *const InterceptionKeyStroke) }
}

// ── Constructors ──────────────────────────────────────────────

impl InterceptionKeyStroke {
    pub fn new(code: u16, state: u16) -> Self {
        Self { code, state, information: 0 }
    }
}

impl InterceptionMouseStroke {
    pub fn new(state: u16, flags: u16, rolling: i16, x: i32, y: i32) -> Self {
        Self { state, flags, rolling, x, y, information: 0 }
    }
}
