//! CPU 核心亲和性与进程优先级管理 — Core affinity and process priority management.
//!
//! 将游戏进程和工具进程分配到不同 CPU 核心以减少争用，并提升优先级以降低输入延迟。
//! Separates game and tool processes onto different CPU cores to reduce
//! contention, and boosts priority for lower input latency.
//!
//! 使用 Rust 原生模式：RAII 句柄、迭代器、`Result` 错误处理，替代 C 风格手动资源管理。
//! Uses Rust-native patterns: RAII handles, iterators, and `Result`
//! error handling instead of C-style manual resource management.

use std::fmt;
use std::process;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, OpenProcess, SetPriorityClass, SetProcessAffinityMask,
    SetThreadAffinityMask, SetThreadPriority, THREAD_PRIORITY_LOWEST, PROCESS_CREATION_FLAGS,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, REALTIME_PRIORITY_CLASS,
};

// ── Core masks ────────────────────────────────────────────────
// 8C16T layout:  物理核 0    1-5      6        7
//                线程  [0,1] [2-11]  [12,13]  [14,15]
//                用途  系统  游戏   GUI渲染  输入处理

/// ALL MASK
pub const ALL_CORES_MASK: usize = 0b1111_1111_1111_1111;

/// 游戏进程（全部核心 — 隔离逻辑保留游戏全核，见 optimize_game）。
pub const GAME_CORES_MASK: usize = ALL_CORES_MASK;

/// 输入处理核心（物理核 7，逻辑 14,15）— Engine 事件循环 + 全部功能线程。
/// 与 GUI 渲染核心（12,13）分离：渲染卡顿不影响输入注入时序。
pub const ENGINE_CORES_MASK: usize = 0b1100_0000_0000_0000;

/// GUI 渲染核心（物理核 6，逻辑 12,13）— eframe 主线程（渲染 + 事件循环）。
pub const GUI_CORES_MASK: usize = 0b0011_0000_0000_0000;

/// 进程级掩码（物理核 6-7，逻辑 12-15）：GUI + 输入处理的总范围。
/// Windows 要求线程掩码 ⊆ 进程掩码 — 先在进程级放出全部范围，
/// 再在各线程内收窄到各自子集。
pub const PROCESS_CORES_MASK: usize = GUI_CORES_MASK | ENGINE_CORES_MASK;

/// 其他进程
pub const OTHER_CORES_MASK: usize = ALL_CORES_MASK;

// ── Error type ────────────────────────────────────────────────

/// 进程操作错误 — Errors from process open / affinity / priority operations.
#[derive(Debug)]
pub enum Error {
    /// 无法打开进程 — Failed to open process by PID.
    OpenProcess {
        pid: u32,
        source: windows::core::Error,
    },
    /// 设置 CPU 亲和性失败 — Failed to set CPU affinity mask.
    SetAffinity {
        pid: u32,
        source: windows::core::Error,
    },
    /// 设置优先级失败 — Failed to set priority class.
    SetPriority {
        pid: u32,
        source: windows::core::Error,
    },
    /// 设置线程 CPU 亲和性失败 — Failed to set thread affinity mask.
    SetThreadAffinity { source: windows::core::Error },
    /// 设置线程优先级失败 — Failed to set thread priority.
    SetThreadPriority { source: windows::core::Error },
    /// 创建进程快照失败 — Failed to create toolhelp snapshot.
    Snapshot { source: windows::core::Error },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OpenProcess { pid, source } => {
                write!(f, "failed to open process {}: {}", pid, source)
            }
            Error::SetAffinity { pid, source } => {
                write!(f, "failed to set affinity for process {}: {}", pid, source)
            }
            Error::SetPriority { pid, source } => {
                write!(f, "failed to set priority for process {}: {}", pid, source)
            }
            Error::SetThreadAffinity { source } => {
                write!(f, "failed to set thread affinity: {}", source)
            }
            Error::SetThreadPriority { source } => {
                write!(f, "failed to set thread priority: {}", source)
            }
            Error::Snapshot { source } => {
                write!(f, "failed to create process snapshot: {}", source)
            }
        }
    }
}

