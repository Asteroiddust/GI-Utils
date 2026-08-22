//! 热线程 pinning — 把游戏最重的线程钉在金银核上。
//!
//! 决策数据：2026-08-22 三次采样（YuanShen.exe，轻载/地图/主城压力）。
//! 核心结论（见 CLAUDE.md 设计决策表）：
//! - 断崖分布 → 只 pin 域内 Top-2（主循环 + MMCSS 关键路径），一金核一住户；
//! - NVIDIA prio-31 呈现线程是第三消费者但**模块过滤**排除（驱动线程不可控）；
//! - Job 池 worker（500 切换/秒的迁移型）一律不 pin；
//! - Ideal CPU 轮转散布证明调度器不优待热线程 — pinning 增量真实。
//!
//! 策略按**进程映像名**注册（新游戏 = 加一行 + 一次采样数据）；
//! 进程信息新鲜度：同 pid 且已有 pin 仍存活 → 沿用；否则（首次/换游戏/
//! 线程死亡）还原旧 pin 后重新采样映射。

use crate::utils::affinity;
use crate::utils::thread_info;
use std::sync::Mutex;
use std::sync::PoisonError;
use tracing::{info, warn};

// ═══════════════════════════════════════════════════════════════════
// 策略注册表 — 进程名 → pinning 参数
// ═══════════════════════════════════════════════════════════════════

/// 单个进程的 pinning 策略。
pub struct PinStrategy {
    /// 进程映像名（策略键，大小写不敏感匹配）。
    pub process_name: &'static str,
    /// 候选域内按 Δcycles 降序取前 N 条（金核住户上限）。
    pub top_n: usize,
    /// 第 i 热线程的目标掩码（LP 对）— 与 top_n 取 min 生效。
    pub masks: &'static [usize],
    /// 策略依据（采样数据说明），日志与维护用。
    pub note: &'static str,
}

/// 策略注册表。新游戏：先跑「线程采样」（F20）拿数据，再加一行。
/// 没有策略的进程 → pinning 步骤静默跳过，「优化游戏」其余步骤
/// （优先级 HIGH + OTHER 隔离 + 前台切换）不受影响 — **这即是保底
/// 策略：未注册 = 只调优先级不作线程绑定**。
///
/// Endfield.exe（终末地）**有意不注册**（2026-08-22）：其反作弊封锁
/// 模块枚举（ACCESS_DENIED，管理员 procexp 亦然）→ MainModule 规则
/// 结构性不可用；PriorityBand(5..15) 评估方案已弃（热池身份仅优先级
/// 签名推断、无地址实证，Top-2 之一是 23 条 Unity Job 池内不可归属的
/// 线程）→ 保底运行。线程句柄查询侧全绿（223/223），未来若获得模块
/// 实证（如 procexp 驱动侧基址标定）可重启评估。
const STRATEGIES: &[PinStrategy] = &[PinStrategy {
    process_name: "YuanShen.exe",
    top_n: 2,
    masks: &[affinity::GOLDEN_A_LP_PAIR, affinity::GOLDEN_B_LP_PAIR],
    note: "Top-2（主循环+关键路径，Δcycles 现场排名定 A/B — 场景可互换）；\
           NVIDIA prio-31 与 Job 池由规则排除（2026-08-22 四窗口数据）",
}];

/// 按进程名查策略（大小写不敏感）。
fn lookup_strategy(process_name: &str) -> Option<&'static PinStrategy> {
    STRATEGIES
        .iter()
        .find(|s| s.process_name.eq_ignore_ascii_case(process_name))
}

/// 已 pin 的单线程记录。
struct PinnedThread {
    tid: u32,
    /// pin 前的原掩码（还原用）。
    old_mask: usize,
    /// 目标掩码（日志用）。
    target_mask: usize,
}

/// 当前 pinning 状态 — 进程级共享（静态 Mutex，跨「优化游戏」实例
/// 重建存活，与 OPTIMIZE_TOGGLE 同理）。
struct PinState {
    pid: u32,
    process_name: String,
    pinned: Vec<PinnedThread>,
}

