//! 注入按下状态登记 — Panic-safe tracking of injected holds.
//!
//! `SendContext` 每次注入前登记「按下」、注入 release 成功后清除；panic hook
//! 在 `exit` 前读取快照并补发全部未松开的合成键（防系统级卡键）。
//!
//! # 为什么用原子位图
//!
//! panic 可发生在任意线程（包括正持有某把锁的线程）。登记表若用 `Mutex`，
//! panic hook 在同线程重入时无法拿锁（std Mutex 不可重入），毒化处理也救不了；
//! 原子位图无锁、无阻塞、无分配，任意时刻可读，天然 panic-safe。
//!
//! # 局限（设计上接受）
//!
//! - 覆盖 panic（`panic=unwind`）路径 + 正常关机单次 best-effort 兜底；
//!   `kill` / `TerminateProcess` / 断电不经过 hook，无法补发。
//! - 快照与并发 in-flight 发送存在竞态（快照后其它线程仍可能送出 press）：
//!   hook 用两遍补发降低，不保证根除（正常停止后 Loop 检查粒度 ~100μs）。
//! - 同一键被多个功能线程按下只记 1 位 — 与 `HeldTracker` 相同：重复 down
//!   幂等（非跨线程引用计数）；极端情况下 panic 补发可能早于另一持有者的
//!   release。
//! - 发送失败语义保守：DOWN 先登记（写失败 → 若该键实际未送达，补发会出现
//!   一次多余 release）；UP 仅写成功才清（写失败 → 留位，panic/关机时补发会
//!   再次尝试，可能重复 release）。逐条补发失败位重新登记：panic hook 有第二遍
//!   重试；正常关机只单次尝试，失败位随进程退出丢弃。
//! - 扫描码 > 0xFF 忽略：驱动只会产生 PS/2 Set1 码（项目 Key 常量最大 0x6F，
//!   E1/PAUSE 不支持），超界值无线上语义。
//!
//! # 补发路径
//!
//! `release_into` 用 swap 取下全部位 → 逐条 `SendContext::send_untracked`
//! （不更新本登记表，避免误清快照后并发按下的位）→ 写失败（`written == 0`）
//! 的位重新 set 回全局表：panic hook 的第二遍补发会重试；正常关机只单次
//! best-effort，失败位随后随进程退出丢弃。
//!
//! # 键盘位索引
//!
//! `idx = (code & 0xFF) | (is_e0 ? 256 : 0)`，8 × u64 = 512 bit，恰好
//! 一 cache line（`#[repr(align(64))]` 防与相邻 static 伪共享）。

use std::sync::atomic::{AtomicU64, Ordering};

use crate::engine::event::InputEvent;
use crate::interception::SendContext;
use crate::interception::protocol::{
    INTERCEPTION_KEY_E0, INTERCEPTION_KEY_UP, INTERCEPTION_MOUSE_BUTTON_4_DOWN,
    INTERCEPTION_MOUSE_BUTTON_5_DOWN, INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN,
    INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN, INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN,
};
use crate::key::{Key, ScanCode};

/// 键盘位图字数：8 × u64 = 512 bit = 256 扫描码 × E0 位面。
const KEY_WORDS: usize = 8;
/// E0 位面在索引中的偏移。
const E0_OFFSET: usize = 0x100;

/// 鼠标按键 DOWN 位（protocol.rs）→ 位图槽位顺序；UP = DOWN << 1。
const MOUSE_DOWN_BITS: [u16; 5] = [
    INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN,
    INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN,
    INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN,
    INTERCEPTION_MOUSE_BUTTON_4_DOWN,
    INTERCEPTION_MOUSE_BUTTON_5_DOWN,
];

/// 键盘位图（恰一 cache line，64B 对齐防伪共享）。
#[repr(align(64))]
struct KeyBits([AtomicU64; KEY_WORDS]);

static KEYS: KeyBits = KeyBits([const { AtomicU64::new(0) }; KEY_WORDS]);
/// 鼠标 5 键位（bit0..bit4 分别对应左右中/4/5）。
static BUTTONS: AtomicU64 = AtomicU64::new(0);

