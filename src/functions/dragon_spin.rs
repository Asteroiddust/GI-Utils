//! 龙王喷水 (Neuvillette spin) — 持续喷射 + 方向实时调整。
//!
//! 用于原神纳维莱特：按住喷水键持续发送共享鼠标相对移动向量（角色旋转喷水），
//! 方向子功能（上/下/左/右/重置）实时修改同一向量，实现喷水中转向。
//!
//! 对应 C++ 原版的 `static InterceptionMouseStroke spin` + 5 个 `friend class`：
//! friend 是 C++ 表达"多个类共享同一私有状态"的妥协 — Rust 用
//! `Arc<Mutex<Spin>>` 共享所有权，方向功能持 clone 直接调方法，无可见性 hack；
//! 顺带消除原版的另一个隐患 — static 向量被喷水线程读、方向线程写，无同步
//! （x86 上碰巧能用，但仍是不定义行为的数据竞争）。

use crate::engine::event::InputEvent;
use crate::engine::function::KeyFunction;
use crate::interception::SendContext;
use crate::utils::{beep, delay};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// 喷射向量 — 共享可变状态（对应 C++ 的 static spin 与四个边界常量）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Spin {
    x: i32,
    y: i32,
}

impl Spin {
    const X_MIN: i32 = -1000;
    const X_MAX: i32 = 1000;
    const Y_MIN: i32 = -100;
    const Y_MAX: i32 = 100;
    /// 初始向量 — 与 C++ 原版一致（右前方微倾）。
    const DEFAULT: Self = Self { x: 100, y: 10 };

    /// 相对调整并钳制边界（方向子功能）。
    fn nudge(&mut self, dx: i32, dy: i32) {
        self.x = (self.x + dx).clamp(Self::X_MIN, Self::X_MAX);
        self.y = (self.y + dy).clamp(Self::Y_MIN, Self::Y_MAX);
    }

    /// 重置回初始向量（重置子功能）。
    fn reset(&mut self) {
        *self = Self::DEFAULT;
    }
}

/// 进程级共享喷射向量 — 6 个功能共享同一实例（对应 C++ static spin）。
/// OnceLock 惰性初始化；模块内唯一入口，所有权共享替代 friend 声明。
fn shared_spin() -> Arc<Mutex<Spin>> {
    static SPIN: OnceLock<Arc<Mutex<Spin>>> = OnceLock::new();
    SPIN.get_or_init(|| Arc::new(Mutex::new(Spin::DEFAULT))).clone()
}

/// 方向微调执行器 — 上/下/左/右 的共享实现
/// （薄包装 C++ 的 `modify_relative`，四个类各自持步长）。
struct NudgeFn {
    spin: Arc<Mutex<Spin>>,
    dx: i32,
    dy: i32,
}

impl NudgeFn {
    fn new(dx: i32, dy: i32) -> Self {
        Self {
            spin: shared_spin(),
            dx,
            dy,
        }
    }

    fn apply(&self) {
        let mut spin = self.spin.lock().unwrap_or_else(|p| p.into_inner());
        spin.nudge(self.dx, self.dy);
        tracing::info!("喷水向量: X = {:>4}, Y = {:>4}", spin.x, spin.y);
    }
}

/// 龙王喷水 (Neuvillette spin) 功能 — Loop 模式。
///
/// 每 1ms 发送一次当前喷射向量（相对鼠标移动），与 C++ 原版一致；
/// 每轮循环从共享状态读取最新向量 — 方向子功能的调整即时生效。
/// 停止响应 ~100μs（`delay_ms_interruptible`；C++ 原版 1ms 不可中断）。
pub struct 龙王喷水 {
    spin: Arc<Mutex<Spin>>,
    send_ctx: Arc<SendContext>,
}

impl 龙王喷水 {
    /// 创建 `龙王喷水` 实例。
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        Self {
            spin: shared_spin(),
            send_ctx,
        }
    }
}

