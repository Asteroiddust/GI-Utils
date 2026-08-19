//! Interception 上下文的类型化封装
//!
//! 底层实现见 [`crate::interception::native`]（原生 Rust 移植，替代原
//! interception.lib FFI）。
//!
//! # 线程安全 (Thread Safety)
//!
//! 根据 Interception 文档，一个上下文可在多个线程间安全共享 **发送** 操作。
//! 每个 **接收** 线程应使用自己的上下文。
//!
//! [`SendContext`] 在类型层面强制执行此约束 — 它只暴露发送方法，
//! 且实现了 `Send + Sync`。
//! [`InterceptionContext`] 实现了 `Send` 但 **不** 实现 `Sync`。

use crate::engine::event::InputEvent;
use crate::interception::native::{
    self, Device, InterceptionKeyStroke, InterceptionMouseStroke, KeyboardDevice,
    MAX_STROKES_PER_IOCTL,
};
use std::marker::PhantomData;

/// 创建原始上下文，失败 panic（驱动未安装）— 与旧 create_raw 语义一致。
fn create_raw() -> native::Context {
    native::Context::create().unwrap_or_else(|e| {
        panic!(
            "Failed to create Interception context ({e}). \
             Is the interception driver installed? \
             (https://github.com/oblitum/Interception)"
        )
    })
}

// ═══════════════════════════════════════════════════════════════════
// InterceptionContext — receive-only, single-thread
// ═══════════════════════════════════════════════════════════════════

/// 只接收的 Interception 上下文。
///
/// 必须在单线程中使用（接收操作非线程安全）。
/// 实现了 `Send`（可跨线程移动），但 **不** 实现 `Sync`。
///
/// Must be used from a single thread at a time (receiving is not
/// thread-safe). Implements `Send` (safe to move), but NOT `Sync`.
pub struct InterceptionContext {
    raw: native::Context,
    /// `*mut ()` 为 !Send + !Sync：阻断 Sync 自动实现（接收仅限单线程）；
    /// Send 由下方显式 unsafe impl 恢复（可跨线程移动）。
    _not_sync: PhantomData<*mut ()>,
}

unsafe impl Send for InterceptionContext {}

impl InterceptionContext {
    /// 创建新的 Interception 接收上下文（独立打开全部 20 个设备）。
    pub fn create() -> Self {
        Self {
            raw: create_raw(),
            _not_sync: PhantomData,
        }
    }

    // ── Receive ──────────────────────────────────────────────

    /// 阻塞等待输入到达。返回有待处理数据的设备；失败返回 `None`
    /// （对齐 C 版返回 0 哨兵）。
    pub fn wait(&self) -> Option<Device> {
        self.raw.wait()
    }

    /// 带超时的等待（毫秒）。超时时返回 `None`。
    pub fn wait_timeout(&self, ms: u32) -> Option<Device> {
        self.raw.wait_with_timeout(ms)
    }

    /// 设置设备过滤器（谓词为 Rust 闭包，替代 C 版 extern "C" 函数指针）。
    pub fn set_filter(&self, predicate: impl Fn(Device) -> bool, filter: u16) {
        self.raw.set_filter(predicate, filter);
    }

    /// 从设备批量接收键盘输入，返回实际读到的前缀切片（防误用 API —
    /// 遍历返回值即遍历真实条目，缓冲尾部陈旧数据不可见）。
    ///
    /// 鼠标接收不在此暴露：引擎过滤器仅设键盘（鼠标事件 pass-through
    /// 不进队列）— 鼠标分支为防御性死代码已删（review）；底层协议 API
    /// 仍在 native.rs 完整保留。
    pub fn receive_keyboard<'a>(
        &self,
        device: KeyboardDevice,
        out: &'a mut [InterceptionKeyStroke],
    ) -> &'a mut [InterceptionKeyStroke] {
        self.raw.receive_keyboard(device, out)
    }
}

// ═══════════════════════════════════════════════════════════════════
// SendContext — send-only, thread-safe to share
// ═══════════════════════════════════════════════════════════════════

