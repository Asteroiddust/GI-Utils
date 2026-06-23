//! CPU core affinity and process priority management.
//!
//! Separates game and tool processes onto different CPU cores to reduce
//! contention, and boosts priority for lower input latency.
//!
//! Uses Rust-native patterns: RAII handles, iterators, and `Result`
//! error handling instead of C-style manual resource management.

use std::fmt;
use std::process;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, SetProcessAffinityMask, SetPriorityClass, PROCESS_CREATION_FLAGS,
    PROCESS_SET_INFORMATION, REALTIME_PRIORITY_CLASS,
};

// ── Core masks ────────────────────────────────────────────────

/// Cores reserved for the game process (cores 2-13).
pub const GAME_CORES_MASK: usize = 0b1111_1111_1111_1100;

/// Cores reserved for this tool (cores 0-1).
pub const TOOL_CORES_MASK: usize = 0b0000_0000_0000_0011;

/// Cores for all other processes (same as tool cores).
pub const OTHER_CORES_MASK: usize = TOOL_CORES_MASK;

// ── Error type ────────────────────────────────────────────────

#[derive(Debug)]
pub enum Error {
    OpenProcess { pid: u32, source: windows::core::Error },
    SetAffinity { pid: u32, source: windows::core::Error },
    SetPriority { pid: u32, source: windows::core::Error },
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

/// Owned Windows HANDLE that automatically closes on drop.
struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe { let _ = CloseHandle(self.0); }
        }
    }
}

// ── Process iterator ──────────────────────────────────────────

/// An iterator over all running processes, obtained from a toolhelp snapshot.
///
/// Replaces the C-style `Process32First` / `Process32Next` loop pattern:
///
/// ```ignore
/// for entry in ProcessIterator::new()? {
///     println!("PID {}: {}", entry.pid, entry.name());
/// }
/// ```
struct ProcessIterator {
    _snapshot: OwnedHandle, // RAII: snapshot is closed when iterator is dropped
    handle:    HANDLE,
    entry:     PROCESSENTRY32,
    started:   bool,
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

/// A single process entry from the snapshot iterator.
pub struct ProcessEntry {
    inner: PROCESSENTRY32,
}

impl ProcessEntry {
    pub fn pid(&self) -> u32 {
        self.inner.th32ProcessID
    }

    /// Best-effort process name (truncated at first null).
    pub fn name(&self) -> &str {
        let bytes = &self.inner.szExeFile; // [i8; 260] in windows-rs
        let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        // szExeFile is [i8] (C char), str::from_utf8 wants [u8]; same layout
        let chars: &[u8] = unsafe { &*(&bytes[..len] as *const [i8] as *const [u8]) };
        std::str::from_utf8(chars).unwrap_or("<invalid utf8>")
    }
}

// ── Low-level operations ──────────────────────────────────────

/// Open a process by PID.
fn open_process(pid: u32) -> Result<OwnedHandle, Error> {
    unsafe { OpenProcess(PROCESS_SET_INFORMATION, false, pid) }
        .map(OwnedHandle)
        .map_err(|e| Error::OpenProcess { pid, source: e })
}

/// Set CPU affinity for a process.
fn set_affinity(h: &OwnedHandle, pid: u32, mask: usize) -> Result<(), Error> {
    unsafe { SetProcessAffinityMask(h.raw(), mask) }
        .map_err(|e| Error::SetAffinity { pid, source: e })
}

/// Set priority class for a process.
fn set_priority(h: &OwnedHandle, pid: u32, prio: PROCESS_CREATION_FLAGS) -> Result<(), Error> {
    unsafe { SetPriorityClass(h.raw(), prio) }
        .map_err(|e| Error::SetPriority { pid, source: e })
}

// ── Public API ────────────────────────────────────────────────

/// Set CPU affinity and priority for a specific process by PID.
pub fn configure_process(pid: u32, mask: usize, prio: u32) -> Result<(), Error> {
    let h = open_process(pid)?;
    set_affinity(&h, pid, mask)?;
    set_priority(&h, pid, PROCESS_CREATION_FLAGS(prio))?;
    Ok(())
}

/// Walk all processes and apply a closure to each.
/// The closure can mutate or return early.
fn for_each_process<F>(mut f: F) -> Result<(), Error>
where
    F: FnMut(ProcessEntry) -> Result<(), Error>,
{
    for entry in ProcessIterator::new()? {
        f(entry)?;
    }
    Ok(())
}

/// Move all non-game, non-self processes off the game cores.
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

/// Restore all processes to full CPU affinity.
pub fn restore_all_affinity() -> Result<(), Error> {
    let self_pid = process::id();
    for_each_process(|entry| {
        let pid = entry.pid();
        if pid != self_pid {
            if let Ok(h) = open_process(pid) {
                let _ = set_affinity(&h, pid, 0xFFFF);
            }
        }
        Ok(())
    })
}

/// Set this process's own CPU affinity and priority to real-time.
/// Must be called at startup, before any timing-critical work.
pub fn configure_self() -> Result<(), Error> {
    let self_pid = process::id();
    let h = open_process(self_pid)?;
    set_affinity(&h, self_pid, TOOL_CORES_MASK)?;
    set_priority(&h, self_pid, REALTIME_PRIORITY_CLASS)?;
    Ok(())
}
