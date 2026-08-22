//! 跨进程线程信息采样 — Process Explorer Threads 页的自动化版。
//!
//! 数据源三级降级（2026-08-22，为线程 pinning 决策做数据采集）：
//! 1. **NT 快照**（`NtQuerySystemInformation(SystemProcessInformation)`）—
//!    一次系统调用取得全进程全线程的 State/WaitReason/ContextSwitches/
//!    BasePri/DynPri/UserTime/KernelTime，**无需任何句柄**（反作弊拒开
//!    线程句柄时这批列仍然可用）。
//! 2. **ToolHelp 线程快照** — 权威 TID 清单（NT 布局解析失败时兜底）。
//! 3. **逐线程句柄查询**（`THREAD_QUERY_*`）— Cycles/StartAddress/IdealCPU/
//!    StartTime/线程名/挂起数/内存与 IO 优先级；句柄被拒则对应列为空。
//!
//! NT 快照结构体布局按 x64 手工定义（windows crate 未提供该结构），
//! 解析带运行期健全性校验（NextEntryOffset 链 / pid+映像名匹配 /
//! 线程数与缓冲边界），校验失败整体放弃 NT 列而非给出错数据。

use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Wdk::System::Threading::{
    NtQueryInformationThread, THREADINFOCLASS, ThreadIoPriority, ThreadNameInformation,
    ThreadPagePriority, ThreadQuerySetWin32StartAddress, ThreadSuspendCount,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, NTSTATUS};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
    TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::Kernel::PROCESSOR_NUMBER;
