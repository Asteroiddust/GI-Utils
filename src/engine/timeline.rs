//! 时间戳引擎 — Timeline-based event scheduling (absolute-time orchestrator).
//!
//! `EventSequence`（Serial）之外的第二种事件编排范式：每个事件带相对播放起点的
//! 绝对时刻，多键按住可重叠（如"W 按住 200ms 期间 A/S/D 交错出现"）。
//! 数据模型与播放模型参照 **MIDI 编辑器/播放器**：
//!
//! | MIDI | 本模块 |
//! |------|--------|
//! | Note (start, duration, pitch) | [`TimelineEvent::Note`]（key 对应 pitch） |
//! | note-on / note-off | 展开为 Down@start / Up@start+duration |
//! | Control/Channel 事件 | [`TimelineEvent::At`]（鼠标、滚轮等任意事件） |
//! | 实时编辑（播放中加音符） | [`Timeline`] 以 `Arc<Mutex<..>>` 共享，播放中动态加入 |
//! | 播放器（事件指针 + 活动音符集合） | [`TimelinePlayer`]（指针推进 + 挂起键集合） |
//!
//! 两种执行器：
//!
//! - [`TimelinePlayer`]：静态/动态表 — 事件可构造时展开或播放中动态加入
//! - [`RollingKeys`] / [`RollingPlayer`]：节奏滚动 — 按下序列按间隔无限
//!   均匀推进，释放事件按下时动态排程，无静态表边界缝隙
//!
//! 对应 NEXT_STEPS 第 3 节的时间轴调度器设计。
//! The second event-orchestration paradigm alongside `EventSequence`:
//! each event carries an absolute time relative to playback start, and
//! multi-key holds can overlap naturally.

use crate::engine::event::InputEvent;
use crate::interception::SendContext;
use crate::key::Key;
use crate::utils::delay;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 时间轴条目 — 一条调度记录（MIDI 音符语义）。
/// 时间均为相对播放起点的毫秒数（绝对时刻调度，无累计漂移）。
#[derive(Debug, Clone, Copy)]
pub enum TimelineEvent {
    /// 音符：`start` 时刻按下，按住 `duration` 后释放（MIDI Note on/off）。
    /// 重叠由多条 Note 天然表达（多键同按）。
    /// 同一按键的 Note 不可重叠（双按语义未定义）。
    Note {
        key: Key,
        start: f64,
        duration: f64,
    },
    /// 在指定时刻发送一个任意事件（鼠标、滚轮、位移等；MIDI Control 事件对应）。
    /// 注意：这里的 Keyboard 事件不参与挂起键清理 — 键盘按住请用 [`Note`]。
    At { at: f64, event: InputEvent },
}

/// 时间轴 — 数据驱动的事件表（MIDI 编辑器的时间线）。
/// 与 EventSequence（链式 builder）相反：这里只登记带绝对时刻的条目，
/// 方法返回 `()` 刻意不支持链式调用。
///
/// 以 `Arc<Mutex<Timeline>>` 与 [`TimelinePlayer`] 共享时，**播放中可动态加入
/// 事件**（MIDI 实时编辑）：持锁调用 `note`/`at`，播放器增量同步。
#[derive(Debug, Clone, Default)]
pub struct Timeline {
    entries: Vec<TimelineEvent>,
}

impl Timeline {
    /// 创建空时间轴。
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个音符（MIDI Note）：`start` 时刻按下，按住 `duration` 后释放。
    /// 零时长 = tap（同刻先按下再松开）。
    pub fn note(&mut self, key: impl Into<Key>, start: f64, duration: f64) {
        self.entries.push(TimelineEvent::Note {
            key: key.into(),
            start,
            duration,
        });
    }

    /// 登记一个任意事件（鼠标/滚轮/位移等）。
    pub fn at(&mut self, at: f64, event: InputEvent) {
        self.entries.push(TimelineEvent::At { at, event });
    }

    /// 展开 + 排序（一次）。panic=abort 下用 total_cmp 保证 f64 全序，不触发排序 panic。
    fn build(&self) -> Vec<ScheduleItem> {
        let mut items = expand(&self.entries, 0);
        items.sort_by(|a, b| {
            a.at()
                .total_cmp(&b.at())
                .then(a.priority().cmp(&b.priority()))
        });
        items
    }
}

