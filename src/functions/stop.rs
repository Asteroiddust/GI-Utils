//! 停止退出 — 设置引擎停止标志，触发程序退出。
//! Once 模式，单次执行。

use crate::engine::function::KeyFunction;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct 停止退出 {
    stop_flag: Arc<AtomicBool>,
}

impl 停止退出 {
    pub fn new(stop_flag: Arc<AtomicBool>) -> Self {
        Self { stop_flag }
    }
}

impl KeyFunction for 停止退出 {
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        self.stop_flag.store(true, Ordering::Release);
    }
}
