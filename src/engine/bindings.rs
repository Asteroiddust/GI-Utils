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

use crate::key::Key;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use tracing::debug;

// ═══════════════════════════════════════════════════════════════════
// KeyFunction — 功能执行契约（自 engine/function.rs 合并入本模块）
// ═══════════════════════════════════════════════════════════════════

/// 所有自动化功能实现此 trait，通过单个 `execute` 方法驱动。
/// All automation functions implement this trait, driven by a single `execute` method.
pub trait KeyFunction: Send + Sync {
    /// 执行功能，直到 `stop_requested` 变为 true（由 manager 在 key-up /
    /// toggle-off 时设置）。函数持有一个 `Arc<AtomicBool>` 克隆，
    /// manager 持有另一个。
    ///
    /// Run until `stop_requested` becomes true (set by the manager on
    /// key-up / toggle-off).
    fn execute(&self, stop_requested: Arc<AtomicBool>);
}

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
    /// 返回上一轮已结束的 Once 句柄（交调用方入 pending_joins 回收 —
    /// review 3.9：Once 句柄此前被丢弃不可回收）。
    fn spawn_once(&mut self, key: Key) -> Option<JoinHandle<()>> {
        let func = self.func.clone();

        // 上一轮的 Once 线程已结束 → 句柄交给调用方回收；仍在运行 → 保留
        // （Once 有 is_running 守卫，运行中不会走到此处，理论兜底）
        let retired = match self.handle.take() {
            Some(h) if h.is_finished() => Some(h),
            Some(h) => {
                self.handle = Some(h);
                None
            }
            None => None,
        };

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
            Ok(handle) => {
                self.handle = Some(handle);
                debug!("Started (once) {:?}", key);
            }
            Err(e) => {
                // guard 在此作用域退出时 drop → active = false
                debug!("Failed to spawn (once) {:?}: {}", key, e);
            }
        }
        retired
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
    ///
    /// 替换已占用键时先停旧线程并回收句柄 — 直接 insert 覆盖会让旧线程的
    /// stop_requested 引用丢失、无人能停止（review 3.8）。
    pub fn register(&self, key: Key, mode: TriggerMode, func: Arc<dyn KeyFunction>) {
        let entry = Entry {
            func,
            mode,
            active: Arc::new(AtomicBool::new(false)),
            stop_requested: None,
            handle: None,
        };
        let stopped = {
            let mut bindings = self.bindings.lock().unwrap();
            let stopped = bindings
                .get_mut(&key)
                .and_then(|entry| entry.signal_stop(key));
            bindings.insert(key, entry);
            stopped
        };
        // 锁序：bindings → pending_joins，与 clear_all 一致
        if let Some(handle) = stopped {
            self.pending_joins.lock().unwrap().push(handle);
        }
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
        // 防抖表同步清空 — 按住某键期间 live-apply 时，旧 held 记录会吞掉
        // 新绑定下同键的第一次 key-down（review 4.5）
        self.keys_held.lock().unwrap().clear();
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
                        // 上一轮已结束的 Once 句柄 → 入 pending 队列回收
                        let retired = entry.spawn_once(key);
                        if retired.is_some() {
                            deferred = retired;
                        }
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

// ═══════════════════════════════════════════════════════════════════
// 状态机单测 — 纯内存结构，无需驱动。覆盖防抖/触发模式/替换/清理/
// 捕获拦截 — 项目 bug 风险最高的一层（幽灵按键、僵尸线程均出于此）。
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Key;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    /// 测试用功能：`gate` 为 Some 时等 gate（Once 模拟，预置 true 即立即
    /// 完成）；None 时等 stop_requested（Loop 模拟）。
    struct TestFunc {
        started: Arc<AtomicUsize>,
        finished: Arc<AtomicUsize>,
        gate: Option<Arc<AtomicBool>>,
    }

    impl TestFunc {
        fn loop_style(started: Arc<AtomicUsize>, finished: Arc<AtomicUsize>) -> Self {
            Self {
                started,
                finished,
                gate: None,
            }
        }

        fn once_style(
            started: Arc<AtomicUsize>,
            finished: Arc<AtomicUsize>,
            gate: Arc<AtomicBool>,
        ) -> Self {
            Self {
                started,
                finished,
                gate: Some(gate),
            }
        }
    }

    impl KeyFunction for TestFunc {
        fn execute(&self, stop_requested: Arc<AtomicBool>) {
            self.started.fetch_add(1, Ordering::SeqCst);
            let wait_cond = match &self.gate {
                Some(gate) => gate.clone(),
                None => stop_requested.clone(),
            };
            while !wait_cond.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.finished.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn counter() -> Arc<AtomicUsize> {
        Arc::new(AtomicUsize::new(0))
    }

    /// 轮询等待条件成立（上限 2s，测试线程各自 <50ms 完成）。
    fn wait_until(cond: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        cond()
    }

    fn once_gate() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    // ── 防抖 / Loop 生命周期 ────────────────────────────────

    #[test]
    fn debounce_ignores_auto_repeat_and_loop_stops_on_key_up() {
        let bindings = KeyBindings::new();
        let (started, finished) = (counter(), counter());
        bindings.register(
            Key::F1,
            TriggerMode::Loop,
            Arc::new(TestFunc::loop_style(started.clone(), finished.clone())),
        );

        bindings.process_key_down(Key::F1);
        assert!(wait_until(|| started.load(Ordering::SeqCst) == 1));

        // 按住期间 auto-repeat 的 key-down 被防抖吞掉 — 不重复 spawn
        bindings.process_key_down(Key::F1);
        bindings.process_key_down(Key::F1);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(started.load(Ordering::SeqCst), 1);

        // key-up 停止 Loop 线程；句柄入 pending，drain 后回收
        bindings.process_key_up(Key::F1);
        assert!(wait_until(|| finished.load(Ordering::SeqCst) == 1));
        bindings.drain_pending_joins();
        assert!(bindings.pending_joins.lock().unwrap().is_empty());
    }

    // ── Once 生命周期 ───────────────────────────────────────

    #[test]
    fn once_blocks_while_running_and_can_retrigger() {
        let bindings = KeyBindings::new();
        let (started, finished) = (counter(), counter());
        let gate = once_gate();
        bindings.register(
            Key::F1,
            TriggerMode::Once,
            Arc::new(TestFunc::once_style(
                started.clone(),
                finished.clone(),
                gate.clone(),
            )),
        );

        bindings.process_key_down(Key::F1);
        assert!(wait_until(|| started.load(Ordering::SeqCst) == 1));
        bindings.process_key_up(Key::F1); // 真实按键成对：松开后防抖清除

        // 运行中再次按下被 is_running 守卫拦截（防抖已清，走得到分支）
        bindings.process_key_down(Key::F1);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(started.load(Ordering::SeqCst), 1);
        bindings.process_key_up(Key::F1);

        // 放行后完成；可再次触发（退役句柄经 deferred 入 pending）
        gate.store(true, Ordering::SeqCst);
        assert!(wait_until(|| finished.load(Ordering::SeqCst) == 1));

        bindings.process_key_down(Key::F1);
        assert!(wait_until(|| started.load(Ordering::SeqCst) == 2));
        assert!(wait_until(|| finished.load(Ordering::SeqCst) == 2));
        bindings.drain_pending_joins();
        assert!(bindings.pending_joins.lock().unwrap().is_empty());
    }

    // ── Toggle 生命周期 ─────────────────────────────────────

    #[test]
    fn toggle_second_press_stops_and_third_restarts() {
        let bindings = KeyBindings::new();
        let (started, finished) = (counter(), counter());
        bindings.register(
            Key::F2,
            TriggerMode::Toggle,
            Arc::new(TestFunc::loop_style(started.clone(), finished.clone())),
        );

        bindings.process_key_down(Key::F2);
        assert!(wait_until(|| started.load(Ordering::SeqCst) == 1));
        bindings.process_key_up(Key::F2); // 防抖清除（Toggle 每次按需完整 down→up）

        // 第二次按下 → 停止；句柄经 deferred 入 pending
        bindings.process_key_down(Key::F2);
        assert!(wait_until(|| finished.load(Ordering::SeqCst) == 1));
        bindings.process_key_up(Key::F2);

        // 第三次按下 → 重新启动
        bindings.process_key_down(Key::F2);
        assert!(wait_until(|| started.load(Ordering::SeqCst) == 2));

        // 收尾：stop_all 全停 + join（Drop 亦会执行）
        bindings.stop_all();
        assert!(wait_until(|| finished.load(Ordering::SeqCst) == 2));
    }

    // ── 注册替换 / 注销 ─────────────────────────────────────

    #[test]
    fn register_replacement_stops_old_running_thread() {
        let bindings = KeyBindings::new();
        let (started_a, finished_a) = (counter(), counter());
        let (started_b, finished_b) = (counter(), counter());
        bindings.register(
            Key::F1,
            TriggerMode::Loop,
            Arc::new(TestFunc::loop_style(started_a.clone(), finished_a.clone())),
        );
        bindings.process_key_down(Key::F1);
        assert!(wait_until(|| started_a.load(Ordering::SeqCst) == 1));

        // 替换已占用键 → 旧线程被 signal_stop（review 3.8）
        bindings.register(
            Key::F1,
            TriggerMode::Loop,
            Arc::new(TestFunc::loop_style(started_b.clone(), finished_b.clone())),
        );
        assert!(wait_until(|| finished_a.load(Ordering::SeqCst) == 1));
        assert_eq!(started_b.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unregister_removes_binding() {
        let bindings = KeyBindings::new();
        let (started, finished) = (counter(), counter());
        bindings.register(
            Key::F1,
            TriggerMode::Loop,
            Arc::new(TestFunc::loop_style(started.clone(), finished.clone())),
        );
        bindings.unregister(Key::F1);
        bindings.process_key_down(Key::F1);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(started.load(Ordering::SeqCst), 0);
        assert_eq!(finished.load(Ordering::SeqCst), 0);
    }

    // ── clear_all ───────────────────────────────────────────

    #[test]
    fn clear_all_stops_running_and_clears_debounce_table() {
        let bindings = KeyBindings::new();
        let (started, finished) = (counter(), counter());
        bindings.register(
            Key::F1,
            TriggerMode::Loop,
            Arc::new(TestFunc::loop_style(started.clone(), finished.clone())),
        );
        bindings.register(
            Key::F2,
            TriggerMode::Loop,
            Arc::new(TestFunc::loop_style(started.clone(), finished.clone())),
        );
        bindings.process_key_down(Key::F1);
        bindings.process_key_down(Key::F2);
        assert!(wait_until(|| started.load(Ordering::SeqCst) == 2));

        bindings.clear_all();
        assert!(wait_until(|| finished.load(Ordering::SeqCst) == 2));

        // 4.5 回归：clear_all 必须同步清空防抖表 — F1 此前被按下未松开，
        // 重新注册后立即按下若被防抖吞掉即失败
        let (started2, finished2) = (counter(), counter());
        let gate = once_gate();
        gate.store(true, Ordering::SeqCst); // 立即完成
        bindings.register(
            Key::F1,
            TriggerMode::Once,
            Arc::new(TestFunc::once_style(
                started2.clone(),
                finished2.clone(),
                gate,
            )),
        );
        bindings.process_key_down(Key::F1);
        assert!(wait_until(|| started2.load(Ordering::SeqCst) == 1));
        assert!(wait_until(|| finished2.load(Ordering::SeqCst) == 1));
    }

    // ── 按键捕获 ────────────────────────────────────────────

    #[test]
    fn capture_mode_intercepts_and_skips_dispatch() {
        let bindings = KeyBindings::new();
        let (started, finished) = (counter(), counter());
        bindings.register(
            Key::F1,
            TriggerMode::Loop,
            Arc::new(TestFunc::loop_style(started.clone(), finished.clone())),
        );

        let rx = bindings.enable_capture();
        bindings.process_key_down(Key::F1);
        // 捕获分支：键进 channel，不分发到绑定
        assert_eq!(rx.recv_timeout(Duration::from_millis(100)), Ok(Key::F1));
        assert_eq!(started.load(Ordering::SeqCst), 0);

        bindings.disable_capture();
        // 捕获分支也记录了 held — 松开清除后正常分发路径恢复
        bindings.process_key_up(Key::F1);
        bindings.process_key_down(Key::F1);
        assert!(wait_until(|| started.load(Ordering::SeqCst) == 1));
        bindings.process_key_up(Key::F1);
        assert!(wait_until(|| finished.load(Ordering::SeqCst) == 1));
    }
}
