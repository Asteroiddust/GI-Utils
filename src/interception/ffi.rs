//! Raw FFI bindings to the Interception driver library.
//!
//! These declarations mirror `interception.h` exactly.
//! All functions in this module are unsafe to call directly.
//! Use the safe wrappers in `super::context` and `super::strokes` instead.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::c_void;

// ── Type aliases ─────────────────────────────────────────────

pub type InterceptionContext = *mut c_void;
pub type InterceptionDevice = i32;
pub type InterceptionPrecedence = i32;
pub type InterceptionFilter = u16;

/// Predicate function pointer type for `interception_set_filter`.
pub type InterceptionPredicate =
    unsafe extern "C" fn(InterceptionDevice) -> i32;

// ── Device index macros (re-exported as const fns) ───────────

pub const INTERCEPTION_MAX_KEYBOARD: i32 = 10;
pub const INTERCEPTION_MAX_MOUSE: i32 = 10;
pub const INTERCEPTION_MAX_DEVICE: i32 =
    INTERCEPTION_MAX_KEYBOARD + INTERCEPTION_MAX_MOUSE;

pub const fn interception_keyboard(index: i32) -> InterceptionDevice {
    index + 1
}

pub const fn interception_mouse(index: i32) -> InterceptionDevice {
    INTERCEPTION_MAX_KEYBOARD + index + 1
}

// ── Key state flags ──────────────────────────────────────────

pub const INTERCEPTION_KEY_DOWN: u16 = 0x00;
pub const INTERCEPTION_KEY_UP: u16 = 0x01;
pub const INTERCEPTION_KEY_E0: u16 = 0x02;
pub const INTERCEPTION_KEY_E1: u16 = 0x04;
pub const INTERCEPTION_KEY_TERMSRV_SET_LED: u16 = 0x08;
pub const INTERCEPTION_KEY_TERMSRV_SHADOW: u16 = 0x10;
pub const INTERCEPTION_KEY_TERMSRV_VKPACKET: u16 = 0x20;

// ── Filter key states ────────────────────────────────────────

pub const INTERCEPTION_FILTER_KEY_NONE: u16 = 0x0000;
pub const INTERCEPTION_FILTER_KEY_ALL: u16 = 0xFFFF;
pub const INTERCEPTION_FILTER_KEY_DOWN: u16 = INTERCEPTION_KEY_UP;
pub const INTERCEPTION_FILTER_KEY_UP: u16 = INTERCEPTION_KEY_UP << 1;
pub const INTERCEPTION_FILTER_KEY_E0: u16 = INTERCEPTION_KEY_E0 << 1;
pub const INTERCEPTION_FILTER_KEY_E1: u16 = INTERCEPTION_KEY_E1 << 1;
pub const INTERCEPTION_FILTER_KEY_TERMSRV_SET_LED: u16 =
    INTERCEPTION_KEY_TERMSRV_SET_LED << 1;
pub const INTERCEPTION_FILTER_KEY_TERMSRV_SHADOW: u16 =
    INTERCEPTION_KEY_TERMSRV_SHADOW << 1;
pub const INTERCEPTION_FILTER_KEY_TERMSRV_VKPACKET: u16 =
    INTERCEPTION_KEY_TERMSRV_VKPACKET << 1;

// ── Mouse state flags ────────────────────────────────────────

pub const INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN: u16 = 0x001;
pub const INTERCEPTION_MOUSE_LEFT_BUTTON_UP: u16 = 0x002;
pub const INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN: u16 = 0x004;
pub const INTERCEPTION_MOUSE_RIGHT_BUTTON_UP: u16 = 0x008;
pub const INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN: u16 = 0x010;
pub const INTERCEPTION_MOUSE_MIDDLE_BUTTON_UP: u16 = 0x020;
pub const INTERCEPTION_MOUSE_BUTTON_4_DOWN: u16 = 0x040;
pub const INTERCEPTION_MOUSE_BUTTON_4_UP: u16 = 0x080;
pub const INTERCEPTION_MOUSE_BUTTON_5_DOWN: u16 = 0x100;
pub const INTERCEPTION_MOUSE_BUTTON_5_UP: u16 = 0x200;
pub const INTERCEPTION_MOUSE_WHEEL: u16 = 0x400;
pub const INTERCEPTION_MOUSE_HWHEEL: u16 = 0x800;

// Aliases
pub const INTERCEPTION_MOUSE_BUTTON_1_DOWN: u16 = INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN;
pub const INTERCEPTION_MOUSE_BUTTON_1_UP:   u16 = INTERCEPTION_MOUSE_LEFT_BUTTON_UP;
pub const INTERCEPTION_MOUSE_BUTTON_2_DOWN: u16 = INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN;
pub const INTERCEPTION_MOUSE_BUTTON_2_UP:   u16 = INTERCEPTION_MOUSE_RIGHT_BUTTON_UP;
pub const INTERCEPTION_MOUSE_BUTTON_3_DOWN: u16 = INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN;
pub const INTERCEPTION_MOUSE_BUTTON_3_UP:   u16 = INTERCEPTION_MOUSE_MIDDLE_BUTTON_UP;

// ── Filter mouse states ──────────────────────────────────────

pub const INTERCEPTION_FILTER_MOUSE_NONE: u16 = 0x0000;
pub const INTERCEPTION_FILTER_MOUSE_ALL: u16 = 0xFFFF;