// ═══════════════════════════════════════════════════════════════════
// 索引与纯状态 — 无驱动可单测
// ═══════════════════════════════════════════════════════════════════

/// `(code, is_e0)` → 位索引；code > 0xFF 无线上语义，返回 `None`（忽略）。
fn key_index(code: u16, is_e0: bool) -> Option<usize> {
    if code > 0xFF {
        return None;
    }
    Some(code as usize | if is_e0 { E0_OFFSET } else { 0 })
}

/// 位索引 → `Key`（E0 从索引高位恢复）。
fn key_from_index(idx: usize) -> Key {
    Key {
        code: ScanCode((idx & 0xFF) as u16),
        is_e0: idx & E0_OFFSET != 0,
    }
}

/// 键盘事件 → (位索引, 是否按下)；非键盘/越界扫描码返回 `None`。
fn key_event(event: &InputEvent) -> Option<(usize, bool)> {
    let InputEvent::Keyboard { code, state } = event else {
        return None;
    };
    let down = state & INTERCEPTION_KEY_UP == 0;
    key_index(code.raw(), state & INTERCEPTION_KEY_E0 != 0).map(|idx| (idx, down))
}

/// 对鼠标事件中的每个已知按键回调 `(槽位, 是否按下)`；滚轮/移动无按键位。
fn for_each_mouse_change(event: &InputEvent, mut f: impl FnMut(usize, bool)) {
    let InputEvent::Mouse { state, .. } = event else {
        return;
    };
    for (slot, down) in MOUSE_DOWN_BITS.into_iter().enumerate() {
        if state & down != 0 {
            f(slot, true);
        }
        if state & (down << 1) != 0 {
            f(slot, false);
        }
    }
}

/// 对「按下」类操作回调键盘位索引 / 鼠标槽位（发送前登记）。
fn for_each_down(event: &InputEvent, mut key: impl FnMut(usize), mut button: impl FnMut(usize)) {
    if let Some((idx, true)) = key_event(event) {
        key(idx);
    }
    for_each_mouse_change(event, |slot, down| {
        if down {
            button(slot);
        }
    });
}

/// 对「松开」类操作回调（仅写成功后清除）。
fn for_each_up(event: &InputEvent, mut key: impl FnMut(usize), mut button: impl FnMut(usize)) {
    if let Some((idx, false)) = key_event(event) {
        key(idx);
    }
    for_each_mouse_change(event, |slot, down| {
        if !down {
            button(slot);
        }
    });
}

/// 挂起位纯状态：补发路径用它承载 swap 快照；同时作为全局原子版本的
/// 无驱动测试镜像（同一套映射 helper，测试可完全脱离驱动）。
#[derive(Default)]
struct PressedState {
    keys: [u64; KEY_WORDS],
    buttons: u64,
}

impl PressedState {
    fn new() -> Self {
        Self::default()
    }

    fn is_empty(&self) -> bool {
        self.keys.iter().all(|w| *w == 0) && self.buttons == 0
    }

    /// 测试/调试辅助：快照对应的全部 release 事件（生产补发走
    /// `for_each_release` 逐条发送，不先收集 Vec）。
    #[cfg(test)]
    fn releases(&self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        self.for_each_release(|_, event| {
            out.push(event);
            true
        });
        out
    }

