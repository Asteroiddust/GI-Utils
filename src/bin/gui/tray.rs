//! Win32 托盘子系统 — Tray icon (Shell_NotifyIconW) subsystem.
//!
//! 独立于 GUI 状态：托盘线程通过 `Sender<TrayAction>` 与主线程通信，
//! 不引用 `GuiApp`/egui。包括托盘图标像素生成、消息窗口与消息泵。

use std::sync::mpsc::Sender;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NOTIFYICON_VERSION_4, NIF_ICON, NIF_MESSAGE, NIF_TIP,
    NIM_ADD, NIM_DELETE, NIM_SETVERSION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, FindWindowW, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, IsWindow, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetWindowLongPtrW, SendMessageW, ShowWindow, TrackPopupMenu, HICON,
    ICONINFO, MF_STRING, MSG, SW_SHOW, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP,
    WM_SETICON, WM_USER, GWLP_USERDATA, ICON_BIG, ICON_SMALL,
};

/// 托盘 → GUI 的消息类型。
pub enum TrayAction {
    Show,
    Exit,
    /// 托盘图标创建结果（NIM_ADD 成功与否）。
    Ready(bool),
}

// ═══════════════════════════════════════════════════════════════════
// 托盘图标工具
// ═══════════════════════════════════════════════════════════════════

/// 生成 32x32 RGBA 像素数据（蓝色圆形 + 白色 "G" 字样）。
pub fn create_tray_icon_pixels() -> (Vec<u8>, u32, u32) {
    let size = 32u32;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    let blue = [0x1Au8, 0x73, 0xE8, 0xFF];
    let white = [0xFFu8, 0xFF, 0xFF, 0xFF];
    let transparent = [0x00u8, 0x00, 0x00, 0x00];
    let g_shape: &[(u32, u32)] = &[
        (10,6),(11,6),(12,6),(13,6),(14,6),(15,6),(16,6),(17,6),(18,6),(19,6),(20,6),(21,6),
        (9,7),(9,8),(9,9),(9,10),(9,11),(9,12),(9,13),(9,14),(9,15),(9,16),(9,17),(9,18),(9,19),(9,20),(9,21),(9,22),(9,23),
        (10,24),(11,24),(12,24),(13,24),(14,24),(15,24),(16,24),(17,24),(18,24),(19,24),(20,24),(21,24),
        (22,19),(22,20),(22,21),(22,22),(22,23),
        (16,15),(17,15),(18,15),(19,15),(20,15),(21,15),(22,15),
    ];
    for y in 0..size {
        for x in 0..size {
            let is_g = g_shape.contains(&(x, y));
            let dx = x as f32 - 15.5f32;
            let dy = y as f32 - 15.5f32;
            let in_circle = (dx * dx + dy * dy).sqrt() < 14.5f32;
            if is_g {
                pixels.extend_from_slice(&white);
            } else if in_circle {
                pixels.extend_from_slice(&blue);
            } else {
                pixels.extend_from_slice(&transparent);
            }
        }
    }
    (pixels, size, size)
}

/// 从 RGBA 像素创建 HICON（在托盘线程内调用，HICON 非 Send）。
/// 失败返回 None — 调用方负责发送 TrayAction::Ready(false) 并退出。
unsafe fn create_hicon_from_rgba(
    rgba: &[u8],
    w: u32,
    h: u32,
) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    // RGBA → BGRA + 翻转
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    for row in (0..h).rev() {
        let start = (row * w * 4) as usize;
        let end = start + (w * 4) as usize;
        for px in rgba[start..end].chunks(4) {
            bgra.push(px[2]); // B
            bgra.push(px[1]); // G
            bgra.push(px[0]); // R
            bgra.push(px[3]); // A
        }
    }
    // AND mask 全 0（1bpp）：hbmColor 是 32bpp BGRA，alpha 通道已承载
    // 透明度。全 0xFF 会按"掩蔽"语义渲染 — 图标呈不透明方形（L1）。
    // All-zero AND mask: the 32bpp color bitmap carries alpha; an
    // all-0xFF mask would mask out every pixel and show a square icon.
    let mask_bits: Vec<u8> = vec![0; ((w * h) as usize + 7) / 8];
    let hbm_mask = windows::Win32::Graphics::Gdi::CreateBitmap(
        w as i32, h as i32, 1, 1, Some(mask_bits.as_ptr() as *const std::ffi::c_void),
    );
    let hbm_color = windows::Win32::Graphics::Gdi::CreateBitmap(
        w as i32, h as i32, 1, 32, Some(bgra.as_ptr() as *const std::ffi::c_void),
    );
    use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};

    // CreateBitmap 失败或资源不足 → 不进入 panic 路径，返回 None 优雅降级
    if hbm_mask.is_invalid() || hbm_color.is_invalid() {
        let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
        let _ = DeleteObject(HGDIOBJ(hbm_color.0));
        return None;
    }
    let icon_info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: hbm_mask,
        hbmColor: hbm_color,
    };
    let icon = match CreateIconIndirect(&icon_info) {
        Ok(icon) if !icon.is_invalid() => Some(icon),
        _ => None,
    };
    // CreateIconIndirect 复制了位图内容 — 释放临时位图（L2：避免泄漏）
    let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
    let _ = DeleteObject(HGDIOBJ(hbm_color.0));
    icon
}

