//! 窗口句柄操作 — Ghost-window defense, single entry (L3).
//!
//! 所有 HWND 操作集中于此处：安全包装函数（内部 unsafe 调用），忽略失败
//! 返回值（best-effort 语义），绝不 panic。任何缓存 HWND 使用前必须经
//! `is_valid` 重校验 — winit 延迟销毁窗口（Window::drop 只投递 DESTROY
//! 消息，实际销毁在下一事件循环启动时），FindWindowW 可能命中旧窗口。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowThreadProcessId, ICON_BIG, ICON_SMALL, IsWindow, PostMessageW,
    SetForegroundWindow, ShowWindow, HICON, SW_HIDE, SW_SHOW, WM_CLOSE, WM_SETICON,
};

/// FindWindowW(None, "GI-Utils Configuration") + IsWindow 校验。
pub fn find_main_window() -> Option<HWND> {
    unsafe {
        FindWindowW(None, windows::core::w!("GI-Utils Configuration"))
            .ok()
            .filter(|h| !h.is_invalid())
            .filter(|h| is_valid(*h))
    }
}

/// FindWindowW(Some("GIUtilsTrayWindow"), None) + IsWindow
/// + GetWindowThreadProcessId == 本进程 pid（跨进程同类名防御）。
pub fn find_tray_window() -> Option<HWND> {
    unsafe {
        let me = std::process::id();
        FindWindowW(Some(&windows::core::w!("GIUtilsTrayWindow")), None)
            .ok()
            .filter(|h| !h.is_invalid())
            .filter(|h| is_valid(*h))
            .filter(|h| {
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(*h, Some(&mut pid));
                pid == me
            })
    }
}

/// !hwnd.is_invalid() && IsWindow(Some(hwnd)).as_bool()。
pub fn is_valid(hwnd: HWND) -> bool {
    unsafe { !hwnd.is_invalid() && IsWindow(Some(hwnd)).as_bool() }
}

/// ShowWindow(hwnd, SW_HIDE)，忽略返回值。
pub fn hide_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// ShowWindow(hwnd, SW_SHOW) + SetForegroundWindow（best-effort）。
pub fn show_and_activate(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// WM_SETICON ICON_BIG + ICON_SMALL → icon（两次 PostMessageW，异步不阻塞），
/// 忽略返回值（app 帧循环 ~2s 重试兜底 — 幽灵窗口期设到旧窗口也不致命）。
///
/// 必须异步：托盘线程跨线程调用，SendMessageW 到不泵消息的窗口（渲染 panic
/// 回卷中）会无限期阻塞 — WM_CLOSE 无法处理、线程永不退出（review 发现双
/// 托盘图标根因之一）。
pub fn set_window_icon(hwnd: HWND, icon: HICON) {
    unsafe {
        let _ = PostMessageW(
            Some(hwnd),
            WM_SETICON,
            WPARAM(ICON_BIG as usize),
            LPARAM(icon.0 as isize),
        );
        let _ = PostMessageW(
            Some(hwnd),
            WM_SETICON,
            WPARAM(ICON_SMALL as usize),
            LPARAM(icon.0 as isize),
        );
    }
}

/// WM_SETICON ICON_BIG/SMALL → None（自建图标销毁前必调 — 窗口不得持有悬空
/// 句柄）。同样异步投递：消息落在已销毁窗口上即丢弃，窗口已无引用者，无害。
pub fn clear_window_icon(hwnd: HWND) {
    unsafe {
        // lParam = 0（NULL）：移除窗口图标（WM_SETICON 语义）
        let _ = PostMessageW(Some(hwnd), WM_SETICON, WPARAM(ICON_BIG as usize), LPARAM(0));
        let _ = PostMessageW(Some(hwnd), WM_SETICON, WPARAM(ICON_SMALL as usize), LPARAM(0));
    }
}

/// PostMessageW(WM_CLOSE)，异步不阻塞，忽略返回值。
pub fn post_close(hwnd: HWND) {
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}
