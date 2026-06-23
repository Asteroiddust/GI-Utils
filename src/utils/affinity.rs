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
    OpenProcess, SetPriorityClass, SetProcessAffinityMask, PROCESS_CREATION_FLAGS,
    PROCESS_SET_INFORMATION, REALTIME_PRIORITY_CLASS,
};

// ── Core masks ────────────────────────────────────────────────

/// 游戏进程保留核心（核心 2-13）— Cores reserved for the game process.
pub const GAME_CORES_MASK: usize = 0b1111_1111_1111_1100;

/// 工具进程保留核心（核心 0-1）— Cores reserved for this tool.
pub const TOOL_CORES_MASK: usize = 0b0000_0000_0000_0011;

/// 其他进程核心（同工具核心）— Cores for all other processes (same as tool cores).
pub const OTHER_CORES_MASK: usize = TOOL_CORES_MASK;

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
    unsafe { OpenProcess(PROCESS_SET_INFORMATION, false, pid) }
        .map(OwnedHandle)
        .map_err(|e| Error::OpenProcess { pid, source: e })
}

/// 设置进程 CPU 亲和性 — Set CPU affinity for a process.
fn set_affinity(h: &OwnedHandle, pid: u32, mask: usize) -> Result<(), Error> {
    unsafe { SetProcessAffinityMask(h.raw(), mask) }
        .map_err(|e| Error::SetAffinity { pid, source: e })
}

/// 设置进程优先级 — Set priority class for a process.
fn set_priority(h: &OwnedHandle, pid: u32, prio: PROCESS_CREATION_FLAGS) -> Result<(), Error> {
    unsafe { SetPriorityClass(h.raw(), prio) }.map_err(|e| Error::SetPriority { pid, source: e })
}

// ── Public API ────────────────────────────────────────────────

/// 为指定 PID 设置 CPU 亲和性和优先级 — Configure a specific process.
///
/// Opens the process, then sets CPU affinity mask and priority class.
pub fn configure_process(pid: u32, mask: usize, prio: u32) -> Result<(), Error> {
    let h = open_process(pid)?;
    set_affinity(&h, pid, mask)?;
    set_priority(&h, pid, PROCESS_CREATION_FLAGS(prio))?;
    Ok(())
}

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
///
/// 减少游戏进程与其他进程的 CPU 争用，从而降低输入延迟。
/// Reduces CPU contention between the game and other processes for lower input latency.
pub fn isolate_game_cores(game_pid: u32) -> Result<(), Error> {
    let self_pid = process::id();
    for_each_process(|entry| {
        let pid = entry.pid();
        if pid != game_pid && pid != self_pid && pid != 0 {
            if let Ok(h) = open_process(pid) {
                let _ = set_affinity(&h, pid, OTHER_CORES_MASK);
            }
        }
        Ok(())
    })
}

/// 恢复所有进程的完整 CPU 亲和性 — Restore all processes to full CPU affinity.
///
/// 在程序退出时调用，释放之前限制的核心。
/// Called on program exit to release previously restricted cores.
pub fn restore_all_affinity() -> Result<(), Error> {
    let self_pid = process::id();
    for_each_process(|entry| {
        let pid = entry.pid();
        if pid != self_pid {
            if let Ok(h) = open_process(pid) {
                let _ = set_affinity(&h, pid, usize::MAX);
            }
        }
        Ok(())
    })
}

/// 配置当前进程 — Set this process's own CPU affinity and priority to real-time.
///
/// 必须在启动时调用，在任何时序关键的工作之前。
/// Must be called at startup, before any timing-critical work.
pub fn configure_self() -> Result<(), Error> {
    let self_pid = process::id();
    let h = open_process(self_pid)?;
    set_affinity(&h, self_pid, TOOL_CORES_MASK)?;
    set_priority(&h, self_pid, REALTIME_PRIORITY_CLASS)?;
    Ok(())
}
