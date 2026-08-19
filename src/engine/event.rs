//! 输入事件类型与事件序列 (Input Events & Event Sequences).
//!
//! 使用 Rust enum 替代 C++ 的 tag + union 模式，消除 `is_keyboard` 标志与 `reinterpret_cast`。
//! Uses Rust enums instead of the C++ tag+union pattern:
//! no more `is_keyboard` flag + `reinterpret_cast`.
//!
//! ## 设计哲学 — Philosophy
//!
//! 每个基本动作 (`press`, `left_down`, `wheel`, ...) 立即发送，**不含内置延迟**。
//! 时序由独立的 `sleep` 原语在动作之间插入控制。
//! 常用模式如 `tap` 或 `hold` 由调用方组合这些原语实现。
//!
//! Every primitive action (`press`, `left_down`, `wheel`, …) sends instantly
//! with **no built-in delay**. Timing is controlled by the orthogonal `sleep`
//! primitive, inserted between actions. Common patterns like tap or hold are
//! composed by the caller from these primitives.
//!
//! ```
//! use gi_utils::engine::event::{EventSequence, ScrollDir};
//! use gi_utils::key::Key;
//!
//! let mut seq = EventSequence::new();
//!
//! // 按键一次 (按下, hold 50ms, 释放)
//! seq.press(Key::F).sleep(50.0).release(Key::F);
//!
//! // 长按 500ms
//! seq.press(Key::W).sleep(500.0).release(Key::W);
//!
//! // 点击然后滚轮
//! seq.left_click().sleep(80.0).wheel(ScrollDir::DOWN).sleep(5.0);
//! ```

use crate::interception::native::*;
use crate::key::Key;
use crate::scan_code::ScanCode;

// ═══════════════════════════════════════════════════════════════
// 滚轮方向 — Scroll direction constants
// ═══════════════════════════════════════════════════════════════

/// Windows 标准鼠标滚轮步进值 (Standard WHEEL_DELTA).
pub const WHEEL_DELTA: i16 = 120;

/// Interception 绝对坐标虚拟屏幕尺寸（归一化最大值 0..65535）。
/// Interception virtual screen size for absolute coordinates.
pub const VIRTUAL_SCREEN_SIZE: i32 = 65535;

/// 屏幕像素坐标 → Interception 归一化绝对坐标。
/// `pixel * VIRTUAL_SCREEN_SIZE / screen_size`，i64 中间值防溢出。
#[inline]
pub fn normalize_absolute(pixel: i32, screen_size: i32) -> i32 {
    (pixel as i64 * VIRTUAL_SCREEN_SIZE as i64 / screen_size.max(1) as i64) as i32
}

/// 滚轮方向常量。正数 = 上/右滚动，负数 = 下/左滚动。
/// Semantic direction constants for `wheel()`.
/// Positive = scroll up/right, negative = scroll down/left.
pub struct ScrollDir;
impl ScrollDir {
    /// 向上/右滚动 (Scroll up/right).
    pub const UP: i16 = WHEEL_DELTA;
    /// 向下/左滚动 (Scroll down/left).
    pub const DOWN: i16 = -WHEEL_DELTA;
}

// ═══════════════════════════════════════════════════════════════
// InputEvent — 单个输入动作
// ═══════════════════════════════════════════════════════════════

/// 单个输入动作：按键、鼠标动作或延时。
/// A single input action: keyboard stroke, mouse stroke, or a delay.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    /// 按下或释放按键 (Key press or release).
    Keyboard { code: ScanCode, state: u16 },
    /// 鼠标按键、滚轮或光标移动 (Mouse button, wheel, or cursor).
    Mouse {
        state: u16,
        flags: u16,
        rolling: i16,
        x: i32,
        y: i32,
    },
    /// 纯延时 — 暂停序列 `ms` 毫秒 (Pure delay).
    Sleep { ms: f64 },
}

// ── 构造器 — Constructors ─────────────────────────────────────

impl InputEvent {
    /// 创建按键按下事件 (Create a key-down event).
    pub fn press(key: impl Into<Key>) -> Self {
        let key = key.into();
        InputEvent::Keyboard {
            code: key.code,
            state: key.down_state(),
        }
    }

    /// 创建按键释放事件 (Create a key-up event).
    pub fn release(key: impl Into<Key>) -> Self {
        let key = key.into();
        InputEvent::Keyboard {
            code: key.code,
            state: key.up_state(),
        }
    }

