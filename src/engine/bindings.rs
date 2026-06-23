//! KeyBindings — maps keys to functions and manages their lifecycle.
//!
//! Supports three trigger modes (Logitech G Hub style):
//!
//! | Mode     | Key-down            | Key-up              |
//! |----------|---------------------|---------------------|
//! | `Once`   | spawn, run to end   | —                   |
//! | `Loop`   | spawn loop          | cancel + join       |
//! | `Toggle` | toggle start / stop | —                   |

use crate::engine::function::KeyFunction;
use crate::key::Key;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use tracing::debug;

// ═══════════════════════════════════════════════════════════════════
// Trigger mode
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMode {
    Once,
    Loop,
    Toggle,
}

// ═══════════════════════════════════════════════════════════════════
// ActiveGuard — ensures active is cleared even on panic
// ═══════════════════════════════════════════════════════════════════

struct ActiveGuard(Arc<AtomicBool>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

// ═══════════════════════════════════════════════════════════════════
// Entry
// ═══════════════════════════════════════════════════════════════════

struct Entry {
    func: Arc<dyn KeyFunction>,
    mode: TriggerMode,
    /// True while any invocation is in progress (all modes).
    active: Arc<AtomicBool>,
    /// For Loop / Toggle: true signals the function to stop.
    stop_requested: Option<Arc<AtomicBool>>,
    /// For Loop / Toggle: thread handle for join-on-stop.
    handle: Option<JoinHandle<()>>,
}

impl Entry {
    fn is_running(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Spawn for Loop / Toggle: looping, cancelable.
    fn spawn_loop(&mut self, key: Key) {
        let func = self.func.clone();
        let stop_requested = Arc::new(AtomicBool::new(false));
        let stop = stop_requested.clone();

        self.active.store(true, Ordering::Release);
        let guard = ActiveGuard(self.active.clone());
        let handle = thread::Builder::new()
            .name(format!("func-{:?}", key))
            .spawn(move || {
                let _guard = guard; // move into closure, drops on scope exit (incl. panic)
                func.execute(stop);
            })
            .expect("Failed to spawn function thread");

        self.stop_requested = Some(stop_requested);
        self.handle = Some(handle);
        debug!("Started {:?}", key);
    }

    /// Spawn for Once: runs to completion, no cancellation needed.
    fn spawn_once(&mut self, key: Key) {
        let func = self.func.clone();

        self.active.store(true, Ordering::Release);
        let guard = ActiveGuard(self.active.clone());
        thread::Builder::new()
            .name(format!("func-{:?}", key))
            .spawn(move || {
                let _guard = guard;
                func.execute(Arc::new(AtomicBool::new(false)));
            })
            .expect("Failed to spawn function thread");
        debug!("Started (once) {:?}", key);
    }

    /// Signal stop and take the thread handle for deferred join.
    /// The caller must join the returned handle OUTSIDE the mutex lock.
    fn signal_stop(&mut self, key: Key) -> Option<JoinHandle<()>> {
        if let Some(ref stop_requested) = self.stop_requested {
            stop_requested.store(true, Ordering::Release);
        }
        self.stop_requested = None;
        self.active.store(false, Ordering::Release);
        let handle = self.handle.take();
        debug!("Signalled stop for {:?}", key);
        handle
    }
}

// ═══════════════════════════════════════════════════════════════════
// KeyBindings
// ═══════════════════════════════════════════════════════════════════

pub struct KeyBindings {
    bindings: Mutex<HashMap<Key, Entry>>,
    keys_held: Mutex<HashMap<Key, bool>>,
}

impl KeyBindings {
    pub fn new() -> Self {
        Self {
            bindings: Mutex::new(HashMap::new()),
            keys_held: Mutex::new(HashMap::new()),
        }
    }

    /// Register a function with a trigger mode.
    pub fn register(&self, key: Key, mode: TriggerMode, func: Arc<dyn KeyFunction>) {
        let entry = Entry {
            func,
            mode,
            active: Arc::new(AtomicBool::new(false)),
            stop_requested: None,
            handle: None,
        };
        self.bindings.lock().unwrap().insert(key, entry);
        debug!("Registered {:?} ({:?})", key, mode);
    }

    pub fn unregister(&self, key: Key) {
        self.bindings.lock().unwrap().remove(&key);
        debug!("Unregistered {:?}", key);
    }

    // ── Key-down ────────────────────────────────────────────

    pub fn process_key_down(&self, key: Key) {
        // Debounce: ignore auto-repeat while held
        {
            let mut held = self.keys_held.lock().unwrap();
            if held.get(&key).copied().unwrap_or(false) {
                return;
            }
            held.insert(key, true);
        }

        let mut deferred = None;
        {
            let mut bindings = self.bindings.lock().unwrap();
            if let Some(entry) = bindings.get_mut(&key) {
                match entry.mode {
                    TriggerMode::Once => {
                        if entry.is_running() {
                            return;
                        }
                        entry.spawn_once(key);
                    }
                    TriggerMode::Loop => {
                        if entry.is_running() {
                            return;
                        }
                        entry.spawn_loop(key);
                    }
                    TriggerMode::Toggle => {
                        if entry.is_running() {
                            deferred = entry.signal_stop(key);
                        } else {
                            entry.spawn_loop(key);
                        }
                    }
                }
            }
        } // Mutex released — join outside
        if let Some(h) = deferred {
            debug!("Joining {:?}...", key);
            let _ = h.join();
            debug!("Stopped {:?}", key);
        }
    }

    // ── Key-up ──────────────────────────────────────────────

    pub fn process_key_up(&self, key: Key) {
        self.keys_held.lock().unwrap().remove(&key);

        let deferred: Option<JoinHandle<()>>;
        {
            let mut bindings = self.bindings.lock().unwrap();
            if let Some(entry) = bindings.get_mut(&key) {
                match entry.mode {
                    TriggerMode::Loop => {
                        deferred = entry.signal_stop(key);
                    }
                    _ => {
                        deferred = None;
                    }
                }
            } else {
                deferred = None;
            }
        } // Mutex released — join outside
        if let Some(h) = deferred {
            debug!("Joining {:?}...", key);
            let _ = h.join();
            debug!("Stopped {:?}", key);
        }
    }

    // ── Shutdown ────────────────────────────────────────────

    pub fn stop_all(&self) {
        let handles: Vec<JoinHandle<()>>;
        {
            let mut bindings = self.bindings.lock().unwrap();
            handles = bindings
                .iter_mut()
                .filter_map(|(key, entry)| entry.signal_stop(*key))
                .collect();
        } // Mutex released
        for h in handles {
            let _ = h.join();
        }
    }
}

impl Drop for KeyBindings {
    fn drop(&mut self) {
        self.stop_all();
    }
}
