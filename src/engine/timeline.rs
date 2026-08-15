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
//! - [`TimelinePlayer`]：静态/动态表 — 事件可构造时展开或播放中动态加入；
//!   **表空不结束**（live-edit 语义：0.5ms 轮询等待编辑器追加，结束只由
//!   `stop_requested` 决定）；回放前缀通知编辑器清理（内存有界）
//! - [`RollingKeys`] / [`RollingPlayer`]：节奏滚动 — 按下序列按间隔无限
//!   均匀推进，释放事件按下时动态排程，无静态表边界缝隙
//!
//! 对应时间轴调度器设计（原 NEXT_STEPS §3，2026-08 并入 CLAUDE.md 决策表）。
//! The second event-orchestration paradigm alongside `EventSequence`:
//! each event carries an absolute time relative to playback start, and
//! multi-key holds can overlap naturally.

use crate::engine::event::InputEvent;
use crate::interception::ffi::{INTERCEPTION_KEY_E0, INTERCEPTION_KEY_UP};
use crate::interception::SendContext;
use crate::key::Key;
use crate::utils::delay;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

/// 时间轴可表达的最大时刻（24 小时，毫秒）。+inf 在展开时钳制到此值 —
/// 防 deadline 换算溢出与排序谓词非单调（NaN 按 `total_cmp` 排在 +inf 后，
/// 但 `ms_to_ticks(NaN) = 0`，会破坏 `partition_point` 的单调性假设）。
const MAX_AT_MS: f64 = 86_400_000.0;

/// 表空/长等待时的轮询窗口（毫秒）— 编辑器追加的条目至多延迟一个窗口被
/// 捕获；忙等空转成本可忽略（~2kHz 的 TSC 比较 + pause）。
const POLL_WINDOW_MS: f64 = 0.5;

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
    ///
    /// 携带 `InputEvent::Keyboard` 的 At 事件**同样参与挂起键清理**（press 入队、
    /// release 出队，停止时与 Note 一致补发 release）——键盘按住推荐用 [`Note`]，
    /// 但 At 键盘事件不会卡键。携带 `InputEvent::Sleep` 是静默 no-op：时间轴的
    /// 时序由 `at` 承担，Sleep 在此没有意义，请勿使用。
    At { at: f64, event: InputEvent },
}

/// 时间轴 — 数据驱动的事件表（MIDI 编辑器的时间线）。
/// 与 EventSequence（链式 builder）相反：这里只登记带绝对时刻的条目，
/// 方法返回 `()` 刻意不支持链式调用。
///
/// 以 `Arc<Mutex<Timeline>>` 与 [`TimelinePlayer`] 共享时，**播放中可动态加入
/// 事件**（MIDI 实时编辑）：持锁调用 `note`/`at`，播放器增量同步。
/// 已完全回放的前缀由播放器经 [`remove_played_prefix`](Self::remove_played_prefix)
/// 通知移除（长会话内存有界）。
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

    /// 播放器回放通知 — 移除最前 `n` 个已被完全回放的条目。
    ///
    /// 由播放器状态机在每次触发后按前缀语义调用（编辑器不应自行调用）：
    /// 长会话 live-edit 下条目表保持有界。非前缀的已回放条目不会被移除 —
    /// 等待其前方条目完成后随前缀一并清理（滑动窗口语义）。
    pub(crate) fn remove_played_prefix(&mut self, n: usize) {
        self.entries.drain(..n);
    }

    /// 展开 + 排序（一次）。panic=abort 下用 total_cmp 保证 f64 全序，不触发排序 panic。
    fn build(&self) -> Vec<ScheduleItem> {
        let mut items = expand(&self.entries, 0);
        sort_items(&mut items);
        items
    }
}

/// 非有限时刻钳制（展开时单点执行，build/sync 两条路径一致）：
/// NaN/-inf → 0（立即触发，作者错误即刻暴露）；+inf → [`MAX_AT_MS`]（远未来）。
/// 有限值（含负数）原样保留 — 负时刻语义：执行端 `ms_to_ticks` 归零 → 播放起点立即触发。
#[inline]
fn sanitize_at(at: f64) -> f64 {
    if at.is_nan() || at == f64::NEG_INFINITY {
        0.0
    } else if at == f64::INFINITY {
        MAX_AT_MS
    } else {
        at
    }
}

