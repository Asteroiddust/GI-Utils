//! Interception 上下文的类型化封装
//!
//! 底层实现见 [`crate::interception::protocol`]（原生 Rust 移植，替代原
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
use crate::interception::protocol::{
    self, Device, InterceptionKeyStroke, InterceptionMouseStroke, KeyboardDevice,
    MAX_STROKES_PER_IOCTL,
};
use std::marker::PhantomData;

/// 创建原始上下文，失败 panic（驱动未安装）— 与旧 create_raw 语义一致。
fn create_raw() -> protocol::Context {
    protocol::Context::create().unwrap_or_else(|e| {
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
    raw: protocol::Context,
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
    raw: protocol::Context,
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
            InputEvent::Keyboard { .. } => {
                let stroke = event.to_key_stroke().expect("Keyboard 分支");
                self.raw
                    .send_keyboard(protocol::keyboard(0), std::slice::from_ref(&stroke));
            }
            InputEvent::Mouse { .. } => {
                let stroke = event.to_mouse_stroke().expect("Mouse 分支");
                self.raw
                    .send_mouse(protocol::mouse(0), std::slice::from_ref(&stroke));
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
        for segment in segments(events) {
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
                        strokes[i] = event.to_key_stroke().expect("键盘段仅含 Keyboard 事件");
                    }
                    self.raw
                        .send_keyboard(protocol::keyboard(0), &strokes[..chunk.len()]);
                }
            }
            InputEvent::Mouse { .. } => {
                let mut strokes = [InterceptionMouseStroke::default(); MAX_STROKES_PER_IOCTL];
                for chunk in segment.chunks(MAX_STROKES_PER_IOCTL) {
                    for (i, event) in chunk.iter().enumerate() {
                        strokes[i] = event.to_mouse_stroke().expect("鼠标段仅含 Mouse 事件");
                    }
                    self.raw
                        .send_mouse(protocol::mouse(0), &strokes[..chunk.len()]);
                }
            }
            InputEvent::Sleep { .. } => unreachable!("split_segments 已排除 Sleep"),
        }
    }
}

// Sync 声明在此层而非 protocol::Context：SendContext 仅暴露发送方法，
// 驱动文档明确支持通过同一上下文并发发送（protocol::Context 不标 Sync —
// 其 receive 方法若被共享会并发瓜分驱动队列，review）。
unsafe impl Sync for SendContext {}

/// 事件序列 → 可合并发送的连续段（**惰性迭代器，零分配**）。
///
/// 边界规则：Sleep 代表时间流逝，不可与前后动作合并；键盘与鼠标是
/// 不同设备号且必须保持严格顺序，不可按类型分组。事件序列是构造后
/// 不可变的静态数据 — 段边界在每次播放时重算，惰性迭代消除每迭代
/// 一次的 Vec 分配（连点器 v1 每 10ms 周期调用一次）。
fn segments(events: &[InputEvent]) -> impl Iterator<Item = &[InputEvent]> {
    struct Segments<'a> {
        events: &'a [InputEvent],
        pos: usize,
    }
    impl<'a> Iterator for Segments<'a> {
        type Item = &'a [InputEvent];

        fn next(&mut self) -> Option<Self::Item> {
            while self.pos < self.events.len() {
                let start = self.pos;
                let mut kind: Option<bool> = None; // Some(true)=键盘段，Some(false)=鼠标段
                while self.pos < self.events.len() {
                    match self.events[self.pos] {
                        InputEvent::Sleep { .. } => break,
                        InputEvent::Keyboard { .. } => {
                            if kind == Some(false) {
                                break; // 设备类型切换 → 切批
                            }
                            kind = Some(true);
                            self.pos += 1;
                        }
                        InputEvent::Mouse { .. } => {
                            if kind == Some(true) {
                                break;
                            }
                            kind = Some(false);
                            self.pos += 1;
                        }
                    }
                }
                if self.pos > start {
                    return Some(&self.events[start..self.pos]);
                }
                // 当前位置是 Sleep（不发送）：跳过
                self.pos += 1;
            }
            None
        }
    }
    Segments { events, pos: 0 }
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
        let segments: Vec<&[InputEvent]> = segments(&events).collect();
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].len(), 2); // 键盘对
        assert_eq!(segments[1].len(), 2); // 鼠标对
        assert_eq!(segments[2].len(), 1); // Sleep 后的滚轮
    }

    #[test]
    fn split_handles_empty_and_sleep_edges() {
        assert!(segments(&[]).next().is_none());
        // 纯 Sleep 序列 → 无段（Sleep 不发送）
        assert!(segments(&[InputEvent::Sleep { ms: 1.0 }]).next().is_none());
        // 连续 Sleep：不产生空段
        let events = vec![
            InputEvent::press(Key::F),
            InputEvent::Sleep { ms: 1.0 },
            InputEvent::Sleep { ms: 2.0 },
            InputEvent::release(Key::F),
        ];
        let segments: Vec<&[InputEvent]> = segments(&events).collect();
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
        let segments: Vec<&[InputEvent]> = segments(&events).collect();
        assert_eq!(segments.len(), 3);
        assert!(matches!(segments[0][0], InputEvent::Keyboard { .. }));
        assert!(matches!(segments[1][0], InputEvent::Mouse { .. }));
        assert!(matches!(segments[2][0], InputEvent::Keyboard { .. }));
    }
}
