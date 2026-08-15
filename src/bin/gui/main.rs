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
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, SetLastError,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

// 模块拆分：tray（托盘线程）+ tray_icon（图标原料/共享句柄）+ window_ops
// （HWND 安全包装，幽灵窗口防御唯一入口）。tray.rs 通过 crate:: 路径引用
// tray_icon / window_ops — 三者必须同级声明。
mod tray;
mod tray_icon;
mod window_ops;
use tray::TrayAction;

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
    /// 全局日志收集器 — 每帧 drain 功能线程的 tracing 输出到日志面板。
    log_collector: gi_utils::utils::log::LogCollector,

    /// 托盘消息接收端。
    tray_rx: Receiver<TrayAction>,
    /// 真正的退出标志（托盘菜单 Exit 或 F12 触发）。
    should_exit: bool,
    /// 托盘图标是否已成功创建（NIM_ADD 成功）— 进程级共享：
    /// 崩溃恢复重建 app 时继承崩溃前的值（新实例 Ready 前沿用旧判定，
    /// 否则恢复后关窗会直接退出而非隐藏）。
    /// false 时窗口关闭直接退出而非隐藏 — 否则图标不可用、窗口永远无法恢复。
    ///
    /// 可用性判定 = tray_ok（共享继承）&& tray_ready（本轮已收到 Ready）。
    /// 三重机制（共享 + per-attempt + spawn 失败重置）是有意设计：共享继承
    /// 解决"恢复后 Ready 窗口期关窗直接退出"（review 二轮 #5）；tray_ready
    /// 防止"继承 true 但本轮无图标"误入隐藏路径；spawn 失败重置兜底。
    /// 三者缺一不可 — 改动前先读这段（review：曾提议 per-attempt 化，
    /// 会回归恢复窗口期关窗即退出）。
    tray_ok: Arc<AtomicBool>,
    /// 本轮尝试是否已收到托盘线程的 Ready。tray_ok 是进程级继承值（崩溃前
    /// 轮的写入），关窗隐藏必须 tray_ok 与 tray_ready 同时为真 — 崩溃恢复轮
    /// spawn 失败时继承的 true 会误导隐藏判定（无图标可唤回窗口），review 发现。
    tray_ready: bool,
    /// 配置是否成功加载。false 时禁用保存 — 防止用空列表覆盖损坏的 config.toml。
    config_ok: bool,
    /// 主窗口是否隐藏到托盘 — 进程级共享（崩溃恢复继承隐藏态，窗口不弹回桌面）。
    /// 隐藏时降低重绘频率，托盘消息仍可处理。
    hidden: Arc<AtomicBool>,
    /// 本 app 实例是否已应用过隐藏态（每轮尝试各自把新窗口藏起来一次）。
    hidden_applied: bool,
    /// 隐藏态应用重试截止时刻 — 幽灵窗口延迟销毁期间 FindWindowW 可能
    /// 匹配到旧窗口，本实例在截止前每帧重试 SW_HIDE（~2s）。
    hidden_apply_deadline: Option<std::time::Instant>,

    /// 共享窗口图标（主线程预加载）— 本 app 每轮尝试给自己的窗口重设
    /// WM_SETICON：恢复轮托盘线程可能把图标设到了幽灵窗口上（winit
    /// 延迟销毁），app 帧循环运行于新窗口存续期，重设必达真窗口。
    window_icon: Option<tray_icon::SharedIcon>,
    /// 本 app 实例是否已应用过窗口图标。
    icon_applied: bool,
    /// 窗口图标应用重试截止时刻（与 hidden 同模式，~2s）。
    icon_apply_deadline: Option<std::time::Instant>,

    /// 托盘 Show 重试截止时刻 — Show 是单击动作，落在幽灵窗口期会被吞
    /// （review #5）；与 hide/icon 同模式，~2s 内每帧重试 show_and_activate。
    show_until: Option<std::time::Instant>,

    /// 启动时加载的 [gui] 配置 — save() 需原样写回（fail-closed：不读磁盘，
    /// 读回失败静默回退默认会清空用户 icon_path — review #3）。
    gui_config: gi_utils::config::GuiConfig,
}

/// 按键捕获状态。
struct CaptureState {
    active: bool,
    /// 目标绑定的 `GuiBinding.id`（非行索引 — 捕获期间行可能被增删，索引会漂移）。
    binding_id: Option<usize>,
    rx: Option<Receiver<Key>>,
}

// ═══════════════════════════════════════════════════════════════════
// eframe::App implementation
// ═══════════════════════════════════════════════════════════════════

