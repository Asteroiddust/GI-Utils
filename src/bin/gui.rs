//! GI-Utils GUI 配置面板 — GUI Configuration Panel (egui).
//!
//! 可视化绑定管理：增删改按键绑定，修改即时生效，保存写入 config.toml。
//! Visual binding management: add/edit/delete key bindings,
//! live-apply changes, save to config.toml.

#![windows_subsystem = "windows"]

use eframe::egui;
use gi_utils::config::{self, Binding};
use gi_utils::engine::function::KeyFunction;
use gi_utils::engine::Engine;
use gi_utils::engine::TriggerMode;
use gi_utils::interception::SendContext;
use gi_utils::key::Key;
use gi_utils::utils;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NOTIFYICON_VERSION_4, NIF_ICON, NIF_MESSAGE, NIF_TIP,
    NIM_ADD, NIM_DELETE, NIM_SETVERSION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, FindWindowW, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    SetWindowLongPtrW, ShowWindow, TrackPopupMenu, ICONINFO, MF_STRING, MSG, SW_HIDE, SW_SHOW,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_CREATE,
    WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WM_USER, GWLP_USERDATA,
};

// ═══════════════════════════════════════════════════════════════════
// GuiBinding — 表格中一行绑定数据
// ═══════════════════════════════════════════════════════════════════

struct GuiBinding {
    id: usize,
    /// None 表示尚未设置按键（新增行等待捕获）。
    key: Option<Key>,
    /// 配置名如 "F13"、"NumpadAdd"。
    key_name: String,
    func: String,
    mode: TriggerMode,
}

// ═══════════════════════════════════════════════════════════════════
// GuiApp — egui 应用状态
// ═══════════════════════════════════════════════════════════════════

struct GuiApp {
    /// 绑定列表（GUI 显示用）。
    bindings_list: Vec<GuiBinding>,
    next_id: usize,
    dirty: bool,
    error_msg: Option<String>,

    /// 共享的 Engine 绑定注册表。
    key_bindings: Arc<gi_utils::engine::bindings::KeyBindings>,
    /// 用于创建功能实例。
    send_ctx: Arc<SendContext>,
    /// 停止 Engine 的信号。
    stop_flag: Arc<AtomicBool>,
    /// Engine 后台线程句柄。
    engine_handle: Option<JoinHandle<()>>,

    /// 按键捕获状态。
    capture: CaptureState,

    /// 所有可用功能名称（下拉框选项）。
    function_names: Vec<&'static str>,

    /// CJK 字体是否已加载。
    font_loaded: bool,
    /// GUI 内嵌日志。
    log_messages: Vec<String>,
    /// 日志面板是否可见。
    log_visible: bool,

    /// 托盘消息接收端。
    tray_rx: Receiver<TrayAction>,
    /// 真正的退出标志（托盘菜单 Exit 或 F12 触发）。
    should_exit: bool,
}

/// 托盘 → GUI 的消息类型。
enum TrayAction {
    Show,
    Exit,
}

/// 按键捕获状态。
struct CaptureState {
    active: bool,
    binding_idx: Option<usize>,
    rx: Option<Receiver<Key>>,
}

// ═══════════════════════════════════════════════════════════════════
// eframe::App implementation
// ═══════════════════════════════════════════════════════════════════

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 每 100ms 唤醒一次，确保隐藏窗口时也能收到托盘消息
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        // -1. 处理托盘消息
        match self.tray_rx.try_recv() {
            Ok(TrayAction::Show) => {
                ctx.request_repaint();
            }
            Ok(TrayAction::Exit) => {
                self.should_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }

        // -0.8. 监控 Engine stop_flag（F12 按下时 Engine 设置此标志）
        if self.stop_flag.load(Ordering::Acquire) {
            self.should_exit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // -0.5. 窗口关闭 → 隐藏到托盘（除非托盘菜单或 F12 触发退出）
        if ctx.input(|i| i.viewport().close_requested()) && !self.should_exit {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            // 用原生 ShowWindow(SW_HIDE) 替代 egui 的 Visible(false)
            // egui 的 Visible(false) 会导致 update() 停止调用，托盘消息无法处理
            unsafe {
                if let Ok(hwnd) = FindWindowW(None, windows::core::w!("GI-Utils Configuration")) {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
        }

        // 0. 首次加载 CJK 字体
        if !self.font_loaded {
            self.load_cjk_font(ctx);
            self.font_loaded = true;
        }

        // 1. 处理异步事件
        self.handle_capture_result();

        // 2. 请求重绘（捕获等待期间需要轮询 mpsc channel）
        if self.capture.active {
            ctx.request_repaint();
        }

        // 3. 渲染 UI
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("GI-Utils v1.0.0  Configuration");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.log_visible { "▸ Log" } else { "▹ Log" };
                    if ui.button(label).clicked() {
                        self.log_visible = !self.log_visible;
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.show_binding_table(ui);
                ui.add_space(8.0);
                self.show_action_buttons(ui);
            });
        });

        // 4. 右侧日志面板
        if self.log_visible {
            egui::SidePanel::right("log_panel")
                .resizable(true)
                .default_width(320.0)
                .min_width(160.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("Log");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").clicked() {
                                self.log_visible = false;
                            }
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for msg in &self.log_messages {
                                ui.label(msg.as_str());
                            }
                        });
                });
        }

        // 5. 状态栏
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Status: Running");
                if self.dirty {
                    ui.separator();
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 200, 0),
                        "(unsaved changes)",
                    );
                }
            });
        });

        // 6. 弹窗
        self.show_capture_dialog(ctx);
        self.show_error_dialog(ctx);
    }
}