impl KeyFunction for 龙王喷水 {
    /// 持续喷射循环：读共享向量 → 发送相对移动 → 1ms 可中断等待。
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        while !stop_requested.load(Ordering::Acquire) {
            let (x, y) = {
                let spin = self.spin.lock().unwrap_or_else(|p| p.into_inner());
                (spin.x, spin.y)
            };
            self.send_ctx.send_event(&InputEvent::move_relative(x, y));
            delay::delay_ms_interruptible(1.0, &stop_requested);
        }
    }
}

/// 向上微调喷射向量 (Once, C++ `上::execute`: modify_relative(0, -5))。
pub struct 上 {
    inner: NudgeFn,
}

impl 上 {
    /// 创建 `上` 实例。
    pub fn new() -> Self {
        Self {
            inner: NudgeFn::new(0, -5),
        }
    }
}

impl KeyFunction for 上 {
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        self.inner.apply();
    }
}

/// 向下微调喷射向量 (Once, C++ `下::execute`: modify_relative(0, 5))。
pub struct 下 {
    inner: NudgeFn,
}

impl 下 {
    /// 创建 `下` 实例。
    pub fn new() -> Self {
        Self {
            inner: NudgeFn::new(0, 5),
        }
    }
}

impl KeyFunction for 下 {
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        self.inner.apply();
    }
}

/// 向左微调喷射向量 (Once, C++ `左::execute`: modify_relative(-10, 0))。
pub struct 左 {
    inner: NudgeFn,
}

impl 左 {
    /// 创建 `左` 实例。
    pub fn new() -> Self {
        Self {
            inner: NudgeFn::new(-10, 0),
        }
    }
}

impl KeyFunction for 左 {
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        self.inner.apply();
    }
}

/// 向右微调喷射向量 (Once, C++ `右::execute`: modify_relative(10, 0))。
pub struct 右 {
    inner: NudgeFn,
}

impl 右 {
    /// 创建 `右` 实例。
    pub fn new() -> Self {
        Self {
            inner: NudgeFn::new(10, 0),
        }
    }
}

impl KeyFunction for 右 {
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        self.inner.apply();
    }
}

/// 重置喷射向量 (Once, C++ `重置::execute`: modify_absolute(100, 10) + beep)。
pub struct 重置 {
    spin: Arc<Mutex<Spin>>,
}

impl 重置 {
    /// 创建 `重置` 实例。
    pub fn new() -> Self {
        Self {
            spin: shared_spin(),
        }
    }
}

impl KeyFunction for 重置 {
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        let mut spin = self.spin.lock().unwrap_or_else(|p| p.into_inner());
        spin.reset();
        tracing::info!("喷水向量已重置: X = {:>4}, Y = {:>4}", spin.x, spin.y);
        beep::beep_async(750, 300);
    }
}

// ── Tests ─────────────────────────────────────────────────────
// Spin 纯数据逻辑（钳制/重置/共享实例），不触碰驱动。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudge_clamps_to_bounds() {
        let mut spin = Spin::DEFAULT; // (100, 10)
        spin.nudge(0, -1000);
        assert_eq!(spin.y, Spin::Y_MIN, "y = 10-1000 → 钳制到 -100");
        spin.nudge(-2000, 0);
        assert_eq!(spin.x, Spin::X_MIN, "x = 100-2000 → 钳制到 -1000");
        spin.nudge(5000, 5000);
        assert_eq!(spin.x, Spin::X_MAX);
        assert_eq!(spin.y, Spin::Y_MAX);
    }

    #[test]
    fn reset_restores_default() {
        let mut spin = Spin::DEFAULT;
        spin.nudge(10, 10);
        spin.nudge(20, -5);
        assert_ne!(spin, Spin::DEFAULT);
        spin.reset();
        assert_eq!(spin, Spin::DEFAULT);
    }

    #[test]
    fn shared_spin_same_instance() {
        // 6 个功能共享同一状态实例（Arc 共享所有权替代 C++ friend/static）
        let a = shared_spin();
        let b = shared_spin();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
