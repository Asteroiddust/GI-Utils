//! 坐标颜色 — 实时显示光标位置和像素颜色。
//! Loop 模式：按住时持续显示，松开停止。

use crate::engine::function::KeyFunction;
use crate::utils::delay;
use crate::utils::screen::{self, PixelReader};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct 坐标颜色 {
    reader: PixelReader,
}

impl 坐标颜色 {
    pub fn new() -> Self {
        Self {
            reader: PixelReader::new().expect("Failed to acquire screen DC"),
        }
    }
}

impl KeyFunction for 坐标颜色 {
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        while !stop_requested.load(Ordering::Acquire) {
            if let Some(pos) = screen::get_cursor_pos() {
                if let Some(color) = self.reader.read(pos.x, pos.y) {
                    print!(
                        "\r  x:{:>5}  y:{:>5}  R:{:>3} G:{:>3} B:{:>3}  #{:02X}{:02X}{:02X}",
                        pos.x, pos.y, color.r, color.g, color.b, color.r, color.g, color.b
                    );
                    io::stdout().flush().ok();
                }
            }
            delay::delay_ms_interruptible(20.0, &stop_requested);
        }
        println!(); // final newline after \r
    }
}