/// 从 .ico 文件加载 32×32 HICON。任何失败返回 None。
/// Loads a 32×32 HICON from a .ico file. Returns None on any failure.
unsafe fn load_ico_from_file(path: &str) -> Option<HICON> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{IMAGE_ICON, LR_LOADFROMFILE, LoadImageW};

    let wide: Vec<u16> = path.encode_utf16().collect();
    match LoadImageW(
        None,
        PCWSTR(wide.as_ptr()),
        IMAGE_ICON,
        32,
        32,
        LR_LOADFROMFILE,
    ) {
        Ok(handle) if !handle.is_invalid() => Some(HICON(handle.0)),
        _ => None,
    }
}

/// 托盘图标加载入口 — 配置了 icon_path 且加载成功时用 .ico，
/// 否则回退程序生成图标（create_hicon_from_rgba）。绝不失败。
/// Tray icon entry: uses the configured .ico when it loads successfully,
/// otherwise falls back to the generated icon. Never fails.
unsafe fn load_icon(icon_path: &str, pixels: &[u8], w: u32, h: u32) -> Option<HICON> {
    if !icon_path.is_empty() {
        if let Some(icon) = load_ico_from_file(icon_path) {
            return Some(icon);
        }
        tracing::warn!(
            "tray icon '{}' could not be loaded, using generated icon",
            icon_path
        );
    }
    create_hicon_from_rgba(pixels, w, h)
}

// ═══════════════════════════════════════════════════════════════════
// Win32 托盘 (Shell_NotifyIconW)
// ═══════════════════════════════════════════════════════════════════

const WM_TRAY_CALLBACK: u32 = WM_USER + 1;
const IDM_SHOW: u32 = 1;
const IDM_EXIT: u32 = 2;

struct TrayContext {
    tx: Sender<TrayAction>,
    /// 主窗口句柄 — Cell：崩溃恢复场景下旧窗口延迟销毁（winit 延迟到
    /// 下一事件循环），缓存句柄可能失效，每次动作前重校验并按需重搜。
    main_hwnd: std::cell::Cell<HWND>,
}

unsafe fn find_main_window() -> Option<HWND> {
    FindWindowW(None, windows::core::w!("GI-Utils Configuration"))
        .ok()
        .filter(|h| !h.is_invalid())
}