static PIN_STATE: Mutex<Option<PinState>> = Mutex::new(None);

fn with_state<T>(f: impl FnOnce(&mut Option<PinState>) -> T) -> T {
    let mut guard = PIN_STATE.lock().unwrap_or_else(PoisonError::into_inner);
    f(&mut guard)
}

// ═══════════════════════════════════════════════════════════════════
// 线程归属校验与 pin/还原原语
// ═══════════════════════════════════════════════════════════════════

/// 线程是否存活且仍属于 `pid`（防 TID 复用误伤他进程）。
fn thread_owned_by(tid: u32, pid: u32) -> bool {
    use windows::Win32::System::Threading::{
        GetProcessIdOfThread, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
    };
    // GetProcessIdOfThread 返回 pid；失败返回 0（win32 语义，非 Result）
    unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, tid) }
        .map(|h| {
            let owner = unsafe { GetProcessIdOfThread(h) };
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(h) };
            owner != 0 && owner == pid
        })
        .unwrap_or(false)
}

/// 把线程 pin 到 `new_mask`，返回原掩码。
/// 需要 `THREAD_SET_INFORMATION`（pinning 生死题 — 被拒返回 None 降级）。
/// SetThreadAffinityMask 返回原掩码；失败返回 0（win32 语义，非 Result）。
fn pin_thread(tid: u32, new_mask: usize) -> Option<usize> {
    use windows::Win32::System::Threading::{
        OpenThread, SetThreadAffinityMask, THREAD_QUERY_LIMITED_INFORMATION, THREAD_SET_INFORMATION,
    };
    let handle = match unsafe {
        OpenThread(
            THREAD_SET_INFORMATION | THREAD_QUERY_LIMITED_INFORMATION,
            false,
            tid,
        )
    } {
        Ok(h) => h,
        Err(e) => {
            warn!("pinning: TID {tid} OpenThread 被拒（SET 权限？）: {e}");
            return None;
        }
    };
    let prev = unsafe { SetThreadAffinityMask(handle, new_mask) };
    let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
    if prev != 0 {
        Some(prev)
    } else {
        let e = windows::core::Error::from_thread();
        warn!("pinning: TID {tid} SetThreadAffinityMask 失败: {e}");
        None
    }
}

/// 还原单线程掩码（线程死亡/易主则静默跳过）。
fn restore_thread(tid: u32, pid: u32, old_mask: usize) -> bool {
    use windows::Win32::System::Threading::{
        OpenThread, SetThreadAffinityMask, THREAD_QUERY_LIMITED_INFORMATION, THREAD_SET_INFORMATION,
    };
    if !thread_owned_by(tid, pid) {
        return false;
    }
    unsafe {
        OpenThread(
            THREAD_SET_INFORMATION | THREAD_QUERY_LIMITED_INFORMATION,
            false,
            tid,
        )
    }
    .map(|h| {
        // 返回原掩码（= pin 后的 target）；0 = 失败
        let reverted = unsafe { SetThreadAffinityMask(h, old_mask) } != 0;
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(h) };
        reverted
    })
    .unwrap_or(false)
}

// ═══════════════════════════════════════════════════════════════════
// 候选域：双采样差分 + 主模块过滤 + 排序
// ═══════════════════════════════════════════════════════════════════

/// 差分排序条目（pinning 只需的子集 — 完整画像走「线程采样」）。
struct Ranked {
    tid: u32,
    cycles_delta: u64,
    cpu_pct: f64,
    /// 起始地址符号化结果（"YuanShen.exe+0x..."）— 模块过滤键。
    start_resolved: String,
}

