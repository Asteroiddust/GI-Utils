//! 物理按键标识符 — PS/2 Scan Code Set 1 + E0 标志
//!
//! 将 PS/2 扫描码和 E0 扩展标志组合为单一 [`Key`] 类型，消除歧义。
//! Combines PS/2 scan code and E0 extension flag into a single unambiguous type.
//!
//! ## 为什么需要这个类型 (Why This Type)
//!
//! PS/2 Set 1 中存在多个按键共享相同扫描码值，仅靠 E0 标志区分：
//!
//! | Code | is_e0=false | is_e0=true |
//! |------|-------------|------------|
//! | 0x38 | Alt         | RAlt       |
//! | 0x1D | Ctrl        | RCtrl      |
//! | 0x47 | Numpad7     | Home       |
//!
//! `Key` 将两个字段捆绑在一起，在注册热键或构造输入事件时不会遗漏 E0 标志。
//! For the raw scan code value without E0 context, see [`ScanCode`].

#![allow(dead_code)]

use crate::interception::ffi;
use crate::scan_code::ScanCode;

// ═══════════════════════════════════════════════════════════════════
// Key
// ═══════════════════════════════════════════════════════════════════

/// 完全消歧的物理按键，包含原始扫描码和 E0 标志。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: ScanCode,
    pub is_e0: bool,
}

impl Key {
    /// 构造按键按下状态值，E0 标志自动设置。
    #[inline(always)]
    pub fn down_state(self) -> u16 {
        if self.is_e0 {
            ffi::INTERCEPTION_KEY_DOWN | ffi::INTERCEPTION_KEY_E0
        } else {
            ffi::INTERCEPTION_KEY_DOWN
        }
    }

    /// 构造按键松开状态值，E0 标志自动设置。
    #[inline(always)]
    pub fn up_state(self) -> u16 {
        if self.is_e0 {
            ffi::INTERCEPTION_KEY_UP | ffi::INTERCEPTION_KEY_E0
        } else {
            ffi::INTERCEPTION_KEY_UP
        }
    }

    /// 返回按键的人类可读名称，考虑 E0 标志。
    pub fn name(self) -> &'static str {
        if self.is_e0 {
            match self.code.0 {
                0x1D => return "RCtrl",
                0x38 => return "RAlt",
                0x47 => return "Home",
                0x48 => return "Up",
                0x49 => return "PageUp",
                0x4B => return "Left",
                0x4D => return "Right",
                0x4F => return "End",
                0x50 => return "Down",
                0x51 => return "PageDown",
                0x52 => return "Insert",
                0x53 => return "Delete",
                0x1C => return "NumpadEnter",
                0x35 => return "NumpadDivide",
                0x37 => return "PrintScreen",
                0x5B => return "LWin",
                0x5C => return "RWin",
                0x5D => return "Apps",
                // Media (E0-only)
                0x10 => return "PrevTrack",
                0x19 => return "NextTrack",
                0x20 => return "Mute",
                0x21 => return "Calculator",
                0x22 => return "PlayPause",
                0x24 => return "Stop",
                0x2E => return "VolumeDown",
                0x30 => return "VolumeUp",
                0x32 => return "WWWHome",
                // ACPI (E0-only)
                0x5E => return "Power",
                0x5F => return "Sleep",
                0x63 => return "Wake",
                _ => {}
            }
        } else {
            // Non-E0 overrides for codes shared with E0 keys
            match self.code.0 {
                0x37 => return "NumpadMultiply",
                0x47 => return "Numpad7",
                0x48 => return "Numpad8",
                0x49 => return "Numpad9",
                0x4B => return "Numpad4",
                0x4D => return "Numpad6",
                0x4F => return "Numpad1",
                0x50 => return "Numpad2",
                0x51 => return "Numpad3",
                0x52 => return "Numpad0",
                0x53 => return "NumpadPeriod",
                0x1C => return "Enter",
                0x35 => return "Slash",
                0x1D => return "Ctrl",
                0x38 => return "Alt",
                0x5B => return "LWin",
                0x5C => return "RWin",
                0x5D => return "Apps",
                _ => {}
            }
        }
        self.code.name()
    }
}

