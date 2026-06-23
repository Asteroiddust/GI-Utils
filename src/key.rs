//! Physical key identifier — PS/2 Scan Code Set 1 + E0 flag.
//!
//! ## Why this exists
//!
//! PS/2 Set 1 has duplicate scan code values disambiguated by the E0 flag:
//!
//! | Code | is_e0=false | is_e0=true |
//! |------|-------------|------------|
//! | 0x38 | Alt         | RAlt       |
//! | 0x1D | Ctrl        | RCtrl      |
//! | 0x47 | Numpad7     | Home       |
//!
//! `Key` combines both fields so you can't accidentally forget the E0 flag
//! when registering hotkeys or constructing input events.
//!
//! For the raw scan code value without E0 context, see [`ScanCode`].

#![allow(dead_code)]

use crate::interception::ffi;
use crate::scan_code::ScanCode;

// ═══════════════════════════════════════════════════════════════════
// Key
// ═══════════════════════════════════════════════════════════════════

/// A fully disambiguated physical key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: ScanCode,
    pub is_e0: bool,
}

impl Key {
    /// Build a key-down state value with E0 flag set if needed.
    #[inline(always)]
    pub fn down_state(self) -> u16 {
        if self.is_e0 {
            ffi::INTERCEPTION_KEY_DOWN | ffi::INTERCEPTION_KEY_E0
        } else {
            ffi::INTERCEPTION_KEY_DOWN
        }
    }

    /// Build a key-up state value with E0 flag set if needed.
    #[inline(always)]
    pub fn up_state(self) -> u16 {
        if self.is_e0 {
            ffi::INTERCEPTION_KEY_UP | ffi::INTERCEPTION_KEY_E0
        } else {
            ffi::INTERCEPTION_KEY_UP
        }
    }

    /// Human-readable name, taking E0 flag into account.
    pub fn name(self) -> &'static str {
        if self.is_e0 {
            match self.code.0 {
                0x1D => "RCtrl",
                0x38 => "RAlt",
                0x47 => "Home",
                0x48 => "Up",
                0x49 => "PageUp",
                0x4B => "Left",
                0x4D => "Right",
                0x4F => "End",
                0x50 => "Down",
                0x51 => "PageDown",
                0x52 => "Insert",
                0x53 => "Delete",
                0x1C => "NumpadEnter",
                0x35 => "NumpadDivide",
                0x37 => "PrintScreen",
                0x5B => "LWin",
                0x5C => "RWin",
                0x5D => "Apps",
                // Media (E0-only)
                0x10 => "PrevTrack",
                0x19 => "NextTrack",
                0x20 => "Mute",
                0x21 => "Calculator",
                0x22 => "PlayPause",
                0x24 => "Stop",
                0x2E => "VolumeDown",
                0x30 => "VolumeUp",
                0x32 => "WWWHome",
                // ACPI (E0-only)
                0x5E => "Power",
                0x5F => "Sleep",
                0x63 => "Wake",
                _ => return self.code.name(),
            }
        } else {
            self.code.name()
        }
    }
}

impl From<ScanCode> for Key {
    fn from(code: ScanCode) -> Self {
        Key { code, is_e0: false }
    }
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Key({:#04X}{}, \"{}\")",
            self.code.0,
            if self.is_e0 { " E0" } else { "" },
            self.name()
        )
    }
}

// ═══════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════