impl eframe::App for GuiApp {
    fn ui(&mut self, central_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = central_ui.ctx().clone();

        // 回收已退出的功能线程句柄（is_finished 检查，绝不阻塞帧 — L4）
        self.key_bindings.drain_pending_joins();

        // 收集功能线程/共享代码的 tracing 输出（优化游戏、坐标颜色等）—
        // 全局 subscriber 写入共享 buffer，每帧 drain 进日志面板
        for line in self.log_collector.drain() {
            self.log(line);
        }

        // 周期性唤醒：可见时 100ms。隐藏态下 winit 挂起 redraw、周期帧
        // 实际不触发（review 实证 — 旧注释"500ms 确保隐藏时收托盘消息"
        // 与 F12 隐藏退出 bug 的存在互相矛盾）：托盘动作（Show/Exit）与
        // 引擎退出的 WM_CLOSE 走原生窗口消息通道，不依赖周期帧。
        let interval = if self.hidden.load(Ordering::Acquire) {
            std::time::Duration::from_millis(500)
        } else {
            std::time::Duration::from_millis(100)
        };
        ctx.request_repaint_after(interval);

        // -1. 处理托盘消息
        match self.tray_rx.try_recv() {
            Ok(TrayAction::Show) => {
                self.hidden.store(false, Ordering::Release);
                // 单次 show_and_activate 可能打在幽灵窗口上（L3）— 记录
                // 截止时刻，由下方 deadline 块每帧重试直到 ~2s（review #5）
                self.show_until = Some(
                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                );
                ctx.request_repaint();
            }
            Ok(TrayAction::Exit) => {
                self.should_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Ok(TrayAction::Ready(ok)) => {
                self.tray_ok.store(ok, Ordering::Release);
                // 本轮已收到 Ready — 关窗隐藏判定从此刻起可用（见 tray_ready）
                self.tray_ready = true;
                if !ok {
                    self.log("WARNING: tray icon creation failed — closing will exit instead of hiding.");
                }
            }
            _ => {}
        }

        // -0.8. 监控 Engine stop_flag（F12 按下时 Engine 设置此标志）
        if self.stop_flag.load(Ordering::Acquire) {
            self.should_exit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // -0.7. 崩溃恢复继承隐藏态：新窗口首帧起按 hidden 补做 SW_HIDE。
        // 幽灵窗口（winit 延迟销毁）期间 FindWindowW 可能匹配到旧窗口 —
        // 在 ~2s 截止内每帧重试，直至新窗口被真正隐藏。
        if self.hidden.load(Ordering::Acquire) && !self.hidden_applied {
            let deadline = *self
                .hidden_apply_deadline
                .get_or_insert_with(|| std::time::Instant::now() + std::time::Duration::from_secs(2));
            if std::time::Instant::now() < deadline {
                // window_ops::find_main_window 自带 IsWindow 重校验（L3 纪律）
                if let Some(hwnd) = window_ops::find_main_window() {
                    window_ops::hide_window(hwnd);
                }
            } else {
                self.hidden_applied = true;
            }
        }

        // -0.65. 托盘 Show 重试：deadline 内每帧 show_and_activate（幽灵
        // 窗口期点击被吞后自动补发 — review #5）。
        // 隐藏态立即终止重试：用户 Show 后马上关闭窗口时，close 处理器
        // 置 hidden + SW_HIDE，若本块继续 show 会把刚藏掉的窗口每帧拉回 —
        // 关不掉 + 闪烁（实测 bug）。
        if let Some(deadline) = self.show_until {
            if self.hidden.load(Ordering::Acquire) {
                self.show_until = None;
            } else if std::time::Instant::now() < deadline {
                if let Some(hwnd) = window_ops::find_main_window() {
                    window_ops::show_and_activate(hwnd);
                }
                ctx.request_repaint();
            } else {
                self.show_until = None;
            }
        }

        // -0.6. 窗口图标重设（任务栏/标题栏/Alt-Tab）：恢复轮的托盘线程
        // 可能把 WM_SETICON 发到了幽灵窗口（延迟销毁）— 本 app 的帧循环
        // 运行于新窗口存续期（幽灵已在事件循环启动时销毁），从首帧起重试
        // 设置直到 ~2s 截止，保证真窗口拿到图标。
        if self.window_icon.is_some() && !self.icon_applied {
            let deadline = *self
                .icon_apply_deadline
                .get_or_insert_with(|| std::time::Instant::now() + std::time::Duration::from_secs(2));
            if std::time::Instant::now() < deadline {
                // IsWindow 重校验（L3）+ set_window_icon（PostMessageW 异步，
                // 同线程向自己窗口投递，事件循环随即处理）
                if let Some(hwnd) = window_ops::find_main_window() {
                    let icon = self.window_icon.as_ref().unwrap().raw();
                    window_ops::set_window_icon(hwnd, icon);
                }
            } else {
                self.icon_applied = true;
            }
        }

        // -0.5. 窗口关闭 → 隐藏到托盘（除非托盘菜单或 F12 触发退出）
        // F12 触发的 WM_CLOSE 无需在此处理：帧监视器（-0.8）先于本块运行，
        // stop_flag 置位时 should_exit 已为 true、close 直接放行（review：
        // 曾在此加 stop_flag 分支 — 实测不可达，删除）。
        if ctx.input(|i| i.viewport().close_requested()) && !self.should_exit {
            // 托盘图标不可用时隐藏 = 应用永远无法恢复 — 直接退出。
            // tray_ready 要求本轮已收到 Ready：崩溃恢复轮 spawn 失败时继承的
            // tray_ok=true 不得让关窗走隐藏路径（无图标可唤回窗口），review 发现。
            if !(self.tray_ok.load(Ordering::Acquire) && self.tray_ready) {
                self.should_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            // 用原生 ShowWindow(SW_HIDE) 替代 egui 的 Visible(false)
            // egui 的 Visible(false) 会导致 update() 停止调用，托盘消息无法处理
            self.hidden.store(true, Ordering::Release);
            if let Some(hwnd) = window_ops::find_main_window() {
                window_ops::hide_window(hwnd);
            }
        }

        // 0. 首次加载 CJK 字体
        if !self.font_loaded {
            self.load_cjk_font(&ctx);
            self.font_loaded = true;
        }

        // 1. 处理异步事件
        self.handle_capture_result();

        // 2. 请求重绘（捕获等待期间需要轮询 mpsc channel）
        if self.capture.active {
            ctx.request_repaint();
        }

        // 3. 渲染 UI
        // egui 0.36 的 Panel 在 Ui 内布局（`Panel::show(ui)`）：每个面板 show 后
        // 会推进父 ui 光标（Top 下压、Right 收缩右边界）。因此**所有面板必须先于
        // 中央内容** — 否则 ScrollArea 已占满剩余空间，后续 right/bottom 面板
        // 只剩零高度区域，Log 面板会被挤到窗口底部（位置异常）。
        egui::Panel::top("header").show(central_ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(format!(
                    "GI-Utils v{}  Configuration",
                    env!("CARGO_PKG_VERSION")
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.log_visible { "▸ Log" } else { "▹ Log" };
                    if ui.button(label).clicked() {
                        self.log_visible = !self.log_visible;
                    }
                });
            });
        });

        // 4. 右侧日志面板 — 必须先于中央内容（见上）
        if self.log_visible {
            egui::Panel::right("log_panel")
                .resizable(true)
                .default_size(320.0)
                .min_size(160.0)
                .show(central_ui, |ui| {
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
                        // 0.36 默认 auto_shrink=true — 不撑满则面板 frame 背景
                        // 只盖住内容实际区域（min_rect），其余露出黑底
                        .auto_shrink(false)
                        .show(ui, |ui| {
                            for msg in &self.log_messages {
                                ui.label(msg.as_str());
                            }
                        });
                });
        }

        // 5. 状态栏
        egui::Panel::bottom("status").show(central_ui, |ui| {
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

        // 6. 中央内容最后
        // App::ui 的 root_ui 无背景 frame（0.31 时由 CentralPanel 绘制），
        // 不补背景则 wgpu 清屏黑直接露出。`Frame::central_panel` 即旧
        // CentralPanel 的同款 frame（inner_margin 8 + panel_fill 填充）。
        // Frame 背景按内容 min_rect 绘制，因此 ScrollArea 必须撑满
        // （auto_shrink(false) — 0.36 默认 true），否则背景只盖住
        // 内容实际占用区域，其余部分露出黑底。
        egui::Frame::central_panel(central_ui.style()).show(central_ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    self.show_binding_table(ui);
                    ui.add_space(8.0);
                    self.show_action_buttons(ui);
                });
        });

