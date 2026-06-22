//! PS/2 Scan Code Set 1 — newtype wrapper with associated constants.
//!
//! ## Background
//!
//! Modern keyboards transmit **Set 2**, but the PC's 8042 controller translates
//! to **Set 1** before the OS sees it. All Windows APIs (including Interception)
//! return Set 1 codes.
//!
//! ## Duplicate values
//!
//! Some physical keys share the same scan code value and are disambiguated by
//! the E0/E1 flags in the key state. For example:
//!
//! | Code | Without E0 | With E0 |
//! |------|------------|---------|
//! | 0x38 | LAlt       | RAlt    |
//! | 0x1D | LCtrl      | RCtrl   |
//! | 0x47 | Numpad7    | Home    |
//!
//! This module provides multiple named constants for the same raw value where
//! appropriate (e.g. `LALT`, `RALT` are both `ScanCode(0x38)`).

#![allow(dead_code)]

// ═══════════════════════════════════════════════════════════════
// Newtype
// ═══════════════════════════════════════════════════════════════

/// A PS/2 Scan Code Set 1 value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ScanCode(pub u16);

impl ScanCode {
    /// The raw u16 scan code value for FFI / Interception.
    #[inline(always)]
    pub fn raw(self) -> u16 {
        self.0
    }

    /// Human-readable name (best-effort — see module docs for shared codes).
    pub fn name(self) -> &'static str {
        // Use raw literals here because Rust doesn't allow `CONST.field` in
        // match patterns. The type-safe `ScanCode::*` constants are for
        // external use; this match only needs to cover each unique u16 value.
        match self.0 {
            0x01 => "Esc",
            0x3B => "F1",
            0x3C => "F2",
            0x3D => "F3",
            0x3E => "F4",
            0x3F => "F5",
            0x40 => "F6",
            0x41 => "F7",
            0x42 => "F8",
            0x43 => "F9",
            0x44 => "F10",
            0x57 => "F11",
            0x58 => "F12",
            0x64 => "F13",
            0x65 => "F14",
            0x66 => "F15",
            0x67 => "F16",
            0x68 => "F17",
            0x69 => "F18",
            0x6A => "F19",
            0x6B => "F20",
            0x6C => "F21",
            0x6D => "F22",
            0x6E => "F23",
            0x6F => "F24",
            0x37 => "PrintScreen",
            0x46 => "ScrollLock",

            // Row 2
            0x29 => "`",
            0x02 => "1",
            0x03 => "2",
            0x04 => "3",
            0x05 => "4",
            0x06 => "5",
            0x07 => "6",
            0x08 => "7",
            0x09 => "8",
            0x0A => "9",
            0x0B => "0",
            0x0C => "-",
            0x0D => "=",
            0x0E => "Backspace",

            // Row 3
            0x0F => "Tab",
            0x10 => "Q",
            0x11 => "W",
            0x12 => "E",
            0x13 => "R",
            0x14 => "T",
            0x15 => "Y",
            0x16 => "U",
            0x17 => "I",
            0x18 => "O",
            0x19 => "P",
            0x1A => "[",
            0x1B => "]",
            0x2B => "\\",

            // Row 4
            0x3A => "CapsLock",
            0x1E => "A",
            0x1F => "S",
            0x20 => "D",
            0x21 => "F",
            0x22 => "G",
            0x23 => "H",
            0x24 => "J",
            0x25 => "K",
            0x26 => "L",
            0x27 => ";",
            0x28 => "'",
            0x1C => "Enter",

            // Row 5
            0x2A => "LShift",
            0x2C => "Z",
            0x2D => "X",
            0x2E => "C",
            0x2F => "V",
            0x30 => "B",
            0x31 => "N",
            0x32 => "M",
            0x33 => ",",
            0x34 => ".",
            0x35 => "/",
            0x36 => "RShift",

            // Row 6
            0x1D => "Ctrl",
            0x5B => "LWin",
            0x38 => "Alt",
            0x39 => "Space",
            0x5C => "RWin",
            0x5D => "Apps",

            // Locks (0x45 = NumLock = Pause; Pause uses E1 prefix)
            0x45 => "NumLock",
            0x54 => "SysRq",

            // Numpad
            0x52 => "Numpad0",
            0x4F => "Numpad1",
            0x50 => "Numpad2",
            0x51 => "Numpad3",
            0x4B => "Numpad4",
            0x4C => "Numpad5",
            0x4D => "Numpad6",
            0x47 => "Numpad7",
            0x48 => "Numpad8",
            0x49 => "Numpad9",
            0x4E => "Numpad+",
            0x4A => "Numpad-",
            0x53 => "Numpad.",

            // ACPI
            0x5E => "Power",
            0x5F => "Sleep",
            0x63 => "Wake",
            0x56 => "Oem5",

            _ => "-",
        }
    }
}