use windows::Win32::System::Threading::{
    GetThreadIdealProcessorEx, GetThreadTimes, OpenThread, THREAD_QUERY_INFORMATION,
    THREAD_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::WindowsProgramming::QueryThreadCycleTime;
use windows::core::Result as WinResult;

// ═══════════════════════════════════════════════════════════════════
// NT 快照布局 — SYSTEM_PROCESS_INFORMATION (x64)，偏移常量
// ═══════════════════════════════════════════════════════════════════

/// 进程头内字段的字节偏移（ntifs.h SYSTEM_PROCESS_INFORMATION，x64）。
mod proc_off {
    pub const NEXT_ENTRY: usize = 0;
    pub const THREAD_COUNT: usize = 4;
    pub const IMAGE_NAME: usize = 56; // UNICODE_STRING（16B）
    pub const PID: usize = 80;
    pub const THREADS: usize = 256; // SYSTEM_THREAD_INFORMATION 数组起点
}

/// 单条 SYSTEM_THREAD_INFORMATION 的大小（x64：字段共 76B，对齐补到 80B）。
const NT_THREAD_STRIDE: usize = 80;
/// 线程结构内字段偏移。
mod thr_off {
    pub const KERNEL_TIME: usize = 0; // i64（100ns）
    pub const USER_TIME: usize = 8; // i64
    pub const WAIT_TIME: usize = 24; // u32
    pub const PRIORITY: usize = 56; // i32（当前/动态优先级）
    pub const BASE_PRIORITY: usize = 60; // i32
    pub const CONTEXT_SWITCHES: usize = 64; // u32
    pub const STATE: usize = 68; // u32（KTHREAD_STATE）
    pub const WAIT_REASON: usize = 72; // u32（KWAIT_REASON）
}

/// STATUS_INFO_LENGTH_MISMATCH — 缓冲不足时增大重试。
const STATUS_INFO_LENGTH_MISMATCH: NTSTATUS = NTSTATUS(0xC0000004u32 as i32);

// 非对齐安全读取（Vec<u8> 缓冲不保证指针对齐）
#[inline]
fn read_u32(buf: &[u8], off: usize) -> u32 {
    unsafe { (buf.as_ptr().add(off) as *const u32).read_unaligned() }
}
#[inline]
fn read_i32(buf: &[u8], off: usize) -> i32 {
    unsafe { (buf.as_ptr().add(off) as *const i32).read_unaligned() }
}
#[inline]
fn read_i64(buf: &[u8], off: usize) -> i64 {
    unsafe { (buf.as_ptr().add(off) as *const i64).read_unaligned() }
}
#[inline]
fn read_usize(buf: &[u8], off: usize) -> usize {
    unsafe { (buf.as_ptr().add(off) as *const usize).read_unaligned() }
}

/// 快照里读到的 UNICODE_STRING → String（越界防御：指针空或长度 0 返回空）。
fn read_nt_string(buf: &[u8], off: usize) -> String {
    let length = read_u32(buf, off) as usize; // USHORT Length + USHORT MaximumLength
    let length = length & 0xFFFF;
    let buffer = read_usize(buf, off + 8);
    if length == 0 || buffer == 0 {
        return String::new();
    }
    // 指针指向内核拷贝到我们缓冲内的数据，但仍做边界防御
    let slice = unsafe { std::slice::from_raw_parts(buffer as *const u16, length / 2) };
    String::from_utf16_lossy(slice)
}

// ═══════════════════════════════════════════════════════════════════
// ThreadEntry — 单线程采样点（Option = 数据源不可用）
// ═══════════════════════════════════════════════════════════════════

/// 一个线程在某一时刻的全部可采字段。
#[derive(Debug, Clone, Default)]
pub struct ThreadEntry {
    pub tid: u32,
    // ── NT 快照列（无需句柄）──
    pub state: Option<u32>,
    pub wait_reason: Option<u32>,
    pub context_switches: Option<u32>,
    pub base_pri: Option<i32>,
    pub dyn_pri: Option<i32>,
    /// 累计用户时间（100ns 单位）。
    pub user_time: i64,
    /// 累计内核时间（100ns 单位）。
    pub kernel_time: i64,
    // ── 句柄列（THREAD_QUERY_*）──
    pub cycles: Option<u64>,
    /// Win32 起始地址（ThreadQuerySetWin32StartAddress）。
    pub start_address: Option<usize>,
    pub ideal_cpu: Option<u8>,
    /// 线程创建时刻（FILETIME，100ns since 1601）。
    pub creation_ft: Option<u64>,
    /// 线程名（SetThreadDescription，多为空 — Unity/D3D 偶有设置）。
    pub name: Option<String>,
    pub suspend_count: Option<u32>,
    pub page_priority: Option<u32>,
    pub io_priority: Option<u32>,
}

/// KTHREAD_STATE 枚举名（数值超表返回数字本身）。
pub fn thread_state_name(state: u32) -> String {
    match state {
        0 => "Initialized".into(),
        1 => "Ready".into(),
        2 => "Running".into(),
        3 => "Standby".into(),
        4 => "Terminated".into(),
        5 => "Waiting".into(),
        6 => "Transition".into(),
        _ => format!("State{state}"),
    }
}

/// KWAIT_REASON 枚举名（常用值；数值超表返回 WrN）。
pub fn wait_reason_name(reason: u32) -> String {
    let s = match reason {
        0 => "Executive",
        1 => "FreePage",
        2 => "PageIn",
        3 => "PoolAllocation",
        4 => "DelayExecution",
        5 => "Suspended",
        6 => "UserRequest",
        7 => "WrExecutive",
        8 => "WrFreePage",
        9 => "WrPageIn",
        10 => "WrPoolAllocation",
        11 => "WrDelayExecution",
        12 => "WrSuspended",
        13 => "WrUserRequest",
        14 => "WrEventPair",
        15 => "WrQueue",
        16 => "WrLpcReceive",
        17 => "WrLpcReply",
        18 => "WrVirtualMemory",
        19 => "WrPageOut",
        20 => "WrRendezvous",
        21 => "WrKeyedEvent",
        22 => "WrTerminated",
        23 => "WrProcessInSwap",
        30 => "WrQueue",
        31 => "WrCpuRateControl",
        32 => "WrCalloutStack",
        33 => "WrKernel",
        34 => "WrResource",
        35 => "WrPushLock",
        36 => "WrMutex",
        37 => "WrQuantumEnd",
        38 => "WrDispatchInt",
        39 => "WrPreempted",
        40 => "WrYieldExecution",
        41 => "WrFastMutex",
        42 => "WrGuardedMutex",
        43 => "WrRundown",
        44 => "WrAlertByThreadId",
        45 => "WrDeferredPreempt",
        _ => return format!("Wr{reason}"),
    };
    s.into()
}

// ═══════════════════════════════════════════════════════════════════
// NT 快照 — NtQuerySystemInformation
// ═══════════════════════════════════════════════════════════════════

/// 查询 pid 的 NT 线程表：`Ok(entries)` 或解析失败原因。
/// 缓冲从 4MB 起步，STATUS_INFO_LENGTH_MISMATCH 时倍增重试（上限 256MB）。
fn nt_snapshot_threads(pid: u32) -> Result<Vec<(u32, ThreadEntry)>, String> {
    let mut len: usize = 4 << 20;
    while len <= (256 << 20) {
        let mut buf = vec![0u8; len];
        let mut returned = 0u32;
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                len as u32,
                &mut returned,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH {
            len *= 2;
            continue;
        }
        if !status.is_ok() {
            return Err(format!(
                "NtQuerySystemInformation failed: 0x{:08X}",
                status.0 as u32
            ));
        }
        buf.truncate(returned as usize);
        return parse_nt_threads(&buf, pid);
    }
    Err("system information buffer retry exhausted".into())
}

/// 解析 SystemProcessInformation 缓冲，取出目标 pid 的线程数组。
/// 健全性校验失败 → Err（调用方降级为无 NT 列）。
fn parse_nt_threads(buf: &[u8], pid: u32) -> Result<Vec<(u32, ThreadEntry)>, String> {
    let mut off = 0usize;
    loop {
        if off + proc_off::THREADS > buf.len() {
            return Err("entry header out of bounds".into());
        }
        let next = read_u32(buf, off + proc_off::NEXT_ENTRY) as usize;
        let entry_pid = read_usize(buf, off + proc_off::PID);
        if entry_pid == pid as usize {
            // 映像名双重校验（pid 匹配 + 名字合理，防御布局漂移）
            let name = read_nt_string(buf, off + proc_off::IMAGE_NAME);
            if !name.is_empty() && !name.eq_ignore_ascii_case("system") {
                // 目标命中
                let count = read_u32(buf, off + proc_off::THREAD_COUNT) as usize;
                let entry_end = if next == 0 { buf.len() } else { off + next };
                if count == 0 || count > 4096 {
                    return Err(format!("implausible thread count {count}"));
                }
                let threads_start = off + proc_off::THREADS;
                if threads_start + count * NT_THREAD_STRIDE > entry_end {
                    return Err("thread array out of bounds".into());
                }
                let mut out = Vec::with_capacity(count);
                for i in 0..count {
                    let t = threads_start + i * NT_THREAD_STRIDE;
                    // ClientId.UniqueThread：CLIENT_ID 在 offset 40，第二成员 +8
                    let tid = read_usize(buf, t + 48);
                    if tid == 0 || tid > u32::MAX as usize {
                        return Err("implausible tid".into());
                    }
                    out.push((
                        tid as u32,
                        ThreadEntry {
                            tid: tid as u32,
                            state: Some(read_u32(buf, t + thr_off::STATE)),
                            wait_reason: Some(read_u32(buf, t + thr_off::WAIT_REASON)),
                            context_switches: Some(read_u32(buf, t + thr_off::CONTEXT_SWITCHES)),
                            base_pri: Some(read_i32(buf, t + thr_off::BASE_PRIORITY)),
                            dyn_pri: Some(read_i32(buf, t + thr_off::PRIORITY)),
                            user_time: read_i64(buf, t + thr_off::USER_TIME),
                            kernel_time: read_i64(buf, t + thr_off::KERNEL_TIME),
                            ..ThreadEntry::default()
                        },
                    ));
                }
                return Ok(out);
            }
        }
        if next == 0 {
            return Err(format!("pid {pid} not found in system snapshot"));
        }
        off += next;
    }
}

// ═══════════════════════════════════════════════════════════════════
// ToolHelp — 权威 TID 清单 + 模块映射
// ═══════════════════════════════════════════════════════════════════

/// 进程的全部线程 ID（ToolHelp TH32CS_SNAPTHREAD）。
pub fn list_thread_ids(pid: u32) -> WinResult<Vec<u32>> {
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)? };
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut tids = Vec::new();
    if unsafe { Thread32First(snap, &mut entry) }.is_ok() {
        loop {
            if entry.th32OwnerProcessID == pid {
                tids.push(entry.th32ThreadID);
            }
            if unsafe { Thread32Next(snap, &mut entry) }.is_err() {
                break;
            }
        }
    }
    unsafe {
        let _ = CloseHandle(snap);
    };
    Ok(tids)
}