        // 7. 弹窗
        self.show_capture_dialog(&ctx);
        self.show_error_dialog(&ctx);
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

        // L3: 捕获期间禁用表格交互 — 防止捕获中改/删行导致 binding_id 悬空
        // 或状态混乱（按键捕获窗口仍可用 Cancel 按钮取消）
        ui.add_enabled_ui(!self.capture.active, |ui| {
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
                        if self.capture.active && self.capture.binding_id == Some(binding.id) {
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
        }); // add_enabled_ui — 捕获期间禁用

        // 延迟处理（避免在 grid 闭包中 borrow self）
        if let Some(idx) = remove_idx {
            // 删除捕获目标行时同步取消捕获，避免 id 悬空
            if let Some(id) = self.capture.binding_id {
                if self.bindings_list.get(idx).map(|g| g.id) == Some(id) {
                    self.cancel_capture();
                }
            }
            self.bindings_list.remove(idx);
            need_apply = true;
        }
        if let Some(idx) = capture_idx {
            if let Some(id) = self.bindings_list.get(idx).map(|g| g.id) {
                self.start_capture(id);
            }
        }
        if need_apply {
            self.live_apply();
        }
    }

    /// 操作按钮 — 新增 / 保存。
    fn show_action_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // L3: 捕获期间禁用新增 — 新增行会自动进入捕获，会打断当前捕获
            if ui
                .add_enabled(!self.capture.active, egui::Button::new("+ Add Binding"))
                .clicked()
            {
                let id = self.next_id;
                self.next_id += 1;
                self.bindings_list.push(GuiBinding {
                    id,
                    key: None,
                    key_name: "...".into(),
                    // 默认"连点器"而非列表第一项 — 第一项是"停止退出"，
                    // 用户加行后不选功能直接按键会导致程序退出
                    func: "连点器".into(),
                    mode: TriggerMode::Loop,
                });
                // 新增行自动进入按键捕获
                self.start_capture(id);
            }