    /// 按「键盘先、鼠标后」顺序遍历快照对应的 release 事件；`f` 返回
    /// `false` 表示该条补发失败。返回成功条数；位序稳定（键盘按索引升序、
    /// 鼠标按槽位升序），供补发/失败重登记与单测共用。
    fn for_each_release(&self, mut f: impl FnMut(HeldBit, InputEvent) -> bool) -> usize {
        let mut sent = 0usize;
        for (word_idx, word) in self.keys.iter().enumerate() {
            let mut bits = *word;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let idx = word_idx * 64 + bit;
                let event = InputEvent::release(key_from_index(idx));
                if f(HeldBit::Key(idx), event) {
                    sent += 1;
                }
            }
        }
        for (slot, down) in MOUSE_DOWN_BITS.into_iter().enumerate() {
            if self.buttons & (1 << slot) != 0 {
                let event = InputEvent::Mouse {
                    state: down << 1, // UP = DOWN << 1
                    flags: 0,
                    rolling: 0,
                    x: 0,
                    y: 0,
                };
                if f(HeldBit::Button(slot), event) {
                    sent += 1;
                }
            }
        }
        sent
    }

    /// 测试镜像：等价于全局 `before_send`。
    #[cfg(test)]
    fn press(&mut self, event: &InputEvent) {
        for_each_down(
            event,
            |idx| self.keys[idx / 64] |= 1 << (idx % 64),
            |slot| self.buttons |= 1 << slot,
        );
    }

    /// 测试镜像：等价于全局 `after_send`（written=0 不清）。
    #[cfg(test)]
    fn release(&mut self, event: &InputEvent, written: usize) {
        if written == 0 {
            return;
        }
        for_each_up(
            event,
            |idx| self.keys[idx / 64] &= !(1 << (idx % 64)),
            |slot| self.buttons &= !(1 << slot),
        );
    }
}

/// 快照中的一项挂起位（补发失败时按位重新登记）。
#[derive(Clone, Copy)]
enum HeldBit {
    Key(usize),
    Button(usize),
}

/// 逐条补发快照：`try_send` 返回写成功与否；失败位收集到返回的
/// `PressedState`（调用方重新并入全局登记：panic hook 第二遍会重试，正常关机
/// 单次、失败随退出丢弃）。
fn dispatch_releases(
    snapshot: &PressedState,
    mut try_send: impl FnMut(&InputEvent) -> bool,
) -> (usize, PressedState) {
    let mut failed = PressedState::new();
    let sent = snapshot.for_each_release(|bit, event| {
        if try_send(&event) {
            return true;
        }
        match bit {
            HeldBit::Key(idx) => failed.keys[idx / 64] |= 1 << (idx % 64),
            HeldBit::Button(slot) => failed.buttons |= 1 << slot,
        }
        false
    });
    (sent, failed)
}

// ═══════════════════════════════════════════════════════════════════
// 全局登记 — SendContext 注入路径调用（panic hook 读取）
// ═══════════════════════════════════════════════════════════════════

/// 发送前登记：只记录按下（DOWN）类事件；重复按下同键幂等。
pub fn before_send(event: &InputEvent) {
    for_each_down(
        event,
        |idx| {
            KEYS.0[idx / 64].fetch_or(1 << (idx % 64), Ordering::Relaxed);
        },
        |slot| {
            BUTTONS.fetch_or(1 << slot, Ordering::Relaxed);
        },
    );
}

/// 发送成功后清除：仅松开（UP）类事件且实际写入成功（`written > 0`）才清。
pub fn after_send(event: &InputEvent, written: usize) {
    if written == 0 {
        return;
    }
    for_each_up(
        event,
        |idx| {
            KEYS.0[idx / 64].fetch_and(!(1 << (idx % 64)), Ordering::Relaxed);
        },
        |slot| {
            BUTTONS.fetch_and(!(1 << slot), Ordering::Relaxed);
        },
    );
}

/// 取走全局登记的完整快照（swap(0)）— 只由补发路径调用。
fn take_snapshot() -> PressedState {
    let mut snapshot = PressedState::new();
    for (i, word) in KEYS.0.iter().enumerate() {
        snapshot.keys[i] = word.swap(0, Ordering::Relaxed);
    }
    snapshot.buttons = BUTTONS.swap(0, Ordering::Relaxed);
    snapshot
}

/// 补发失败的位重新登记（panic hook 第二遍会重试；正常关机单次）。
fn merge_failed(failed: &PressedState) {
    for (word_idx, word) in failed.keys.iter().enumerate() {
        let mut bits = *word;
        while bits != 0 {
            let bit = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            let idx = word_idx * 64 + bit;
            KEYS.0[idx / 64].fetch_or(1 << (idx % 64), Ordering::Relaxed);
        }
    }
    let mut bits = failed.buttons;
    while bits != 0 {
        let slot = bits.trailing_zeros() as usize;
        bits &= bits - 1;
        BUTTONS.fetch_or(1 << slot, Ordering::Relaxed);
    }
}

