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
use std::sync::mpsc::{self, Receiver, Sender};
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
                // L12: 功能线程固定到输入处理核心（14,15）— 进程掩码较宽
                // （GUI 版 12-15）时新线程继承进程掩码，需显式收窄。
                // 失败不致命 — 仅降低时序隔离性。
                let _ = crate::utils::affinity::pin_current_thread(
                    crate::utils::affinity::ENGINE_CORES_MASK,
                );
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
                // L12: 功能线程固定到输入处理核心（14,15），与 GUI 渲染分离
                let _ = crate::utils::affinity::pin_current_thread(
                    crate::utils::affinity::ENGINE_CORES_MASK,
                );
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
    /// GUI 按键捕获：有捕获请求时，下一个按下的键通过此 Sender 发送。
    capture_tx: Mutex<Option<Sender<Key>>>,
    /// 待 join 的已停止线程句柄。
    ///
    /// 停止 Loop/Toggle 时**不在 Engine 线程 join** — 功能线程若处于不可中断
    /// 延时（如优化游戏最长 2s），join 会冻结事件循环，导致按键转发停止、
    /// F12 退出失效。句柄暂存此处，由 GUI 线程通过 [`drain_pending_joins`]
    /// 异步 join。线程仍会自行退出（stop_requested + ActiveGuard 清除 active），
    /// join 只是回收句柄。
    pending_joins: Mutex<Vec<JoinHandle<()>>>,
}

impl KeyBindings {
    /// 创建空的按键绑定管理器 (Create an empty binding registry).
    pub fn new() -> Self {
        Self {
            bindings: Mutex::new(HashMap::new()),
            keys_held: Mutex::new(HashMap::new()),
            capture_tx: Mutex::new(None),
            pending_joins: Mutex::new(Vec::new()),
        }
    }

    /// join 已结束的线程句柄（应在 GUI/UI 线程调用，锁外 join）。
    /// 未结束的保留在队列中 — `is_finished()` 检查绝不阻塞调用线程，
    /// 每帧调用一次即可回收所有已退出的功能线程。
    /// Join finished thread handles — call from the GUI thread.
    /// Handles whose threads are still running stay queued (never blocks).
    pub fn drain_pending_joins(&self) {
        let mut pending = self.pending_joins.lock().unwrap();
        let mut remaining: Vec<JoinHandle<()>> = Vec::with_capacity(pending.len());
        for h in pending.drain(..) {
            if h.is_finished() {
                let _ = h.join();
            } else {
                remaining.push(h);
            }
        }
        *pending = remaining;
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

    /// 停止所有运行中的绑定并清空注册表。
    /// Stop all running bindings and clear the registry.
    /// 用于 GUI live-apply 时全量替换绑定列表。
    ///
    /// 必须先 drain 再 join：若先 stop_all(join 全部线程) 再 clear()，
    /// join 完成后到 clear 之间的窗口内 Engine 线程可能收到一次 key-down
    /// （此时旧线程已退出、active 已被 ActiveGuard 清除），在旧 Entry 上
    /// spawn 新线程 — 随后 clear() 连同 stop_requested 一起 drop，
    /// 新线程将无人能停止（僵尸 Loop 线程）。
    ///
    /// 句柄不在此同步 join：功能线程可能处于不可中断延时（优化游戏最长
    /// 2s），join 会阻塞调用线程（GUI 帧）。推入 pending 队列，由 GUI
    /// 帧循环通过 [`drain_pending_joins`] 惰性回收（is_finished 检查）。
    /// 线程仍会自行退出（stop_requested + ActiveGuard 清除 active）。
    pub fn clear_all(&self) {
        {
            let mut bindings = self.bindings.lock().unwrap();
            // drain 使 map 立即清空：期间任何 key-down 查表必 miss，无法 spawn
            let handles = bindings
                .drain()
                .filter_map(|(key, mut entry)| entry.signal_stop(key));
            // 锁序：bindings → pending_joins，与 process_key_down 一致，不嵌套反转
            self.pending_joins.lock().unwrap().extend(handles);
        }
        debug!("Cleared all bindings");
    }

    // ── 按键捕获 — Key capture for GUI ────────────────────

    /// 启用按键捕获：下一个按下的键通过返回的 Receiver 发送。
    /// 捕获的键正常转发到系统（Engine 先 forward 再 dispatch），但不会被分发到功能绑定。
    /// Enable key capture: the next key-down is sent through the Receiver.
    pub fn enable_capture(&self) -> Receiver<Key> {
        let (tx, rx) = mpsc::channel();
        *self.capture_tx.lock().unwrap() = Some(tx);
        rx
    }

    /// 取消按键捕获。
    pub fn disable_capture(&self) {
        *self.capture_tx.lock().unwrap() = None;
    }

    // ── 按下处理 — Key-down dispatch ──────────────────────

    /// 处理按键按下事件：防抖后根据 TriggerMode 启动或切换功能。
    /// Handle key-down: debounce, then start or toggle the bound function.
    pub fn process_key_down(&self, key: Key) {
        // 按键捕获模式：拦截并发送给 GUI，不分发到功能绑定。
        // Key capture mode: forward to GUI via channel, skip normal dispatch.
        if let Some(tx) = self.capture_tx.lock().unwrap().take() {
            let _ = tx.send(key);
            // 捕获分支也必须记录 held：capture_tx 已被 take 一次性取走，
            // 若按住不放，auto-repeat 的 key-down 会走正常分发路径 —
            // 防抖表无记录就会误触发刚绑定的功能（如 F12 停止退出）。
            // 记录后 repeat 被防抖吞掉，key-up 时正常清除。
            self.keys_held.lock().unwrap().insert(key, true);
            return;
        }

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
        } // Mutex 已释放 — 交给 pending 队列异步 join
        if let Some(h) = deferred {
            // 不在 Engine 线程 join：功能线程的不可中断延时可能长达 2s
            // （优化游戏），阻塞 join 会冻结事件循环 — 按键转发停止、F12 失效。
            // 句柄入队，由 GUI 线程 drain_pending_joins 回收。
            self.pending_joins.lock().unwrap().push(h);
            debug!("Stopped {:?} (deferred join)", key);
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
        } // Mutex 已释放 — 交给 pending 队列异步 join
        if let Some(h) = deferred {
            self.pending_joins.lock().unwrap().push(h);
            debug!("Stopped {:?} (deferred join)", key);
        }
    }

    // ── 全部停止 — Shutdown all ─────────────────────────────

    /// 停止所有正在运行的绑定，等待所有线程结束。
    /// Stop all running bindings and join all threads.
    pub fn stop_all(&self) {
        let mut handles: Vec<JoinHandle<()>>;
        {
            let mut bindings = self.bindings.lock().unwrap();
            handles = bindings
                .iter_mut()
                .filter_map(|(key, entry)| entry.signal_stop(*key))
                .collect();
        }
        // 顺带回收 pending 队列中的已停止线程（两段锁依次获取，不嵌套）
        {
            let mut pending = self.pending_joins.lock().unwrap();
            handles.extend(pending.drain(..));
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