// ═══════════════════════════════════════════════════════════════════
// UI 渲染方法
// ═══════════════════════════════════════════════════════════════════

impl GuiApp {
    /// 绑定表格 — egui::Grid 展示所有绑定。
    fn show_binding_table(&mut self, ui: &mut egui::Ui) {
        let mut need_apply = false;
        let mut remove_idx: Option<usize> = None;
        let mut capture_idx: Option<usize> = None;
        let function_names = self.function_names.clone(); // 循环外克隆一次

        egui::ScrollArea::horizontal().show(ui, |ui| {
            egui::Grid::new("binding_grid")
                .striped(true)
                .min_col_width(80.0)
                .show(ui, |ui| {
                    ui.strong("Key");
                    ui.strong("Function");
                    ui.strong("Mode");
                    ui.strong("Actions");
                    ui.end_row();

                    for (i, binding) in self.bindings_list.iter_mut().enumerate() {
                        // ---- Key 列 ----
                        if self.capture.active && self.capture.binding_idx == Some(i) {
                            ui.label("(capturing...)");
                        } else if let Some(ref name) = binding.key.map(|_| binding.key_name.clone()) {
                            ui.label(name);
                        } else {
                            ui.colored_label(
                                egui::Color32::from_rgb(150, 150, 150),
                                "[not set]",
                            );
                        }

                        // ---- Function 列 ----
                        let func_before = binding.func.clone();
                        egui::ComboBox::from_id_salt(format!("func_{}", binding.id))
                            .selected_text(func_before.as_str())
                            .show_ui(ui, |ui| {
                                for name in &function_names {
                                    if ui.selectable_label(binding.func == *name, *name).clicked() {
                                        binding.func = name.to_string();
                                    }
                                }
                            });
                        if binding.func != func_before {
                            need_apply = true;
                        }

                        // ---- Mode 列 ----
                        let mode_before = binding.mode;
                        egui::ComboBox::from_id_salt(format!("mode_{}", binding.id))
                            .selected_text(format!("{:?}", binding.mode))
                            .show_ui(ui, |ui| {
                                for &m in &[TriggerMode::Once, TriggerMode::Loop, TriggerMode::Toggle] {
                                    if ui.selectable_label(binding.mode == m, format!("{:?}", m)).clicked() {
                                        binding.mode = m;
                                    }
                                }
                            });
                        if binding.mode != mode_before {
                            need_apply = true;
                        }

                        // ---- Actions 列 ----
                        ui.horizontal(|ui| {
                            if ui.button("Set Key").clicked() {
                                capture_idx = Some(i);
                            }
                            if ui.small_button("Del").clicked() {
                                remove_idx = Some(i);
                            }
                        });

                        ui.end_row();
                    }
                });
        });

        // 延迟处理（避免在 grid 闭包中 borrow self）
        if let Some(idx) = remove_idx {
            self.bindings_list.remove(idx);
            need_apply = true;
        }
        if let Some(idx) = capture_idx {
            self.start_capture(idx);
        }
        if need_apply {
            self.live_apply();
        }
    }