/// 展开时间轴条目为调度项（私有）。`from` 为条目编号起点（会话级绝对索引），
/// 调用方传入已切好的 slice，本函数不再切片。Note → Down@start / Up@start+duration；
/// 负时长钳制为 0（同刻按下松开，不 panic）。每个调度项带源条目索引 `entry`，
/// 与 base 配合定位回放进度。
fn expand(entries: &[TimelineEvent], from: usize) -> Vec<ScheduleItem> {
    let mut items = Vec::with_capacity(entries.len() * 2);
    for (i, e) in entries.iter().enumerate() {
        let entry = from + i;
        match *e {
            TimelineEvent::Note {
                key,
                start,
                duration,
            } => {
                let duration = duration.max(0.0);
                items.push(ScheduleItem::Down {
                    at: sanitize_at(start),
                    key,
                    entry,
                });
                items.push(ScheduleItem::Up {
                    at: sanitize_at(start + duration),
                    key,
                    entry,
                });
            }
            TimelineEvent::At { at, event } => {
                items.push(ScheduleItem::Event {
                    at: sanitize_at(at),
                    event,
                    entry,
                });
            }
        }
    }
    items
}

/// 展开后的调度项 — 已排序。私有，仅执行器与单测可见。
/// `entry` 为源条目在共享时间轴中的会话级绝对索引。
#[derive(Debug, Clone, Copy)]
enum ScheduleItem {
    Down { at: f64, key: Key, entry: usize },
    Up { at: f64, key: Key, entry: usize },
    Event { at: f64, event: InputEvent, entry: usize },
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

    #[inline]
    fn entry(&self) -> usize {
        match self {
            ScheduleItem::Down { entry, .. }
            | ScheduleItem::Up { entry, .. }
            | ScheduleItem::Event { entry, .. } => *entry,
        }
    }
}

/// 排序比较器 — 时刻升序，同刻按优先级。build()（全量）与 sync()（增量归并）
/// 共用同一比较器，两条管线的排序契约不会漂移。
#[inline]
fn cmp_items(a: &ScheduleItem, b: &ScheduleItem) -> Ordering {
    a.at().total_cmp(&b.at()).then(a.priority().cmp(&b.priority()))
}

fn sort_items(items: &mut [ScheduleItem]) {
    // sort_by 按 std 契约是稳定排序（sort_unstable_by 才不稳定）—
    // 同刻同优先级条目（和弦的多个 Down）保持插入序，是作曲契约的一部分。
    items.sort_by(cmp_items);
}

/// 已排序两段归并（sync 增量路径）：新块 `m` 项排序后与存量 `n` 项归并，
/// O(m log m + n) — 每帧全量重排 O(n log n) 的退化由此避免。
fn merge_sorted(existing: &mut Vec<ScheduleItem>, fresh: &[ScheduleItem]) {
    let mut merged = Vec::with_capacity(existing.len() + fresh.len());
    let (mut i, mut j) = (0, 0);
    while i < existing.len() && j < fresh.len() {
        if cmp_items(&existing[i], &fresh[j]) != Ordering::Greater {
            merged.push(existing[i]);
            i += 1;
        } else {
            merged.push(fresh[j]);
            j += 1;
        }
    }
    merged.extend_from_slice(&existing[i..]);
    merged.extend_from_slice(&fresh[j..]);
    *existing = merged;
}

/// MIDI 播放器状态机 — 纯数据层（同步/到期触发决策，不触碰发送）。
///
/// 与 [`Timeline`] 通过 `Arc<Mutex<Timeline>>` 共享，**播放中可实时编辑**
/// （MIDI 编辑器语义）：其他线程持锁调用 `note`/`at` 动态加入事件，
/// 状态机增量同步（`sync`）并归并进有序表；**已触发条目即从表中移除**
/// （触发指针恒为表首），新条目在到期帧被 `partition_point` 捕获，
/// 不重复、不遗漏。
///
/// **回放清理通知**：状态机每次触发后计算"完全回放的最长前缀"，调用
/// [`Timeline::remove_played_prefix`] 通知编辑器（Timeline 即编辑器的共享
/// 数据对象）移除这些条目 — 长时间 live-edit 会话内存有界。前缀之后已
/// 回放的条目等待其前方长音符结束一并移除（滑动窗口语义，非前缀不删）。
///
/// 私有（模块内测试可直达）：公开形态是持有发送上下文的 [`TimelinePlayer`]。
struct TimelinePlayerState {
    timeline: Arc<Mutex<Timeline>>,
    /// 有序展开项缓存 — 恒为未触发条目（已触发即移除）。
    items: RefCell<Vec<ScheduleItem>>,
    /// 已展开但尚未完全回放的条目数。
    seen: RefCell<usize>,
    /// 已从共享时间轴移除的前缀长度（会话级绝对索引的平移基准）。
    base: RefCell<usize>,
    /// 每个已展开条目的未触发项数（Note=2, At=1），与当前 `entries` 一一对应。
    /// 全零即该条目完全回放（可进清理前缀）。
    remaining: RefCell<Vec<u16>>,
}