/// 补发全部挂起合成键（panic hook / 正常关机）：swap 取快照 → 逐条
/// `send_untracked`（**不更新登记表**，避免误清快照后并发按下的位）→ 写失败
/// 的位重新登记（panic hook 第二遍会重试；正常关机单次，失败随退出丢弃）。
/// 返回逐条补发成功条数；空表零 IOCTL。
pub fn release_into(ctx: &SendContext) -> usize {
    let snapshot = take_snapshot();
    if snapshot.is_empty() {
        return 0;
    }
    let (sent, failed) = dispatch_releases(&snapshot, |event| ctx.send_untracked(event) > 0);
    if !failed.is_empty() {
        merge_failed(&failed);
    }
    sent
}

// ═══════════════════════════════════════════════════════════════════
// 单测 — 纯逻辑，不依赖驱动
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interception::protocol::{
        INTERCEPTION_KEY_DOWN, INTERCEPTION_MOUSE_BUTTON_4_DOWN, INTERCEPTION_MOUSE_BUTTON_5_DOWN,
        INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN, INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN,
        INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN, INTERCEPTION_MOUSE_RIGHT_BUTTON_UP,
        INTERCEPTION_MOUSE_WHEEL,
    };

    /// 构造原始鼠标事件（DOWN/UP 位由调用方给出）。
    fn mouse(state: u16) -> InputEvent {
        InputEvent::Mouse {
            state,
            flags: 0,
            rolling: 0,
            x: 0,
            y: 0,
        }
    }

    /// 对应某按键 DOWN 位的 UP 事件（UP = DOWN << 1）。
    fn mouse_up(down: u16) -> InputEvent {
        mouse(down << 1)
    }

    /// 全局位图测试串行化（多个测试共享 static 位图）。
    static GLOBAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset_global() {
        for word in KEYS.0.iter() {
            word.store(0, std::sync::atomic::Ordering::Relaxed);
        }
        BUTTONS.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    fn drain_global() -> PressedState {
        take_snapshot()
    }

    #[test]
    fn key_index_distinguishes_e0_and_bounds() {
        assert_eq!(key_index(0x1D, false), Some(0x01D));
        assert_eq!(key_index(0x1D, true), Some(0x11D));
        assert_ne!(key_index(0x1D, false), key_index(0x1D, true));
        assert_eq!(key_index(0xFF, true), Some(0x1FF));
        assert_eq!(key_index(0x100, false), None);
        assert_eq!(key_index(u16::MAX, true), None);
    }

    #[test]
    fn key_event_restores_e0_and_direction() {
        assert_eq!(
            key_event(&InputEvent::press(Key::RALT)),
            Some((0x138, true))
        );
        assert_eq!(
            key_event(&InputEvent::release(Key::RALT)),
            Some((0x138, false))
        );
        assert_eq!(
            key_event(&InputEvent::press(Key::LCTRL)),
            Some((0x01D, true))
        );
        assert_eq!(key_event(&InputEvent::sleep(1.0)), None);
        // >0xFF 忽略
        let over = InputEvent::Keyboard {
            code: ScanCode(0x100),
            state: INTERCEPTION_KEY_DOWN,
        };
        assert_eq!(key_event(&over), None);
    }

    #[test]
    fn mouse_changes_cover_five_buttons() {
        let downs = [
            INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN,
            INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN,
            INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN,
            INTERCEPTION_MOUSE_BUTTON_4_DOWN,
            INTERCEPTION_MOUSE_BUTTON_5_DOWN,
        ];
        for (slot, down) in downs.into_iter().enumerate() {
            let mut down_seen = Vec::new();
            for_each_mouse_change(&mouse(down), |s, d| down_seen.push((s, d)));
            assert_eq!(down_seen, vec![(slot, true)], "down slot {slot}");
            let mut up_seen = Vec::new();
            for_each_mouse_change(&mouse(down << 1), |s, d| up_seen.push((s, d)));
            assert_eq!(up_seen, vec![(slot, false)], "up slot {slot}");
        }
        // 滚轮 / 纯移动无按键位
        for event in [mouse(INTERCEPTION_MOUSE_WHEEL), mouse(0)] {
            let mut seen = Vec::new();
            for_each_mouse_change(&event, |s, d| seen.push((s, d)));
            assert!(seen.is_empty());
        }
    }

    #[test]
    fn duplicate_press_is_idempotent() {
        let mut state = PressedState::new();
        state.press(&InputEvent::press(Key::S));
        state.press(&InputEvent::press(Key::S));
        assert_eq!(state.releases().len(), 1, "同键重复按下只补一条 release");
    }

    #[test]
    fn release_clears_only_on_successful_write() {
        let mut state = PressedState::new();
        state.press(&InputEvent::press(Key::S));
        state.release(&InputEvent::release(Key::S), 0);
        assert_eq!(
            state.releases(),
            vec![InputEvent::release(Key::S)],
            "写失败保留登记"
        );
        state.release(&InputEvent::release(Key::S), 1);
        assert!(state.releases().is_empty(), "写成功后清除");
        state.release(&InputEvent::release(Key::S), 1); // 幂等
        assert!(state.releases().is_empty());
    }

    #[test]
    fn take_snapshot_then_empty() {
        let mut state = PressedState::new();
        state.press(&InputEvent::press(Key::W));
        let snapshot = std::mem::take(&mut state);
        assert!(state.is_empty(), "take 后原状态清空");
        assert_eq!(snapshot.releases(), vec![InputEvent::release(Key::W)]);
    }

    #[test]
    fn releases_keyboard_first_then_mouse_with_correct_bits() {
        let mut state = PressedState::new();
        state.press(&InputEvent::press(Key::RALT)); // E0 键
        state.press(&InputEvent::left_down());
        state.press(&mouse(INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN));
        assert_eq!(
            state.releases(),
            vec![
                InputEvent::release(Key::RALT),
                InputEvent::Mouse {
                    state: INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN << 1,
                    flags: 0,
                    rolling: 0,
                    x: 0,
                    y: 0,
                },
                InputEvent::Mouse {
                    state: INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN << 1,
                    flags: 0,
                    rolling: 0,
                    x: 0,
                    y: 0,
                },
            ]
        );
    }

    #[test]
    fn releases_preserve_e0_at_boundary() {
        let key = Key {
            code: ScanCode(0xFF),
            is_e0: true,
        };
        let mut state = PressedState::new();
        state.press(&InputEvent::press(key));
        assert_eq!(state.releases(), vec![InputEvent::release(key)]);
    }

    // release_into 依赖真实 SendContext（驱动），当前未直接测试：以下覆盖其
    // 组成部分 take_snapshot（经 global_tracking_roundtrip 的 drain_global 间接
    // 验证）/ dispatch_releases / merge_failed；三者与 release_into 的全局
    // wiring 未单独测试。sender 闭包化（便于无驱动直测）留作后续可选项。

    #[test]
    fn mouse_up_written_zero_keeps_registration() {
        let mut state = PressedState::new();
        state.press(&InputEvent::left_down());
        state.release(&InputEvent::left_up(), 0);
        assert_eq!(
            state.releases(),
            vec![mouse_up(INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN)],
            "鼠标 UP 写失败必须保留登记"
        );
        state.release(&InputEvent::left_up(), 1);
        assert!(state.releases().is_empty());
    }

    #[test]
    fn mixed_mouse_event_down_and_up_in_one() {
        let mut state = PressedState::new();
        state.press(&mouse(INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN));
        let mixed = mouse(INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN | INTERCEPTION_MOUSE_RIGHT_BUTTON_UP);
        state.press(&mixed); // 同一事件内含左 DOWN + 右 UP
        state.release(&mixed, 1);
        assert_eq!(
            state.releases(),
            vec![mouse_up(INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN)],
            "单事件混合位：只清 UP 位、保留 DOWN 位"
        );
    }

    #[test]
    fn dispatch_reregisters_failed_bits() {
        let mut snapshot = PressedState::new();
        snapshot.press(&InputEvent::press(Key::S));
        snapshot.press(&InputEvent::left_down());
        // 全部失败 → 全部位重新登记
        let (sent, failed) = dispatch_releases(&snapshot, |_| false);
        assert_eq!(sent, 0);
        assert_eq!(failed.releases(), snapshot.releases());
        // 键盘成功、鼠标失败 → 只重登记鼠标位
        let (sent, failed) = dispatch_releases(&snapshot, |event| {
            matches!(event, InputEvent::Keyboard { .. })
        });
        assert_eq!(sent, 1);
        assert_eq!(
            failed.releases(),
            vec![mouse_up(INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN)]
        );
        // 重试失败位 → 成功且不再残留
        let (sent, failed) = dispatch_releases(&failed, |_| true);
        assert_eq!(sent, 1);
        assert!(failed.is_empty());
        // 快照本身不被消费
        assert_eq!(snapshot.releases().len(), 2);
    }

    #[test]
    fn release_path_does_not_clear_concurrent_press() {
        let mut global = PressedState::new();
        global.press(&InputEvent::press(Key::S));
        let snapshot = std::mem::take(&mut global); // 对应 take_snapshot()
        global.press(&InputEvent::press(Key::S)); // 快照之后并发按下同键
        let (sent, failed) = dispatch_releases(&snapshot, |_| true);
        assert_eq!(sent, 1);
        assert!(failed.is_empty());
        // release_into 只 merge 失败位、从不清位：并发 press 位必须保留
        assert_eq!(global.releases(), vec![InputEvent::release(Key::S)]);
    }

    #[test]
    fn global_tracking_roundtrip_respects_written() {
        let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_global();
        before_send(&InputEvent::press(Key::S));
        before_send(&InputEvent::left_down());
        after_send(&InputEvent::release(Key::S), 0); // 写失败不清
        assert_eq!(
            drain_global().releases(),
            vec![
                InputEvent::release(Key::S),
                mouse_up(INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN),
            ]
        );
        reset_global();
        before_send(&InputEvent::press(Key::S));
        after_send(&InputEvent::release(Key::S), 1); // 写成功清除
        assert!(drain_global().is_empty());
    }

    #[test]
    fn merge_failed_reregisters_keyboard_and_mouse_bits() {
        let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_global();
        let mut failed = PressedState::new();
        failed.press(&InputEvent::press(Key::RALT)); // E0 键
        failed.press(&mouse(INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN));
        merge_failed(&failed);
        assert_eq!(
            drain_global().releases(),
            vec![
                InputEvent::release(Key::RALT),
                mouse_up(INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN),
            ],
            "merge_failed 必须把键盘与鼠标失败位都重新登记"
        );
        // 幂等：再次 merge 同一失败位不产生重复（位图去重）
        merge_failed(&failed);
        assert_eq!(drain_global().releases().len(), 2);
    }

    #[test]
    fn global_tracking_handles_mouse_and_e0() {
        let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_global();
        before_send(&InputEvent::press(Key::RALT)); // E0 键
        before_send(&mouse(INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN));
        after_send(&InputEvent::release(Key::RALT), 0); // 写失败不清
        after_send(&mouse_up(INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN), 0);
        assert_eq!(
            drain_global().releases(),
            vec![
                InputEvent::release(Key::RALT),
                mouse_up(INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN),
            ],
            "全局登记必须保留 E0 键与鼠标键（写失败）"
        );
        reset_global();
        before_send(&InputEvent::press(Key::RALT));
        before_send(&mouse(INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN));
        after_send(&InputEvent::release(Key::RALT), 1);
        after_send(&mouse_up(INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN), 1);
        assert!(drain_global().is_empty());
    }
}
