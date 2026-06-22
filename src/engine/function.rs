//! KeyFunction trait and trigger modes.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub trait KeyFunction: Send + Sync {
    /// Run until `running` becomes false (set by the manager on key-up / toggle-off).
    ///
    /// The function owns one `Arc<AtomicBool>` clone; the manager holds the other.
    ///
    /// ## G Hub "Sequence" mode (press→seq1, hold→seq2, release→seq3)
    ///
    /// ```ignore
    /// fn execute(&self, running: Arc<AtomicBool>) {
    ///     do_seq1();
    ///     while running.load(Ordering::Acquire) { do_seq2(); }
    ///     do_seq3();
    /// }
    /// ```
    fn execute(&self, running: Arc<AtomicBool>);
}