    /// 创建鼠标左键按下事件 (Left button down).
    pub fn left_down() -> Self {
        InputEvent::Mouse {
            state: INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN,
            flags: 0,
            rolling: 0,
            x: 0,
            y: 0,
        }
    }

    /// 创建鼠标左键释放事件 (Left button up).
    pub fn left_up() -> Self {
        InputEvent::Mouse {
            state: INTERCEPTION_MOUSE_LEFT_BUTTON_UP,
            flags: 0,
            rolling: 0,
            x: 0,
            y: 0,
        }
    }

    /// 创建鼠标右键按下事件 (Right button down).
    pub fn right_down() -> Self {
        InputEvent::Mouse {
            state: INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN,
            flags: 0,
            rolling: 0,
            x: 0,
            y: 0,
        }
    }

    /// 创建鼠标右键释放事件 (Right button up).
    pub fn right_up() -> Self {
        InputEvent::Mouse {
            state: INTERCEPTION_MOUSE_RIGHT_BUTTON_UP,
            flags: 0,
            rolling: 0,
            x: 0,
            y: 0,
        }
    }

    /// 创建滚轮事件，`delta` 为 WHEEL_DELTA 的倍数，正上负下。
    /// `delta` is one or more WHEEL_DELTA units. Positive = up/right, negative = down/left.
    pub fn wheel(delta: i16) -> Self {
        InputEvent::Mouse {
            state: INTERCEPTION_MOUSE_WHEEL,
            flags: 0,
            rolling: delta,
            x: 0,
            y: 0,
        }
    }

    /// 创建相对移动事件 (Relative cursor movement by dx, dy).
    pub fn move_relative(dx: i32, dy: i32) -> Self {
        InputEvent::Mouse {
            state: 0,
            flags: INTERCEPTION_MOUSE_MOVE_RELATIVE,
            rolling: 0,
            x: dx,
            y: dy,
        }
    }

    /// 创建绝对移动事件 — 坐标为 Interception **归一化值 0..65535**
    /// （虚拟屏幕 u16::MAX），**不是屏幕像素**（review 2.2）。
    /// 像素换算用 [`normalize_absolute`]：`normalized = pixel * 65535 / screen_size`
    /// （对应 C++ 原版 `VIRTUAL_SCREEN_WIDTH * x / screen_width`；
    /// 添加好友/申请加入等绝对定位功能移植时必须照此换算）。
    /// Creates an absolute movement event with Interception-normalized
    /// coordinates (0..65535 virtual screen), NOT screen pixels.
    pub fn move_absolute(x: i32, y: i32) -> Self {
        InputEvent::Mouse {
            state: 0,
            flags: INTERCEPTION_MOUSE_MOVE_ABSOLUTE,
            rolling: 0,
            x,
            y,
        }
    }

    /// 创建延时事件，暂停 `ms` 毫秒 (Create a delay event).
    pub fn sleep(ms: f64) -> Self {
        InputEvent::Sleep { ms }
    }
}

// ═══════════════════════════════════════════════════════════════
// EventSequence — 可组合的事件序列
// ═══════════════════════════════════════════════════════════════

/// 可组合的输入原语有序序列。
/// 所有动作方法立即发送，**不含内置延迟**，通过 [`sleep`](Self::sleep) 控制时序。
///
/// An ordered sequence of composable input primitives.
/// All action methods send instantly with **no built-in delay**.
/// Insert [`sleep`](Self::sleep) between actions to control timing.
///
/// # 示例 — Example
///
/// ```
/// use gi_utils::engine::event::{EventSequence, ScrollDir};
/// use gi_utils::key::Key;
///
/// let mut seq = EventSequence::new();
/// seq.press(Key::F)
///    .sleep(50.0)
///    .release(Key::F)
///    .sleep(30.0)
///    .left_click()
///    .sleep(10.0)
///    .wheel(ScrollDir::DOWN);
/// ```
#[derive(Debug, Clone, Default)]
pub struct EventSequence {
    events: Vec<InputEvent>,
}

impl EventSequence {
    /// 创建空的事件序列 (Create an empty event sequence).
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    // ── 访问器 — Accessors ────────────────────────────────

    /// 序列中的事件数量 (Number of events in the sequence).
    pub fn len(&self) -> usize {
        self.events.len()
    }
    /// 序列是否为空 (Whether the sequence is empty).
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    /// 获取事件切片引用 (Get a slice reference to the events).
    pub fn events(&self) -> &[InputEvent] {
        &self.events
    }
    /// 清空序列 (Clear all events from the sequence).
    pub fn clear(&mut self) {
        self.events.clear();
    }

    // ── 原始推送 — Raw push ─────────────────────────────────