impl TimelinePlayerState {
    /// 绑定共享时间轴，构造时执行首次增量同步。
    fn new(timeline: Arc<Mutex<Timeline>>) -> Self {
        let state = Self {
            timeline,
            items: RefCell::new(Vec::new()),
            seen: RefCell::new(0),
            base: RefCell::new(0),
            remaining: RefCell::new(Vec::new()),
        };
        state.sync();
        state
    }

    /// 增量同步：把共享时间轴的新条目展开、排序后归并进有序列表。
    ///
    /// 锁内只做 `entries.len()` 读取与快照复制；展开/排序/归并在锁外进行
    /// （锁内排序会阻塞编辑器线程一整帧）。毒锁视为可恢复 — 取回内容。
    fn sync(&self) {
        let new_entries = {
            let tl = self.timeline.lock().unwrap_or_else(|p| p.into_inner());
            let base = *self.base.borrow();
            let seen = *self.seen.borrow();
            if tl.entries.len() <= base + seen {
                return;
            }
            tl.entries[base + seen..].to_vec()
        };
        let from = *self.base.borrow() + *self.seen.borrow();
        let mut fresh = expand(&new_entries, from);
        sort_items(&mut fresh);
        merge_sorted(&mut self.items.borrow_mut(), &fresh);
        *self.seen.borrow_mut() += new_entries.len();
        self.remaining
            .borrow_mut()
            .extend(new_entries.iter().map(item_count));
    }

    /// 取出已到期（at <= 相对播放起点 ticks 进度）的展开项，并从表中移除。
    /// 到期判断与等待共用同一换算公式（`ms_to_ticks`），无双重换算误差。
    ///
    /// 同时维护回放进度：递减各条目剩余项数，并将完全回放的前缀从共享
    /// 时间轴移除（回放清理通知），`base`/`seen`/`remaining` 同步平移 —
    /// 增量同步的索引不变量保持成立。
    fn drain_due(&self, elapsed_ticks: u64) -> Vec<ScheduleItem> {
        let due = {
            let mut items = self.items.borrow_mut();
            let upto = items.partition_point(|it| delay::ms_to_ticks(it.at()) <= elapsed_ticks);
            items.drain(..upto).collect::<Vec<_>>()
        };

        // 回放进度：每个触发项递减所属条目的剩余项数
        {
            let base = *self.base.borrow();
            let mut remaining = self.remaining.borrow_mut();
            for it in &due {
                let idx = it.entry() - base;
                remaining[idx] = remaining[idx].saturating_sub(1);
            }
        }

        // 完全回放的前缀 → 通知编辑器（Timeline）移除，索引基准同步平移
        let watermark = {
            let remaining = self.remaining.borrow();
            remaining.iter().take_while(|r| **r == 0).count()
        };
        if watermark > 0 {
            self.timeline
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove_played_prefix(watermark);
            *self.base.borrow_mut() += watermark;
            *self.seen.borrow_mut() -= watermark;
            self.remaining.borrow_mut().drain(..watermark);
        }
        due
    }

    /// 最近到期时刻（毫秒）；空表返回 INFINITY（无到期条目的等待信号）。
    fn next_at(&self) -> f64 {
        self.items
            .borrow()
            .first()
            .map_or(f64::INFINITY, ScheduleItem::at)
    }
}

/// 条目的调度项数量：Note → Down+Up（2），At → 单事件（1）。
fn item_count(e: &TimelineEvent) -> u16 {
    match e {
        TimelineEvent::Note { .. } => 2,
        TimelineEvent::At { .. } => 1,
    }
}