            // 配置加载失败时禁用保存 — 防止用空列表覆盖损坏的 config.toml
            // egui 0.31：on_hover_text 是 Response 的方法（Button 上没有）
            let save_resp = ui.add_enabled(self.config_ok, egui::Button::new("Save to Config"));
            let save_clicked = save_resp.clicked();
            if !self.config_ok {
                save_resp.on_hover_text("config.toml 加载失败，保存会覆盖现有配置");
            }
            if save_clicked {
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
                ui.add_space(8.0);
                if ui.button("Cancel").clicked() {
                    self.cancel_capture();
                }
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
    /// 上限 200 条，超出时丢弃最旧的 — 长会话下日志无限增长会耗尽内存（L9）。
    fn log(&mut self, msg: impl Into<String>) {
        const MAX_LOG: usize = 200;
        self.log_messages.push(msg.into());
        if self.log_messages.len() > MAX_LOG {
            let excess = self.log_messages.len() - MAX_LOG;
            self.log_messages.drain(0..excess);
        }
    }
    /// 开始按键捕获。`binding_id` 是目标行的 `GuiBinding.id`（与行号无关）。
    fn start_capture(&mut self, binding_id: usize) {
        self.capture.active = true;
        self.capture.binding_id = Some(binding_id);
        self.capture.rx = Some(self.key_bindings.enable_capture());
    }

    /// 取消按键捕获（弹窗 Cancel 按钮）。
    fn cancel_capture(&mut self) {
        self.key_bindings.disable_capture();
        self.capture.active = false;
        self.capture.binding_id = None;
        self.capture.rx = None;
    }

    /// 检查是否有捕获到的按键。
    fn handle_capture_result(&mut self) {
        if !self.capture.active {
            return;
        }
        if let Some(ref rx) = self.capture.rx {
            if let Ok(key) = rx.try_recv() {
                // L3: Esc 取消捕获（与弹窗 Cancel 按钮等价）
                if key == Key::ESCAPE {
                    self.log("Capture cancelled (Esc)");
                    self.cancel_capture();
                    return;
                }

                let name = config::key_display_name(key);

                // 按 id 定位行 — 捕获期间增删行不导致写错行
                if let Some(id) = self.capture.binding_id {
                    if let Some(binding) = self.bindings_list.iter_mut().find(|g| g.id == id) {
                        // L7: 键未变化 → 不应用、不置 dirty（重复按同一键不产生变更）
                        if binding.key == Some(key) {
                            self.log(format!("Key '{}' unchanged — no apply", key.name()));
                            self.cancel_capture();
                            return;
                        }
                        binding.key = Some(key);
                        binding.key_name = name;
                    }
                    // 目标行已被删除 → 按键静默丢弃，仅结束捕获
                }

                self.cancel_capture();
                self.live_apply();
            }
        }
    }

    /// 校验绑定列表的双向唯一性（key 唯一、func 唯一）。未设键的行跳过。
    /// 与 config::load 的校验规则一致；live_apply 与 save_config 共用。
    fn validate_bindings(&self) -> Result<(), String> {
        let mut keys = std::collections::HashSet::new();
        let mut funcs = std::collections::HashSet::new();
        for g in &self.bindings_list {
            if let Some(key) = g.key {
                if !keys.insert(key) {
                    return Err(format!(
                        "duplicate key: '{}' is bound to multiple functions",
                        key.name()
                    ));
                }
            }
            if !funcs.insert(g.func.clone()) {
                return Err(format!(
                    "duplicate function: '{}' is bound to multiple keys",
                    g.func
                ));
            }
        }
        Ok(())
    }

    /// 即时应用：将所有 GUI 绑定注册到 Engine 的 KeyBindings。
    fn live_apply(&mut self) {
        // 唯一性校验失败则不应用 — 保持现有绑定，避免两行同键时
        // HashMap::insert 静默后写覆盖（表格显示两行、实际只有一行生效）
        if let Err(e) = self.validate_bindings() {
            self.error_msg = Some(e);
            return;
        }

        // 先清空旧绑定，再全量重新注册（覆盖删除/改键的条目）。
        self.key_bindings.clear_all();

        let mut errors: Vec<String> = Vec::new();
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
                        // L8: 聚合所有错误 — 不覆盖只显示最后一个
                        errors.push(format!("'{}': {}", g.func, e));
                        continue;
                    }
                }
            };

            self.key_bindings.register(key, g.mode, func);
        }
        if !errors.is_empty() {
            self.error_msg = Some(errors.join("\n"));
        }
        self.dirty = true; // 标记为有未保存变更
    }

    /// 保存绑定列表到 config.toml。
    fn save_config(&self) -> Result<(), String> {
        self.validate_bindings()?;

        // 校验每个键可序列化为配置名 — KEY_PAIRS 之外的捕获键（异形键盘宏键/
        // 厂商扩展码）拒绝保存，绝不写 "?"（写了下次启动解析失败、Save 被禁用
        // 且无法自愈，review 发现）。
        for g in &self.bindings_list {
            if let Some(key) = g.key {
                if config::key_to_config_name(key).is_none() {
                    return Err(format!(
                        "key '{}' cannot be serialized to config — rebind it to a supported key",
                        key.name()
                    ));
                }
            }
        }

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

        config::save(&bindings, &self.gui_config)
    }
}

// ═══════════════════════════════════════════════════════════════════
// Drop — 清理关闭
// ═══════════════════════════════════════════════════════════════════

impl Drop for GuiApp {
    fn drop(&mut self) {
        // 只做无害清理。完整关机序列（托盘收尾/停引擎/stop_all/亲和性恢复）
        // 已移至 main 的 shutdown_all — Drop 在渲染 panic 回卷时也会执行：
        // 若此处执行关机序列，catch_unwind 捕获之前引擎已被杀死、stop_flag
        // 被锁死为 true，自愈重试拿到的是死引擎（review 发现）。因此这里
        // 绝不能包含任何破坏性动作。
        self.key_bindings.disable_capture();
    }
}

