//! 线程采样 — YuanShen.exe 专属的线程画像采集（Once）。
//!
//! Process Explorer Threads 页的自动化版：两次采样差分得到 CPU%/Cycles
//! Delta，全列输出到 exe 旁 `thread_sample.txt`（追加），日志面板给
//! 摘要（线程数 / 句柄打开率 = pinning 可行性信号 / Top 5）。
//! 为"热线程 pin 到金银核"功能做决策数据（2026-08-22）。

use crate::engine::bindings::KeyFunction;
use crate::utils;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tracing::{error, info, warn};

/// 两次采样之间的间隔（毫秒）— 差分窗口。
const SAMPLE_INTERVAL_MS: u64 = 1000;
/// 日志面板摘要的 Top N（完整表始终写文件）。
const TOP_N_LOG: usize = 5;

/// 线程采样功能 — Once 模式。
pub struct 线程采样;

impl 线程采样 {
    pub fn new() -> Self {
        Self
    }
}

impl Default for 线程采样 {
    fn default() -> Self {
        Self::new()
    }
}

/// 差分行 — 两帧合并后的分析视图。
struct Row {
    tid: u32,
    name: Option<String>,
    state: Option<u32>,
    wait_reason: Option<u32>,
    cpu_percent: f64,
    user_ms: i64,
    kernel_ms: i64,
    cycles_delta: Option<u64>,
    cycles: Option<u64>,
    context_switches: Option<u32>,
    base_pri: Option<i32>,
    dyn_pri: Option<i32>,
    ideal_cpu: Option<u8>,
    suspend_count: Option<u32>,
    page_priority: Option<u32>,
    io_priority: Option<u32>,
    start_resolved: String,
    start_time: Option<String>,
}

impl KeyFunction for 线程采样 {
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        // 1. 按映像名找 pid（affinity::ProcessIterator — TH32CS_SNAPPROCESS）
        let pid = match find_yuanshen_pid() {
            Some(pid) => pid,
            None => {
                warn!("线程采样: 未找到 YuanShen.exe 进程");
                return;
            }
        };

        // 2. 模块映射（起始地址符号化；失败退裸地址）
        let mods = utils::thread_info::module_map(pid);

        // 3. 双采样差分（std sleep — 分析用途，无需 TSC 精度）
        let s1 = match utils::thread_info::snapshot_threads(pid) {
            Ok(s) => s,
            Err(e) => {
                error!("线程采样: 第一次采样失败: {e}");
                return;
            }
        };
        let started = std::time::Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(SAMPLE_INTERVAL_MS));
        let s2 = match utils::thread_info::snapshot_threads(pid) {
            Ok(s) => s,
            Err(e) => {
                error!("线程采样: 第二次采样失败: {e}");
                return;
            }
        };
        let interval_100ns = started.elapsed().as_nanos() / 100;

        // 4. 按 TID 合并 → 差分行
        use std::collections::HashMap;
        let first: HashMap<u32, &utils::thread_info::ThreadEntry> =
            s1.entries.iter().map(|e| (e.tid, e)).collect();
        let mut rows: Vec<Row> = s2
            .entries
            .iter()
            .filter_map(|e2| {
                let e1 = first.get(&e2.tid).copied()?; // 两帧都在的线程才可差分
                let delta = (e2.user_time - e1.user_time) + (e2.kernel_time - e1.kernel_time);
                let cpu = delta as f64 / interval_100ns as f64 * 100.0;
                Some(Row {
                    tid: e2.tid,
                    name: e2.name.clone(),
                    state: e2.state,
                    wait_reason: e2.wait_reason,
                    cpu_percent: cpu.max(0.0),
                    user_ms: e2.user_time / 10_000,
                    kernel_ms: e2.kernel_time / 10_000,
                    cycles_delta: match (e2.cycles, e1.cycles) {
                        (Some(a), Some(b)) => Some(a.saturating_sub(b)),
                        _ => None,
                    },
                    cycles: e2.cycles,
                    context_switches: e2.context_switches,
                    base_pri: e2.base_pri,
                    dyn_pri: e2.dyn_pri,
                    ideal_cpu: e2.ideal_cpu,
                    suspend_count: e2.suspend_count,
                    page_priority: e2.page_priority,
                    io_priority: e2.io_priority,
                    start_resolved: e2
                        .start_address
                        .map(|a| utils::thread_info::resolve_address(&mods, a))
                        .unwrap_or_else(|| "-".into()),
                    start_time: e2.creation_ft.map(filetime_to_local),
                })
            })
            .collect();

        // 5. 排序：Cycles Delta 降序（无 cycles 数据按 CPU%），同级互比
        rows.sort_by(|a, b| {
            let ka = a.cycles_delta.unwrap_or(0);
            let kb = b.cycles_delta.unwrap_or(0);
            kb.cmp(&ka).then(b.cpu_percent.total_cmp(&a.cpu_percent))
        });

        // 6. 摘要 → 日志面板（句柄打开率 = 线程级 pinning 可行性信号）
        let total = s2.entries.len();
        info!(
            "线程采样: YuanShen.exe (PID {pid}) — {total} 线程，句柄打开 {}/{} {}，NT 快照{}",
            s2.handles_opened,
            total,
            if s2.handles_opened == total {
                "（pinning 绿灯）"
            } else if s2.handles_opened == 0 {
                "（线程句柄被拒 — pinning 受阻）"
            } else {
                ""
            },
            if s2.nt_available {
                "可用"
            } else {
                "不可用（State/Wait 列为空）"
            },
        );
        for (i, r) in rows.iter().take(TOP_N_LOG).enumerate() {
            let name = r.name.as_deref().unwrap_or("-");
            let cycles = r
                .cycles_delta
                .map(|c| format!("{c:.3e}"))
                .unwrap_or_else(|| "-".into());
            info!(
                "  #{} TID {} CPU {:5.1}%  Δcyc {}  {}  [{}]",
                i + 1,
                r.tid,
                r.cpu_percent,
                cycles,
                r.start_resolved,
                name,
            );
        }

        // 7. 全表 → exe 旁 thread_sample.txt（追加）
        let path = write_table(&rows, pid, s2.nt_available);
        match path {
            Ok(p) => info!("线程采样: 完整表已追加至 {}", p.display()),
            Err(e) => error!("线程采样: 写文件失败: {e}"),
        }
    }
}