/// 展开时间轴条目为调度项（私有）。`from` 为增量起点（动态加入：只展开新条目）。
/// Note → Down@start / Up@start+duration；负时长钳制为 0（同刻按下松开，不 panic）。
fn expand(entries: &[TimelineEvent], from: usize) -> Vec<ScheduleItem> {
    let mut items = Vec::with_capacity((entries.len() - from) * 2);
    for e in &entries[from..] {
        match *e {
            TimelineEvent::Note {
                key,
                start,
                duration,
            } => {
                let duration = duration.max(0.0);
                items.push(ScheduleItem::Down { at: start, key });
                items.push(ScheduleItem::Up {
                    at: start + duration,
                    key,
                });
            }
            TimelineEvent::At { at, event } => {
                items.push(ScheduleItem::Event { at, event });
            }
        }
    }
    items
}

/// 展开后的调度项 — 已排序。私有，仅执行器与单测可见。
#[derive(Debug, Clone, Copy)]
enum ScheduleItem {
    Down { at: f64, key: Key },
    Up { at: f64, key: Key },
    Event { at: f64, event: InputEvent },
}

impl ScheduleItem {
    #[inline]
    fn at(&self) -> f64 {
        match self {
            ScheduleItem::Down { at, .. }
            | ScheduleItem::Up { at, .. }
            | ScheduleItem::Event { at, .. } => *at,
        }
    }

    /// 同刻排序优先级：Down(0) < Event(1) < Up(2) — 同刻先按下再松开。
    #[inline]
    fn priority(&self) -> u8 {
        match self {
            ScheduleItem::Down { .. } => 0,
            ScheduleItem::Event { .. } => 1,
            ScheduleItem::Up { .. } => 2,
        }
    }
}

/// MIDI 播放器状态机 — 纯数据层（同步/到期触发决策，不触碰发送）。
///
/// 与 [`Timeline`] 通过 `Arc<Mutex<Timeline>>` 共享，**播放中可实时编辑**
/// （MIDI 编辑器语义）：其他线程持锁调用 `note`/`at` 动态加入事件，
/// 状态机增量同步（`sync`）并合并进有序表；**已触发条目即从表中移除**
/// （触发指针恒为表首），新条目在到期帧被 `partition_point` 捕获，
/// 不重复、不遗漏。表空（无可触发条目）即播放完成。
///
/// 私有（模块内测试可直达）：公开形态是持有发送上下文的 [`TimelinePlayer`]。
struct TimelinePlayerState {
    timeline: Arc<Mutex<Timeline>>,
    /// 有序展开项缓存 — 恒为未触发条目（已触发即移除）。
    items: RefCell<Vec<ScheduleItem>>,
    /// 已同步的条目数（增量展开起点）。
    seen: RefCell<usize>,
}

impl TimelinePlayerState {
    /// 绑定共享时间轴，构造时执行首次增量同步。
    fn new(timeline: Arc<Mutex<Timeline>>) -> Self {
        let state = Self {
            timeline,
            items: RefCell::new(Vec::new()),
            seen: RefCell::new(0),
        };
        state.sync();
        state
    }

    /// 增量同步：把共享时间轴的新条目展开、合并进有序列表。
    ///
    /// 锁内只做 `entries.len()` 读取与快照复制；展开与排序在锁外进行
    /// （锁内排序会阻塞编辑器线程一整帧）。毒锁视为可恢复 — 取回内容。
    fn sync(&self) {
        let new_entries = {
            let tl = self.timeline.lock().unwrap_or_else(|p| p.into_inner());
            let seen = *self.seen.borrow();
            if tl.entries.len() <= seen {
                return;
            }
            tl.entries[seen..].to_vec()
        };
        let mut items = self.items.borrow_mut();
        items.extend(expand(&new_entries, 0));
        items.sort_by(|a, b| {
            a.at()
                .total_cmp(&b.at())
                .then(a.priority().cmp(&b.priority()))
        });
        *self.seen.borrow_mut() += new_entries.len();
    }