/// 只发送的 Interception 上下文，可在线程间安全共享。
///
/// 仅暴露 `send_event()` 与 `forward_*` 方法。
/// Interception 驱动明确支持通过同一个句柄并发发送。
///
/// Thread-safe to share across threads. Exposes only send
/// methods — the driver supports concurrent sends through one handle.
pub struct SendContext {
    raw: native::Context,
}

impl SendContext {
    /// 创建新的 Interception 发送上下文（独立第二个上下文 —
    /// 双上下文架构，与历史行为一致）。
    pub fn create() -> Self {
        Self { raw: create_raw() }
    }

    /// 发送一个 [`InputEvent`]，自动路由到正确的设备（键盘 0 / 鼠标 0）。
    pub fn send_event(&self, event: &InputEvent) {
        match event {
            InputEvent::Keyboard { code, state } => {
                let stroke = InterceptionKeyStroke {
                    code: code.raw(),
                    state: *state,
                    information: 0,
                };
                self.raw
                    .send_keyboard(native::keyboard(0), std::slice::from_ref(&stroke));
            }
            InputEvent::Mouse {
                state,
                flags,
                rolling,
                x,
                y,
            } => {
                let stroke = InterceptionMouseStroke {
                    state: *state,
                    flags: *flags,
                    rolling: *rolling,
                    x: *x,
                    y: *y,
                    information: 0,
                };
                self.raw
                    .send_mouse(native::mouse(0), std::slice::from_ref(&stroke));
            }
            InputEvent::Sleep { .. } => {}
        }
    }

    /// 引擎转发用：整批转发接收到的键盘 stroke（一次 IOCTL_WRITE，
    /// 与批量接收对称 — review：逐条转发浪费读侧省下的系统调用，
    /// 且更易与功能线程的发送交错）。
    pub fn forward_keyboard(&self, device: KeyboardDevice, strokes: &[InterceptionKeyStroke]) {
        self.raw.send_keyboard(device, strokes);
    }

    /// 发送事件序列：连续的同类非 Sleep 事件合并为一次 IOCTL_WRITE。
    ///
    /// Sleep 与键盘/鼠标切换是批次边界（时序与设备顺序不可跨越）。
    /// 合并使段内事件成为**驱动级原子送达**：无外部插入、间隔 ≈ 0 —
    /// 连点器 v1 的 [down, up] 点击对由两次系统调用变为一次驱动请求，
    /// 点击时长最短且不受其他功能线程并发发送的干扰。
    pub fn send_events(&self, events: &[InputEvent]) {
        for segment in split_segments(events) {
            self.send_segment(segment);
        }
    }

    /// 发送一个已切好的同设备连续段（内部按 32/批上限分块）。
    fn send_segment(&self, segment: &[InputEvent]) {
        match segment[0] {
            InputEvent::Keyboard { .. } => {
                let mut strokes = [InterceptionKeyStroke::default(); MAX_STROKES_PER_IOCTL];
                for chunk in segment.chunks(MAX_STROKES_PER_IOCTL) {
                    for (i, event) in chunk.iter().enumerate() {
                        if let InputEvent::Keyboard { code, state } = event {
                            strokes[i] = InterceptionKeyStroke {
                                code: code.raw(),
                                state: *state,
                                information: 0,
                            };
                        }
                    }
                    self.raw
                        .send_keyboard(native::keyboard(0), &strokes[..chunk.len()]);
                }
            }
            InputEvent::Mouse { .. } => {
                let mut strokes = [InterceptionMouseStroke::default(); MAX_STROKES_PER_IOCTL];
                for chunk in segment.chunks(MAX_STROKES_PER_IOCTL) {
                    for (i, event) in chunk.iter().enumerate() {
                        if let InputEvent::Mouse {
                            state,
                            flags,
                            rolling,
                            x,
                            y,
                        } = event
                        {
                            strokes[i] = InterceptionMouseStroke {
                                state: *state,
                                flags: *flags,
                                rolling: *rolling,
                                x: *x,
                                y: *y,
                                information: 0,
                            };
                        }
                    }
                    self.raw
                        .send_mouse(native::mouse(0), &strokes[..chunk.len()]);
                }
            }
            InputEvent::Sleep { .. } => unreachable!("split_segments 已排除 Sleep"),
        }
    }
}