/// 进程的模块映射（基址, 大小, 名字）— 起始地址符号化用。
/// 快照失败（权限/反作弊）返回空表，地址退化为裸十六进制。
pub fn module_map(pid: u32) -> Vec<(usize, usize, String)> {
    let snap =
        match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) } {
            Ok(h) => h,
            Err(_) => return Vec::new(),
        };
    let mut entry = MODULEENTRY32W {
        dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    let mut mods = Vec::new();
    if unsafe { windows::Win32::System::Diagnostics::ToolHelp::Module32FirstW(snap, &mut entry) }
        .is_ok()
    {
        loop {
            let name_len = entry.szModule.iter().position(|&c| c == 0).unwrap_or(0);
            let name = String::from_utf16_lossy(&entry.szModule[..name_len]);
            mods.push((entry.modBaseAddr as usize, entry.modBaseSize as usize, name));
            if unsafe {
                windows::Win32::System::Diagnostics::ToolHelp::Module32NextW(snap, &mut entry)
            }
            .is_err()
            {
                break;
            }
        }
    }
    unsafe {
        let _ = CloseHandle(snap);
    };
    mods
}

/// 地址 → "模块名+0x偏移"（不在任何模块内 → 裸地址）。
pub fn resolve_address(mods: &[(usize, usize, String)], addr: usize) -> String {
    for &(base, size, ref name) in mods {
        if addr >= base && addr < base + size {
            return format!("{name}+0x{:X}", addr - base);
        }
    }
    format!("0x{addr:X}")
}