/// 双采样差分：Δcycles 降序（无 cycles 数据按 CPU% 兜底比较）。
fn sample_ranked(pid: u32, interval_ms: u64) -> Result<Vec<Ranked>, String> {
    let s1 = thread_info::snapshot_threads(pid)?;
    let started = std::time::Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(interval_ms));
    let s2 = thread_info::snapshot_threads(pid)?;
    let interval_100ns = started.elapsed().as_nanos() / 100;

    let mods = thread_info::module_map(pid);
    use std::collections::HashMap;
    let first: HashMap<u32, &thread_info::ThreadEntry> =
        s1.entries.iter().map(|e| (e.tid, e)).collect();

    let mut ranked: Vec<Ranked> = s2
        .entries
        .iter()
        .filter_map(|e2| {
            let e1 = first.get(&e2.tid).copied()?;
            let delta = (e2.user_time - e1.user_time) + (e2.kernel_time - e1.kernel_time);
            Some(Ranked {
                tid: e2.tid,
                cycles_delta: match (e2.cycles, e1.cycles) {
                    (Some(a), Some(b)) => a.saturating_sub(b),
                    // 无句柄 → 无 cycles → 排序垫底（CPU% 兜底仍可比较）
                    _ => 0,
                },
                cpu_pct: (delta as f64 / interval_100ns as f64 * 100.0).max(0.0),
                start_resolved: e2
                    .start_address
                    .map(|a| thread_info::resolve_address(&mods, a))
                    .unwrap_or_default(),
            })
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.cycles_delta
            .cmp(&a.cycles_delta)
            .then(b.cpu_pct.total_cmp(&a.cpu_pct))
    });
    Ok(ranked)
}

/// 候选域过滤：起始地址在进程主模块内（`"进程名+0x"` 前缀）。
/// 模块映射不可用 → 无一通过（安全默认：无法验证归属的线程绝不 pin）。
fn in_main_module(r: &Ranked, process_name: &str) -> bool {
    r.start_resolved.starts_with(&format!("{process_name}+0x"))
}

// ═══════════════════════════════════════════════════════════════════
// 对外入口 — apply / restore
// ═══════════════════════════════════════════════════════════════════

/// 差分采样窗口（毫秒）— 三采样实证 1s 足够分离断崖。
const RANK_INTERVAL_MS: u64 = 1000;

/// 对 pid 应用 pinning 策略（「优化游戏」优化的收尾步骤）。
///
/// 新鲜度语义（2026-08-22 需求）：
/// - 同 pid 且已有 pin 存活 → **沿用**现有映射（不重采样）；
/// - 首次 / 换进程 / pin 全灭 → 还原旧状态后**重新映射**（采样 → 过滤 → pin）。
/// - 进程无策略 → 跳过（其他游戏的占位行为）。
pub fn apply_for_pid(pid: u32) {
    // 1. 进程名 → 策略
    let Some(process_name) = affinity::find_name_by_pid(pid) else {
        warn!("pinning: PID {pid} 查名失败，跳过");
        return;
    };
    let Some(strategy) = lookup_strategy(&process_name) else {
        info!("pinning: {process_name} 无策略（未注册），跳过");
        return;
    };

    // 2. 新鲜度：同 pid 且有存活 pin → 沿用
    let fresh = with_state(|state| {
        if let Some(s) = state
            && s.pid == pid
        {
            let alive = s
                .pinned
                .iter()
                .filter(|p| thread_owned_by(p.tid, pid))
                .count();
            if alive > 0 {
                info!(
                    "pinning: 沿用现有映射（PID {pid}，{alive}/{} 条仍存活）— {}",
                    s.pinned.len(),
                    strategy.note
                );
                return true;
            }
        }
        false
    });
    if fresh {
        return;
    }

    // 3. 重新映射：先还原旧状态（跨游戏/重启残留清理）
    restore();

    // 4. 双采样差分 → 候选域（主模块）→ Top-N
    let ranked = match sample_ranked(pid, RANK_INTERVAL_MS) {
        Ok(r) => r,
        Err(e) => {
            warn!("pinning: 采样失败: {e}");
            return;
        }
    };
    let effective_n = strategy.top_n.min(strategy.masks.len());
    let candidates: Vec<&Ranked> = ranked
        .iter()
        .filter(|r| in_main_module(r, &process_name))
        .take(effective_n)
        .collect();
    if candidates.is_empty() {
        warn!("pinning: {process_name} 候选域为空（模块映射被拒或线程全闲置），跳过");
        return;
    }
    if candidates.len() < effective_n {
        info!(
            "pinning: 候选仅 {}/{} 条（其余未通过主模块过滤）— 按 Actual 数 pin",
            candidates.len(),
            effective_n
        );
    }

    // 5. 逐条 pin（失败降级：跳过该条，其余照常）
    let mut pinned: Vec<PinnedThread> = Vec::with_capacity(candidates.len());
    for (i, r) in candidates.iter().enumerate() {
        let target = strategy.masks[i];
        match pin_thread(r.tid, target) {
            Some(old) => {
                info!(
                    "pinning: #{} TID {} Δcyc {:.2e} CPU {:.1}% → LP 掩码 0x{:X}（原 0x{:X}）",
                    i + 1,
                    r.tid,
                    r.cycles_delta as f64,
                    r.cpu_pct,
                    target,
                    old
                );
                pinned.push(PinnedThread {
                    tid: r.tid,
                    old_mask: old,
                    target_mask: target,
                });
            }
            None => {
                warn!(
                    "pinning: #{} TID {} 失败跳过（句柄/设置权被拒 — 官服反作弊?）",
                    i + 1,
                    r.tid
                );
            }
        }
    }

    if pinned.is_empty() {
        warn!("pinning: 全军覆没（SET 权限全线被拒）— 其余优化步骤不受影响");
        return;
    }

    with_state(|state| {
        *state = Some(PinState {
            pid,
            process_name: process_name.clone(),
            pinned,
        })
    });
    info!(
        "pinning: {} 条线程已入住金核（{}）",
        strategy.top_n.min(candidates.len()),
        strategy.note
    );
}