/// 完整关机序列 — 仅在正常退出与重试耗尽时由 main 显式调用。
/// 顺序：托盘收尾 → 停引擎 → stop_all（先于亲和性恢复，防优化游戏竞态）
/// → 蜂鸣 → 恢复亲和性（决策 21）→ 共享图标销毁（最后，确认托盘线程已退出）。
fn shutdown_all(
    engine_handle: Option<JoinHandle<()>>,
    tray_handle: Option<JoinHandle<()>>,
    tray_quit: &AtomicBool,
    tray_icon: Option<tray_icon::SharedIcon>,
    stop_flag: &AtomicBool,
    key_bindings: &Arc<gi_utils::engine::bindings::KeyBindings>,
) {
    // 先停引擎（stop_flag → join），再收尾托盘 — 托盘收尾最坏有 2s 有界等待，
    // 放前面会拖长退出路径（review #14）。
    stop_flag.store(true, Ordering::Release);
    if let Some(handle) = engine_handle {
        let _ = handle.join();
    }

    // 有界等待托盘线程退出；返回值 = 线程已确认退出（共享图标引用方清零）。
    let tray_exited = stop_tray_thread(tray_handle, tray_quit);

    // 显式停止所有功能线程 — 必须在亲和性恢复**之前**（决策 21）
    key_bindings.stop_all();

    // 退出蜂鸣 — 异步播放（~300ms），由随后的亲和性恢复耗时覆盖
    gi_utils::utils::beep::beep_async(375, 300);

    // 恢复所有进程的完整 CPU 亲和性（best-effort，静默吞错误）
    let _ = gi_utils::utils::affinity::restore_all_affinity();

    // 共享托盘图标最后销毁 — 仅当托盘线程已确认退出（"所有引用方已退出"
    // 由 is_finished 实证而非时序假设，review 发现）；未退出则跳过销毁：
    // 进程即将退出，GDI 对象由 OS 随进程回收，绝不带着存活引用方销毁句柄。
    if let Some(icon) = tray_icon {
        if tray_exited {
            icon.destroy();
        } else {
            tracing::warn!(
                "tray thread still alive after 2s wait — skipping shared icon destroy (OS reclaims at process exit)"
            );
        }
    }
}

/// 停止托盘线程：置位 quit 标志（托盘自身唯一退出通道 — 泵每 ~200ms 检查；
/// ⑨ 搜索循环每轮检查），然后**有界等待**（最多 ~2s）。绝不无限 join
/// （渲染 panic 收尾路径不能悬挂）。返回线程是否在等待期内确认退出
/// （共享图标销毁的前提）。
fn stop_tray_thread(handle: Option<JoinHandle<()>>, quit: &AtomicBool) -> bool {
    let Some(handle) = handle else { return true };
    quit.store(true, Ordering::Release);
    for _ in 0..40 {
        if handle.is_finished() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    tracing::warn!("tray thread did not exit within 2s — detaching");
    handle.is_finished()
}

/// 把绑定列表注册到注册表（startup 与崩溃恢复共用的唯一注册路径）。
///
/// `replace_existing = true`（崩溃恢复）：先 `stop_all` 停止旧功能线程 —
/// clear_all 只移除条目，旧线程若持有 stop_requested 引用会永久失联存活 —
/// 再全量替换。
fn register_all_bindings(
    key_bindings: &Arc<gi_utils::engine::bindings::KeyBindings>,
    stop_func: &Arc<dyn KeyFunction>,
    bindings: &[Binding],
    send_ctx: &Arc<SendContext>,
    replace_existing: bool,
) -> Vec<String> {
    if replace_existing {
        key_bindings.stop_all();
        key_bindings.clear_all();
    }
    let mut log = Vec::new();
    for b in bindings {
        let func: Arc<dyn KeyFunction> = if b.func == "停止退出" {
            stop_func.clone()
        } else {
            match config::create_function(&b.func, send_ctx.clone()) {
                Ok(f) => f,
                Err(e) => {
                    log.push(format!("  ERROR: '{}' -> '{}': {}", b.key.name(), b.func, e));
                    continue;
                }
            }
        };
        key_bindings.register(b.key, b.mode, func);
    }
    log
}

// ═══════════════════════════════════════════════════════════════════

/// 用 MessageBox 显示错误 — GUI 无控制台时的用户反馈通道。
/// 用于 panic hook 与致命启动错误。
fn show_message_box(title: &str, msg: &str) {
    let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let msg_wide: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            windows::core::PCWSTR(msg_wide.as_ptr()),
            windows::core::PCWSTR(title_wide.as_ptr()),
            MB_ICONERROR | MB_OK,
        );
    }
}