/// 计算本轮忙等目标（绝对 TSC 时刻）— 纯函数，可单测。
///
/// 目标 = min(事件到期时刻, 轮询窗口上限)：
/// - 事件到期 = 播放起点 + 条目偏移（**饱和加法** — 超大 `at` 换算出的
///   ticks 饱和到 u64::MAX 也不会回绕成过去时刻，杜绝忙等死循环）
/// - 轮询窗口 = 现在 + [`POLL_WINDOW_MS`]（表空/超大 at 时每窗口醒一次，
///   重新 sync 捕获编辑器追加的条目 — 追加至多延迟一个窗口）
fn wait_deadline(start_ticks: u64, next_at: f64, now_ticks: u64) -> u64 {
    let event_deadline = start_ticks.saturating_add(delay::ms_to_ticks(next_at));
    let poll_cap = now_ticks.saturating_add(delay::ms_to_ticks(POLL_WINDOW_MS));
    event_deadline.min(poll_cap)
}

/// 时间轴执行器 — MIDI 播放器：状态机（[`TimelinePlayerState`]）+ 发送上下文。
///
/// 退出时保证无挂起按键：停止时补发 release（活动音符 note-off，含 At 键盘
/// 事件）。表空不结束 — 编辑器追加新条目后播放继续；结束只由 `stop_requested`
/// 决定（MIDI 编辑器语义：播放器永不自杀）。
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

    /// 持续播放（live-edit），直到 `stop_requested` 置位。退出时保证无挂起按键。
    ///
    /// 表空时以 0.5ms 轮询等待编辑器追加（追加条目至多延迟一个窗口被捕获）；
    /// 表非空时忙等到最近到期时刻（等待同样以轮询窗口封顶 — 等待期间追加的
    /// 更早时刻条目在窗口内被发现，不会晚一整个等待长度）。
    ///
    /// 时间语义：deadline 是绝对 TSC 时刻（播放起点 + 条目偏移，饱和加法），
    /// 无累计漂移——前序条目耗时超标时后续条目自然"追时"，时间轴整体对齐
    /// 播放起点。动态加入的条目同样按播放起点换算（MIDI 时间线对拍）。
    pub fn play(&self, stop_requested: &AtomicBool) {
        if stop_requested.load(AtomicOrdering::Acquire) {
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
                if stop_requested.load(AtomicOrdering::Acquire) {
                    break;
                }
                match item {
                    ScheduleItem::Down { key, .. } => {
                        self.send_ctx.send_event(&InputEvent::press(key));
                        pending.push(key);
                    }
                    ScheduleItem::Up { key, .. } => {
                        self.send_ctx.send_event(&InputEvent::release(key));
                        remove_pending(&mut pending, key);
                    }
                    ScheduleItem::Event { event, .. } => {
                        self.send_ctx.send_event(&event);
                        // At 键盘事件同样参与挂起键清理（防卡键）
                        track_keyboard(&mut pending, &event);
                    }
                }
            }

            // 3. 停止请求 → 补发挂起键后退出（最坏停止延迟 ~100μs + 本帧突发发送）
            if stop_requested.load(AtomicOrdering::Acquire) {
                self.release_pending(&mut pending);
                return;
            }

            // 4. 忙等到 min(最近到期时刻, 轮询窗口) — 表空时每窗口醒一次等待追加
            let now = delay::tsc_now();
            let target = wait_deadline(start, self.state.next_at(), now);
            delay::wait_until_interruptible(target, stop_requested);
        }
    }

    /// 补发挂起键的 release 并清空列表（MIDI 活动音符 note-off）。
    fn release_pending(&self, pending: &mut Vec<Key>) {
        for key in pending.drain(..) {
            self.send_ctx.send_event(&InputEvent::release(key));
        }
    }
}

/// At 键盘事件的挂起键跟踪 — 与 Note 同等地参与防卡键清理：
/// press 入队、release 出队（E0 标志从 state 恢复）。
fn track_keyboard(pending: &mut Vec<Key>, event: &InputEvent) {
    if let InputEvent::Keyboard { code, state } = event {
        let key = Key {
            code: *code,
            is_e0: *state & INTERCEPTION_KEY_E0 != 0,
        };
        if *state & INTERCEPTION_KEY_UP == 0 {
            pending.push(key);
        } else {
            remove_pending(pending, key);
        }
    }
}