impl Key {
    // ── Row 1 ──────────────────────────────────────────────
    pub const ESCAPE: Self = Self { code: ScanCode(0x01), is_e0: false };
    pub const F1: Self = Self { code: ScanCode(0x3B), is_e0: false };
    pub const F2: Self = Self { code: ScanCode(0x3C), is_e0: false };
    pub const F3: Self = Self { code: ScanCode(0x3D), is_e0: false };
    pub const F4: Self = Self { code: ScanCode(0x3E), is_e0: false };
    pub const F5: Self = Self { code: ScanCode(0x3F), is_e0: false };
    pub const F6: Self = Self { code: ScanCode(0x40), is_e0: false };
    pub const F7: Self = Self { code: ScanCode(0x41), is_e0: false };
    pub const F8: Self = Self { code: ScanCode(0x42), is_e0: false };
    pub const F9: Self = Self { code: ScanCode(0x43), is_e0: false };
    pub const F10: Self = Self { code: ScanCode(0x44), is_e0: false };
    pub const F11: Self = Self { code: ScanCode(0x57), is_e0: false };
    pub const F12: Self = Self { code: ScanCode(0x58), is_e0: false };
    pub const F13: Self = Self { code: ScanCode(0x64), is_e0: false };
    pub const F14: Self = Self { code: ScanCode(0x65), is_e0: false };
    pub const F15: Self = Self { code: ScanCode(0x66), is_e0: false };
    pub const F16: Self = Self { code: ScanCode(0x67), is_e0: false };
    pub const F17: Self = Self { code: ScanCode(0x68), is_e0: false };
    pub const F18: Self = Self { code: ScanCode(0x69), is_e0: false };
    pub const F19: Self = Self { code: ScanCode(0x6A), is_e0: false };
    pub const F20: Self = Self { code: ScanCode(0x6B), is_e0: false };
    pub const F21: Self = Self { code: ScanCode(0x6C), is_e0: false };
    pub const F22: Self = Self { code: ScanCode(0x6D), is_e0: false };
    pub const F23: Self = Self { code: ScanCode(0x6E), is_e0: false };
    pub const F24: Self = Self { code: ScanCode(0x6F), is_e0: false };
    pub const PRINT_SCREEN: Self = Self { code: ScanCode(0x37), is_e0: true }; // E0.2A E0.37
    pub const SCROLL_LOCK: Self = Self { code: ScanCode(0x46), is_e0: false };
    pub const PAUSE: Self = Self { code: ScanCode(0x45), is_e0: false }; // E1.1D.45

    // ── Row 2 ──────────────────────────────────────────────
    pub const GRAVE: Self = Self { code: ScanCode(0x29), is_e0: false };
    pub const N1: Self = Self { code: ScanCode(0x02), is_e0: false };
    pub const N2: Self = Self { code: ScanCode(0x03), is_e0: false };
    pub const N3: Self = Self { code: ScanCode(0x04), is_e0: false };
    pub const N4: Self = Self { code: ScanCode(0x05), is_e0: false };
    pub const N5: Self = Self { code: ScanCode(0x06), is_e0: false };
    pub const N6: Self = Self { code: ScanCode(0x07), is_e0: false };
    pub const N7: Self = Self { code: ScanCode(0x08), is_e0: false };
    pub const N8: Self = Self { code: ScanCode(0x09), is_e0: false };
    pub const N9: Self = Self { code: ScanCode(0x0A), is_e0: false };
    pub const N0: Self = Self { code: ScanCode(0x0B), is_e0: false };
    pub const MINUS: Self = Self { code: ScanCode(0x0C), is_e0: false };
    pub const EQUALS: Self = Self { code: ScanCode(0x0D), is_e0: false };
    pub const BACKSPACE: Self = Self { code: ScanCode(0x0E), is_e0: false };

    // ── Row 3 ──────────────────────────────────────────────
    pub const TAB: Self = Self { code: ScanCode(0x0F), is_e0: false };
    pub const Q: Self = Self { code: ScanCode(0x10), is_e0: false };
    pub const W: Self = Self { code: ScanCode(0x11), is_e0: false };
    pub const E: Self = Self { code: ScanCode(0x12), is_e0: false };
    pub const R: Self = Self { code: ScanCode(0x13), is_e0: false };
    pub const T: Self = Self { code: ScanCode(0x14), is_e0: false };
    pub const Y: Self = Self { code: ScanCode(0x15), is_e0: false };
    pub const U: Self = Self { code: ScanCode(0x16), is_e0: false };
    pub const I: Self = Self { code: ScanCode(0x17), is_e0: false };
    pub const O: Self = Self { code: ScanCode(0x18), is_e0: false };
    pub const P: Self = Self { code: ScanCode(0x19), is_e0: false };
    pub const LBRACKET: Self = Self { code: ScanCode(0x1A), is_e0: false };
    pub const RBRACKET: Self = Self { code: ScanCode(0x1B), is_e0: false };
    pub const BACKSLASH: Self = Self { code: ScanCode(0x2B), is_e0: false };