/// 从原始 [`ScanCode`] 转换为 `Key`，E0 标志默认为 `false`。
impl From<ScanCode> for Key {
    fn from(code: ScanCode) -> Self {
        Key { code, is_e0: false }
    }
}

/// 显示按键的人类可读名称（调用 `name()`）。
impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// 调试输出：`Key(0x1D, "RCtrl")`，带原始值和名称。
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

/// 预定义的按键常量，按键盘物理布局分组。
impl Key {
    // ── Row 1 ──────────────────────────────────────────────
    pub const ESCAPE: Self = Self {
        code: ScanCode(0x01),
        is_e0: false,
    };
    pub const F1: Self = Self {
        code: ScanCode(0x3B),
        is_e0: false,
    };
    pub const F2: Self = Self {
        code: ScanCode(0x3C),
        is_e0: false,
    };
    pub const F3: Self = Self {
        code: ScanCode(0x3D),
        is_e0: false,
    };
    pub const F4: Self = Self {
        code: ScanCode(0x3E),
        is_e0: false,
    };
    pub const F5: Self = Self {
        code: ScanCode(0x3F),
        is_e0: false,
    };
    pub const F6: Self = Self {
        code: ScanCode(0x40),
        is_e0: false,
    };
    pub const F7: Self = Self {
        code: ScanCode(0x41),
        is_e0: false,
    };
    pub const F8: Self = Self {
        code: ScanCode(0x42),
        is_e0: false,
    };
    pub const F9: Self = Self {
        code: ScanCode(0x43),
        is_e0: false,
    };
    pub const F10: Self = Self {
        code: ScanCode(0x44),
        is_e0: false,
    };
    pub const F11: Self = Self {
        code: ScanCode(0x57),
        is_e0: false,
    };
    pub const F12: Self = Self {
        code: ScanCode(0x58),
        is_e0: false,
    };
    pub const F13: Self = Self {
        code: ScanCode(0x64),
        is_e0: false,
    };
    pub const F14: Self = Self {
        code: ScanCode(0x65),
        is_e0: false,
    };
    pub const F15: Self = Self {
        code: ScanCode(0x66),
        is_e0: false,
    };
    pub const F16: Self = Self {
        code: ScanCode(0x67),
        is_e0: false,
    };
    pub const F17: Self = Self {
        code: ScanCode(0x68),
        is_e0: false,
    };
    pub const F18: Self = Self {
        code: ScanCode(0x69),
        is_e0: false,
    };
    pub const F19: Self = Self {
        code: ScanCode(0x6A),
        is_e0: false,
    };
    pub const F20: Self = Self {
        code: ScanCode(0x6B),
        is_e0: false,
    };
    pub const F21: Self = Self {
        code: ScanCode(0x6C),
        is_e0: false,
    };
    pub const F22: Self = Self {
        code: ScanCode(0x6D),
        is_e0: false,
    };
    pub const F23: Self = Self {
        code: ScanCode(0x6E),
        is_e0: false,
    };
    pub const F24: Self = Self {
        code: ScanCode(0x6F),
        is_e0: false,
    };
    pub const PRINT_SCREEN: Self = Self {
        code: ScanCode(0x37),
        is_e0: true,
    }; // E0.2A E0.37
    pub const SCROLL_LOCK: Self = Self {
        code: ScanCode(0x46),
        is_e0: false,
    };
    /// 暂停键使用 E1 前缀（E1.1D.45），无法用标准 Key 表示。此常量仅为占位。
    #[deprecated(
        note = "PAUSE uses E1 prefix (E1.1D.45), not representable as Key. Use with E1-aware parsing."
    )]
    pub const PAUSE: Self = Self {
        code: ScanCode(0x45),
        is_e0: false,
    };

    // ── Row 2 ──────────────────────────────────────────────
    pub const GRAVE: Self = Self {
        code: ScanCode(0x29),
        is_e0: false,
    };
    pub const N1: Self = Self {
        code: ScanCode(0x02),
        is_e0: false,
    };
    pub const N2: Self = Self {
        code: ScanCode(0x03),
        is_e0: false,
    };
    pub const N3: Self = Self {
        code: ScanCode(0x04),
        is_e0: false,
    };
    pub const N4: Self = Self {
        code: ScanCode(0x05),
        is_e0: false,
    };
    pub const N5: Self = Self {
        code: ScanCode(0x06),
        is_e0: false,
    };
    pub const N6: Self = Self {
        code: ScanCode(0x07),
        is_e0: false,
    };
    pub const N7: Self = Self {
        code: ScanCode(0x08),
        is_e0: false,
    };
    pub const N8: Self = Self {
        code: ScanCode(0x09),
        is_e0: false,
    };
    pub const N9: Self = Self {
        code: ScanCode(0x0A),
        is_e0: false,
    };
    pub const N0: Self = Self {
        code: ScanCode(0x0B),
        is_e0: false,
    };
    pub const MINUS: Self = Self {
        code: ScanCode(0x0C),
        is_e0: false,
    };
    pub const EQUALS: Self = Self {
        code: ScanCode(0x0D),
        is_e0: false,
    };
    pub const BACKSPACE: Self = Self {
        code: ScanCode(0x0E),
        is_e0: false,
    };

    // ── Row 3 ──────────────────────────────────────────────
    pub const TAB: Self = Self {
        code: ScanCode(0x0F),
        is_e0: false,
    };
    pub const Q: Self = Self {
        code: ScanCode(0x10),
        is_e0: false,
    };
    pub const W: Self = Self {
        code: ScanCode(0x11),
        is_e0: false,
    };
    pub const E: Self = Self {
        code: ScanCode(0x12),
        is_e0: false,
    };
    pub const R: Self = Self {
        code: ScanCode(0x13),
        is_e0: false,
    };
    pub const T: Self = Self {
        code: ScanCode(0x14),
        is_e0: false,
    };
    pub const Y: Self = Self {
        code: ScanCode(0x15),
        is_e0: false,
    };
    pub const U: Self = Self {
        code: ScanCode(0x16),
        is_e0: false,
    };
    pub const I: Self = Self {
        code: ScanCode(0x17),
        is_e0: false,
    };
    pub const O: Self = Self {
        code: ScanCode(0x18),
        is_e0: false,
    };
    pub const P: Self = Self {
        code: ScanCode(0x19),
        is_e0: false,
    };
    pub const LBRACKET: Self = Self {
        code: ScanCode(0x1A),
        is_e0: false,
    };
    pub const RBRACKET: Self = Self {
        code: ScanCode(0x1B),
        is_e0: false,
    };
    pub const BACKSLASH: Self = Self {
        code: ScanCode(0x2B),
        is_e0: false,
    };

    // ── Row 4 ──────────────────────────────────────────────
    pub const CAPS_LOCK: Self = Self {
        code: ScanCode(0x3A),
        is_e0: false,
    };
    pub const A: Self = Self {
        code: ScanCode(0x1E),
        is_e0: false,
    };
    pub const S: Self = Self {
        code: ScanCode(0x1F),
        is_e0: false,
    };
    pub const D: Self = Self {
        code: ScanCode(0x20),
        is_e0: false,
    };
    pub const F: Self = Self {
        code: ScanCode(0x21),
        is_e0: false,
    };
    pub const G: Self = Self {
        code: ScanCode(0x22),
        is_e0: false,
    };
    pub const H: Self = Self {
        code: ScanCode(0x23),
        is_e0: false,
    };
    pub const J: Self = Self {
        code: ScanCode(0x24),
        is_e0: false,
    };
    pub const K: Self = Self {
        code: ScanCode(0x25),
        is_e0: false,
    };
    pub const L: Self = Self {
        code: ScanCode(0x26),
        is_e0: false,
    };
    pub const SEMICOLON: Self = Self {
        code: ScanCode(0x27),
        is_e0: false,
    };
    pub const QUOTE: Self = Self {
        code: ScanCode(0x28),
        is_e0: false,
    };
    pub const ENTER: Self = Self {
        code: ScanCode(0x1C),
        is_e0: false,
    };

    // ── Row 5 ──────────────────────────────────────────────
    pub const LSHIFT: Self = Self {
        code: ScanCode(0x2A),
        is_e0: false,
    };
    pub const Z: Self = Self {
        code: ScanCode(0x2C),
        is_e0: false,
    };
    pub const X: Self = Self {
        code: ScanCode(0x2D),
        is_e0: false,
    };
    pub const C: Self = Self {
        code: ScanCode(0x2E),
        is_e0: false,
    };
    pub const V: Self = Self {
        code: ScanCode(0x2F),
        is_e0: false,
    };
    pub const B: Self = Self {
        code: ScanCode(0x30),
        is_e0: false,
    };
    pub const N: Self = Self {
        code: ScanCode(0x31),
        is_e0: false,
    };
    pub const M: Self = Self {
        code: ScanCode(0x32),
        is_e0: false,
    };
    pub const COMMA: Self = Self {
        code: ScanCode(0x33),
        is_e0: false,
    };
    pub const PERIOD: Self = Self {
        code: ScanCode(0x34),
        is_e0: false,
    };
    pub const SLASH: Self = Self {
        code: ScanCode(0x35),
        is_e0: false,
    };
    pub const RSHIFT: Self = Self {
        code: ScanCode(0x36),
        is_e0: false,
    };

    // ── Row 6 ──────────────────────────────────────────────
    pub const LCTRL: Self = Self {
        code: ScanCode(0x1D),
        is_e0: false,
    };
    pub const LALT: Self = Self {
        code: ScanCode(0x38),
        is_e0: false,
    };
    pub const SPACE: Self = Self {
        code: ScanCode(0x39),
        is_e0: false,
    };

    // E0 variants
    pub const RCTRL: Self = Self {
        code: ScanCode(0x1D),
        is_e0: true,
    };
    pub const RALT: Self = Self {
        code: ScanCode(0x38),
        is_e0: true,
    };
    pub const LWIN: Self = Self {
        code: ScanCode(0x5B),
        is_e0: true,
    };
    pub const RWIN: Self = Self {
        code: ScanCode(0x5C),
        is_e0: true,
    };
    pub const APPS: Self = Self {
        code: ScanCode(0x5D),
        is_e0: true,
    };

    // ── Locks ──────────────────────────────────────────────
    pub const NUM_LOCK: Self = Self {
        code: ScanCode(0x45),
        is_e0: false,
    };
    pub const SYS_RQ: Self = Self {
        code: ScanCode(0x54),
        is_e0: false,
    };

    // ── Numpad ─────────────────────────────────────────────
    pub const NUMPAD_7: Self = Self {
        code: ScanCode(0x47),
        is_e0: false,
    };
    pub const NUMPAD_8: Self = Self {
        code: ScanCode(0x48),
        is_e0: false,
    };
    pub const NUMPAD_9: Self = Self {
        code: ScanCode(0x49),
        is_e0: false,
    };
    pub const NUMPAD_4: Self = Self {
        code: ScanCode(0x4B),
        is_e0: false,
    };
    pub const NUMPAD_5: Self = Self {
        code: ScanCode(0x4C),
        is_e0: false,
    };
    pub const NUMPAD_6: Self = Self {
        code: ScanCode(0x4D),
        is_e0: false,
    };
    pub const NUMPAD_1: Self = Self {
        code: ScanCode(0x4F),
        is_e0: false,
    };
    pub const NUMPAD_2: Self = Self {
        code: ScanCode(0x50),
        is_e0: false,
    };
    pub const NUMPAD_3: Self = Self {
        code: ScanCode(0x51),
        is_e0: false,
    };
    pub const NUMPAD_0: Self = Self {
        code: ScanCode(0x52),
        is_e0: false,
    };
    pub const NUMPAD_ADD: Self = Self {
        code: ScanCode(0x4E),
        is_e0: false,
    };
    pub const NUMPAD_SUBTRACT: Self = Self {
        code: ScanCode(0x4A),
        is_e0: false,
    };
    pub const NUMPAD_MULTIPLY: Self = Self {
        code: ScanCode(0x37),
        is_e0: false,
    };
    pub const NUMPAD_ENTER: Self = Self {
        code: ScanCode(0x1C),
        is_e0: true,
    };
    pub const NUMPAD_DIVIDE: Self = Self {
        code: ScanCode(0x35),
        is_e0: true,
    };
    pub const NUMPAD_PERIOD: Self = Self {
        code: ScanCode(0x53),
        is_e0: false,
    };

    // ── Nav (E0-prefixed) ──────────────────────────────────
    pub const HOME: Self = Self {
        code: ScanCode(0x47),
        is_e0: true,
    };
    pub const UP: Self = Self {
        code: ScanCode(0x48),
        is_e0: true,
    };
    pub const PAGEUP: Self = Self {
        code: ScanCode(0x49),
        is_e0: true,
    };
    pub const LEFT: Self = Self {
        code: ScanCode(0x4B),
        is_e0: true,
    };
    pub const RIGHT: Self = Self {
        code: ScanCode(0x4D),
        is_e0: true,
    };
    pub const END: Self = Self {
        code: ScanCode(0x4F),
        is_e0: true,
    };
    pub const DOWN: Self = Self {
        code: ScanCode(0x50),
        is_e0: true,
    };
    pub const PAGEDOWN: Self = Self {
        code: ScanCode(0x51),
        is_e0: true,
    };
    pub const INSERT: Self = Self {
        code: ScanCode(0x52),
        is_e0: true,
    };
    pub const DELETE: Self = Self {
        code: ScanCode(0x53),
        is_e0: true,
    };

    // ── 102-key layout ──────────────────────────────────────
    pub const OEM5: Self = Self {
        code: ScanCode(0x56),
        is_e0: false,
    };

    // ── Media (E0-prefixed) ─────────────────────────────────
    pub const MEDIA_PREV_TRACK: Self = Self {
        code: ScanCode(0x10),
        is_e0: true,
    };
    pub const MEDIA_NEXT_TRACK: Self = Self {
        code: ScanCode(0x19),
        is_e0: true,
    };
    pub const MEDIA_MUTE: Self = Self {
        code: ScanCode(0x20),
        is_e0: true,
    };
    pub const MEDIA_CALCULATOR: Self = Self {
        code: ScanCode(0x21),
        is_e0: true,
    };
    pub const MEDIA_PLAY_PAUSE: Self = Self {
        code: ScanCode(0x22),
        is_e0: true,
    };
    pub const MEDIA_STOP: Self = Self {
        code: ScanCode(0x24),
        is_e0: true,
    };
    pub const MEDIA_VOLUME_DOWN: Self = Self {
        code: ScanCode(0x2E),
        is_e0: true,
    };
    pub const MEDIA_VOLUME_UP: Self = Self {
        code: ScanCode(0x30),
        is_e0: true,
    };
    pub const MEDIA_WWW_HOME: Self = Self {
        code: ScanCode(0x32),
        is_e0: true,
    };

    // ── ACPI (E0-prefixed) ──────────────────────────────────
    pub const ACPI_POWER: Self = Self {
        code: ScanCode(0x5E),
        is_e0: true,
    };
    pub const ACPI_SLEEP: Self = Self {
        code: ScanCode(0x5F),
        is_e0: true,
    };
    pub const ACPI_WAKE: Self = Self {
        code: ScanCode(0x63),
        is_e0: true,
    };
}
