//! 优化游戏 — 查找游戏窗口、提升优先级、切换前台。
//! 用于原神/崩铁/绝区零/终末地 (UnityWndClass) 以及鸣潮/黑猴 (UnrealWindow)。
//! Once 模式，按下 NumpadAdd 执行一次。

use crate::engine::bindings::KeyFunction;
use crate::utils;
use crate::utils::delay;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use tracing::{error, info};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{
    HIGH_PRIORITY_CLASS, OpenProcess, PROCESS_SET_INFORMATION, SetPriorityClass,
    SetProcessAffinityMask,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
    SwitchToThisWindow,
};

/// 优化游戏功能 — Once 模式。
///
/// 查找游戏窗口 → 获取 PID → CPU 核心隔离 → 提升优先级 → 切换前台。
/// 支持原神/崩铁/绝区零/终末地 (UnityWndClass) 以及鸣潮/黑猴 (UnrealWindow)。
pub struct 优化游戏 {
    /// 上次捕获的游戏窗口（原子存 HWND 原始值 — 实例跨线程共享，
    /// 换游戏后可刷新）。
    hwnd: AtomicIsize,
    /// 上次捕获的游戏 pid — HWND 复用双重校验：换游戏后旧句柄可能被
    /// 无关窗口复用，仅 IsWindow 不充分。
    pid: AtomicU32,
}

/// 奇偶切换状态 — 模块级 static（进程级共享）。
/// 实例会被 live-apply 与崩溃恢复重建：状态若随实例走，重建后奇偶归零，
/// 已隔离的游戏会被二次隔离而非恢复（review 发现）。
static OPTIMIZE_TOGGLE: AtomicBool = AtomicBool::new(false);

// hwnd/pid 为原子字段（HWND 以 isize 原始值存储）— Send/Sync 自动派生。

impl 优化游戏 {
    /// 创建 `优化游戏` 实例。
    ///
    /// 捕获当前游戏窗口与 pid（换游戏检测的基准）。找不到时不 panic，
    /// `execute` 中会重新查找。
    pub fn new() -> Self {
        let hwnd = find_game_window();
        let mut pid = 0u32;
        if is_valid_window(hwnd) {
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        }
        Self {
            hwnd: AtomicIsize::new(hwnd.0 as isize),
            pid: AtomicU32::new(pid),
        }
    }
}

impl KeyFunction for 优化游戏 {
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        // 换游戏检测（2026-08-22）：持有的 hwnd/pid 过时（游戏退出/句柄被
        // 复用）→ 不论奇偶，重新捕获并走**优化**方向 — 新游戏没有被优化过，
        // "恢复"对它无语义；toggle 置 true（原本 true 则不变）。找不到
        // 新游戏则按奇偶原逻辑（恢复=清场释放隔离，优化=失败重试）。
        if !self.info_valid() {
            let hwnd = find_game_window();
            if is_valid_window(hwnd) {
                self.store_info(hwnd);
                if self.optimize() {
                    OPTIMIZE_TOGGLE.store(true, Ordering::Release);
                }
                return;
            }
        }

        // 切换奇偶：odd → 优化，even → 恢复（进程级共享状态）。
        // 仅在分支成功后翻转 — 先翻后执行时失败路径会永久卡在错误的
        // 奇偶相（如找不到窗口，下次按下误走恢复 — review 4.8）。
        let optimize = !OPTIMIZE_TOGGLE.load(Ordering::Acquire);
        let succeeded = if optimize {
            self.optimize()
        } else {
            self.restore()
        };
        if succeeded {
            OPTIMIZE_TOGGLE.fetch_xor(true, Ordering::AcqRel);
        }
    }
}

// ── 换游戏检测辅助 ────────────────────────────────────────────

impl 优化游戏 {
    /// 当前持有的窗口句柄（0 = 从未捕获或无效）。
    fn current_hwnd(&self) -> HWND {
        HWND(self.hwnd.load(Ordering::Acquire) as *mut std::ffi::c_void)
    }

