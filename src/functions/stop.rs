//! 停止退出 — 设置 Engine `stop_flag`，触发程序退出。
//! Once 模式，单次执行。

use crate::engine::bindings::KeyFunction;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 停止退出功能 — Once 模式。
///
/// 按下绑定键时设置 `stop_flag` 为 `true`，Engine 检测到后退出事件循环并终止进程。
pub struct 停止退出 {
    stop_flag: Arc<AtomicBool>,
}

impl 停止退出 {
    /// 创建 `停止退出` 实例。
    ///
    /// 接收一个由 Engine 持有的 `Arc<AtomicBool>` 作为共享停止标志。
    pub fn new(stop_flag: Arc<AtomicBool>) -> Self {
        Self { stop_flag }
    }
}

impl KeyFunction for 停止退出 {
    /// 执行停止操作：将 `stop_flag` 写入 `true`。
    ///
    /// 使用 `Release` 排序保证 Engine 主循环的 `Acquire` 读能及时可见。
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        self.stop_flag.store(true, Ordering::Release);
    }
}