impl std::error::Error for Error {}

// ── RAII Handle ───────────────────────────────────────────────

/// 拥有的 Windows HANDLE — Owned Windows HANDLE that automatically closes on drop.
struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

// ── Process iterator ──────────────────────────────────────────

/// 全进程迭代器 — An iterator over all running processes from a toolhelp snapshot.
///
/// 替代 C 风格的 `Process32First` / `Process32Next` 循环模式。
/// Replaces the C-style `Process32First` / `Process32Next` loop pattern:
///
/// ```ignore
/// for entry in ProcessIterator::new()? {
///     println!("PID {}: {}", entry.pid, entry.name());
/// }
/// ```
struct ProcessIterator {
    _snapshot: OwnedHandle, // RAII: snapshot is closed when iterator is dropped
    handle: HANDLE,
    entry: PROCESSENTRY32,
    started: bool,
}

impl ProcessIterator {
    fn new() -> Result<Self, Error> {
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|e| Error::Snapshot { source: e })?;
        let snapshot = OwnedHandle(handle);

        Ok(Self {
            _snapshot: snapshot,
            handle,
            entry: PROCESSENTRY32 {
                dwSize: std::mem::size_of::<PROCESSENTRY32>() as u32,
                ..Default::default()
            },
            started: false,
        })
    }
}

impl Iterator for ProcessIterator {
    type Item = ProcessEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let ok = if !self.started {
            self.started = true;
            unsafe { Process32First(self.handle, &mut self.entry) }
        } else {
            unsafe { Process32Next(self.handle, &mut self.entry) }
        };
        ok.ok()?;
        Some(ProcessEntry { inner: self.entry })
    }
}

/// 进程条目 — A single process entry from the snapshot iterator.
pub struct ProcessEntry {
    inner: PROCESSENTRY32,
}

impl ProcessEntry {
    /// 获取进程 PID — Get the process ID.
    pub fn pid(&self) -> u32 {
        self.inner.th32ProcessID
    }

    /// 获取进程名（尽力而为）— Best-effort process name.
    ///
    /// Truncated at first null byte. 使用 lossy UTF-8 转换，因为 `szExeFile`
    /// 是 ANSI 编码，不保证为合法 UTF-8。
    /// Uses lossy UTF-8 conversion because `szExeFile` is ANSI-codepage
    /// encoded, not guaranteed to be valid UTF-8.
    pub fn name(&self) -> &str {
        // `String::from_utf8_lossy` returns `Cow<str>`, but we need `&str`.
        // Store the owned String in the struct so we can return a reference.
        // For now, returning static fallback on invalid UTF-8 is good enough —
        // game executable names are always ASCII.
        let bytes = &self.inner.szExeFile; // [i8; 260] in windows-rs
        let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let chars: &[u8] = unsafe { &*(&bytes[..len] as *const [i8] as *const [u8]) };
        std::str::from_utf8(chars).unwrap_or("<?>")
    }
}

// ── Low-level operations ──────────────────────────────────────

/// 按 PID 打开进程 — Open a process by PID.
fn open_process(pid: u32) -> Result<OwnedHandle, Error> {
    // PROCESS_QUERY_LIMITED_INFORMATION is needed on modern Windows 10/11;
    // PROCESS_SET_INFORMATION alone may be denied even for admin.
    let access = PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION;
    unsafe { OpenProcess(access, false, pid) }
        .map(OwnedHandle)
        .map_err(|e| Error::OpenProcess { pid, source: e })
}

/// 设置进程优先级 — Set priority class for a process.
fn set_priority(h: &OwnedHandle, pid: u32, prio: PROCESS_CREATION_FLAGS) -> Result<(), Error> {
    unsafe { SetPriorityClass(h.raw(), prio) }.map_err(|e| Error::SetPriority { pid, source: e })
}

// ── Public API ────────────────────────────────────────────────

/// 遍历所有进程 — Walk all processes and apply a closure to each.
fn for_each_process<F>(mut f: F) -> Result<(), Error>
where
    F: FnMut(ProcessEntry) -> Result<(), Error>,
{
    for entry in ProcessIterator::new()? {
        f(entry)?;
    }
    Ok(())
}

