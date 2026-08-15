//! Win32 托盘子系统 — Tray icon (Shell_NotifyIconW) subsystem.
//!
//! 独立于 GUI 状态：托盘线程通过 `Sender<TrayAction>` 与主线程通信，
//! 不引用 `GuiApp`/egui。本文件只含托盘线程（消息窗口 + 消息泵）与
//! `TrayAction`；图标原料/共享句柄在 `tray_icon`，HWND 操作在 `window_ops`
//! （本文件只调用，不重复实现）。
//!
//! 模块依赖：main.rs 需声明同级模块 `mod tray_icon; mod window_ops;`
//! （与 `mod tray;` 同层）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NOTIFYICON_VERSION_4, NIF_ICON, NIF_MESSAGE, NIF_TIP,
    NIM_ADD, NIM_DELETE, NIM_SETVERSION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, GetWindowLongPtrW,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HWND_MESSAGE, MF_STRING, MSG,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_DESTROY,
    WM_LBUTTONDBLCLK, WM_RBUTTONUP, WM_USER, WNDCLASSW,
};

/// 托盘 → GUI 的消息类型。
pub enum TrayAction {
    /// 双击 / 菜单 "Show Panel" → GUI 帧：hidden=false + SW_SHOW。
    Show,
    /// 菜单 "Exit" → GUI 帧：should_exit=true + ViewportCommand::Close。
    Exit,
    /// NIM_ADD 结果 → GUI 帧：写 tray_ok；false 时记日志。
    Ready(bool),
}

// ═══════════════════════════════════════════════════════════════════
// 托盘消息窗口
// ═══════════════════════════════════════════════════════════════════

const WM_TRAY_CALLBACK: u32 = WM_USER + 1;
const IDM_SHOW: u32 = 1;
const IDM_EXIT: u32 = 2;

struct TrayContext {
    tx: Sender<TrayAction>,
    /// 缓存主窗口句柄 — Cell：每次动作前 IsWindow 重校验（幽灵窗口，L3）。
    main_hwnd: std::cell::Cell<HWND>,
    /// 线程退出请求 — stop_tray_thread 置位；⑨ 搜索循环与 ensure_main_window
    /// 每轮检查立即退出（WM_CLOSE 在搜索期间不泵消息、不可达，必须由标志
    /// 接管 — L5 的 Ready 守卫只覆盖 ⑦ 之前，review 发现）。
    quit: Arc<AtomicBool>,
}