    /// 取出已到期（at <= 相对播放起点 ticks 进度）的展开项，并从表中移除。
    /// 到期判断与等待共用同一换算公式（`ms_to_ticks`），无双重换算误差。
    fn drain_due(&self, elapsed_ticks: u64) -> Vec<ScheduleItem> {
        let mut items = self.items.borrow_mut();
        let upto = items.partition_point(|it| delay::ms_to_ticks(it.at()) <= elapsed_ticks);
        items.drain(..upto).collect()
    }

    /// 最近到期时刻（毫秒）；空表返回 INFINITY（播放完成信号）。
    fn next_at(&self) -> f64 {
        self.items
            .borrow()
            .first()
            .map_or(f64::INFINITY, ScheduleItem::at)
    }
}

/// 时间轴执行器 — MIDI 播放器：状态机（[`TimelinePlayerState`]）+ 发送上下文。
///
/// 退出时保证无挂起按键：正常播放结束或提前停止都会补发 release
/// （活动音符补发 note-off，防卡键）。连续编排由编辑器在播放中不断
/// 追加（追加无阻塞，播放器每帧合并）；表空即正常结束。
pub struct TimelinePlayer {
    state: TimelinePlayerState,
    send_ctx: Arc<SendContext>,
}

impl TimelinePlayer {
    /// 绑定共享时间轴与发送上下文（状态机构造时首次增量同步）。
    pub fn new(timeline: Arc<Mutex<Timeline>>, send_ctx: Arc<SendContext>) -> Self {
        Self {
            state: TimelinePlayerState::new(timeline),
            send_ctx,
        }
    }

    /// 单遍播放（含播放中的动态加入）。退出时保证无挂起按键：
    /// 正常跑完或提前停止都会补发 release（活动音符 note-off）。
    ///
    /// 时间语义：deadline 是绝对 TSC 时刻（播放起点 + 条目偏移），无累计漂移——
    /// 前序条目耗时超标时后续条目自然"追时"，时间轴整体对齐播放起点。
    /// 动态加入的条目同样按播放起点换算（MIDI 时间线对拍）。
    pub fn play(&self, stop_requested: &AtomicBool) {
        if stop_requested.load(Ordering::Acquire) {
            return;
        }
        // 播放起点：绝对时刻基准，所有 at 相对它换算（一次换算）
        let start = delay::tsc_now();
        // 活动音符集合：Down 已发、Up 未到的键（对应 MIDI 活动 note 集）
        let mut pending: Vec<Key> = Vec::new();

        loop {
            // 1. 增量同步（播放中动态加入 — MIDI 实时编辑）
            self.state.sync();

            // 2. 触发已到期条目（at <= 播放进度）：整段前缀从表中移除并发送。
            //    已触发即移除 → 表恒为未触发条目；动态插入的新条目在到期帧
            //    被 partition_point 捕获，天然不重复、不遗漏。
            let due = self.state.drain_due(delay::tsc_now() - start);
            for item in due {
                if stop_requested.load(Ordering::Acquire) {
                    break;
                }
                match item {
                    ScheduleItem::Down { key, .. } => {
                        self.send_ctx.send_event(&InputEvent::press(key));
                        pending.push(key);
                    }
                    ScheduleItem::Up { key, .. } => {
                        self.send_ctx.send_event(&InputEvent::release(key));
                        pending.retain(|k| *k != key);
                    }
                    ScheduleItem::Event { event, .. } => {
                        self.send_ctx.send_event(&event);
                    }
                }
            }

            // 3. 停止请求 → 补发挂起键后退出（最坏停止延迟 ~100μs + 本帧突发发送）
            if stop_requested.load(Ordering::Acquire) {
                self.release_pending(&mut pending);
                return;
            }

            // 4. 播放完成：表空 → 补发挂起键后收尾退出（兜底双保险：
            //    正常路径 pending 已空，仅作者契约错误时非空 — 不卡键）
            let next_at = self.state.next_at();
            if !next_at.is_finite() {
                self.release_pending(&mut pending);
                return;
            }

            // 5. 忙等到最近到期时刻（等待期间编辑器可追加新条目，
            //    下一帧 sync 增量合并；100μs 检查节奏即时响应停止）
            delay::wait_until_interruptible(start + delay::ms_to_ticks(next_at), stop_requested);
        }
    }