/// 安装 panic hook：恢复 CPU 亲和性 + MessageBox 显示错误。
///
/// panic=abort 时 Drop 不执行（`restore_all_affinity` 在 Drop 路径中），
/// 若 panic 前已隔离游戏核心，其它进程会停留在受限核心 — hook 兜底恢复。
/// 必须在任何可能 panic 的代码之前安装。
/// GUI 渲染重试上下文的 panic 标记 — 仅当 panic 发生在 **GUI 主线程** 且处于
/// run_native 期间（catch_unwind 会恢复）时 hook 静默；引擎/功能/托盘线程的
/// panic 仍走完整兜底（弹窗 + 恢复亲和性）。
static IN_GUI_RETRY: AtomicBool = AtomicBool::new(false);

/// GUI 主线程 id — 与 IN_GUI_RETRY 组合判断 panic 是否属于可恢复的渲染 panic。
static GUI_MAIN_THREAD: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let on_gui_thread = GUI_MAIN_THREAD
            .get()
            .map(|id| std::thread::current().id() == *id)
            .unwrap_or(false);
        if IN_GUI_RETRY.load(Ordering::Relaxed) && on_gui_thread {
            // 渲染 panic 由 catch_unwind 恢复 — 不弹窗、不恢复亲和性
            // （引擎仍在运行，恢复会破坏游戏优化）。
            return;
        }
        let _ = utils::affinity::restore_all_affinity();
        show_message_box(
            "GI-Utils 错误",
            &format!("GI-Utils 发生致命错误，即将退出：\n\n{}", info),
        );
        // 非渲染 panic（引擎/功能/托盘线程）→ 弹框后立即终止进程。
        // unwind 语义下线程静默死亡会让 Loop 功能继续注入、F12 失效
        // （僵尸进程）— 恢复 panic=abort 时代的 fail-fast 语义。
        std::process::exit(1);
    }));
}

/// 从 panic payload 提取可读消息（&str / String / 未知）。
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

// ═══════════════════════════════════════════════════════════════════
// main — GUI 入口
// ═══════════════════════════════════════════════════════════════════