/// 将非游戏进程移出游戏核心 — Move all non-game, non-self processes off the game cores.
pub fn isolate_game_cores(game_pid: u32) -> Result<(), Error> {
    let self_pid = process::id();
    for_each_process(|entry| {
        let pid = entry.pid();
        if pid != game_pid && pid != self_pid && pid != 0 {
            if let Ok(h) = open_process(pid) {
                let _ = unsafe { SetProcessAffinityMask(h.raw(), OTHER_CORES_MASK) };
            }
        }
        Ok(())
    })
}

/// 恢复所有进程的完整 CPU 亲和性 — Restore all processes to full CPU affinity.
pub fn restore_all_affinity() -> Result<(), Error> {
    let self_pid = process::id();
    for_each_process(|entry| {
        let pid = entry.pid();
        if pid != self_pid {
            if let Ok(h) = open_process(pid) {
                let _ = unsafe { SetProcessAffinityMask(h.raw(), ALL_CORES_MASK) };
            }
        }
        Ok(())
    })
}

/// 将当前线程固定到指定核心掩码 — Pin the current thread to the given core mask.
///
/// 线程掩码必须是进程掩码的子集。新 spawn 的线程继承**进程**亲和性
/// （而非父线程），因此进程掩码较宽（GUI 版 12-15）时，时序关键线程
/// 必须在闭包内显式收窄到输入处理核心（14,15）。
pub fn pin_current_thread(mask: usize) -> Result<(), Error> {
    // SetThreadAffinityMask 返回旧掩码；失败返回 0（win32 语义，非 Result）。
    let prev = unsafe { SetThreadAffinityMask(GetCurrentThread(), mask) };
    if prev == 0 {
        Err(Error::SetThreadAffinity {
            source: windows::core::Error::from_thread(),
        })
    } else {
        Ok(())
    }
}

/// GUI 版线程配置 — GUI variant: widen the process mask to 12-15 and pin
/// the calling thread (eframe main/UI thread) to the GUI render cores at
/// reduced thread priority.
///
/// 必须在 [`configure_self`] 之后、创建 Engine 线程之前调用：新线程继承
/// 进程掩码，需先扩展进程掩码到 12-15，Engine/功能线程才能收窄到 14,15。
pub fn configure_gui_self() -> Result<(), Error> {
    // 1. 扩展进程掩码至 12-15 — 线程掩码必须 ⊆ 进程掩码，
    //    GUI(12,13) 与输入处理(14,15) 才能分别收窄
    let self_pid = process::id();
    let h = open_process(self_pid)?;
    unsafe { SetProcessAffinityMask(h.raw(), PROCESS_CORES_MASK) }
        .map_err(|e| Error::SetAffinity { pid: self_pid, source: e })?;
    // 2. GUI 主线程 → 12,13（渲染与输入处理分离，互不干扰）
    pin_current_thread(GUI_CORES_MASK)?;
    // 3. GUI 线程降为线程级最低优先级（REALTIME base 24 - 2 = 22）：
    //    渲染线程不应抢占输入线程（24）的调度 — 渲染降级不牺牲注入时序。
    //    22 仍高于所有普通进程（HIGH 为 13），不影响响应性。
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST) }
        .map_err(|e| Error::SetThreadPriority { source: e })
}

/// 配置当前进程 — Set this process's own CPU affinity and priority to real-time.
///
/// 必须在启动时调用，在任何时序关键的工作之前。
/// Must be called at startup, before any timing-critical work.
pub fn configure_self() -> Result<(), Error> {
    let self_pid = process::id();
    let h = open_process(self_pid)?;
    unsafe { SetProcessAffinityMask(h.raw(), ENGINE_CORES_MASK) }.map_err(|e| {
        Error::SetAffinity {
            pid: self_pid,
            source: e,
        }
    })?;
    set_priority(&h, self_pid, REALTIME_PRIORITY_CLASS)?;
    Ok(())
}