/// 从挂起集中移除键 — find-first + swap_remove（查找 O(n)，移除 O(1)）。
/// 同键重叠违反契约时只移除一个挂起标记（作者错误不被静默掩盖）。
fn remove_pending(pending: &mut Vec<Key>, key: Key) {
    if let Some(pos) = pending.iter().position(|k| *k == key) {
        pending.swap_remove(pos);
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
    /// 非正值/NaN 钳制为 0.5ms（防零间隔输入风暴，作者错误静默兜底）。
    pub fn interval(mut self, ms: f64) -> Self {
        self.interval_ms = ms.max(0.5);
        self
    }

    /// 设置按住时长（毫秒）— 每个键按满这么久后释放。
    /// 非正值/NaN 钳制为 0（同刻按下松开）。
    pub fn duration(mut self, ms: f64) -> Self {
        self.duration_ms = ms.max(0.0);
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
    /// 每轮循环：停止检查 → 到期释放 → 到期按下 → 忙等到最近到期时刻。
    /// 停止检查在循环顶部 — stop 置位后不再有任何按下（含等待窗口期）。
    /// 按下序列按绝对节奏推进（`next_press += interval`，无累计漂移）；
    /// 卡顿后节拍重锚于当前时刻（`next_press = max(next_press, now) + interval`）：
    /// 错过的拍子即弃、不突发追拍（突发 = 同刻多键齐按，游戏手感最差；
    /// 有意偏离 C++ 原版的逐循环补按追拍行为）。释放锚定实际按下时刻，
    /// 排出队列 FIFO（按住时长恒定 → 到期顺序 = 按下顺序）。
    pub fn play(&self, stop_requested: &AtomicBool) {
        if self.keys.is_empty() || stop_requested.load(AtomicOrdering::Acquire) {
            return;
        }
        let interval = delay::ms_to_ticks(self.interval_ms);
        let duration = delay::ms_to_ticks(self.duration_ms);

        // 节奏器状态：下次按下时刻（绝对 ticks）、键序列索引
        let mut next_press = delay::tsc_now();
        let mut key_idx = 0usize;
        // 动态排程的释放队列：(到期时刻, 键)，FIFO，队首即最早到期
        let mut releases: VecDeque<(u64, Key)> = VecDeque::new();

        loop {
            let now = delay::tsc_now();

            // 0. 停止优先于一切发送：stop 置位 → 补发所有挂起释放后退出。
            //    （等待期 stop 在 ~100μs 检查点返回后由本检查兜底，不会多发按键）
            if stop_requested.load(AtomicOrdering::Acquire) {
                for (_, key) in releases.drain(..) {
                    self.send_ctx.send_event(&InputEvent::release(key));
                }
                return;
            }

            // 1. 到期释放（先释放后按下，同刻语义与静态表 Down<Event<Up 一致）
            while let Some(&(due, key)) = releases.front() {
                if due > now {
                    break;
                }
                releases.pop_front();
                self.send_ctx.send_event(&InputEvent::release(key));
            }

            // 2. 到期按下 — 动态加入按下事件，并排程其释放。
            //    节拍重锚：卡顿错过的拍子即弃，不突发追拍。
            if now >= next_press {
                let key = self.keys[key_idx % self.keys.len()];
                self.send_ctx.send_event(&InputEvent::press(key));
                // 释放锚定实际按下时刻（而非循环顶部 now — 本迭代已先排空到期释放）
                releases.push_back((delay::tsc_now().saturating_add(duration), key));
                next_press = next_press.max(now).saturating_add(interval);
                key_idx += 1;
            }

            // 3. 忙等到最近的到期时刻（下次按下或最早释放）
            let earliest_release = releases.front().map_or(u64::MAX, |r| r.0);
            let target = next_press.min(earliest_release);
            if now < target {
                delay::wait_until_interruptible(target, stop_requested);
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────
// 数据管线（展开/排序/钳制/增量同步/到期触发/前缀清理/等待目标换算）
// 完全脱离驱动环境；TimelinePlayerState 直接可达（模块内），
// 不调用 SendContext::create。

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
            ScheduleItem::Down { at, key, .. } if at == 0.0 && key == Key::W
        ));
        assert!(matches!(
            items[1],
            ScheduleItem::Up { at, key, .. } if at == 1.0 && key == Key::W
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
        assert!(matches!(items[0], ScheduleItem::Down { at, key, .. } if at == 0.0 && key == Key::W));
        assert!(matches!(items[1], ScheduleItem::Down { at, key, .. } if at == 50.0 && key == Key::A));
        assert!(matches!(items[2], ScheduleItem::Up { at, key, .. } if at == 150.0 && key == Key::A));
        assert!(matches!(items[3], ScheduleItem::Up { at, key, .. } if at == 200.0 && key == Key::W));
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
        // 有限负时刻条目保留在表中；执行端 ms_to_ticks 归零 → 播放起点立即触发
        let mut tl = Timeline::new();
        tl.at(-5.0, InputEvent::left_down());
        let items = tl.build();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], ScheduleItem::Event { at, .. } if at == -5.0));
    }

    #[test]
    fn nonfinite_at_sanitized() {
        // NaN/-inf → 0（立即触发）；+inf → MAX_AT_MS（远未来）—
        // 消除 partition_point 谓词非单调（NaN 按 total_cmp 排在 +inf 后，
        // 但 ms_to_ticks(NaN)=0 会让谓词呈 T,F,T 锯齿）
        let mut tl = Timeline::new();
        tl.at(f64::NAN, InputEvent::left_down());
        tl.at(f64::NEG_INFINITY, InputEvent::left_up());
        tl.at(f64::INFINITY, InputEvent::right_down());
        let items = tl.build();
        assert!(matches!(items[0], ScheduleItem::Event { at, .. } if at == 0.0));
        assert!(matches!(items[1], ScheduleItem::Event { at, .. } if at == 0.0));
        assert!(matches!(items[2], ScheduleItem::Event { at, .. } if at == MAX_AT_MS));
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

        // elapsed 5ms：W-up@1 积压到期（1ms ≤ 5ms），A-down@10 未到期。
        // W 完全回放 → 前缀清理通知编辑器：entries 只剩 A
        let due = state.drain_due(delay::ms_to_ticks(5.0));
        assert_eq!(due.len(), 1);
        assert!(matches!(due[0], ScheduleItem::Up { key, .. } if key == Key::W));
        assert_eq!(tl.lock().unwrap().entries.len(), 1);

        // 到期帧逐个触发：A-down@10 → A-up@15
        let due = state.drain_due(delay::ms_to_ticks(10.0));
        assert_eq!(due.len(), 1);
        assert!(matches!(due[0], ScheduleItem::Down { key, .. } if key == Key::A));
        let due = state.drain_due(delay::ms_to_ticks(15.0));
        assert_eq!(due.len(), 1);
        assert!(matches!(due[0], ScheduleItem::Up { key, .. } if key == Key::A));

        // 全部回放：表空 → next_at = INFINITY，共享时间轴也被清空
        assert_eq!(state.items.borrow().len(), 0);
        assert!(state.next_at().is_infinite());
        assert!(tl.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn player_state_prefix_cleanup_waits_for_long_note() {
        // 滑动窗口语义：短音符（A）先完全回放，但前缀被长音符（W）挡住 —
        // A 不被提前移除；W 完成后两条目随前缀一并清理
        let tl = Arc::new(Mutex::new(Timeline::new()));
        tl.lock().unwrap().note(Key::W, 0.0, 200.0);
        tl.lock().unwrap().note(Key::A, 50.0, 100.0);
        let state = TimelinePlayerState::new(tl.clone());

        state.drain_due(delay::ms_to_ticks(0.0)); // W-down
        state.drain_due(delay::ms_to_ticks(50.0)); // A-down
        state.drain_due(delay::ms_to_ticks(150.0)); // A-up — A 完全回放
        assert_eq!(tl.lock().unwrap().entries.len(), 2, "前缀被 W 挡住，不提前清理");

        state.drain_due(delay::ms_to_ticks(200.0)); // W-up — W 完全回放
        assert!(tl.lock().unwrap().entries.is_empty(), "前缀整体清理");
    }

    #[test]
    fn player_state_drain_never_duplicates() {
        // 同一 elapsed 重复调用：已移除条目不会再次触发
        let tl = Arc::new(Mutex::new(Timeline::new()));
        tl.lock().unwrap().note(Key::W, 0.0, 10.0);
        let state = TimelinePlayerState::new(tl.clone());

        let once = state.drain_due(delay::ms_to_ticks(20.0));
        assert_eq!(once.len(), 2);
        assert!(state.items.borrow().is_empty());
        let twice = state.drain_due(delay::ms_to_ticks(20.0));
        assert!(twice.is_empty());
        assert!(tl.lock().unwrap().entries.is_empty());
    }

    // ── 等待目标换算（纯函数，防回绕/防空转）──

    #[test]
    fn wait_deadline_uses_event_deadline_when_soon() {
        // 正常条目：目标 = 播放起点 + 条目偏移（未触达轮询上限）
        let start = 1_000_000u64;
        let now = start;
        let target = wait_deadline(start, 0.1, now);
        assert_eq!(target, start.saturating_add(delay::ms_to_ticks(0.1)));
    }

    #[test]
    fn wait_deadline_polls_when_empty_or_huge() {
        // 表空（INFINITY）/超大 at：退化为轮询窗口，等待编辑器追加
        let start = 1_000_000u64;
        let now = start;
        let poll = now.saturating_add(delay::ms_to_ticks(POLL_WINDOW_MS));
        assert_eq!(wait_deadline(start, f64::INFINITY, now), poll);
        assert_eq!(wait_deadline(start, 4.0e12, now), poll);
    }

    #[test]
    fn wait_deadline_never_wraps() {
        // 播放起点接近 u64::MAX：饱和加法不回绕成过去时刻（无忙等死循环）
        let start = u64::MAX - 100;
        let now = start;
        let target = wait_deadline(start, 0.5, now);
        assert!(target >= now, "目标不得回绕到过去：{} < {}", target, now);
    }

    // ── 挂起键跟踪（At 键盘事件 + swap_remove 语义）──

    #[test]
    fn track_keyboard_press_release_roundtrip() {
        let mut pending = Vec::new();
        track_keyboard(&mut pending, &InputEvent::press(Key::W));
        track_keyboard(&mut pending, &InputEvent::press(Key::A));
        assert_eq!(pending, vec![Key::W, Key::A]);

        track_keyboard(&mut pending, &InputEvent::release(Key::W));
        assert_eq!(pending, vec![Key::A]);

        // 重复 release（契约外）不移除无关键；swap_remove 只移除第一个匹配
        track_keyboard(&mut pending, &InputEvent::release(Key::W));
        assert_eq!(pending, vec![Key::A]);
    }

    #[test]
    fn remove_pending_removes_first_occurrence_only() {
        let mut pending = vec![Key::W, Key::A, Key::W];
        remove_pending(&mut pending, Key::W);
        assert_eq!(pending.len(), 2, "只移除一个挂起标记，契约错误不被静默掩盖");
        assert!(pending.contains(&Key::W));
    }

    // ── 归并排序（sync 增量路径）──

    #[test]
    fn merge_sorted_interleaves_with_priority() {
        // 存量 [W-D@0, W-U@10] 与新块 [A-D@5, A-U@10] 归并 —
        // 同刻（10ms）按 Down<Event<Up 优先级排列
        let mut existing = vec![
            ScheduleItem::Down { at: 0.0, key: Key::W, entry: 0 },
            ScheduleItem::Up { at: 10.0, key: Key::W, entry: 0 },
        ];
        let fresh = vec![
            ScheduleItem::Down { at: 5.0, key: Key::A, entry: 1 },
            ScheduleItem::Up { at: 10.0, key: Key::A, entry: 1 },
        ];
        merge_sorted(&mut existing, &fresh);
        let ats: Vec<(f64, u8)> = existing.iter().map(|i| (i.at(), i.priority())).collect();
        assert_eq!(ats, vec![(0.0, 0), (5.0, 0), (10.0, 2), (10.0, 2)]);
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
    fn rolling_keys_clamps_invalid_params() {
        // 零/负间隔与 NaN 钳制到 0.5ms（防零间隔输入风暴）；
        // 负时长与 NaN 钳制到 0（同刻按下松开）
        let rk = RollingKeys::new().interval(0.0).interval(-3.0).interval(f64::NAN);
        assert_eq!(rk.interval_ms, 0.5);
        let rk = RollingKeys::new().duration(-1.0).duration(f64::NAN);
        assert_eq!(rk.duration_ms, 0.0);
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