// ── Conversions ───────────────────────────────────────────────

impl From<ScanCode> for u16 {
    fn from(sc: ScanCode) -> u16 {
        sc.0
    }
}

impl From<u16> for ScanCode {
    fn from(raw: u16) -> ScanCode {
        ScanCode(raw)
    }
}

impl std::fmt::Display for ScanCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::fmt::Debug for ScanCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ScanCode({:#04X}, \"{}\")", self.0, self.name())
    }
}

// ═══════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════

impl ScanCode {
    // ── Row 1 ──────────────────────────────────────────────
    pub const ESCAPE: Self = Self(0x01);
    pub const F1: Self = Self(0x3B);
    pub const F2: Self = Self(0x3C);
    pub const F3: Self = Self(0x3D);
    pub const F4: Self = Self(0x3E);
    pub const F5: Self = Self(0x3F);
    pub const F6: Self = Self(0x40);
    pub const F7: Self = Self(0x41);
    pub const F8: Self = Self(0x42);
    pub const F9: Self = Self(0x43);
    pub const F10: Self = Self(0x44);
    pub const F11: Self = Self(0x57);
    pub const F12: Self = Self(0x58);
    pub const F13: Self = Self(0x64);
    pub const F14: Self = Self(0x65);
    pub const F15: Self = Self(0x66);
    pub const F16: Self = Self(0x67);
    pub const F17: Self = Self(0x68);
    pub const F18: Self = Self(0x69);
    pub const F19: Self = Self(0x6A);
    pub const F20: Self = Self(0x6B);
    pub const F21: Self = Self(0x6C);
    pub const F22: Self = Self(0x6D);
    pub const F23: Self = Self(0x6E);
    pub const F24: Self = Self(0x6F);
    pub const PRINT_SCREEN: Self = Self(0x37); // E0.2A E0.37
    pub const SCROLL_LOCK: Self = Self(0x46);
    pub const PAUSE: Self = Self(0x45); // E1.1D.45

    // ── Row 2 ──────────────────────────────────────────────
    pub const GRAVE: Self = Self(0x29); // ` ~
    pub const N1: Self = Self(0x02);
    pub const N2: Self = Self(0x03);
    pub const N3: Self = Self(0x04);
    pub const N4: Self = Self(0x05);
    pub const N5: Self = Self(0x06);
    pub const N6: Self = Self(0x07);
    pub const N7: Self = Self(0x08);
    pub const N8: Self = Self(0x09);
    pub const N9: Self = Self(0x0A);
    pub const N0: Self = Self(0x0B);
    pub const MINUS: Self = Self(0x0C); // - _
    pub const EQUALS: Self = Self(0x0D); // = +
    pub const BACKSPACE: Self = Self(0x0E);

    // ── Row 3 ──────────────────────────────────────────────
    pub const TAB: Self = Self(0x0F);
    pub const Q: Self = Self(0x10);
    pub const W: Self = Self(0x11);
    pub const E: Self = Self(0x12);
    pub const R: Self = Self(0x13);
    pub const T: Self = Self(0x14);
    pub const Y: Self = Self(0x15);
    pub const U: Self = Self(0x16);
    pub const I: Self = Self(0x17);
    pub const O: Self = Self(0x18);
    pub const P: Self = Self(0x19);
    pub const LBRACKET: Self = Self(0x1A); // [ {
    pub const RBRACKET: Self = Self(0x1B); // ] }
    pub const BACKSLASH: Self = Self(0x2B); // \ |

    // ── Row 4 ──────────────────────────────────────────────
    pub const CAPS_LOCK: Self = Self(0x3A);
    pub const A: Self = Self(0x1E);
    pub const S: Self = Self(0x1F);
    pub const D: Self = Self(0x20);
    pub const F: Self = Self(0x21);
    pub const G: Self = Self(0x22);
    pub const H: Self = Self(0x23);
    pub const J: Self = Self(0x24);
    pub const K: Self = Self(0x25);
    pub const L: Self = Self(0x26);
    pub const SEMICOLON: Self = Self(0x27); // ; :
    pub const QUOTE: Self = Self(0x28); // ' "
    pub const ENTER: Self = Self(0x1C);

    // ── Row 5 ──────────────────────────────────────────────
    pub const LSHIFT: Self = Self(0x2A);
    pub const Z: Self = Self(0x2C);
    pub const X: Self = Self(0x2D);
    pub const C: Self = Self(0x2E);
    pub const V: Self = Self(0x2F);
    pub const B: Self = Self(0x30);
    pub const N: Self = Self(0x31);
    pub const M: Self = Self(0x32);
    pub const COMMA: Self = Self(0x33); // , <
    pub const PERIOD: Self = Self(0x34); // . >
    pub const SLASH: Self = Self(0x35); // / ?
    pub const RSHIFT: Self = Self(0x36);