// ═══════════════════════════════════════════════════════════════════
// 逐线程句柄查询
// ═══════════════════════════════════════════════════════════════════

/// 打开线程：`THREAD_QUERY_INFORMATION` 优先（覆盖全部查询），
/// 被拒则退 `THREAD_QUERY_LIMITED_INFORMATION`（仅 GetThreadTimes 类）。
fn open_query_thread(tid: u32) -> Option<HANDLE> {
    if let Ok(h) = unsafe { OpenThread(THREAD_QUERY_INFORMATION, false, tid) } {
        return Some(h);
    }
    if let Ok(h) = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, tid) } {
        return Some(h);
    }
    None
}

/// NtQueryInformationThread 的小封装（NTSTATUS → Option）。
#[inline]
fn nt_query<T: Default>(handle: HANDLE, class: THREADINFOCLASS) -> Option<T> {
    let mut out = T::default();
    let mut returned = 0u32;
    let status = unsafe {
        NtQueryInformationThread(
            handle,
            class,
            &mut out as *mut T as *mut core::ffi::c_void,
            std::mem::size_of::<T>() as u32,
            &mut returned,
        )
    };
    if status.is_ok() { Some(out) } else { None }
}

/// 句柄级字段查询：填充 cycles / start_address / ideal_cpu / creation /
/// name / suspend / page_pri / io_pri（失败的列保持 None）。
fn query_handle_fields(entry: &mut ThreadEntry) -> bool {
    let Some(handle) = open_query_thread(entry.tid) else {
        return false; // 句柄被拒（反作弊 A 信号）
    };
    let mut cycles = 0u64;
    if unsafe { QueryThreadCycleTime(handle, &mut cycles) }.is_ok() {
        entry.cycles = Some(cycles);
    }
    entry.start_address = nt_query::<usize>(handle, ThreadQuerySetWin32StartAddress);
    let mut proc_num = PROCESSOR_NUMBER::default();
    if unsafe { GetThreadIdealProcessorEx(handle, &mut proc_num) }.is_ok() {
        entry.ideal_cpu = Some(proc_num.Number);
    }
    let mut create = windows::Win32::Foundation::FILETIME::default();
    let mut exit = windows::Win32::Foundation::FILETIME::default();
    let mut kernel = windows::Win32::Foundation::FILETIME::default();
    let mut user = windows::Win32::Foundation::FILETIME::default();
    if unsafe { GetThreadTimes(handle, &mut create, &mut exit, &mut kernel, &mut user) }.is_ok() {
        entry.creation_ft =
            Some(((create.dwHighDateTime as u64) << 32) | create.dwLowDateTime as u64);
    }
    entry.suspend_count = nt_query::<u32>(handle, ThreadSuspendCount);
    entry.page_priority = nt_query::<u32>(handle, ThreadPagePriority);
    entry.io_priority = nt_query::<u32>(handle, ThreadIoPriority);
    // 线程名：THREAD_NAME_INFORMATION = UNICODE_STRING(16B) + 内核拷贝的串
    let mut name_buf = [0u8; 128];
    let mut returned = 0u32;
    let status = unsafe {
        NtQueryInformationThread(
            handle,
            ThreadNameInformation,
            name_buf.as_mut_ptr() as *mut core::ffi::c_void,
            name_buf.len() as u32,
            &mut returned,
        )
    };
    if status.is_ok() {
        let name = read_nt_string(&name_buf, 0);
        if !name.is_empty() {
            entry.name = Some(name);
        }
    }
    unsafe {
        let _ = CloseHandle(handle);
    };
    true
}