    // ── Row 4 ──────────────────────────────────────────────
    pub const CAPS_LOCK: Self = Self { code: ScanCode(0x3A), is_e0: false };
    pub const A: Self = Self { code: ScanCode(0x1E), is_e0: false };
    pub const S: Self = Self { code: ScanCode(0x1F), is_e0: false };
    pub const D: Self = Self { code: ScanCode(0x20), is_e0: false };
    pub const F: Self = Self { code: ScanCode(0x21), is_e0: false };
    pub const G: Self = Self { code: ScanCode(0x22), is_e0: false };
    pub const H: Self = Self { code: ScanCode(0x23), is_e0: false };
    pub const J: Self = Self { code: ScanCode(0x24), is_e0: false };
    pub const K: Self = Self { code: ScanCode(0x25), is_e0: false };
    pub const L: Self = Self { code: ScanCode(0x26), is_e0: false };
    pub const SEMICOLON: Self = Self { code: ScanCode(0x27), is_e0: false };
    pub const QUOTE: Self = Self { code: ScanCode(0x28), is_e0: false };
    pub const ENTER: Self = Self { code: ScanCode(0x1C), is_e0: false };

    // ── Row 5 ──────────────────────────────────────────────
    pub const LSHIFT: Self = Self { code: ScanCode(0x2A), is_e0: false };
    pub const Z: Self = Self { code: ScanCode(0x2C), is_e0: false };
    pub const X: Self = Self { code: ScanCode(0x2D), is_e0: false };
    pub const C: Self = Self { code: ScanCode(0x2E), is_e0: false };
    pub const V: Self = Self { code: ScanCode(0x2F), is_e0: false };
    pub const B: Self = Self { code: ScanCode(0x30), is_e0: false };
    pub const N: Self = Self { code: ScanCode(0x31), is_e0: false };
    pub const M: Self = Self { code: ScanCode(0x32), is_e0: false };
    pub const COMMA: Self = Self { code: ScanCode(0x33), is_e0: false };
    pub const PERIOD: Self = Self { code: ScanCode(0x34), is_e0: false };
    pub const SLASH: Self = Self { code: ScanCode(0x35), is_e0: false };
    pub const RSHIFT: Self = Self { code: ScanCode(0x36), is_e0: false };

    // ── Row 6 ──────────────────────────────────────────────
    pub const LCTRL: Self = Self { code: ScanCode(0x1D), is_e0: false };
    pub const LALT: Self = Self { code: ScanCode(0x38), is_e0: false };
    pub const SPACE: Self = Self { code: ScanCode(0x39), is_e0: false };

    // E0 variants
    pub const RCTRL: Self = Self { code: ScanCode(0x1D), is_e0: true };
    pub const RALT: Self = Self { code: ScanCode(0x38), is_e0: true };
    pub const LWIN: Self = Self { code: ScanCode(0x5B), is_e0: true };
    pub const RWIN: Self = Self { code: ScanCode(0x5C), is_e0: true };
    pub const APPS: Self = Self { code: ScanCode(0x5D), is_e0: true };

    // ── Locks ──────────────────────────────────────────────
    pub const NUM_LOCK: Self = Self { code: ScanCode(0x45), is_e0: false };
    pub const SYS_RQ: Self = Self { code: ScanCode(0x54), is_e0: false };