// Sync 声明在此层而非 native::Context：SendContext 仅暴露发送方法，
// 驱动文档明确支持通过同一上下文并发发送（native::Context 不标 Sync —
// 其 receive 方法若被共享会并发瓜分驱动队列，review）。
unsafe impl Sync for SendContext {}

/// 将事件序列切分为可合并发送的连续段：**Sleep 与键盘/鼠标切换是边界**。
/// Sleep 代表时间流逝，不可与前后动作合并；键盘与鼠标是不同设备号且必须
/// 保持严格顺序，不可按类型分组。纯函数（无驱动依赖），单测覆盖。
fn split_segments(events: &[InputEvent]) -> Vec<&[InputEvent]> {
    let mut segments = Vec::new();
    let mut start = 0usize;
    let mut kind: Option<bool> = None; // Some(true)=键盘段，Some(false)=鼠标段

    for (i, event) in events.iter().enumerate() {
        let is_keyboard = matches!(event, InputEvent::Keyboard { .. });
        match event {
            InputEvent::Sleep { .. } => {
                if i > start {
                    segments.push(&events[start..i]);
                }
                start = i + 1;
                kind = None;
            }
            _ => {
                if kind == Some(!is_keyboard) {
                    // 设备类型切换 → 切批，保持严格顺序
                    segments.push(&events[start..i]);
                    start = i;
                }
                kind = Some(is_keyboard);
            }
        }
    }
    if start < events.len() {
        segments.push(&events[start..]);
    }
    segments
}

// ═══════════════════════════════════════════════════════════════════
// 切段逻辑单测 — 纯函数，无驱动依赖
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::event::{InputEvent, ScrollDir};
    use crate::key::Key;

    #[test]
    fn split_respects_sleep_and_device_boundaries() {
        // K,K（键盘段）| M,M（类型切换切批）| Sleep | K（新段）
        let events = vec![
            InputEvent::press(Key::F),
            InputEvent::release(Key::F),
            InputEvent::left_down(),
            InputEvent::left_up(),
            InputEvent::Sleep { ms: 10.0 },
            InputEvent::wheel(ScrollDir::DOWN),
        ];
        let segments = split_segments(&events);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].len(), 2); // 键盘对
        assert_eq!(segments[1].len(), 2); // 鼠标对
        assert_eq!(segments[2].len(), 1); // Sleep 后的滚轮
    }

    #[test]
    fn split_handles_empty_and_sleep_edges() {
        assert!(split_segments(&[]).is_empty());
        // 纯 Sleep 序列 → 无段（Sleep 不发送）
        assert!(split_segments(&[InputEvent::Sleep { ms: 1.0 }]).is_empty());
        // 连续 Sleep：不产生空段
        let events = vec![
            InputEvent::press(Key::F),
            InputEvent::Sleep { ms: 1.0 },
            InputEvent::Sleep { ms: 2.0 },
            InputEvent::release(Key::F),
        ];
        let segments = split_segments(&events);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].len(), 1);
        assert_eq!(segments[1].len(), 1);
    }

    #[test]
    fn split_keeps_strict_order_across_device_switch() {
        // K,M,K 交替 → 必须按序切成三段（不得按类型分组乱序）
        let events = vec![
            InputEvent::press(Key::W),
            InputEvent::left_down(),
            InputEvent::release(Key::W),
        ];
        let segments = split_segments(&events);
        assert_eq!(segments.len(), 3);
        assert!(matches!(segments[0][0], InputEvent::Keyboard { .. }));
        assert!(matches!(segments[1][0], InputEvent::Mouse { .. }));
        assert!(matches!(segments[2][0], InputEvent::Keyboard { .. }));
    }
}
