//! 优化游戏 — 查找游戏窗口、提升优先级、切换前台。
//! 用于原神/崩铁/绝区零/终末地 (UnityWndClass) 以及鸣潮/黑猴 (UnrealWindow)。
//! Once 模式，按下 F20 执行一次。

use crate::engine::function::KeyFunction;
use crate::utils;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{
    OpenProcess, SetPriorityClass, HIGH_PRIORITY_CLASS, PROCESS_SET_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
    SwitchToThisWindow,
};

/// 优化游戏功能 — Once 模式。
///
/// 查找游戏窗口 (UnityWndClass → UnrealWindow fallback)，
/// 提升游戏进程优先级为高，将游戏窗口切换到前台。
/// 进程核心亲和性设置代码暂时注释保留，后续可按需恢复。
pub struct 优化游戏 {
    hwnd: HWND,
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
        }
    }
}

impl KeyFunction for 优化游戏 {
    fn execute(&self, _stop_requested: Arc<AtomicBool>) {
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

        // ── 2. 获取进程 PID ───────────────────────────
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 {
            eprintln!("优化游戏: 获取进程 ID 失败！");
            utils::beep::beep_async(1000, 500);
            return;
        }

        println!("优化游戏: 找到窗口 (PID: {}, HWND: {:?})", pid, hwnd.0);

        // ── 3. 进程核心亲和性 — 暂时注释保留 ─────────
        // 如需启用 CPU 核心隔离，解除以下注释：
        //   OpenProcess → SetProcessAffinityMask(game, GAME_CORES_MASK)
        //                  + isolate_game_cores(pid)

        // ── 4. 提升优先级 ─────────────────────────────
        match unsafe { OpenProcess(PROCESS_SET_INFORMATION, false, pid) } {
            Ok(h) => {
                unsafe { SetPriorityClass(h, HIGH_PRIORITY_CLASS) }.ok();
                unsafe { windows::Win32::Foundation::CloseHandle(h) }.ok();
                println!("优化游戏: 进程优先级已提升为高");
            }
            Err(e) => {
                eprintln!("优化游戏: 打开进程失败: {}", e);
            }
        }

        // ── 5. 切换到前台 ─────────────────────────────
        let foreground = unsafe { GetForegroundWindow() };
        if foreground != hwnd {
            while unsafe { GetForegroundWindow() } != hwnd {
                unsafe { SwitchToThisWindow(hwnd, true) };
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            utils::beep::beep_async(750, 300);
        }

        println!("优化游戏: 完成");
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