    /// 追加任意 `InputEvent`，返回 `&mut Self` 以支持链式调用。
    /// Append any `InputEvent`. Returns `&mut Self` for chaining.
    pub fn push(&mut self, event: InputEvent) -> &mut Self {
        self.events.push(event);
        self
    }

    // ── 键盘原语 — Keyboard primitives ─────────────────────

    /// 按下按键，无延时 (Press a key down).
    pub fn press(&mut self, key: impl Into<Key>) -> &mut Self {
        self.push(InputEvent::press(key))
    }

    /// 释放按键，无延时 (Release a key).
    pub fn release(&mut self, key: impl Into<Key>) -> &mut Self {
        self.push(InputEvent::release(key))
    }

    /// 长按按键：press → sleep(`duration_ms`) → release。
    /// Hold a key for a given duration.
    pub fn hold(&mut self, key: impl Into<Key>, duration_ms: f64) -> &mut Self {
        let k = key.into();
        self.press(k).sleep(duration_ms).release(k)
    }

    /// 轻触按键：press 后立即 release，中间无延时。
    /// Tap a key (press then immediately release).
    pub fn tap(&mut self, key: impl Into<Key>) -> &mut Self {
        let k = key.into();
        self.press(k).release(k)
    }

    // ── 鼠标按键原语 — Mouse button primitives ─────────────

    /// 左键按下 (Left button down).
    pub fn left_down(&mut self) -> &mut Self {
        self.push(InputEvent::left_down())
    }

    /// 左键释放 (Left button up).
    pub fn left_up(&mut self) -> &mut Self {
        self.push(InputEvent::left_up())
    }

    /// 左键单击：按下后立即释放 (Left click: down then immediately up).
    pub fn left_click(&mut self) -> &mut Self {
        self.left_down().left_up()
    }

    /// 按住左键：down → sleep(`duration_ms`) → up。
    /// Hold left button for a given duration.
    pub fn hold_left(&mut self, duration_ms: f64) -> &mut Self {
        self.left_down().sleep(duration_ms).left_up()
    }

    /// 右键按下 (Right button down).
    pub fn right_down(&mut self) -> &mut Self {
        self.push(InputEvent::right_down())
    }

    /// 右键释放 (Right button up).
    pub fn right_up(&mut self) -> &mut Self {
        self.push(InputEvent::right_up())
    }

    /// 右键单击：按下后立即释放 (Right click: down then immediately up).
    pub fn right_click(&mut self) -> &mut Self {
        self.right_down().right_up()
    }

    /// 按住右键：down → sleep(`duration_ms`) → up。
    /// Hold right button for a given duration.
    pub fn hold_right(&mut self, duration_ms: f64) -> &mut Self {
        self.right_down().sleep(duration_ms).right_up()
    }

    // ── 鼠标移动原语 — Mouse movement primitives ───────────

    /// 相对移动 (Relative move by dx, dy).
    pub fn move_rel(&mut self, dx: i32, dy: i32) -> &mut Self {
        self.push(InputEvent::move_relative(dx, dy))
    }

    /// 绝对移动 (Absolute move to screen x, y).
    pub fn move_abs(&mut self, x: i32, y: i32) -> &mut Self {
        self.push(InputEvent::move_absolute(x, y))
    }

    /// 滚动鼠标滚轮一个步进。详见 [`ScrollDir::UP`] / [`ScrollDir::DOWN`]。
    /// Scroll the mouse wheel once.
    pub fn wheel(&mut self, delta: i16) -> &mut Self {
        self.push(InputEvent::wheel(delta))
    }

    /// 一次事件中滚动 `delta × times` 格。Interception 直接发送累积滚动量，无需多个事件。
    /// `scroll(ScrollDir::DOWN, 7)` 等价于 `rolling = -840` 的单个事件。
    ///
    /// Scroll the mouse wheel by `delta` × `times` notches in a single event.
    pub fn scroll(&mut self, delta: i16, times: usize) -> &mut Self {
        // times 先 clamp 到 i16 范围再乘 — `usize as i16` 对大值回绕
        // （review 4.2），例如 times=70000 会绕成 4464 的滚动量
        self.wheel(delta.saturating_mul(times.min(i16::MAX as usize) as i16))
    }

    // ── 时序原语 — Timing primitive ─────────────────────────

    /// 暂停序列 `ms` 毫秒 (Pause the sequence for `ms` milliseconds).
    pub fn sleep(&mut self, ms: f64) -> &mut Self {
        self.push(InputEvent::sleep(ms))
    }
}