fn main() {
    use windows::core::w;

    // 启动日志（GUI 无控制台，收集到内存后在日志面板显示）
    let mut startup_log: Vec<String> = vec![
        format!("GI-Utils GUI v{}", env!("CARGO_PKG_VERSION")),
        "Initializing...".into(),
    ];

    // ── 0. panic hook + 单实例保护（在任何副作用之前）────────
    install_panic_hook();

    // 全局日志收集：功能线程（优化游戏、坐标颜色等）的 tracing 输出
    // 经全局 subscriber 汇入共享 buffer，GUI 帧循环 drain 到日志面板。
    // 必须在配置加载（config.rs 首次生成提示）之前安装。
    let log_collector = gi_utils::utils::log::LogCollector::install(200);

    // 单实例：已有实例时激活其窗口并退出。
    // mutex 句柄故意不释放 — 进程退出时由 OS 自动释放，保持排他。
    unsafe {
        SetLastError(ERROR_SUCCESS);
        let _single_instance_mutex = CreateMutexW(None, true, w!("GIUtilsSingleInstance"));
        match GetLastError() {
            ERROR_ALREADY_EXISTS => {
                // 激活既有实例：find_main_window 自带 IsWindow 校验 — 持有者
                // 若处崩溃恢复的幽灵窗口期（同标题仍是有效 HWND），打中幽灵
                // 的 SW_SHOW 是静默空操作，不伪造"已激活"（review 发现）。
                if let Some(hwnd) = window_ops::find_main_window() {
                    window_ops::show_and_activate(hwnd);
                }
                return;
            }
            ERROR_SUCCESS => {} // 本实例持有互斥体
            err => {
                // 其他失败（权限/跨会话等）— best-effort：记录并继续运行，
                // 单实例保护降级但功能不受影响（review 发现）。
                startup_log.push(format!(
                    "Single-instance mutex failed (error {err:?}) — running anyway"
                ));
            }
        }
    }

    // ── 1. 系统初始化 ───────────────────────────────────────
    // init 返回日志行（GUI 无控制台，println 会被静默丢弃）
    startup_log.extend(utils::init());

    // L12: GUI 线程核心分离 — 进程掩码扩展至 12-15，当前线程（GUI 渲染）
    // pin 12,13 并降为线程级最低优先级；Engine/功能线程在 14,15。
    // 必须在 Engine 线程 spawn 之前完成（新线程继承进程掩码）。
    match utils::affinity::configure_gui_self() {
        Ok(()) => startup_log.push("Cores: GUI 12,13 (LOWEST) / input 14,15 (REALTIME)".into()),
        Err(e) => startup_log.push(format!("GUI core config failed: {}", e)),
    }

    // ── 2. 加载配置 ─────────────────────────────────────────
    // config_ok 可变 — 崩溃恢复轮重载成功后同步更新（Save 可用性与磁盘
    // 可解析性保持一致，review 发现）。
    let (config_bindings, mut config_ok) = match config::load() {
        Ok(b) => {
            startup_log.push(format!("Loaded {} bindings from config.toml", b.len()));
            (b, true)
        }
        Err(e) => {
            startup_log.push(format!("Config error: {}", e));
            (Vec::new(), false)
        }
    };

    // ── 3. 创建 Engine ──────────────────────────────────────
    let engine = Engine::new();
    let key_bindings = engine.bindings();
    let send_ctx = engine.send_context();
    let stop_flag = engine.stop_flag();

    // ── 4. 注册初始绑定 ─────────────────────────────────────
    let stop_func: Arc<dyn KeyFunction> =
        Arc::new(gi_utils::functions::stop::停止退出::new(stop_flag.clone()));
    startup_log.push("Registered functions:".into());
    startup_log.extend(register_all_bindings(
        &key_bindings,
        &stop_func,
        &config_bindings,
        &send_ctx,
        false,
    ));
    for b in &config_bindings {
        startup_log.push(format!(
            "  {:>12}  {:<12}  {:?}",
            b.key.name(),
            b.func,
            b.mode
        ));
    }

    // ── 5. 托盘图标：主线程预加载一次（健康 GDI/WIC 状态），跨崩溃恢复
    // 轮共享 — 睡眠唤醒的 GL 崩溃会连带污染进程内 WIC 图标加载（恢复轮
    // LoadImageW 永久失败），必须在启动时完成加载。预加载失败回退程序
    // 生成图标（纯 GDI 路径，恢复轮仍可用）。
    let gui_cfg = config::load_gui_config();
    let (tray_pixels, tray_w, tray_h) = tray_icon::create_tray_icon_pixels();
    let mut preloaded_icon =
        tray_icon::preload_tray_icon(&gui_cfg.icon_path, &tray_pixels, tray_w, tray_h);
    if gui_cfg.icon_path.is_empty() {
        startup_log.push("Tray icon: generated".into());
    } else if preloaded_icon.is_some() {
        startup_log.push(format!("Tray icon: {} (preloaded)", gui_cfg.icon_path));
    } else {
        startup_log.push(format!(
            "Tray icon: {} (preload failed — generated fallback)",
            gui_cfg.icon_path
        ));
    }

    // ── 6. 启动 Engine 后台线程（只 spawn 一次 — 重试循环之外）──
    // 句柄留在 main（shutdown_all 需要），app 不再持有 — 渲染 panic 回卷时
    // Drop 不会碰它（见 GuiApp::drop 的注释）。
    let mut engine_handle = Some(
        std::thread::Builder::new()
            .name("engine".into())
            .spawn(move || {
                engine.run();
                // run() 仅因 stop_flag 置位返回（"停止退出"热键的语义标志，
                // 按键由 config 绑定）— 此处直接向主窗口投 WM_CLOSE。
                // 真实机制（review 实证）：CloseRequested → egui-winit 强制
                // 同步 update（隐藏窗口也执行）→ 帧监视器驱动退出。隐藏态
                // 下没有周期帧（winit 挂起 redraw），窗口消息是唯一即时
                // 通道 — 退出不再延迟到窗口唤出，也不依赖托盘线程存活。
                if let Some(hwnd) = window_ops::find_main_window() {
                    window_ops::post_close(hwnd);
                }
            })
            .expect("Failed to spawn engine thread"),
    );
    startup_log.push("Engine running.".into());

    // 记录 GUI 主线程 id — panic hook 据此区分"渲染 panic（可恢复）"
    // 与"其他线程 panic（弹窗 + 恢复亲和性）"。
    let _ = GUI_MAIN_THREAD.set(std::thread::current().id());

    // ── 7. GUI 事件循环（崩溃自愈重试，最多 MAX_GUI_RETRIES 次）──
    // 睡眠唤醒/显示变更会使 wgl 上下文失效，eframe 在 glow_integration 的
    // make_current 处 unwrap panic。catch_unwind 捕获后：收尾旧托盘线程 →
    // 重建注册表与 app → 重试 — 引擎全程存活（关机序列已移出 Drop）。
    const MAX_GUI_RETRIES: u32 = 3;
    let function_names = config::list_function_names();
    let mut tray_handle: Option<JoinHandle<()>> = None;
    let mut last_error: Option<String> = None;

    // 托盘可用性 / 窗口隐藏态 — 进程级共享：崩溃恢复重建 app 时继承
    // 崩溃前的值（否则恢复后关窗直接退出、隐藏态弹回桌面）
    let tray_ok_shared = Arc::new(AtomicBool::new(false));
    let hidden_shared = Arc::new(AtomicBool::new(false));
    // 托盘线程退出请求标志 — 每轮尝试独立（stop_tray_thread 置位后，旧线程
    // ⑨ 搜索循环/泵感知并立即收尾；新线程必须用新标志，否则会被旧 quit 误伤）。
    // 此初值仅满足定义，循环首行立即覆盖（review 记录的潜在误读点）。
    let mut tray_quit = Arc::new(AtomicBool::new(false));

    for attempt in 0..MAX_GUI_RETRIES {
        // 本轮独立 quit 标志（见上）
        tray_quit = Arc::new(AtomicBool::new(false));

        // 绑定来源：首次尝试用启动快照；重试从磁盘重载 — 快照可能落后于
        // 用户已保存的修改（review #2），恢复必须以磁盘为准
        let attempt_bindings: Vec<Binding> = if attempt == 0 {
            config_bindings.clone()
        } else {
            match config::load() {
                Ok(b) => {
                    startup_log.push("配置已从 config.toml 重新加载（崩溃恢复）".into());
                    // 磁盘可解析 → 恢复 Save 可用性（与重载结果一致，review 发现）
                    config_ok = true;
                    b
                }
                Err(e) => {
                    startup_log.push(format!(
                        "config.toml 重载失败（沿用启动快照）: {}",
                        e
                    ));
                    config_bindings.clone()
                }
            }
        };

        // 注册表对齐：重试轮先停旧功能线程（clear_all 只移除条目，旧线程
        // 会永久失联存活）再全量替换（review #8）
        if attempt > 0 {
            startup_log.extend(register_all_bindings(
                &key_bindings,
                &stop_func,
                &attempt_bindings,
                &send_ctx,
                true,
            ));
        }

        // 托盘：每轮尝试独立 channel + quit 标志 + 线程（上一轮的旧线程在
        // panic 路径已收尾）
        let (tray_tx, tray_rx) = mpsc::channel::<TrayAction>();
        tray_handle = match std::thread::Builder::new()
            .name("tray".into())
            .spawn({
                let icon = preloaded_icon.clone();
                let pixels = tray_pixels.clone();
                let quit = tray_quit.clone();
                move || tray::run_tray_thread(tray_tx, quit, icon, pixels, tray_w, tray_h)
            }) {
            Ok(h) => Some(h),
            Err(e) => {
                startup_log.push(format!("Tray thread spawn failed: {}", e));
                // 显式重置 tray_ok — 崩溃恢复轮这里继承的是崩溃前的 true，
                // 若不重置，用户关窗走隐藏路径却没有任何托盘图标可唤回
                // （review 发现）；GUI 仍可用
                tray_ok_shared.store(false, Ordering::Release);
                None
            }
        };

        // GUI 状态重建：绑定表从 attempt_bindings 重建，共享句柄全部 Clone
        let gui_bindings: Vec<GuiBinding> = attempt_bindings
            .iter()
            .enumerate()
            .map(|(i, b)| GuiBinding {
                id: i,
                key: Some(b.key),
                key_name: config::key_display_name(b.key),
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
            key_bindings: key_bindings.clone(),
            send_ctx: send_ctx.clone(),
            stop_flag: stop_flag.clone(),
            capture: CaptureState {
                active: false,
                binding_id: None,
                rx: None,
            },
            function_names: function_names.clone(),
            font_loaded: false,
            // 每轮尝试都携带完整启动历史（panic 消息可见于日志面板）
            log_messages: startup_log.clone(),
            log_visible: true,
            log_collector: log_collector.clone(),
            tray_rx,
            should_exit: false,
            // tray_ok / hidden 为进程级共享标志 — 继承崩溃前的值；
            // hidden_applied 每轮尝试各自执行（把新窗口藏起来）
            tray_ok: tray_ok_shared.clone(),
            // 本轮 Ready 尚未收到 — 关窗隐藏判定需 tray_ok && tray_ready
            tray_ready: false,
            config_ok,
            hidden: hidden_shared.clone(),
            hidden_applied: false,
            hidden_apply_deadline: None,
            window_icon: preloaded_icon.clone(),
            icon_applied: false,
            icon_apply_deadline: None,
            show_until: None,
            gui_config: gui_cfg.clone(),
        };

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([800.0, 500.0])
                .with_min_inner_size([600.0, 300.0]),
            ..Default::default()
        };

        // 标志窗口与 catch_unwind 范围精确对齐：store(true) 在闭包内、
        // store(false) 紧随 catch_unwind 返回 — 闭包外的 GUI 主线程 panic
        // 不被 hook 静默（走完整兜底：弹窗 + 恢复亲和性），review 发现。
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            IN_GUI_RETRY.store(true, Ordering::Relaxed);
            eframe::run_native(
                "GI-Utils Configuration",
                options,
                Box::new(|_cc| Ok(Box::new(app))),
            )
        }));
        IN_GUI_RETRY.store(false, Ordering::Relaxed);

        match result {
            // 正常退出（窗口关闭 / 托盘退出）— 完整关机序列
            Ok(Ok(())) => {
                shutdown_all(
                    engine_handle.take(),
                    tray_handle.take(),
                    &tray_quit,
                    preloaded_icon.take(),
                    &stop_flag,
                    &key_bindings,
                );
                return;
            }
            // 启动错误（非 panic）— 不可恢复
            Ok(Err(e)) => {
                last_error = Some(e.to_string());
                break;
            }
            // 渲染线程 panic — 收尾旧托盘线程后重试（注册表对齐在
            // 下一轮尝试顶部执行）
            Err(payload) => {
                let msg = panic_message(&payload);
                startup_log.push(format!("GUI 渲染线程 panic：{}", msg));
                // 主线程已恢复 — 旧托盘线程安全收尾（quit 置位 + WM_CLOSE +
                // 有界等待，防止误找到新窗口造成双托盘图标）
                stop_tray_thread(tray_handle.take(), &tray_quit);
                if attempt + 1 >= MAX_GUI_RETRIES {
                    last_error = Some(msg);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1000));
            }
        }
    }

    // 重试耗尽 / 启动失败 — 完整关机后提示退出
    shutdown_all(
        engine_handle.take(),
        tray_handle.take(),
        &tray_quit,
        preloaded_icon.take(),
        &stop_flag,
        &key_bindings,
    );
    show_message_box(
        "GI-Utils 启动失败",
        &format!("GUI 初始化失败：\n\n{}", last_error.unwrap_or_default()),
    );
    std::process::exit(1);
}