/// 还原全部 pin（「优化游戏」恢复路径 / 退出安全网）。
/// 线程死亡或 TID 易主 → 静默跳过（pid 校验防误伤他进程）。
pub fn restore() {
    let Some(state) = with_state(|state| state.take()) else {
        return; // 无状态 — 幂等
    };
    let mut restored = 0usize;
    for p in &state.pinned {
        if restore_thread(p.tid, state.pid, p.old_mask) {
            restored += 1;
        }
    }
    if !state.pinned.is_empty() {
        info!(
            "pinning: 已还原 {}/{} 条（{}，PID {}）",
            restored,
            state.pinned.len(),
            state.process_name,
            state.pid
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// 单测 — 纯逻辑：策略查找 / 模块过滤 / 排序
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_lookup_case_insensitive() {
        assert!(lookup_strategy("YuanShen.exe").is_some());
        assert!(lookup_strategy("yuanshen.EXE").is_some());
        assert!(lookup_strategy("Client-Win64-Shipping.exe").is_none());
        let s = lookup_strategy("YuanShen.exe").unwrap();
        assert_eq!(s.top_n, 2);
        assert_eq!(s.masks.len(), 2);
    }

    #[test]
    fn main_module_filter_excludes_dll_threads() {
        let mk = |s: &str| Ranked {
            tid: 1,
            cycles_delta: 0,
            cpu_pct: 0.0,
            start_resolved: s.into(),
        };
        // 主模块 ✓（大小写不敏感场景由进程名来源保证 — 此处测前缀语义）
        assert!(in_main_module(
            &mk("YuanShen.exe+0x15CD290"),
            "YuanShen.exe"
        ));
        // 驱动/SDK/裸地址/空 — 全部排除
        assert!(!in_main_module(
            &mk("nvwgf2umx.dll+0xEBA250"),
            "YuanShen.exe"
        ));
        assert!(!in_main_module(&mk("0x7FF800000000"), "YuanShen.exe"));
        assert!(!in_main_module(&mk(""), "YuanShen.exe"));
        // 前缀陷阱：主模块名是其他模块的前缀也不能混入
        assert!(!in_main_module(
            &mk("YuanShen.exe.helper.dll+0x1"),
            "YuanShen.exe"
        ));
    }
}
