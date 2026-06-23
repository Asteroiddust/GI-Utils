//! KeyBindings — maps keys to functions and manages their lifecycle.
//!
//! Supports three trigger modes (Logitech G Hub style):
//!
//! | Mode        | Key-down            | Key-up              |
//! |-------------|---------------------|---------------------|
//! | `Once`      | spawn, run to end   | —                   |
//! | `WhileHeld` | spawn loop          | cancel + join       |
//! | `Toggle`    | toggle start / stop | —                   |

use crate::engine::function::KeyFunction;
use crate::key::Key;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use tracing::debug;

// ═══════════════════════════════════════════════════════════════
// Trigger mode
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMode {
    Once,
    WhileHeld,
    Toggle,
}

// ═══════════════════════════════════════════════════════════════
// KeyBindings
// ═══════════════════════════════════════════════════════════════

struct Entry {
    func: Arc<dyn KeyFunction>,
    mode: TriggerMode,
    /// True while any invocation is in progress (all modes).
    active: Arc<AtomicBool>,
    /// For WhileHeld / Toggle: true signals the function to stop.
    stop_requested: Option<Arc<AtomicBool>>,
    /// For WhileHeld / Toggle: thread handle for join-on-stop.
    handle: Option<JoinHandle<()>>,
}

impl Entry {
    fn is_running(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Spawn for WhileHeld / Toggle: looping, cancelable.
    fn spawn_loop(&mut self, key: Key) {
        let func = self.func.clone();
        let stop_requested = Arc::new(AtomicBool::new(false));
        let stop = stop_requested.clone();

        self.active.store(true, Ordering::Release);
        let active = self.active.clone();
        let handle = thread::Builder::new()
            .name(format!("func-{:?}", key))
            .spawn(move || {
                func.execute(stop);
                active.store(false, Ordering::Release);
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
        let active = self.active.clone();
        thread::Builder::new()
            .name(format!("func-{:?}", key))
            .spawn(move || {
                func.execute(Arc::new(AtomicBool::new(false))); // dummy — no loop
                active.store(false, Ordering::Release);
            })
            .expect("Failed to spawn function thread");
        debug!("Started (once) {:?}", key);
    }

    /// Stop for WhileHeld / Toggle: signal + join.
    fn stop_loop(&mut self, key: Key) {
        if let Some(ref stop_requested) = self.stop_requested {
            stop_requested.store(true, Ordering::Release);
        }
        if let Some(handle) = self.handle.take() {
            debug!("Joining {:?}...", key);
            let _ = handle.join();
            debug!("Stopped {:?}", key);
        }
        self.stop_requested = None;
        self.active.store(false, Ordering::Release);
    }
}

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

        let mut bindings = self.bindings.lock().unwrap();
        if let Some(entry) = bindings.get_mut(&key) {
            match entry.mode {
                TriggerMode::Once => {
                    if entry.is_running() {
                        return;
                    }
                    entry.spawn_once(key);
                }
                TriggerMode::WhileHeld => {
                    if entry.is_running() {
                        return;
                    }
                    entry.spawn_loop(key);
                }
                TriggerMode::Toggle => {
                    if entry.is_running() {
                        entry.stop_loop(key);
                    } else {
                        entry.spawn_loop(key);
                    }
                }
            }
        }
    }

    // ── Key-up ──────────────────────────────────────────────

    pub fn process_key_up(&self, key: Key) {
        {
            let mut held = self.keys_held.lock().unwrap();
            held.insert(key, false);
        }

        let mut bindings = self.bindings.lock().unwrap();
        if let Some(entry) = bindings.get_mut(&key) {
            match entry.mode {
                TriggerMode::Once => {}
                TriggerMode::WhileHeld => {
                    entry.stop_loop(key);
                }
                TriggerMode::Toggle => {}
            }
        }
    }

    // ── Shutdown ────────────────────────────────────────────

    pub fn stop_all(&self) {
        let mut bindings = self.bindings.lock().unwrap();
        for (key, entry) in bindings.iter_mut() {
            entry.stop_loop(*key);
        }
    }
}

impl Drop for KeyBindings {
    fn drop(&mut self) {
        self.stop_all();
    }
}
