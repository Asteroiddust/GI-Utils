//! PS/2 Scan Code Set 1 — raw value newtype for FFI use.
//!
//! `ScanCode` is the low-level type for Interception FFI. It carries
//! only the raw scan code value, without E0/E1 disambiguation.
//!
//! For register/event-construction use the high-level [`Key`](crate::key::Key)
//! type which combines ScanCode + E0 flag.

// ── Newtype ─────────────────────────────────────────────────

/// A raw PS/2 Scan Code Set 1 value (no E0/E1 context).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ScanCode(pub u16);

impl ScanCode {
    /// The raw u16 scan code value for FFI / Interception.
    #[inline(always)]
    pub fn raw(self) -> u16 {
        self.0
    }

    /// Human-readable name for this raw scan code.
    /// Does **not** consider E0/E1 flags — duplicate values
    /// will show the non-extended name by default.
    pub fn name(self) -> &'static str {
        match self.0 {
            0x01 => "Esc",
            0x3B => "F1",  0x3C => "F2",  0x3D => "F3",  0x3E => "F4",
            0x3F => "F5",  0x40 => "F6",  0x41 => "F7",  0x42 => "F8",
            0x43 => "F9",  0x44 => "F10", 0x57 => "F11", 0x58 => "F12",
            0x64 => "F13", 0x65 => "F14", 0x66 => "F15", 0x67 => "F16",
            0x68 => "F17", 0x69 => "F18", 0x6A => "F19", 0x6B => "F20",
            0x6C => "F21", 0x6D => "F22", 0x6E => "F23", 0x6F => "F24",
            0x37 => "PrintScreen", 0x46 => "ScrollLock",

            0x29 => "`", 0x02 => "1", 0x03 => "2", 0x04 => "3", 0x05 => "4",
            0x06 => "5", 0x07 => "6", 0x08 => "7", 0x09 => "8", 0x0A => "9",
            0x0B => "0", 0x0C => "-", 0x0D => "=", 0x0E => "Backspace",

            0x0F => "Tab", 0x10 => "Q", 0x11 => "W", 0x12 => "E", 0x13 => "R",
            0x14 => "T", 0x15 => "Y", 0x16 => "U", 0x17 => "I", 0x18 => "O",
            0x19 => "P", 0x1A => "[", 0x1B => "]", 0x2B => "\\",

            0x3A => "CapsLock", 0x1E => "A", 0x1F => "S", 0x20 => "D",
            0x21 => "F", 0x22 => "G", 0x23 => "H", 0x24 => "J", 0x25 => "K",
            0x26 => "L", 0x27 => ";", 0x28 => "'", 0x1C => "Enter",

            0x2A => "LShift", 0x2C => "Z", 0x2D => "X", 0x2E => "C",
            0x2F => "V", 0x30 => "B", 0x31 => "N", 0x32 => "M",
            0x33 => ",", 0x34 => ".", 0x35 => "/", 0x36 => "RShift",

            0x1D => "Ctrl", 0x5B => "LWin", 0x38 => "Alt",
            0x39 => "Space", 0x5C => "RWin", 0x5D => "Apps",

            0x45 => "NumLock", 0x54 => "SysRq",

            0x52 => "Numpad0", 0x4F => "Numpad1", 0x50 => "Numpad2",
            0x51 => "Numpad3", 0x4B => "Numpad4", 0x4C => "Numpad5",
            0x4D => "Numpad6", 0x47 => "Numpad7", 0x48 => "Numpad8",
            0x49 => "Numpad9", 0x4E => "Numpad+", 0x4A => "Numpad-",
            0x53 => "Numpad.",

            0x5E => "Power", 0x5F => "Sleep", 0x63 => "Wake",
            0x56 => "Oem5",

            _ => "-",
        }
    }
}

// ── Conversions ─────────────────────────────────────────────

impl From<ScanCode> for u16 {
    fn from(sc: ScanCode) -> u16 { sc.0 }
}
impl From<u16> for ScanCode {
    fn from(raw: u16) -> ScanCode { ScanCode(raw) }
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
