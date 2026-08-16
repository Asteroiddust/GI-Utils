//! 窗口句柄操作 — Ghost-window defense, single entry (L3).
//!
//! 所有 HWND 操作集中于此处：安全包装函数（内部 unsafe 调用），忽略失败
//! 返回值（best-effort 语义），绝不 panic。任何缓存 HWND 使用前必须经
//! `is_valid` 重校验 — winit 延迟销毁窗口（Window::drop 只投递 DESTROY
//! 消息，实际销毁在下一事件循环启动时），FindWindowW 可能命中旧窗口。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowThreadProcessId, ICON_BIG, ICON_SMALL, IsIconic, IsWindow,
    PostMessageW, SetForegroundWindow, ShowWindow, HICON, SW_HIDE, SW_RESTORE, SW_SHOW, WM_CLOSE,
    WM_SETICON,
};

/// FindWindowW(None, "GI-Utils Configuration") + 进程过滤 + IsWindow 校验。
///
/// FindWindowW 返回**第一个**匹配标题的顶层窗口，可能命中其他进程的同名
/// 窗口（等标题窗口虽罕见，但 hit 后所有 HWND 操作都会打向陌生窗口 —
/// review 3.7）。GetWindowThreadProcessId 必须等于本进程 id 才可信，
/// 成本仅一次 API 调用。
pub fn find_main_window() -> Option<HWND> {
    let pid = std::process::id();
    unsafe {
        FindWindowW(None, windows::core::w!("GI-Utils Configuration"))
            .ok()
            .filter(|h| !h.is_invalid())
            .filter(|h| {
                let mut wpid = 0u32;
                GetWindowThreadProcessId(*h, Some(&mut wpid));
                wpid == pid
            })
            .filter(|h| is_valid(*h))
    }
}

/// !hwnd.is_invalid() && IsWindow(Some(hwnd)).as_bool()。
/// 与 lib 侧 optimize_game::is_valid_window 语义相同（一行谓词，
/// 独立维护可接受 — review 记录）。
pub fn is_valid(hwnd: HWND) -> bool {
    unsafe { !hwnd.is_invalid() && IsWindow(Some(hwnd)).as_bool() }
}

/// ShowWindow(hwnd, SW_HIDE)，忽略返回值。
pub fn hide_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// ShowWindow + SetForegroundWindow（best-effort）。
/// SW_SHOW 不能还原最小化窗口（实证：IsIconic 不变）— 最小化时用
/// SW_RESTORE，否则 SW_SHOW（review #2，托盘 Show 静默丢失的根因）。
pub fn show_and_activate(hwnd: HWND) {
    unsafe {
        let minimized = IsIconic(hwnd).as_bool();
        let _ = ShowWindow(hwnd, if minimized { SW_RESTORE } else { SW_SHOW });
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

/// PostMessageW(WM_CLOSE)，异步不阻塞，忽略返回值。
/// 投递前 IsWindow 校验 — PostMessageW(HWND(0)) 会按 Win32 语义投到
/// 调用线程自身队列并返回成功，空句柄静默吞掉（review 发现）。
/// 仅用于**顶层主窗口**（FindWindowW 可找到）；托盘窗口是 HWND_MESSAGE
/// 消息窗口、不在顶层枚举内 — 其退出通道必须走 quit 标志（review 实证教训）。
pub fn post_close(hwnd: HWND) {
    if !is_valid(hwnd) {
        return;
    }
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