/// 校验缓存的主窗口句柄，失效时重搜（崩溃恢复后旧窗口销毁 → 新窗口同标题）。
unsafe fn ensure_main_window(ctx: &TrayContext) -> HWND {
    let cur = ctx.main_hwnd.get();
    if !cur.is_invalid() && IsWindow(Some(cur)).as_bool() {
        return cur;
    }
    // 旧窗口已销毁 — 重搜当前活窗口（有界 ~2s，超时返回旧值兜底）
    for _ in 0..40 {
        if let Some(h) = find_main_window() {
            ctx.main_hwnd.set(h);
            return h;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    cur
}

unsafe fn show_main_window(main_hwnd: HWND) {
    let _ = ShowWindow(main_hwnd, SW_SHOW);
    let _ = SetForegroundWindow(main_hwnd);
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CREATE {
        let cs = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        let ctx = Box::from_raw(cs.lpCreateParams as *mut TrayContext);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(ctx) as isize);
        return LRESULT(0);
    }

    let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayContext;
    if ctx_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ctx = &*ctx_ptr;

    match msg {
        x if x == WM_TRAY_CALLBACK => {
            let lp = lparam.0 as u32;
            // Version 4: lParam packs icon ID in HIWORD, mouse msg in LOWORD
            let mouse_msg = lp & 0xFFFF;
            if mouse_msg == WM_LBUTTONDBLCLK {
                let hwnd = ensure_main_window(ctx);
                show_main_window(hwnd);
            }
            if mouse_msg == WM_RBUTTONUP {
                let mut pos = windows::Win32::Foundation::POINT::default();
                if GetCursorPos(&mut pos).is_ok() {
                    let _ = SetForegroundWindow(hwnd);
                    let menu = CreatePopupMenu().expect("CreatePopupMenu");
                    let _ = AppendMenuW(menu, MF_STRING, IDM_SHOW as usize, windows::core::w!("Show Panel"));
                    let _ = AppendMenuW(menu, MF_STRING, IDM_EXIT as usize, windows::core::w!("Exit"));
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
            return LRESULT(0);
        }
        WM_COMMAND => {
            let cmd = (wparam.0 as u32) & 0xFFFF;
            match cmd {
                IDM_SHOW => {
                    let _ = ctx.tx.send(TrayAction::Show);
                    let hwnd = ensure_main_window(ctx);
                    let _ = ShowWindow(hwnd, SW_SHOW);
                    let _ = SetForegroundWindow(hwnd);
                }
                IDM_EXIT => {
                    let _ = ctx.tx.send(TrayAction::Exit);
                }
                _ => {}
            }
            return LRESULT(0);
        }
        WM_DESTROY => {
            let _ = Box::from_raw(ctx_ptr);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            PostQuitMessage(0);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 运行托盘线程 — 创建消息窗口 + 图标，泵消息直到收到 WM_QUIT。
///
/// `icon_path` 非空且文件存在时用 `LoadImageW` 加载 .ico；否则回退
/// 程序生成图标（`pixels`）。
pub fn run_tray_thread(
    tx: Sender<TrayAction>,
    icon_path: String,
    pixels: Vec<u8>,
    w: u32,
    h: u32,
) {
    use std::mem;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{
        CS_HREDRAW, CS_VREDRAW, WNDCLASSW, HWND_MESSAGE, CW_USEDEFAULT, FindWindowW,
    };

    unsafe {
        // 在托盘线程内创建 HICON（HICON 不 Send）
        // 失败 → 通知 GUI 托盘不可用并退出，不 panic（panic 会留下
        // 无托盘的 GUI 且 close_requested 无 NIM_DELETE — 不可达）
        let icon = match load_icon(&icon_path, &pixels, w, h) {
            Some(icon) => icon,
            None => {
                let _ = tx.send(TrayAction::Ready(false));
                return;
            }
        };

        // 查找主窗口 HWND（用于 Show/Hide）
        // 主窗口在 eframe 初始化后创建；最多等 30s（60 × 500ms）。
        // 标题不匹配时不再无限空转 — 超时按托盘失败处理。
        let mut main_hwnd = None;
        for _ in 0..60 {
            if let Ok(h) = FindWindowW(None, w!("GI-Utils Configuration")) {
                main_hwnd = Some(h);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let Some(main_hwnd) = main_hwnd else {
            let _ = tx.send(TrayAction::Ready(false));
            let _ = DestroyIcon(icon);
            return;
        };

        // 窗口图标：eframe 启动时用 egui 默认 logo 设置过 WM_SETICON，
        // 这里用同一图标源（config icon_path 或程序生成）再覆盖一次，
        // 让任务栏/标题栏/Alt-Tab 与托盘图标一致。HICON 由本线程持有
        // 到退出（消息泵后才 DestroyIcon），窗口引用期间始终有效。
        let _ = SendMessageW(
            main_hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(icon.0 as isize)),
        );
        let _ = SendMessageW(
            main_hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(icon.0 as isize)),
        );

        let hinst = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(_) => {
                let _ = tx.send(TrayAction::Ready(false));
                let _ = DestroyIcon(icon);
                return;
            }
        };

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(tray_wnd_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(hinst.0),
            lpszClassName: w!("GIUtilsTrayWindow"),
            ..Default::default()
        };
        RegisterClassW(&wc);

        // tx 克隆进 ctx（Sender Clone）；外层 tx 保留用于 NIM_ADD 后发送 Ready
        let ctx = Box::new(TrayContext {
            tx: tx.clone(),
            main_hwnd: std::cell::Cell::new(main_hwnd),
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
            Some(windows::Win32::Foundation::HINSTANCE(hinst.0)),
            Some(Box::into_raw(ctx) as *const _ as *const std::ffi::c_void),
        ) {
            Ok(hwnd) => hwnd,
            Err(_) => {
                // ctx 已泄漏（无法从失败调用回收指针）— 进程级资源，可接受；
                // 必须通知 GUI 不可达路径已建立，否则托盘线程死亡后 GUI 无法退出
                let _ = tx.send(TrayAction::Ready(false));
                let _ = DestroyIcon(icon);
                return;
            }
        };

        let mut nid: NOTIFYICONDATAW = mem::zeroed();
        nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.hIcon = icon;
        nid.uCallbackMessage = WM_TRAY_CALLBACK;
        windows::core::w!("GI-Utils")
            .as_wide()
            .iter()
            .take(127)
            .enumerate()
            .for_each(|(i, c)| nid.szTip[i] = *c);

        // NIM_ADD 失败 = 托盘图标不可用，但消息窗口仍然有效 —
        // 退出路径完整（WM_CLOSE → DestroyWindow → PostQuitMessage）。
        let add_ok = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
        // Ready 发送失败 = receiver 已断开（GUI 渲染崩溃重建中）。本线程
        // 可能已误找到重试后的新主窗口（同标题）— 立即清理退出，
        // 避免双托盘图标与死通道残留。
        if tx.send(TrayAction::Ready(add_ok)).is_err() {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            let _ = DestroyIcon(icon);
            let _ = DestroyWindow(hwnd);
            return;
        }

        // NotifyIconVersion 4 (modern Win10+ behavior)
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);

        // 消息泵
        let mut msg: MSG = mem::zeroed();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                break;
            }
            DispatchMessageW(&msg);
        }

        // 清理
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        let _ = DestroyIcon(icon);
        let _ = DestroyWindow(hwnd);
    }
}