    /// 操作按钮 — 新增 / 保存。
    fn show_action_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("+ Add Binding").clicked() {
                let id = self.next_id;
                self.next_id += 1;
                let idx = self.bindings_list.len();
                self.bindings_list.push(GuiBinding {
                    id,
                    key: None,
                    key_name: "...".into(),
                    func: self
                        .function_names
                        .first()
                        .copied()
                        .unwrap_or("连点器")
                        .into(),
                    mode: TriggerMode::Loop,
                });
                // 新增行自动进入按键捕获
                self.start_capture(idx);
            }

            if ui.button("Save to Config").clicked() {
                match self.save_config() {
                    Ok(()) => self.dirty = false,
                    Err(e) => self.error_msg = Some(e),
                }
            }
        });
    }

    /// 按键捕获弹窗。
    fn show_capture_dialog(&mut self, ctx: &egui::Context) {
        if !self.capture.active {
            return;
        }
        egui::Window::new("Set Key")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label("Press any key on your keyboard...");
                ui.add_space(8.0);
                ui.label("Waiting for key press...");
            });
    }

    /// 错误弹窗。
    fn show_error_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        if let Some(ref msg) = self.error_msg.clone() {
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.label(msg);
                    ui.add_space(8.0);
                    if ui.button("OK").clicked() {
                        close = true;
                    }
                });
        }
        if close {
            self.error_msg = None;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// 业务逻辑
// ═══════════════════════════════════════════════════════════════════

impl GuiApp {
    /// 加载系统字体（中文 + 符号），注入 egui 字体系统作为 fallback。
    fn load_cjk_font(&mut self, ctx: &egui::Context) {
        let cjk_font = [r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\simsun.ttc"]
            .iter()
            .find_map(|path| std::fs::read(path).ok().map(|b| egui::FontData::from_owned(b).into()));

        let sym_font = std::fs::read(r"C:\Windows\Fonts\seguisym.ttf")
            .ok()
            .map(|b| egui::FontData::from_owned(b).into());

        if cjk_font.is_some() || sym_font.is_some() {
            let mut fonts = egui::FontDefinitions::default();
            let fam = fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap();
            // 降级链: 默认 Latin → 符号 (▸✕) → CJK 中文
            if let Some(f) = sym_font {
                fonts.font_data.insert("symbol".into(), f);
                fam.push("symbol".into());
            }
            if let Some(f) = cjk_font {
                fonts.font_data.insert("cjk".into(), f);
                fam.push("cjk".into());
            }
            // Monospace 同理
            let fam_mono = fonts
                .families
                .get_mut(&egui::FontFamily::Monospace)
                .unwrap();
            if fonts.font_data.contains_key("symbol") {
                fam_mono.push("symbol".into());
            }
            if fonts.font_data.contains_key("cjk") {
                fam_mono.push("cjk".into());
            }
            ctx.set_fonts(fonts);
            self.log("Fallback fonts: symbol + CJK loaded.");
        } else {
            self.log("WARNING: No fallback fonts found.");
        }
    }

    /// 向 GUI 日志面板追加一条消息。
    fn log(&mut self, msg: impl Into<String>) {
        self.log_messages.push(msg.into());
    }
    /// 开始按键捕获。
    fn start_capture(&mut self, binding_idx: usize) {
        self.capture.active = true;
        self.capture.binding_idx = Some(binding_idx);
        self.capture.rx = Some(self.key_bindings.enable_capture());
    }

    /// 检查是否有捕获到的按键。
    fn handle_capture_result(&mut self) {
        if !self.capture.active {
            return;
        }
        if let Some(ref rx) = self.capture.rx {
            if let Ok(key) = rx.try_recv() {
                let name = config::key_to_config_name(key)
                    .unwrap_or_else(|| key.name())
                    .to_string();

                if let Some(idx) = self.capture.binding_idx {
                    if let Some(binding) = self.bindings_list.get_mut(idx) {
                        binding.key = Some(key);
                        binding.key_name = name;
                    }
                }

                self.capture.active = false;
                self.capture.binding_idx = None;
                self.capture.rx = None;
                self.live_apply();
            }
        }
    }

    /// 即时应用：将所有 GUI 绑定注册到 Engine 的 KeyBindings。
    fn live_apply(&mut self) {
        // 先清空旧绑定，再全量重新注册（覆盖删除/改键的条目）。
        self.key_bindings.clear_all();

        for g in &self.bindings_list {
            let key = match g.key {
                Some(k) => k,
                None => continue,
            };

            let func: Arc<dyn KeyFunction> = if g.func == "停止退出" {
                Arc::new(gi_utils::functions::stop::停止退出::new(
                    self.stop_flag.clone(),
                ))
            } else {
                match config::create_function(&g.func, self.send_ctx.clone()) {
                    Ok(f) => f,
                    Err(e) => {
                        self.error_msg = Some(format!("'{}': {}", g.func, e));
                        continue;
                    }
                }
            };

            self.key_bindings.register(key, g.mode, func);
        }
        self.dirty = true; // 标记为有未保存变更
    }

    /// 保存绑定列表到 config.toml。
    fn save_config(&self) -> Result<(), String> {
        let bindings: Vec<Binding> = self
            .bindings_list
            .iter()
            .filter_map(|g| {
                g.key.map(|key| Binding {
                    key,
                    func: g.func.clone(),
                    mode: g.mode,
                })
            })
            .collect();

        // 验证双向唯一性
        {
            let mut keys = std::collections::HashSet::new();
            let mut funcs = std::collections::HashSet::new();
            for b in &bindings {
                if !keys.insert(b.key) {
                    return Err(format!(
                        "duplicate key: '{}' is bound to multiple functions",
                        b.key.name()
                    ));
                }
                if !funcs.insert(b.func.clone()) {
                    return Err(format!(
                        "duplicate function: '{}' is bound to multiple keys",
                        b.func
                    ));
                }
            }
        }

        config::save(&bindings)
    }
}

// ═══════════════════════════════════════════════════════════════════
// Drop — 清理关闭
// ═══════════════════════════════════════════════════════════════════

impl Drop for GuiApp {
    fn drop(&mut self) {
        // 取消按键捕获
        self.key_bindings.disable_capture();

        // 信号 Engine 停止
        self.stop_flag.store(true, Ordering::Release);

        // 等待 Engine 线程退出
        if let Some(handle) = self.engine_handle.take() {
            let _ = handle.join();
        }
        // Engine Drop → KeyBindings Drop → stop_all() → join 所有功能线程

        // 退出蜂鸣
        gi_utils::utils::beep::beep(375, 300);

        // 恢复所有进程的完整 CPU 亲和性（best-effort，静默吞错误）
        let _ = gi_utils::utils::affinity::restore_all_affinity();
    }
}

// ═══════════════════════════════════════════════════════════════════
// 托盘图标工具
// ═══════════════════════════════════════════════════════════════════

/// 生成 32x32 RGBA 像素数据（蓝色圆形 + 白色 "G" 字样）。
fn create_tray_icon_pixels() -> (Vec<u8>, u32, u32) {
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
unsafe fn create_hicon_from_rgba(rgba: &[u8], w: u32, h: u32) -> windows::Win32::UI::WindowsAndMessaging::HICON {
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
    let mask_bits: Vec<u8> = vec![0xFF; (w * h) as usize];
    let hbm_mask = windows::Win32::Graphics::Gdi::CreateBitmap(
        w as i32, h as i32, 1, 1, Some(mask_bits.as_ptr() as *const std::ffi::c_void),
    );
    let hbm_color = windows::Win32::Graphics::Gdi::CreateBitmap(
        w as i32, h as i32, 1, 32, Some(bgra.as_ptr() as *const std::ffi::c_void),
    );
    let icon_info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: hbm_mask,
        hbmColor: hbm_color,
    };
    CreateIconIndirect(&icon_info).expect("CreateIconIndirect failed")
}

// ═══════════════════════════════════════════════════════════════════
// Win32 托盘 (Shell_NotifyIconW)
// ═══════════════════════════════════════════════════════════════════

const WM_TRAY_CALLBACK: u32 = WM_USER + 1;
const IDM_SHOW: u32 = 1;
const IDM_EXIT: u32 = 2;

struct TrayContext {
    tx: Sender<TrayAction>,
    main_hwnd: HWND,
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
                show_main_window(ctx.main_hwnd);
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
                    let _ = ShowWindow(ctx.main_hwnd, SW_SHOW);
                    let _ = SetForegroundWindow(ctx.main_hwnd);
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

fn run_tray_thread(
    tx: Sender<TrayAction>,
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
        let icon = create_hicon_from_rgba(&pixels, w, h);

        // 查找主窗口 HWND（用于 Show/Hide）
        let main_hwnd = loop {
            let h = FindWindowW(None, w!("GI-Utils Configuration"));
            if h.is_ok() {
                break h.unwrap();
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        };

        let hinst = GetModuleHandleW(None).expect("GetModuleHandleW");

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(tray_wnd_proc),
            hInstance: windows::Win32::Foundation::HINSTANCE(hinst.0),
            lpszClassName: w!("GIUtilsTrayWindow"),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let ctx = Box::new(TrayContext { tx, main_hwnd });
        let hwnd = CreateWindowExW(
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
        )
        .expect("CreateWindowExW for tray");

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

        let _ = Shell_NotifyIconW(NIM_ADD, &nid);

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


// ═══════════════════════════════════════════════════════════════════
// main — GUI 入口
// ═══════════════════════════════════════════════════════════════════

fn main() {
    // 启动日志（GUI 无控制台，收集到内存后在日志面板显示）
    let mut startup_log: Vec<String> = vec![
        "GI-Utils GUI v1.0.0".into(),
        "Initializing...".into(),
    ];

    // ── 1. 系统初始化 ───────────────────────────────────────
    utils::init();
    startup_log.push("DPI awareness: PER_MONITOR_AWARE_V2".into());
    startup_log.push("Priority: real-time, affinity core set.".into());
    startup_log.push("TSC frequency calibrated.".into());

    // ── 2. 加载配置 ─────────────────────────────────────────
    let config_bindings = match config::load() {
        Ok(b) => {
            startup_log.push(format!("Loaded {} bindings from config.toml", b.len()));
            b
        }
        Err(e) => {
            startup_log.push(format!("Config error: {}", e));
            Vec::new()
        }
    };

    // ── 3. 创建 Engine ──────────────────────────────────────
    let engine = Engine::new();
    let key_bindings = engine.bindings();
    let send_ctx = engine.send_context();
    let stop_flag = engine.stop_flag();

    // ── 4. 注册初始绑定 ─────────────────────────────────────
    let stop_func = Arc::new(gi_utils::functions::stop::停止退出::new(stop_flag.clone()));
    startup_log.push("Registered functions:".into());
    for b in &config_bindings {
        let func: Arc<dyn KeyFunction> = if b.func == "停止退出" {
            stop_func.clone()
        } else {
            match config::create_function(&b.func, send_ctx.clone()) {
                Ok(f) => f,
                Err(e) => {
                    startup_log.push(format!(
                        "  ERROR: '{}' -> '{}': {}",
                        b.key.name(),
                        b.func,
                        e
                    ));
                    continue;
                }
            }
        };
        key_bindings.register(b.key, b.mode, func);
        startup_log.push(format!(
            "  {:>12}  {:<12}  {:?}",
            b.key.name(),
            b.func,
            b.mode
        ));
    }

    // ── 5. 创建托盘图标 ─────────────────────────────────────
    let (tray_tx, tray_rx) = mpsc::channel::<TrayAction>();
    let (pixels, w, h) = create_tray_icon_pixels();

    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            run_tray_thread(tray_tx, pixels, w, h);
        })
        .expect("Failed to spawn tray thread");

    // ── 6. 启动 Engine 后台线程 ─────────────────────────────
    let engine_handle = std::thread::Builder::new()
        .name("engine".into())
        .spawn(move || {
            engine.run();
        })
        .expect("Failed to spawn engine thread");
    startup_log.push("Engine running.".into());

    // ── 7. 构建 GUI 状态 ────────────────────────────────────
    let function_names = config::list_function_names();
    let gui_bindings: Vec<GuiBinding> = config_bindings
        .iter()
        .enumerate()
        .map(|(i, b)| GuiBinding {
            id: i,
            key: Some(b.key),
            key_name: config::key_to_config_name(b.key)
                .unwrap_or_else(|| b.key.name())
                .to_string(),
            func: b.func.clone(),
            mode: b.mode,
        })
        .collect();
    let next_id = gui_bindings.len();

    let app = GuiApp {
        bindings_list: gui_bindings,
        next_id,
        dirty: false,
        error_msg: None,
        key_bindings,
        send_ctx,
        stop_flag,
        engine_handle: Some(engine_handle),
        capture: CaptureState {
            active: false,
            binding_idx: None,
            rx: None,
        },
        function_names,
        font_loaded: false,
        log_messages: startup_log,
        log_visible: true,
        tray_rx,
        should_exit: false,
    };

    // ── 8. 运行 GUI 事件循环 ────────────────────────────────
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 500.0])
            .with_min_inner_size([600.0, 300.0]),
        ..Default::default()
    };

    if let Err(_e) = eframe::run_native(
        "GI-Utils Configuration",
        options,
        Box::new(|_cc| Ok(Box::new(app))),
    ) {
        // 无控制台可用，直接退出（错误信息已由 eframe 内部记录）
        std::process::exit(1);
    }
}
