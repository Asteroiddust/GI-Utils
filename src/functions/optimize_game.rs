//! 优化游戏 — 查找游戏窗口、提升优先级、切换前台。
//! 用于原神/崩铁/绝区零/终末地 (UnityWndClass) 以及鸣潮/黑猴 (UnrealWindow)。
//! Once 模式，按下 NumpadAdd 执行一次。

use crate::engine::function::KeyFunction;
use crate::utils;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{
    OpenProcess, SetPriorityClass, SetProcessAffinityMask, HIGH_PRIORITY_CLASS,
    PROCESS_SET_INFORMATION,
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
    hwnd: HWND,
    /// 奇数次 = 优化，偶数次 = 恢复。
    toggle: AtomicBool,
}

// 窗口句柄在进程生命周期内有效，跨线程共享是安全的。
unsafe impl Send for 优化游戏 {}
unsafe impl Sync for 优化游戏 {}

impl 优化游戏 {
    /// 创建 `优化游戏` 实例。
    ///
    /// 依次尝试查找 UnityWndClass → UnrealWindow 窗口。
    /// 找不到时不 panic，`execute` 中会重新查找。
    pub fn new() -> Self {
        Self {
            hwnd: find_game_window(),
            toggle: AtomicBool::new(false),
        }
    }
}

impl KeyFunction for 优化游戏 {
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
        // 切换奇偶：odd → 优化，even → 恢复
        let optimize = !self.toggle.fetch_xor(true, Ordering::AcqRel);

        if optimize {
            self.optimize();
        } else {
            println!("优化游戏: 恢复所有进程 CPU 亲和性");
            if let Err(e) = utils::affinity::restore_all_affinity() {
                eprintln!("优化游戏: 恢复失败: {}", e);
            }
            println!("优化游戏: 已恢复");
            utils::beep::beep_async(375, 300);
        }
    }
}

impl 优化游戏 {
    fn optimize(&self) {
        // ── 1. 窗口验证 / 重新查找 ────────────────────
        let mut hwnd = self.hwnd;
        if !is_valid_window(hwnd) {
            hwnd = find_game_window();
        }

        if !is_valid_window(hwnd) {
            eprintln!("优化游戏: 找不到游戏窗口！");
            utils::beep::beep_async(1000, 500);
            return;
        }

        // ── 2. 获取进程 PID + 窗口标题 ─────────────────
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 {
            eprintln!("优化游戏: 获取进程 ID 失败！");
            utils::beep::beep_async(1000, 500);
            return;
        }
        let title = get_window_title(hwnd);

        println!(
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
                eprintln!("优化游戏: 打开进程失败: {}", e);
                return;
            }
        };

        // ── 4. CPU 核心隔离 ─────────────────────────────
        if let Err(e) =
            unsafe { SetProcessAffinityMask(h_process, utils::affinity::GAME_CORES_MASK) }
        {
            eprintln!("优化游戏: 设置 CPU 亲和性失败: {}", e);
        }
        if let Err(e) = utils::affinity::isolate_game_cores(pid) {
            eprintln!("优化游戏: 隔离其他进程失败: {}", e);
        }

        // ── 5. 提升优先级 ───────────────────────────────
        unsafe { SetPriorityClass(h_process, HIGH_PRIORITY_CLASS) }.ok();
        unsafe { windows::Win32::Foundation::CloseHandle(h_process) }.ok();
        println!("优化游戏: 进程优先级已提升为高");

        // ── 6. 切换到前台 ─────────────────────────────
        let foreground = unsafe { GetForegroundWindow() };
        if foreground != hwnd {
            while unsafe { GetForegroundWindow() } != hwnd {
                unsafe { SwitchToThisWindow(hwnd, true) };
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        println!("优化游戏: 完成");
        utils::beep::beep_async(750, 300);
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