    /// 补发挂起键的 release 并清空列表（MIDI 活动音符 note-off）。
    fn release_pending(&self, pending: &mut Vec<Key>) {
        for key in pending.drain(..) {
            self.send_ctx.send_event(&InputEvent::release(key));
        }
    }
}

/// 无限滚动按键调度参数 — Rolling key schedule (pure data).
///
/// 按下序列无限均匀推进：每 `interval_ms` 按下下一个键（序列轮转），
/// 每个键按 `duration_ms` 后释放。时间线上的事件不在构造时静态展开，
/// 而是在播放中**按节奏动态加入**：按下事件由节奏器在到期时刻实时产生，
/// 释放事件在按下时刻动态排程（按下时刻 + 按住时长，入队等待到期）。
/// 对应 C++ 鬼畜走路的 `next_press_time + scheduled_releases` 设计——
/// 该设计参考了游戏引擎的运行时事件排程：事件在运行时入队，主循环检查到期。
///
/// 与 [`Timeline`]/[`TimelinePlayer`]（有限编排表，可播放中动态加入）
/// 互补：表适合有限长度的编排；滚动调度适合"按下序列无限均匀推进"。
/// 按住时长接近按键周期时（如 199ms 按住 / 50ms 间隔 / 4 键轮转周期
/// 200ms），一次性展开的边界缝隙 ≈ 整个周期，静态表循环完全失效——
/// 滚动器在到期时刻实时产生按下、动态排程释放，等价于编辑器每拍向
/// 时间轴追加音符（MIDI 实时编辑语义）。
///
/// 契约：按住时长不应超过按键轮转周期（同键会双按，语义未定义）。
/// 通过 [`RollingKeys::into_player`] 绑定发送上下文后播放。
#[derive(Debug, Clone)]
pub struct RollingKeys {
    keys: Vec<Key>,
    interval_ms: f64,
    duration_ms: f64,
}

impl RollingKeys {
    /// 创建滚动调度参数。默认与 C++ 原版一致：间隔 50ms、按住 1ms。
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            interval_ms: 50.0,
            duration_ms: 1.0,
        }
    }

    /// 设置轮转键序列（按下顺序 = 序列顺序，循环）。
    pub fn keys(mut self, keys: Vec<Key>) -> Self {
        self.keys = keys;
        self
    }

    /// 设置按下间隔（毫秒）— 每过这么久按下下一个键。
    pub fn interval(mut self, ms: f64) -> Self {
        self.interval_ms = ms;
        self
    }

    /// 设置按住时长（毫秒）— 每个键按满这么久后释放。
    pub fn duration(mut self, ms: f64) -> Self {
        self.duration_ms = ms;
        self
    }

    /// 绑定发送上下文，生成滚动播放器（一次性）。
    pub fn into_player(self, send_ctx: Arc<SendContext>) -> RollingPlayer {
        RollingPlayer {
            keys: self.keys,
            interval_ms: self.interval_ms,
            duration_ms: self.duration_ms,
            send_ctx,
        }
    }
}

/// 滚动播放器 — 绑定发送上下文的 [`RollingKeys`] 执行器。
/// 对应静态执行器 [`TimelinePlayer`]。
pub struct RollingPlayer {
    keys: Vec<Key>,
    interval_ms: f64,
    duration_ms: f64,
    send_ctx: Arc<SendContext>,
}