/// 按映像名查找 YuanShen.exe 的 pid。
fn find_yuanshen_pid() -> Option<u32> {
    utils::affinity::find_pid_by_name("YuanShen.exe")
}

/// FILETIME（100ns since 1601）→ 本地时刻 "HH:MM:SS.mmm"。
/// UTC+8 手工偏移（本机写定 — 项目定位声明，见 CLAUDE.md）。
fn filetime_to_local(ft: u64) -> String {
    const EPOCH_DIFF_MS: u64 = 116_444_736_000_000; // 1601→1970（ms）
    const TZ_OFFSET_MS: u64 = 8 * 3600 * 1000;
    let ms = ft / 10_000;
    let ms = ms.saturating_sub(EPOCH_DIFF_MS) + TZ_OFFSET_MS;
    let secs_of_day = (ms / 1000) % 86_400;
    let h = secs_of_day / 3600;
    let m = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    let frac = ms % 1000;
    format!("{h:02}:{m:02}:{s:02}.{frac:03}")
}

/// CPU 累计时间（用户+内核，ms）→ "1h23m45.6s"。
fn format_cpu_time(user_ms: i64, kernel_ms: i64) -> String {
    let total_s = (user_ms + kernel_ms) as f64 / 1000.0;
    let h = (total_s / 3600.0) as u64;
    let m = ((total_s - h as f64 * 3600.0) / 60.0) as u64;
    let s = total_s - h as f64 * 3600.0 - m as f64 * 60.0;
    format!("{h}h{m:02}m{s:04.1}s")
}

/// 全列表写文件，返回路径。
fn write_table(rows: &[Row], pid: u32, nt_available: bool) -> std::io::Result<std::path::PathBuf> {
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("thread_sample.txt");

    let header = "TID      NAME                         STATE       WAIT                CPU%     USR(ms)    KRN(ms)  CPUTIME      CYCLES_DELTA  CYCLES_TOTAL   CTXSW   BPRIO DPRIO IDEAL SUSP MEMPRI IOPRI START                       STARTED";
    use std::fmt::Write as _;
    let mut out = String::with_capacity(4096 + rows.len() * 220);
    let now = filetime_now_local();
    let _ = writeln!(
        out,
        "\n══ YuanShen.exe (PID {pid}) {now}  采样窗口 {SAMPLE_INTERVAL_MS}ms  NT快照{}  线程 {} ══",
        if nt_available { "可用" } else { "不可用" },
        rows.len(),
    );
    let _ = writeln!(out, "{header}");
    for r in rows {
        let _ = writeln!(
            out,
            "{:<8} {:<28} {:<11} {:<18} {:6.1} {:>10} {:>10}  {:<11} {:>12}  {:>12}  {:>6}  {:>5} {:>5} {:>5} {:>4} {:>6} {:>5} {:<26} {}",
            r.tid,
            truncate(r.name.as_deref().unwrap_or("-"), 28),
            r.state
                .map(utils::thread_info::thread_state_name)
                .unwrap_or_else(|| "-".into()),
            r.wait_reason
                .map(utils::thread_info::wait_reason_name)
                .unwrap_or_else(|| "-".into()),
            r.cpu_percent,
            r.user_ms,
            r.kernel_ms,
            format_cpu_time(r.user_ms, r.kernel_ms),
            r.cycles_delta
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into()),
            r.cycles
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into()),
            r.context_switches.unwrap_or(0),
            r.base_pri.unwrap_or(0),
            r.dyn_pri.unwrap_or(0),
            r.ideal_cpu
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into()),
            r.suspend_count
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into()),
            r.page_priority
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into()),
            r.io_priority
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into()),
            truncate(&r.start_resolved, 26),
            r.start_time.as_deref().unwrap_or("-"),
        );
    }
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    f.write_all(out.as_bytes())?;
    Ok(path)
}

/// 系统当前本地时刻（表头用，UTC+8 本机写定）。
fn filetime_now_local() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
        + 8 * 3600 * 1000;
    let secs_of_day = (ms / 1000) % 86_400;
    let h = secs_of_day / 3600;
    let m = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// 截断超长字符串（表列宽保护）。
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end.saturating_sub(1)])
    }
}
