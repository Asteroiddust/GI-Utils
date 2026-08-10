//! KeyBindings — 按键到函数的映射与生命周期管理。
//!
//! 支持三种触发模式（Logitech G Hub 风格）：
//! Supports three trigger modes (Logitech G Hub style):
//!
//! | Mode     | 按下 (Key-down)     | 释放 (Key-up)       |
//! |----------|---------------------|---------------------|
//! | `Once`   | spawn，运行至结束   | —                   |
//! | `Loop`   | spawn 循环          | cancel + join       |
//! | `Toggle` | 切换 启动 / 停止    | —                   |

use crate::engine::function::KeyFunction;
use crate::key::Key;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use tracing::debug;

// ═══════════════════════════════════════════════════════════════════
// 触发模式 — TriggerMode
// ═══════════════════════════════════════════════════════════════════

/// 功能触发模式 (Trigger mode for bound functions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMode {
    /// 按下时启动，运行至结束自动停止 (Fire-and-forget on key-down).
    Once,
    /// 按下时启动循环，松开时停止 (Loop on key-down, stop on key-up).
    Loop,
    /// 按一次启动，再按一次停止 (Toggle start / stop on each key-down).
    Toggle,
}

// ═══════════════════════════════════════════════════════════════════
// ActiveGuard — 确保 active 状态在 panic 时也被清除
// ═══════════════════════════════════════════════════════════════════

struct ActiveGuard(Arc<AtomicBool>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

// ═══════════════════════════════════════════════════════════════════
// Entry — 绑定条目，包含函数、模式、状态与线程句柄
// ═══════════════════════════════════════════════════════════════════

struct Entry {
    func: Arc<dyn KeyFunction>,
    mode: TriggerMode,
    /// 任何调用进行中为 true (适用于所有模式)。
    active: Arc<AtomicBool>,
    /// Loop / Toggle 模式：true 时通知函数停止。
    stop_requested: Option<Arc<AtomicBool>>,
    /// Loop / Toggle 模式：用于 join 的线程句柄。
    handle: Option<JoinHandle<()>>,
}

impl Entry {
    fn is_running(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// 为 Loop / Toggle 创建线程：可取消的循环。
    /// Spawn for Loop / Toggle: looping, cancelable.
    fn spawn_loop(&mut self, key: Key) {
        let func = self.func.clone();
        let stop_requested = Arc::new(AtomicBool::new(false));
        let stop = stop_requested.clone();

        self.active.store(true, Ordering::Release);
        let guard = ActiveGuard(self.active.clone());
        let result = thread::Builder::new()
            .name(format!("func-{:?}", key))
            .spawn(move || {
                let _guard = guard; // 移入闭包，作用域退出（含 panic）时 drop
                func.execute(stop);
            });

        match result {
            Ok(handle) => {
                self.stop_requested = Some(stop_requested);
                self.handle = Some(handle);
                debug!("Started {:?}", key);
            }
            Err(e) => {
                // guard 在此作用域退出时 drop → active = false
                // 不在持锁状态下 panic，避免 Mutex 毒化
                debug!("Failed to spawn {:?}: {}", key, e);
            }
        }
    }

    /// 为 Once 创建线程：运行至结束，无需取消。
    /// Spawn for Once: runs to completion, no cancellation needed.
    fn spawn_once(&mut self, key: Key) {
        let func = self.func.clone();

        self.active.store(true, Ordering::Release);
        let guard = ActiveGuard(self.active.clone());
        let result = thread::Builder::new()
            .name(format!("func-{:?}", key))
            .spawn(move || {
                let _guard = guard;
                func.execute(Arc::new(AtomicBool::new(false)));
            });

        match result {
            Ok(_) => {
                debug!("Started (once) {:?}", key);
            }
            Err(e) => {
                // guard 在此作用域退出时 drop → active = false
                debug!("Failed to spawn (once) {:?}: {}", key, e);
            }
        }
    }

    /// 发送停止信号，取出线程句柄用于延迟 join。
    /// 调用者必须在 Mutex 锁**外** join 返回的句柄。
    ///
    /// Signal stop and take the thread handle for deferred join.
    /// The caller must join the returned handle OUTSIDE the mutex lock.
    fn signal_stop(&mut self, key: Key) -> Option<JoinHandle<()>> {
        if let Some(ref stop_requested) = self.stop_requested {
            stop_requested.store(true, Ordering::Release);
        }
        self.stop_requested = None;
        // active 由线程内 ActiveGuard::drop 在线程真正退出时清除
        // (不再此处提前清除 — 否则在 join 完成前存在竞态窗口：
        //  另一个 key-down 可看到 active=false 并 spawn 新线程，
        //  导致 handle/stop_requested 被覆盖、旧线程变孤儿)
        // active is cleared by ActiveGuard::drop when the thread
        // actually exits — not here, to avoid a race window between
        // signal_stop and the join.
        let handle = self.handle.take();
        debug!("Signalled stop for {:?}", key);
        handle
    }
}

// ═══════════════════════════════════════════════════════════════════
// KeyBindings — 按键绑定管理器
// ═══════════════════════════════════════════════════════════════════

/// 按键绑定管理器 (Key binding registry and lifecycle manager).
pub struct KeyBindings {
    bindings: Mutex<HashMap<Key, Entry>>,
    keys_held: Mutex<HashMap<Key, bool>>,
}

impl KeyBindings {
    /// 创建空的按键绑定管理器 (Create an empty binding registry).
    pub fn new() -> Self {
        Self {
            bindings: Mutex::new(HashMap::new()),
            keys_held: Mutex::new(HashMap::new()),
        }
    }

    /// 注册按键绑定：将按键与函数和触发模式关联。
    /// Register a function with a trigger mode and a key.
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

    /// 移除指定按键的绑定 (Remove a key binding).
    pub fn unregister(&self, key: Key) {
        self.bindings.lock().unwrap().remove(&key);
        debug!("Unregistered {:?}", key);
    }

    // ── 按下处理 — Key-down dispatch ──────────────────────

    /// 处理按键按下事件：防抖后根据 TriggerMode 启动或切换功能。
    /// Handle key-down: debounce, then start or toggle the bound function.
    pub fn process_key_down(&self, key: Key) {
        // 防抖：按住时忽略 auto-repeat
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
        } // Mutex 已释放 — 在外侧 join
        if let Some(h) = deferred {
            debug!("Joining {:?}...", key);
            let _ = h.join();
            debug!("Stopped {:?}", key);
        }
    }

    // ── 释放处理 — Key-up dispatch ─────────────────────────

    /// 处理按键释放事件：Loop 模式停止，Once/Toggle 无操作。
    /// Handle key-up: stop Loop mode, no-op for Once/Toggle.
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
        } // Mutex 已释放 — 在外侧 join
        if let Some(h) = deferred {
            debug!("Joining {:?}...", key);
            let _ = h.join();
            debug!("Stopped {:?}", key);
        }
    }

    // ── 全部停止 — Shutdown all ─────────────────────────────

    /// 停止所有正在运行的绑定，等待所有线程结束。
    /// Stop all running bindings and join all threads.
    pub fn stop_all(&self) {
        let handles: Vec<JoinHandle<()>>;
        {
            let mut bindings = self.bindings.lock().unwrap();
            handles = bindings
                .iter_mut()
                .filter_map(|(key, entry)| entry.signal_stop(*key))
                .collect();
        } // Mutex 已释放 — 在外侧 join
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
