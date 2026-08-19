//! KeyFunction trait 与触发模式 (KeyFunction trait & trigger modes).
//!
//! 所有自动化功能实现此 trait，通过单个 `execute` 方法驱动。
//! All automation functions implement this trait, driven by a single `execute` method.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub trait KeyFunction: Send + Sync {
    /// 执行功能，直到 `stop_requested` 变为 true（由 manager 在 key-up / toggle-off 时设置）。
    ///
    /// 函数持有一个 `Arc<AtomicBool>` 克隆，manager 持有另一个。
    /// 两者通过 `Ordering::Acquire`/`Release` 原子同步。
    ///
    /// Run until `stop_requested` becomes true (set by the manager on key-up / toggle-off).
    ///
    /// The function owns one `Arc<AtomicBool>` clone; the manager holds the other.
    ///
    /// ## G Hub "Sequence" 模式 (press→seq1, hold→seq2, release→seq3)
    ///
    /// ```ignore
    /// fn execute(&self, stop_requested: Arc<AtomicBool>) {
    ///     do_seq1();
    ///     while !stop_requested.load(Ordering::Acquire) { do_seq2(); }
    ///     do_seq3();
    /// }
    /// ```
    fn execute(&self, stop_requested: Arc<AtomicBool>);
}