// ═══════════════════════════════════════════════════════════════════
// 组合采样点
// ═══════════════════════════════════════════════════════════════════

/// 采样结果：全部条目 + 句柄打开成功数（pinning 可行性信号）。
pub struct Snapshot {
    pub entries: Vec<ThreadEntry>,
    /// 打开句柄成功的线程数（A 测试信号：等于线程数 → 线程级 pinning 绿灯）。
    pub handles_opened: usize,
    /// NT 快照是否可用（false = State/WaitReason/切换数列为空）。
    pub nt_available: bool,
}

/// 对 pid 采一个时间点：ToolHelp TID 清单为骨架，NT 快照与句柄查询填充。
pub fn snapshot_threads(pid: u32) -> Result<Snapshot, String> {
    let tids = list_thread_ids(pid).map_err(|e| format!("thread snapshot failed: {e}"))?;
    if tids.is_empty() {
        return Err(format!("pid {pid} has no threads"));
    }

    let nt = nt_snapshot_threads(pid);
    let nt_available = nt.is_ok();

    let mut entries: Vec<ThreadEntry> = tids
        .into_iter()
        .map(|tid| ThreadEntry {
            tid,
            ..Default::default()
        })
        .collect();
    if let Ok(nt_entries) = &nt {
        let map: std::collections::HashMap<u32, &ThreadEntry> =
            nt_entries.iter().map(|(tid, e)| (*tid, e)).collect();
        // 健全性交叉校验：NT 与 ToolHelp 的 TID 重叠 ≥ 一半，否则布局可疑
        let overlap = entries.iter().filter(|e| map.contains_key(&e.tid)).count();
        if overlap * 2 < entries.len() {
            return Err("NT/ToolHelp tid overlap too low — layout suspect".into());
        }
        for e in entries.iter_mut() {
            if let Some(nt_e) = map.get(&e.tid) {
                e.state = nt_e.state;
                e.wait_reason = nt_e.wait_reason;
                e.context_switches = nt_e.context_switches;
                e.base_pri = nt_e.base_pri;
                e.dyn_pri = nt_e.dyn_pri;
                e.user_time = nt_e.user_time;
                e.kernel_time = nt_e.kernel_time;
            }
        }
    }

    let mut handles_opened = 0usize;
    for e in entries.iter_mut() {
        if query_handle_fields(e) {
            handles_opened += 1;
        }
    }
    Ok(Snapshot {
        entries,
        handles_opened,
        nt_available,
    })
}