/// 校验缓存的主窗口句柄，失效时重搜（崩溃恢复后旧窗口销毁 → 新窗口同标题）。
/// 缓存句柄有效（IsWindow）→ 直接返回；否则有界 2s 重搜（40×50ms
/// find_main_window）；超时返回旧值兜底（调用方是 best-effort 路径）。
unsafe fn ensure_main_window(ctx: &TrayContext) -> HWND {
    let cur = ctx.main_hwnd.get();
    if crate::window_ops::is_valid(cur) {
        return cur;
    }
    // 旧窗口已销毁 — 重搜当前活窗口（有界 ~2s，超时返回旧值兜底）；
    // 每轮检查退出请求 — stop_tray_thread 等待期必须覆盖整个生命周期
    for _ in 0..40 {
        if ctx.quit.load(Ordering::Acquire) {
            return cur;
        }
        if let Some(h) = crate::window_ops::find_main_window() {
            ctx.main_hwnd.set(h);
            return h;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    cur
}

/// show_and_activate(hwnd) 的托盘侧别名（即 SW_SHOW + SetForegroundWindow）。
unsafe fn show_main_window(hwnd: HWND) {
    crate::window_ops::show_and_activate(hwnd);
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // GWLP_USERDATA 空指针 → DefWindowProcW 兜底（窗口创建成功才写指针）
    let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayContext;
    if ctx_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ctx = &*ctx_ptr;

    match msg {
        x if x == WM_TRAY_CALLBACK => {
            // LOWORD(lParam) 是鼠标消息（V4 与降级 V0 一致；HIWORD 是图标 ID）
            let mouse_msg = (lparam.0 as u32) & 0xFFFF;
            if mouse_msg == WM_LBUTTONDBLCLK {
                // 双击 → 通知 GUI 帧（hidden=false，L11）+ 直接显示窗口
                let _ = ctx.tx.send(TrayAction::Show);
                let main_hwnd = ensure_main_window(ctx);
                show_main_window(main_hwnd);
            }
            if mouse_msg == WM_RBUTTONUP {
                let mut pos = windows::Win32::Foundation::POINT::default();
                if GetCursorPos(&mut pos).is_ok() {
                    let _ = SetForegroundWindow(hwnd);
                    // CreatePopupMenu 失败仅跳过，绝不 panic（不变量 17）
                    if let Ok(menu) = CreatePopupMenu() {
                        let _ = AppendMenuW(
                            menu,
                            MF_STRING,
                            IDM_SHOW as usize,
                            windows::core::w!("Show Panel"),
                        );
                        let _ = AppendMenuW(
                            menu,
                            MF_STRING,
                            IDM_EXIT as usize,
                            windows::core::w!("Exit"),
                        );
                        let _ = TrackPopupMenu(
                            menu,
                            TPM_BOTTOMALIGN | TPM_LEFTALIGN,
                            pos.x,
                            pos.y,
                            None,
                            hwnd,
                            None,
                        );
                        let _ = DestroyMenu(menu);
                    }
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd = (wparam.0 as u32) & 0xFFFF;
            match cmd {
                IDM_SHOW => {
                    let _ = ctx.tx.send(TrayAction::Show);
                    let main_hwnd = ensure_main_window(ctx);
                    show_main_window(main_hwnd);
                }
                IDM_EXIT => {
                    let _ = ctx.tx.send(TrayAction::Exit);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // 释放 ctx（恰一次 — GWLP_USERDATA 随即清零，防双重释放）
            let _ = Box::from_raw(ctx_ptr);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ═══════════════════════════════════════════════════════════════════
// 托盘线程主函数
// ═══════════════════════════════════════════════════════════════════

/// 托盘线程主函数 — 每轮尝试独立 spawn，独立 channel；线程内绝不 panic
/// （全部 Win32 调用容错，无 expect）。阶段顺序单一收尾尾（⑪），消除旧
/// 实现 4 处重复 early-exit 清理块。
///
/// 图标来源：主线程预加载的共享图标（`preloaded`，崩溃恢复轮也能用）；
/// 预加载失败时回退程序生成图标（`pixels`，纯 GDI 路径 — WIC 污染后仍可用）。
///
/// 线程不显式 pin 核心（继承进程掩码 12-15 即可，非时序关键，减少与渲染
/// 线程竞争面 — 裁决 #11）。
pub fn run_tray_thread(
    tx: Sender<TrayAction>,
    quit: Arc<AtomicBool>,
    preloaded: Option<crate::tray_icon::SharedIcon>,
    pixels: Vec<u8>,
    w: u32,
    h: u32,
) {
    use std::mem;
    use windows::core::w;

    unsafe {
        // ── ① 图标解析 ──────────────────────────────────────────
        // 优先共享预加载（崩溃恢复轮可用）；否则本线程生成（纯 GDI 路径，
        // 健康状态不依赖）。icon_owned 标记自建图标 — 只有自建图标由本
        // 线程销毁（⑦/⑪ 路径）；共享图标由主线程 shutdown 时统一销毁。
        let (icon, icon_owned) = match preloaded {
            Some(shared) => (shared.raw(), false),
            None => {
                // 失败 → 通知 GUI 托盘不可用并退出；无窗口无图标可清
                let Some(icon) = crate::tray_icon::create_hicon_from_rgba(&pixels, w, h) else {
                    let _ = tx.send(TrayAction::Ready(false));
                    return;
                };
                (icon, true)
            }
        };

        // ── ③ 模块句柄 ──────────────────────────────────────────
        // 契约②③ 因 hInstance 数据依赖交换执行顺序：RegisterClassW 与
        // CreateWindowExW 均需 hInstance，GetModuleHandleW 必须先取；
        // 各阶段的失败语义不变。
        let hinst = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(_) => {
                let _ = tx.send(TrayAction::Ready(false));
                if icon_owned {
                    let _ = DestroyIcon(icon);
                }
                return;
            }
        };

        // ── ② 注册窗口类 ────────────────────────────────────────
        // ERROR_CLASS_ALREADY_EXISTS 良性继续：detach 宽限期旧类记录可
        // 复用（同类名同静态 wndproc，行为一致）；其他失败 → Ready(false)
        // + 自建图标销毁 → return。
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(tray_wnd_proc),
            hInstance: HINSTANCE(hinst.0),
            lpszClassName: w!("GIUtilsTrayWindow"),
            ..Default::default()
        };
        // RegisterClassW 返回 ATOM（0 = 失败），失败原因经 GetLastError 区分
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            let _ = tx.send(TrayAction::Ready(false));
            if icon_owned {
                let _ = DestroyIcon(icon);
            }
            return;
        }

        // ── ④ 创建消息窗口（HWND_MESSAGE, lpParam=NULL）─────────
        // lpParam=NULL：ctx 指针改为创建后经 SetWindowLongPtrW 写入（⑤），
        // 窗口创建完成前无消息可达（消息窗口消息只经消息泵派发），无竞态。
        // ctx 是普通 Box — 创建失败时未写入窗口，随作用域 drop，无泄漏。
        let ctx = Box::new(TrayContext {
            tx: tx.clone(),
            main_hwnd: std::cell::Cell::new(HWND::default()),
            quit: quit.clone(),
        });
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("GIUtilsTrayWindow"),
            w!(""),
            WINDOW_STYLE::default(),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(HWND_MESSAGE),
            None,
            Some(HINSTANCE(hinst.0)),
            None, // lpParam = NULL（契约④）
        ) {
            Ok(hwnd) => hwnd,
            Err(_) => {
                let _ = tx.send(TrayAction::Ready(false));
                if icon_owned {
                    let _ = DestroyIcon(icon);
                }
                return;
            }
        };

        // ── ⑤ 写入 ctx 指针（先设指针后 NIM_ADD — 回调不可能早到）─
        // SetWindowLongPtrW 对有效窗口几乎不失败；失败则 ctx 泄漏（进程级
        // 单窗口，可接受）。best-effort。
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(ctx) as isize);

        // ── ⑥ NIM_ADD ───────────────────────────────────────────
        let mut nid: NOTIFYICONDATAW = mem::zeroed();
        nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.hIcon = icon;
        nid.uCallbackMessage = WM_TRAY_CALLBACK;
        // szTip 写 "GI-Utils" 后显式 NUL 终止 — 防 shell 越界读（D2 #8）
        let tip = w!("GI-Utils");
        let tip_len = tip.as_wide().len().min(127);
        nid.szTip[..tip_len].copy_from_slice(&tip.as_wide()[..tip_len]);
        nid.szTip[tip_len] = 0;

        // NIM_ADD 失败 = 托盘图标不可用，但消息窗口仍然有效 —
        // 退出路径完整（WM_CLOSE → DestroyWindow → PostQuitMessage，L5）。
        let add_ok = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();

        // ── ⑦ Ready 发送 ────────────────────────────────────────
        // 发送失败 = receiver 已断开（GUI 重建中）→ 立即自清理退出（防双
        // 托盘图标）：自建图标先 clear_window_icon(缓存的 main_hwnd) 再
        // DestroyIcon → NIM_DELETE → DestroyWindow。共享图标绝不销毁。
        if tx.send(TrayAction::Ready(add_ok)).is_err() {
            if icon_owned {
                let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayContext;
                if !ctx_ptr.is_null() {
                    let cached = (*ctx_ptr).main_hwnd.get();
                    if crate::window_ops::is_valid(cached) {
                        crate::window_ops::clear_window_icon(cached);
                    }
                }
                let _ = DestroyIcon(icon);
            }
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            let _ = DestroyWindow(hwnd);
            return;
        }

        // ── ⑧ NIM_SETVERSION ────────────────────────────────────
        // 失败仅日志：降级 V0（LOWORD 仍为消息，菜单路径不变，双击可能
        // 失效 — 接受）。
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        if !Shell_NotifyIconW(NIM_SETVERSION, &nid).as_bool() {
            tracing::warn!(
                "NIM_SETVERSION failed — falling back to pre-v4 behavior (double-click may not work)"
            );
        }

        // ── ⑨ 主窗口查找（有界 30s，60×500ms）— 非致命 ──────────
        // NIM_ADD 与主窗口解耦（裁决 #2）：超时仅降级 Show/图标重设，由
        // ensure_main_window 有界重搜 + app 帧循环 deadline 兜底。
        // 每轮检查 quit — stop_tray_thread 置位后立即 break 走收尾，2s 有界
        // 等待才能覆盖整个线程生命周期（review 发现双托盘图标根因）。
        let mut main_hwnd = HWND::default();
        let mut quit_requested = false;
        for _ in 0..60 {
            if quit.load(Ordering::Acquire) {
                quit_requested = true;
                break;
            }
            if let Some(h) = crate::window_ops::find_main_window() {
                main_hwnd = h;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        // 缓存进 ctx（供 wndproc 的 ensure_main_window 复用）+ 窗口图标重设
        // （best-effort：幽灵窗口期可能设到旧窗口，app 帧循环兜底重设真窗口；
        // set_window_icon 已改为 PostMessageW 异步 — 不阻塞退出路径）。
        let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayContext;
        if !ctx_ptr.is_null() {
            let ctx = &*ctx_ptr;
            ctx.main_hwnd.set(main_hwnd);
            if !main_hwnd.is_invalid() {
                crate::window_ops::set_window_icon(main_hwnd, icon);
            }
        }
        if main_hwnd.is_invalid() && !quit_requested {
            tracing::warn!(
                "main window not found within 30s — tray Show/icon degraded (app frame deadline covers)"
            );
        }
        // ⑪ 收尾需读缓存句柄，但消息泵退出时窗口已销毁（WM_DESTROY 已释放
        // ctx）— 泵前快照，避免 use-after-free。
        let cached_main_hwnd = main_hwnd;

        // ── ⑩ 消息泵 ────────────────────────────────────────────
        let mut msg: MSG = mem::zeroed();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 < 0 {
                // 异常终止 — 仍走清理尾（L5）
                tracing::warn!("GetMessageW returned {} — terminating tray thread", ret.0);
                break;
            }
            if ret.0 == 0 {
                break; // WM_QUIT（WM_DESTROY → PostQuitMessage）
            }
            DispatchMessageW(&msg);
        }

        // ── ⑪ 单一收尾尾 ────────────────────────────────────────
        // 自建图标先 clear_window_icon(缓存的 main_hwnd，IsWindow 校验后)
        // 再 DestroyIcon → NIM_DELETE → DestroyWindow。共享图标绝不销毁
        // （主线程 shutdown 时统一处理）。
        if icon_owned {
            if crate::window_ops::is_valid(cached_main_hwnd) {
                crate::window_ops::clear_window_icon(cached_main_hwnd);
            }
            let _ = DestroyIcon(icon);
        }
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        let _ = DestroyWindow(hwnd);
    }
}