    // ── Row 6 ──────────────────────────────────────────────
    pub const CTRL: Self = Self(0x1D); // LCtrl (without E0)
    pub const LWIN: Self = Self(0x5B); // E0.5B
    pub const ALT: Self = Self(0x38); // LAlt (without E0)
    pub const SPACE: Self = Self(0x39);
    pub const RWIN: Self = Self(0x5C); // E0.5C
    pub const APPS: Self = Self(0x5D); // E0.5D (menu)

    // Aliases — same code, different name for RHS modifiers
    pub const RCTRL: Self = Self(0x1D); // LCtrl with E0 flag
    pub const RALT: Self = Self(0x38); // LAlt with E0 flag

    // ── Locks ──────────────────────────────────────────────
    pub const NUM_LOCK: Self = Self(0x45); // also Pause (E1)
    pub const SYS_RQ: Self = Self(0x54);

    // ── Numpad (& nav) ─────────────────────────────────────
    pub const NUMPAD_7: Self = Self(0x47);
    pub const NUMPAD_8: Self = Self(0x48);
    pub const NUMPAD_9: Self = Self(0x49);
    pub const NUMPAD_4: Self = Self(0x4B);
    pub const NUMPAD_5: Self = Self(0x4C);
    pub const NUMPAD_6: Self = Self(0x4D);
    pub const NUMPAD_1: Self = Self(0x4F);
    pub const NUMPAD_2: Self = Self(0x50);
    pub const NUMPAD_3: Self = Self(0x51);
    pub const NUMPAD_0: Self = Self(0x52);
    pub const NUMPAD_ADD: Self = Self(0x4E);
    pub const NUMPAD_SUBTRACT: Self = Self(0x4A);
    pub const NUMPAD_MULTIPLY: Self = Self(0x37); // same as PRINT_SCREEN
    pub const NUMPAD_DIVIDE: Self = Self(0x35); // same as SLASH (has E0)
    pub const NUMPAD_ENTER: Self = Self(0x1C); // same as ENTER (has E0)
    pub const NUMPAD_PERIOD: Self = Self(0x53); // . Del

    // Nav aliases — same codes, distinguished by E0 flag
    pub const HOME: Self = Self(0x47);
    pub const UP: Self = Self(0x48);
    pub const PAGEUP: Self = Self(0x49);
    pub const LEFT: Self = Self(0x4B);
    pub const RIGHT: Self = Self(0x4D);
    pub const END: Self = Self(0x4F);
    pub const DOWN: Self = Self(0x50);
    pub const PAGEDOWN: Self = Self(0x51);
    pub const INSERT: Self = Self(0x52);
    pub const DELETE: Self = Self(0x53);

    // ── 102-key layout ──────────────────────────────────────
    pub const OEM5: Self = Self(0x56); // \ | (between LShift and Z on 102-key)

    // ── Media (E0-prefixed) ─────────────────────────────────
    pub const MEDIA_PREV_TRACK: Self = Self(0x10);
    pub const MEDIA_NEXT_TRACK: Self = Self(0x19);
    pub const MEDIA_MUTE: Self = Self(0x20);
    pub const MEDIA_CALCULATOR: Self = Self(0x21);
    pub const MEDIA_PLAY_PAUSE: Self = Self(0x22);
    pub const MEDIA_STOP: Self = Self(0x24);
    pub const MEDIA_VOLUME_DOWN: Self = Self(0x2E);
    pub const MEDIA_VOLUME_UP: Self = Self(0x30);
    pub const MEDIA_WWW_HOME: Self = Self(0x32);

    // ── ACPI (E0-prefixed) ──────────────────────────────────
    pub const ACPI_POWER: Self = Self(0x5E);
    pub const ACPI_SLEEP: Self = Self(0x5F);
    pub const ACPI_WAKE: Self = Self(0x63);
}

// ═══════════════════════════════════════════════════════════════
// Deprecated — original C++ break-code values (for reference)
// ═══════════════════════════════════════════════════════════════

#[deprecated(note = "Use END (0x4F). 0xCF is the break code.")]
pub const SC_END_BREAK: u16 = 0xCF;

#[deprecated(note = "Use PAGEUP (0x49). 0xC9 is the break code.")]
pub const SC_PAGEUP_BREAK: u16 = 0xC9;

#[deprecated(note = "Use PAGEDOWN (0x51). 0xD1 is the break code.")]
pub const SC_PAGEDOWN_BREAK: u16 = 0xD1;
