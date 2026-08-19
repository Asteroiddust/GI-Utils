//! 坐标颜色 — 实时显示光标位置 (Cursor Position) 和像素颜色。
//! Loop 模式，按住时持续显示，松开停止。

use crate::engine::bindings::KeyFunction;
use crate::utils::delay;
use crate::utils::screen::{self, PixelReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

/// 坐标颜色功能 — Loop 模式。
///
/// 按住绑定键时持续获取光标位置 (Cursor Position) 和该像素的 RGB 颜色值并实时显示。
/// 松开即停止输出。
pub struct 坐标颜色 {
    reader: PixelReader,
}

impl 坐标颜色 {
    /// 创建 `坐标颜色` 实例。
    ///
    /// 初始化 `PixelReader`，获取 screen DC (Device Context) 用于像素读取。
    /// 如果获取 DC 失败则 panic。
    pub fn new() -> Self {
        Self {
            reader: PixelReader::new().expect("Failed to acquire screen DC"),
        }
    }
}

impl KeyFunction for 坐标颜色 {
    /// 执行坐标颜色显示循环。
    ///
    /// 每 20ms 获取一次光标位置，调用 `GetPixel` 读取该点 RGB 值，
    /// 经 tracing 输出（汇入 GUI 日志面板）。
    /// 原 `\r` 控制台覆盖式输出不适用于日志通道，改为每次一行。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        while !stop_requested.load(Ordering::Acquire) {
            if let Some(pos) = screen::get_cursor_pos() {
                let cr = unsafe {
                    windows::Win32::Graphics::Gdi::GetPixel(self.reader.raw_dc(), pos.x, pos.y)
                }
                .0;
                if cr != 0xFFFF_FFFF {
                    let color = screen::PixelColor::from_colorref(cr);
                    info!(
                        "x:{:>5}  y:{:>5}  R:{:>3} G:{:>3} B:{:>3}  #{:02X}{:02X}{:02X}  raw:0x{:08X}",
                        pos.x, pos.y, color.r, color.g, color.b, color.r, color.g, color.b, cr
                    );
                }
            }
            delay::delay_ms_interruptible(20.0, &stop_requested);
        }
    }
}