    // ── Numpad ─────────────────────────────────────────────
    pub const NUMPAD_7: Self = Self { code: ScanCode(0x47), is_e0: false };
    pub const NUMPAD_8: Self = Self { code: ScanCode(0x48), is_e0: false };
    pub const NUMPAD_9: Self = Self { code: ScanCode(0x49), is_e0: false };
    pub const NUMPAD_4: Self = Self { code: ScanCode(0x4B), is_e0: false };
    pub const NUMPAD_5: Self = Self { code: ScanCode(0x4C), is_e0: false };
    pub const NUMPAD_6: Self = Self { code: ScanCode(0x4D), is_e0: false };
    pub const NUMPAD_1: Self = Self { code: ScanCode(0x4F), is_e0: false };
    pub const NUMPAD_2: Self = Self { code: ScanCode(0x50), is_e0: false };
    pub const NUMPAD_3: Self = Self { code: ScanCode(0x51), is_e0: false };
    pub const NUMPAD_0: Self = Self { code: ScanCode(0x52), is_e0: false };
    pub const NUMPAD_ADD: Self = Self { code: ScanCode(0x4E), is_e0: false };
    pub const NUMPAD_SUBTRACT: Self = Self { code: ScanCode(0x4A), is_e0: false };
    pub const NUMPAD_ENTER: Self = Self { code: ScanCode(0x1C), is_e0: true };
    pub const NUMPAD_DIVIDE: Self = Self { code: ScanCode(0x35), is_e0: true };
    pub const NUMPAD_PERIOD: Self = Self { code: ScanCode(0x53), is_e0: false };

    // ── Nav (E0-prefixed) ──────────────────────────────────
    pub const HOME: Self = Self { code: ScanCode(0x47), is_e0: true };
    pub const UP: Self = Self { code: ScanCode(0x48), is_e0: true };
    pub const PAGEUP: Self = Self { code: ScanCode(0x49), is_e0: true };
    pub const LEFT: Self = Self { code: ScanCode(0x4B), is_e0: true };
    pub const RIGHT: Self = Self { code: ScanCode(0x4D), is_e0: true };
    pub const END: Self = Self { code: ScanCode(0x4F), is_e0: true };
    pub const DOWN: Self = Self { code: ScanCode(0x50), is_e0: true };
    pub const PAGEDOWN: Self = Self { code: ScanCode(0x51), is_e0: true };
    pub const INSERT: Self = Self { code: ScanCode(0x52), is_e0: true };
    pub const DELETE: Self = Self { code: ScanCode(0x53), is_e0: true };

    // ── 102-key layout ──────────────────────────────────────
    pub const OEM5: Self = Self { code: ScanCode(0x56), is_e0: false };

    // ── Media (E0-prefixed) ─────────────────────────────────
    pub const MEDIA_PREV_TRACK: Self = Self { code: ScanCode(0x10), is_e0: true };
    pub const MEDIA_NEXT_TRACK: Self = Self { code: ScanCode(0x19), is_e0: true };
    pub const MEDIA_MUTE: Self = Self { code: ScanCode(0x20), is_e0: true };
    pub const MEDIA_CALCULATOR: Self = Self { code: ScanCode(0x21), is_e0: true };
    pub const MEDIA_PLAY_PAUSE: Self = Self { code: ScanCode(0x22), is_e0: true };
    pub const MEDIA_STOP: Self = Self { code: ScanCode(0x24), is_e0: true };
    pub const MEDIA_VOLUME_DOWN: Self = Self { code: ScanCode(0x2E), is_e0: true };
    pub const MEDIA_VOLUME_UP: Self = Self { code: ScanCode(0x30), is_e0: true };
    pub const MEDIA_WWW_HOME: Self = Self { code: ScanCode(0x32), is_e0: true };

    // ── ACPI (E0-prefixed) ──────────────────────────────────
    pub const ACPI_POWER: Self = Self { code: ScanCode(0x5E), is_e0: true };
    pub const ACPI_SLEEP: Self = Self { code: ScanCode(0x5F), is_e0: true };
    pub const ACPI_WAKE: Self = Self { code: ScanCode(0x63), is_e0: true };
}