    /// 持有信息是否仍有效：窗口存活 **且** pid 未变。
    /// pid 双重校验防御 HWND 复用 — 换游戏后旧句柄可能被无关窗口占用，
    /// 此时 IsWindow 仍为真但归属进程已不同。
    fn info_valid(&self) -> bool {
        let hwnd = self.current_hwnd();
        if !is_valid_window(hwnd) {
            return false;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        pid != 0 && pid == self.pid.load(Ordering::Acquire)
    }

    /// 刷新持有的窗口/pid（优化成功后与新游戏捕获时调用）。
    fn store_info(&self, hwnd: HWND) {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        self.hwnd.store(hwnd.0 as isize, Ordering::Release);
        self.pid.store(pid, Ordering::Release);
    }
}

impl 优化游戏 {
    /// 恢复所有进程 CPU 亲和性。成功返回 true（供 execute 翻转奇偶）。
    fn restore(&self) -> bool {
        info!("优化游戏: 恢复所有进程 CPU 亲和性");
        if let Err(e) = utils::affinity::restore_all_affinity() {
            error!("优化游戏: 恢复失败: {}", e);
            return false;
        }
        info!("优化游戏: 已恢复");
        utils::beep::beep_async(375, 300);
        true
    }
}

impl 优化游戏 {
    /// 执行优化流程。任何一步失败返回 false（execute 不翻转奇偶，
    /// 下次按下重试同一动作）。
    fn optimize(&self) -> bool {
        // ── 1. 窗口验证 / 重新查找 ────────────────────
        let mut hwnd = self.current_hwnd();
        if !is_valid_window(hwnd) {
            hwnd = find_game_window();
        }

        if !is_valid_window(hwnd) {
            error!("优化游戏: 找不到游戏窗口！");
            utils::beep::beep_async(1000, 500);
            return false;
        }

        // ── 2. 获取进程 PID + 窗口标题 ─────────────────
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 {
            error!("优化游戏: 获取进程 ID 失败！");
            utils::beep::beep_async(1000, 500);
            return false;
        }
        let title = get_window_title(hwnd);

        info!(
            "优化游戏: {} (PID: {}, HWND: {:?})",
            if title.is_empty() {
                "(unknown)"
            } else {
                &title
            },
            pid,
            hwnd.0
        );

        // ── 3. 打开进程 ───────────────────────────────
        let h_process = match unsafe { OpenProcess(PROCESS_SET_INFORMATION, false, pid) } {
            Ok(h) => h,
            Err(e) => {
                error!("优化游戏: 打开进程失败: {}", e);
                // 对应 C++ 原版 SetProcessAffinityMaskAndPriorityClass 失败蜂鸣 —
                // 游戏反作弊（如 mhyprot）会拦截外部 OpenProcess，此路径不可静默
                utils::beep::beep_async(1000, 500);
                return false;
            }
        };

        // ── 4. CPU 核心隔离 ─────────────────────────────
        if let Err(e) =
            unsafe { SetProcessAffinityMask(h_process, utils::affinity::GAME_CORES_MASK) }
        {
            error!("优化游戏: 设置 CPU 亲和性失败: {}", e);
        }
        if let Err(e) = utils::affinity::isolate_game_cores(pid) {
            error!("优化游戏: 隔离其他进程失败: {}", e);
        }

        // ── 5. 提升优先级 ───────────────────────────────
        unsafe { SetPriorityClass(h_process, HIGH_PRIORITY_CLASS) }.ok();
        unsafe { windows::Win32::Foundation::CloseHandle(h_process) }.ok();
        info!("优化游戏: 进程优先级已提升为高");

        // ── 6. 切换到前台 ─────────────────────────────
        // 最多重试 40 次 (× 50ms = 2s)，防止前台锁定导致死循环
        let foreground = unsafe { GetForegroundWindow() };
        if foreground != hwnd {
            const MAX_RETRIES: u32 = 40;
            let mut attempts: u32 = 0;
            while unsafe { GetForegroundWindow() } != hwnd && attempts < MAX_RETRIES {
                unsafe { SwitchToThisWindow(hwnd, true) };
                delay::delay_ms(50.0);
                attempts += 1;
            }
            if attempts >= MAX_RETRIES {
                error!("优化游戏: 切换前台超时 ({}ms) — 跳过", MAX_RETRIES * 50);
                utils::beep::beep_async(500, 200);
                return false;
            }
        }
        info!("优化游戏: 完成");
        // 成功后刷新捕获基准 — 后续换游戏检测以此为对照
        self.store_info(hwnd);
        utils::beep::beep_async(750, 300);
        true
    }
}

// ── Helpers ────────────────────────────────────────────

/// 按类名依次查找游戏窗口。
fn find_game_window() -> HWND {
    unsafe {
        // 原神、崩铁、绝区零、终末地 (Unity)
        if let Ok(hwnd) = FindWindowW(windows::core::w!("UnityWndClass"), None) {
            if !hwnd.is_invalid() {
                return hwnd;
            }
        }
        // 鸣潮、黑猴 (Unreal Engine)
        if let Ok(hwnd) = FindWindowW(windows::core::w!("UnrealWindow"), None) {
            if !hwnd.is_invalid() {
                return hwnd;
            }
        }
    }
    HWND::default() // NULL handle
}

/// 检查窗口句柄是否有效。
fn is_valid_window(hwnd: HWND) -> bool {
    !hwnd.is_invalid() && unsafe { IsWindow(Some(hwnd)) }.as_bool()
}

/// 获取窗口标题。
fn get_window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}