impl RollingPlayer {
    /// 无限滚动播放，直到 `stop_requested` 置位。退出时补发所有挂起的释放。
    ///
    /// 每轮循环：到期释放 → 到期按下 → 停止检查 → 忙等到最近到期时刻。
    /// 按下序列按绝对节奏推进（`next_press += interval`，无累计漂移）；
    /// 每次循环至多一个按下（与 C++ 一致，节奏丢失不追赶）。
    pub fn play(&self, stop_requested: &AtomicBool) {
        if self.keys.is_empty() || stop_requested.load(Ordering::Acquire) {
            return;
        }
        let interval = delay::ms_to_ticks(self.interval_ms);
        let duration = delay::ms_to_ticks(self.duration_ms);

        // 节奏器状态：下次按下时刻（绝对 ticks）、键序列索引
        let mut next_press = delay::tsc_now();
        let mut key_idx = 0usize;
        // 动态排程的释放队列：(到期时刻, 键)，FIFO（按住时长恒定 → 到期顺序
        // = 按下顺序），队首即最早到期
        let mut releases: VecDeque<(u64, Key)> = VecDeque::new();

        loop {
            let now = delay::tsc_now();

            // 1. 到期释放（先释放后按下，同刻语义与静态表 Down<Event<Up 一致）
            while let Some(&(due, key)) = releases.front() {
                if due > now {
                    break;
                }
                releases.pop_front();
                self.send_ctx.send_event(&InputEvent::release(key));
            }

            // 2. 到期按下 — 动态加入按下事件，并排程其释放
            if now >= next_press {
                let key = self.keys[key_idx % self.keys.len()];
                self.send_ctx.send_event(&InputEvent::press(key));
                releases.push_back((now + duration, key));
                next_press += interval;
                key_idx += 1;
            }

            // 3. 停止 → 补发所有挂起的释放后退出（防卡键，对应 C++ 退出清理）
            if stop_requested.load(Ordering::Acquire) {
                for (_, key) in releases.drain(..) {
                    self.send_ctx.send_event(&InputEvent::release(key));
                }
                return;
            }

            // 4. 忙等到最近的到期时刻（下次按下或最早释放）
            let earliest_release = releases.front().map_or(u64::MAX, |r| r.0);
            let target = next_press.min(earliest_release);
            if now < target {
                delay::wait_until_interruptible(target, stop_requested);
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────
// 数据管线（展开/排序/钳制/增量同步/到期触发）完全脱离驱动环境；
// TimelinePlayerState 直接可达（模块内），不调用 SendContext::create。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_expands_to_down_up() {
        // MIDI Note (start, duration) → note-on @start / note-off @start+duration
        let mut tl = Timeline::new();
        tl.note(Key::W, 0.0, 1.0);
        let items = tl.build();
        assert_eq!(items.len(), 2);
        assert!(matches!(
            items[0],
            ScheduleItem::Down { at, key } if at == 0.0 && key == Key::W
        ));
        assert!(matches!(
            items[1],
            ScheduleItem::Up { at, key } if at == 1.0 && key == Key::W
        ));
    }

    #[test]
    fn build_sorts_unsorted_input() {
        let mut tl = Timeline::new();
        tl.note(Key::D, 150.0, 1.0);
        tl.note(Key::A, 50.0, 1.0);
        tl.note(Key::W, 0.0, 1.0);
        let items = tl.build();
        let ats: Vec<f64> = items.iter().map(ScheduleItem::at).collect();
        assert_eq!(ats, vec![0.0, 1.0, 50.0, 51.0, 150.0, 151.0]);
    }

    #[test]
    fn overlap_window_ordering() {
        // W 按住 0-200ms，A 50-150ms — 重叠窗口 100ms（多键重叠编排）
        let mut tl = Timeline::new();
        tl.note(Key::W, 0.0, 200.0);
        tl.note(Key::A, 50.0, 100.0);
        let items = tl.build();
        assert_eq!(items.len(), 4);
        assert!(matches!(items[0], ScheduleItem::Down { at, key } if at == 0.0 && key == Key::W));
        assert!(matches!(items[1], ScheduleItem::Down { at, key } if at == 50.0 && key == Key::A));
        assert!(matches!(items[2], ScheduleItem::Up { at, key } if at == 150.0 && key == Key::A));
        assert!(matches!(items[3], ScheduleItem::Up { at, key } if at == 200.0 && key == Key::W));
    }

    #[test]
    fn same_timestamp_down_before_up() {
        // tap：duration == 0 — 同刻先按下再松开
        let mut tl = Timeline::new();
        tl.note(Key::W, 0.0, 0.0);
        let items = tl.build();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], ScheduleItem::Down { .. }));
        assert!(matches!(items[1], ScheduleItem::Up { .. }));
    }

    #[test]
    fn negative_duration_clamped() {
        // 作者契约错误：duration < 0 — 钳制到 0（同刻按下松开），不 panic
        let mut tl = Timeline::new();
        tl.note(Key::W, 100.0, -50.0);
        let items = tl.build();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], ScheduleItem::Down { at, .. } if at == 100.0));
        assert!(matches!(items[1], ScheduleItem::Up { at, .. } if at == 100.0));
    }

    #[test]
    fn negative_at_kept() {
        // 负时刻条目保留在表中；执行端 ms_to_ticks 归零 → 播放起点立即触发
        let mut tl = Timeline::new();
        tl.at(-5.0, InputEvent::left_down());
        let items = tl.build();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], ScheduleItem::Event { at, .. } if at == -5.0));
    }

    #[test]
    fn mixed_note_and_generic_sorted() {
        // Note + 通用事件混合，整体按时刻排序
        let mut tl = Timeline::new();
        tl.at(60.0, InputEvent::left_down());
        tl.note(Key::W, 0.0, 200.0);
        tl.note(Key::A, 50.0, 100.0);
        let items = tl.build();
        assert_eq!(items.len(), 5);
        assert!(matches!(items[0], ScheduleItem::Down { key, .. } if key == Key::W));
        assert!(matches!(items[1], ScheduleItem::Down { key, .. } if key == Key::A));
        assert!(matches!(items[2], ScheduleItem::Event { at, .. } if at == 60.0));
        assert!(matches!(items[3], ScheduleItem::Up { key, .. } if key == Key::A));
        assert!(matches!(items[4], ScheduleItem::Up { key, .. } if key == Key::W));
    }

    // ── TimelinePlayerState — 播放中动态加入（MIDI 实时编辑，纯数据）──

    #[test]
    fn player_state_initial_sync_and_due() {
        let tl = Arc::new(Mutex::new(Timeline::new()));
        tl.lock().unwrap().note(Key::W, 0.0, 10.0);
        tl.lock().unwrap().note(Key::A, 50.0, 5.0);
        let state = TimelinePlayerState::new(tl);

        // 构造即首次同步：两音符全部展开（4 项）
        assert_eq!(state.items.borrow().len(), 4);

        // 播放起点（elapsed 0）：只有 at <= 0 的 W-down 到期
        let due = state.drain_due(delay::ms_to_ticks(0.0));
        assert_eq!(due.len(), 1);
        assert!(matches!(due[0], ScheduleItem::Down { key, .. } if key == Key::W));

        // 剩余 3 项：W-up@10, A-down@50, A-up@55；最近到期 10ms
        assert_eq!(state.items.borrow().len(), 3);
        assert_eq!(state.next_at(), 10.0);
    }

    #[test]
    fn player_state_sync_merges_appended_notes() {
        let tl = Arc::new(Mutex::new(Timeline::new()));
        tl.lock().unwrap().note(Key::W, 0.0, 10.0);
        let state = TimelinePlayerState::new(tl.clone());
        assert_eq!(state.items.borrow().len(), 2);

        // 播放中追加：新音符插到既有条目之间（MIDI 时间线编辑）
        tl.lock().unwrap().note(Key::A, 5.0, 3.0);
        state.sync();
        let ats: Vec<f64> = state.items.borrow().iter().map(ScheduleItem::at).collect();
        assert_eq!(ats, vec![0.0, 5.0, 8.0, 10.0]);

        // 幂等：无新条目时 sync 不重复展开
        state.sync();
        assert_eq!(state.items.borrow().len(), 4);
    }

    #[test]
    fn player_state_dynamic_append_caught_when_due() {
        // 关键语义：动态加入的条目在到期帧被 partition_point 捕获（不遗漏），
        // 已触发条目已被移除（不重复）
        let tl = Arc::new(Mutex::new(Timeline::new()));
        tl.lock().unwrap().note(Key::W, 0.0, 1.0);
        let state = TimelinePlayerState::new(tl.clone());

        // 播放起点：W-down 到期触发
        let due = state.drain_due(delay::ms_to_ticks(0.0));
        assert_eq!(due.len(), 1);
        assert!(matches!(due[0], ScheduleItem::Down { key, .. } if key == Key::W));

        // 播放中追加 10ms 的新音符（排在其后）— 与旧条目合并排序
        tl.lock().unwrap().note(Key::A, 10.0, 5.0);
        state.sync();

        // elapsed 5ms：W-up@1 积压到期（1ms ≤ 5ms），A-down@10 未到期
        let due = state.drain_due(delay::ms_to_ticks(5.0));
        assert_eq!(due.len(), 1);
        assert!(matches!(due[0], ScheduleItem::Up { key, .. } if key == Key::W));

        // 到期帧逐个触发：A-down@10 → A-up@15
        let due = state.drain_due(delay::ms_to_ticks(10.0));
        assert_eq!(due.len(), 1);
        assert!(matches!(due[0], ScheduleItem::Down { key, .. } if key == Key::A));
        let due = state.drain_due(delay::ms_to_ticks(15.0));
        assert_eq!(due.len(), 1);
        assert!(matches!(due[0], ScheduleItem::Up { key, .. } if key == Key::A));

        // 全部触发：表空 → next_at = INFINITY（播放完成信号）
        assert_eq!(state.items.borrow().len(), 0);
        assert!(state.next_at().is_infinite());
    }

    #[test]
    fn player_state_drain_never_duplicates() {
        // 同一 elapsed 重复调用：已移除条目不会再次触发
        let tl = Arc::new(Mutex::new(Timeline::new()));
        tl.lock().unwrap().note(Key::W, 0.0, 10.0);
        let state = TimelinePlayerState::new(tl);

        let once = state.drain_due(delay::ms_to_ticks(20.0));
        assert_eq!(once.len(), 2);
        assert!(state.items.borrow().is_empty());
        let twice = state.drain_due(delay::ms_to_ticks(20.0));
        assert!(twice.is_empty());
    }

    // ── RollingKeys — 参数与调度数学（纯数据，无需驱动）──

    #[test]
    fn rolling_keys_defaults_and_builders() {
        // 默认参数与 C++ 原版一致：间隔 50ms、按住 1ms
        let rk = RollingKeys::new();
        assert_eq!(rk.interval_ms, 50.0);
        assert_eq!(rk.duration_ms, 1.0);
        assert!(rk.keys.is_empty());

        let rk = rk.keys(vec![Key::W, Key::A]).interval(50.0).duration(199.0);
        assert_eq!(rk.keys.len(), 2);
        assert_eq!(rk.keys[0], Key::W);
        assert_eq!(rk.interval_ms, 50.0);
        assert_eq!(rk.duration_ms, 199.0);
    }

    #[test]
    fn rolling_schedule_pattern() {
        // 调度数学：按下序列无限均匀（每 interval 一拍）、键序列轮转、
        // 释放时刻 = 按下时刻 + duration。
        // 建模 RollingPlayer::play 的时间推进（相对播放起点，毫秒）：
        //   press_i   = i * interval
        //   release_i = i * interval + duration
        //   key_i     = keys[i % keys.len()]
        let keys = [Key::W, Key::A, Key::S, Key::D];
        let interval = 50.0f64;
        let duration = 199.0f64;
        let period = interval * keys.len() as f64; // 200ms

        let presses: Vec<(usize, f64, f64)> = (0..16)
            .map(|i| {
                (
                    i % keys.len(),
                    i as f64 * interval,
                    i as f64 * interval + duration,
                )
            })
            .collect();

        // 按下间隔严格均匀（含循环边界 — 动态排程无静态表边界缝隙）
        for pair in presses.windows(2) {
            assert_eq!(pair[1].1 - pair[0].1, interval);
        }
        // 键序列轮转
        assert_eq!(presses[0].0, 0);
        assert_eq!(presses[3].0, 3);
        assert_eq!(presses[4].0, 0);
        // 释放 = 按下 + 按住时长；同键周期 > 按住时长 → 同键不重叠
        for &(_, press, release) in &presses {
            assert_eq!(release - press, duration);
        }
        assert!(period > duration, "按住时长不得超过按键轮转周期");
    }
}