pub const INTERCEPTION_FILTER_MOUSE_LEFT_BUTTON_DOWN: u16 =
    INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_LEFT_BUTTON_UP: u16 =
    INTERCEPTION_MOUSE_LEFT_BUTTON_UP;
pub const INTERCEPTION_FILTER_MOUSE_RIGHT_BUTTON_DOWN: u16 =
    INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_RIGHT_BUTTON_UP: u16 =
    INTERCEPTION_MOUSE_RIGHT_BUTTON_UP;
pub const INTERCEPTION_FILTER_MOUSE_MIDDLE_BUTTON_DOWN: u16 =
    INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_MIDDLE_BUTTON_UP: u16 =
    INTERCEPTION_MOUSE_MIDDLE_BUTTON_UP;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_1_DOWN: u16 = INTERCEPTION_MOUSE_BUTTON_1_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_1_UP: u16 = INTERCEPTION_MOUSE_BUTTON_1_UP;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_2_DOWN: u16 = INTERCEPTION_MOUSE_BUTTON_2_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_2_UP: u16 = INTERCEPTION_MOUSE_BUTTON_2_UP;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_3_DOWN: u16 = INTERCEPTION_MOUSE_BUTTON_3_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_3_UP: u16 = INTERCEPTION_MOUSE_BUTTON_3_UP;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_4_DOWN: u16 = INTERCEPTION_MOUSE_BUTTON_4_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_4_UP: u16 = INTERCEPTION_MOUSE_BUTTON_4_UP;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_5_DOWN: u16 = INTERCEPTION_MOUSE_BUTTON_5_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_5_UP: u16 = INTERCEPTION_MOUSE_BUTTON_5_UP;
pub const INTERCEPTION_FILTER_MOUSE_WHEEL: u16 = INTERCEPTION_MOUSE_WHEEL;
pub const INTERCEPTION_FILTER_MOUSE_HWHEEL: u16 = INTERCEPTION_MOUSE_HWHEEL;
pub const INTERCEPTION_FILTER_MOUSE_MOVE: u16 = 0x1000;

// ── Mouse flags ──────────────────────────────────────────────

pub const INTERCEPTION_MOUSE_MOVE_RELATIVE: u16 = 0x000;
pub const INTERCEPTION_MOUSE_MOVE_ABSOLUTE: u16 = 0x001;
pub const INTERCEPTION_MOUSE_VIRTUAL_DESKTOP: u16 = 0x002;
pub const INTERCEPTION_MOUSE_ATTRIBUTES_CHANGED: u16 = 0x004;
pub const INTERCEPTION_MOUSE_MOVE_NOCOALESCE: u16 = 0x008;
pub const INTERCEPTION_MOUSE_TERMSRV_SRC_SHADOW: u16 = 0x100;

// ── Stroke structs (repr(C) to match C layout) ───────────────

/// Keyboard stroke sent/received through Interception.
/// Size: 8 bytes, alignment: 4.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InterceptionKeyStroke {
    pub code: u16,
    pub state: u16,
    pub information: u32,
}

/// Mouse stroke sent/received through Interception.
/// Size: 20 bytes, alignment: 4.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InterceptionMouseStroke {
    pub state: u16,
    pub flags: u16,
    pub rolling: i16,
    pub x: i32,
    pub y: i32,
    pub information: u32,
}

/// The raw stroke buffer (matches `char[sizeof(InterceptionMouseStroke)]`).
/// InterceptionKeyStroke fits inside with overflow zeroed.
pub const STROKE_SIZE: usize = std::mem::size_of::<InterceptionMouseStroke>();

/// Opaque stroke buffer — always passed by pointer to Interception functions.
pub type InterceptionStroke = [u8; STROKE_SIZE];

// ── FFI function declarations ────────────────────────────────

extern "C" {
    pub fn interception_create_context() -> InterceptionContext;
    pub fn interception_destroy_context(context: InterceptionContext);
    pub fn interception_set_filter(
        context: InterceptionContext,
        predicate: InterceptionPredicate,
        filter: InterceptionFilter,
    );
    pub fn interception_wait(context: InterceptionContext) -> InterceptionDevice;
    pub fn interception_wait_with_timeout(
        context: InterceptionContext,
        milliseconds: u32,
    ) -> InterceptionDevice;
    pub fn interception_send(
        context: InterceptionContext,
        device: InterceptionDevice,
        stroke: *const InterceptionStroke,
        nstroke: u32,
    ) -> i32;
    pub fn interception_receive(
        context: InterceptionContext,
        device: InterceptionDevice,
        stroke: *mut InterceptionStroke,
        nstroke: u32,
    ) -> i32;
    pub fn interception_is_keyboard(device: InterceptionDevice) -> i32;
    pub fn interception_is_mouse(device: InterceptionDevice) -> i32;
    pub fn interception_is_invalid(device: InterceptionDevice) -> i32;
    pub fn interception_get_hardware_id(
        context: InterceptionContext,
        device: InterceptionDevice,
        hardware_id_buffer: *mut c_void,
        buffer_size: u32,
    ) -> u32;
    pub fn interception_get_precedence(
        context: InterceptionContext,
        device: InterceptionDevice,
    ) -> InterceptionPrecedence;
    pub fn interception_set_precedence(
        context: InterceptionContext,
        device: InterceptionDevice,
        precedence: InterceptionPrecedence,
    );
    pub fn interception_get_filter(
        context: InterceptionContext,
        device: InterceptionDevice,
    ) -> InterceptionFilter;
}
