//! 类型化 Interception 结构体与原始字节缓冲区之间的安全转换
//!
//! Interception 使用扁平字节缓冲区 (`InterceptionStroke = [u8; 20]`)
//! 同时表示键盘和鼠标输入。缓冲区对齐为 1，而类型化结构体对齐为 4，
//! 因此使用非对齐读写 (unaligned read/write)。
//!
//! Conversion between typed Interception structs and raw byte buffers.
//! Uses unaligned reads because the buffer has alignment 1 while the
//! typed structs have alignment 4.

use super::ffi::*;
use std::mem::size_of;
use std::ptr;

// Compile-time size check.
const _: () = assert!(size_of::<InterceptionKeyStroke>() <= STROKE_SIZE);
const _: () = assert!(size_of::<InterceptionMouseStroke>() <= STROKE_SIZE);

// ── Conversion ────────────────────────────────────────────────

/// 将键盘 stroke 写入原始缓冲区。
pub fn write_key_stroke(buffer: &mut InterceptionStroke, ks: &InterceptionKeyStroke) {
    let ptr = buffer.as_mut_ptr() as *mut InterceptionKeyStroke;
    unsafe { ptr::write_unaligned(ptr, *ks); }
}

/// 将鼠标 stroke 写入原始缓冲区。
pub fn write_mouse_stroke(buffer: &mut InterceptionStroke, ms: &InterceptionMouseStroke) {
    let ptr = buffer.as_mut_ptr() as *mut InterceptionMouseStroke;
    unsafe { ptr::write_unaligned(ptr, *ms); }
}

/// 从原始缓冲区读取键盘 stroke。
pub fn read_key_stroke(buffer: &InterceptionStroke) -> InterceptionKeyStroke {
    unsafe { ptr::read_unaligned(buffer.as_ptr() as *const InterceptionKeyStroke) }
}

// ── Constructors ──────────────────────────────────────────────

impl InterceptionKeyStroke {
    /// 构造键盘 stroke，`information` 自动清零。
    pub fn new(code: u16, state: u16) -> Self {
        Self { code, state, information: 0 }
    }
}

impl InterceptionMouseStroke {
    /// 构造鼠标 stroke，`information` 自动清零。
    pub fn new(state: u16, flags: u16, rolling: i16, x: i32, y: i32) -> Self {
        Self { state, flags, rolling, x, y, information: 0 }
    }
}
