//! GI-Utils GUI 配置面板 — GUI Configuration Panel (egui).
//!
//! 可视化绑定管理：增删改按键绑定，修改即时生效，保存写入配置文件（默认 exe 旁 gi-utils-config.toml，可另存/加载任意路径）。
//! Visual binding management: add/edit/delete key bindings,
//! live-apply changes, save to config.toml.

#![windows_subsystem = "windows"]

use eframe::egui;
use gi_utils::engine::Engine;
use gi_utils::engine::TriggerMode;
use gi_utils::engine::bindings::{KeyFunction, ParamKind};
use gi_utils::interception::SendContext;
use gi_utils::key::Key;
use gi_utils::profile::{self, Binding};
use gi_utils::utils;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, SetLastError};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

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

    /// 参数弹窗打开中的绑定 id（None = 关闭；同一时刻至多一个）。
    param_popup: Option<usize>,

    /// 所有可用功能名称（下拉框选项）。
    function_names: Vec<&'static str>,

    /// CJK 字体是否已加载。
    font_loaded: bool,
    /// GUI 内嵌日志。
    log_messages: Vec<String>,
    /// 日志面板是否可见。
    log_visible: bool,
    /// 全局日志收集器 — 每帧 drain 功能线程的 tracing 输出到日志面板。
    log_collector: gi_utils::utils::log_collector::LogCollector,

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
    gui_config: gi_utils::profile::GuiConfig,
    /// 功能级参数表（per-function）— 参数面板编辑与 Save 持久化的真值；
    /// 运行时值在功能实例的原子槽（live 直写），此表为配置侧快照。
    func_params: gi_utils::profile::FuncParams,

    /// 活动 profile 名（profiles/ 目录内 .toml 的去扩展名）— Save 目标、
    /// 下拉实时切换的选中项。路径由 `profile::profile_path(&active_profile)`
    /// 派生（v1.5.2 自由路径已收敛为 profile 体系）。
    active_profile: String,
    /// New Profile 的名字输入缓冲。
    pending_profile_name: String,
    /// New 按钮已提交（延迟到 show_action_buttons 的延迟处理区执行 —
    /// 避免按钮闭包内可变借用冲突）。
    pending_new_profile: bool,
}

/// 按键捕获状态。
/// 按键捕获目标 — 绑定键（Key 列 Set Key）或功能参数键槽（SpamKey 等）。
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum CaptureTarget {
    /// 绑定某功能的按键（写 GuiBinding.key）。
    Binding(usize),
    /// 功能参数键槽（写参数 Int 槽 + func_params；绑定的 `GuiBinding.id`
    /// 定位行 — 行内 func 名用于 per-function 参数表寻址）。
    KeySlot { binding_id: usize, slot: usize },
}

struct CaptureState {
    active: bool,
    /// 捕获目标（Binding 或 KeySlot — 按 id 定位行，索引漂移安全）。
    target: Option<CaptureTarget>,
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
                self.show_until =
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
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
                    // NIM_ADD 失败：隐藏态不可恢复（无图标可唤回）—
                    // 取消隐藏态并武装 Show 重试把窗口拉回。先置 hidden=false
                    // 再武装 — deadline 块在 hidden=true 时立即取消重试
                    // （review 3.6）。
                    self.hidden.store(false, Ordering::Release);
                    if self.hidden_applied {
                        self.show_until =
                            Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
                    }
                    ctx.request_repaint();
                    self.log(
                        "WARNING: tray icon creation failed — closing will exit instead of hiding.",
                    );
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
            let deadline = *self.hidden_apply_deadline.get_or_insert_with(|| {
                std::time::Instant::now() + std::time::Duration::from_secs(2)
            });
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
            let deadline = *self.icon_apply_deadline.get_or_insert_with(|| {
                std::time::Instant::now() + std::time::Duration::from_secs(2)
            });
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

        // 0. 首次加载 CJK 字体 + 驱动线程掩码收窄
        if !self.font_loaded {
            self.load_cjk_font(&ctx);
            self.font_loaded = true;
            // wgpu 初始化（首帧前完成）已建齐 Vulkan 驱动线程 — 一次性把
            // 未 pin 线程收窄到 GUI 核（防驱动 worker 落 14,15 输入核）
            let narrowed = gi_utils::utils::affinity::narrow_unpinned_threads_to_gui();
            if narrowed > 0 {
                self.log(format!("Narrowed {narrowed} driver threads to GUI cores"));
            }
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
                    let label = if self.log_visible {
                        "▶ Log"
                    } else {
                        "◀ Log"
                    };
                    if ui.button(label).clicked() {
                        self.log_visible = !self.log_visible;
                    }
                });
            });
        });

        // 4. 状态栏 — 先于 Log 面板声明：bottom 先占底部条带，right 高度被
        // 挤压到其上方（后声明者接收剩余空间 — 修复 Log 覆盖状态栏）
        egui::Panel::bottom("status").show(central_ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Status: Running");
                if self.dirty {
                    ui.separator();
                    ui.colored_label(egui::Color32::from_rgb(255, 200, 0), "(unsaved changes)");
                }
            });
        });

        // 5. 右侧日志面板 — 面板顺序：header → status(bottom) → log(right) →
        // 中央内容。bottom 先声明占住底部，right 不再覆盖状态栏；
        // max_size 限制拖宽 — 右边界不得越过主编辑区（中央内容至少
        // 保留 ~55% 窗口宽度）
        if self.log_visible {
            let win_width = central_ui.input(|i| i.viewport_rect().width());
            egui::Panel::right("log_panel")
                .resizable(true)
                .default_size(320.0)
                .min_size(160.0)
                // 拖宽上限：主编辑区至少保留 55% 窗口宽度（用户要求 —
                // 左边界不得越过主编辑区）
                .max_size(win_width * 0.45)
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
                    self.show_profile_bar(ui);
                    ui.add_space(4.0);
                    self.show_binding_table(&ctx, ui);
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
    fn show_binding_table(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mut need_apply = false;
        let mut remove_idx: Option<usize> = None;
        let mut capture_idx: Option<usize> = None;
        let mut gear_idx: Option<usize> = None;
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
                            if self.capture.active
                                && self.capture.target == Some(CaptureTarget::Binding(binding.id))
                            {
                                ui.label("(capturing...)");
                            } else if binding.key.is_some() {
                                ui.label(&binding.key_name);
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
                                        if ui
                                            .selectable_label(binding.func == *name, *name)
                                            .clicked()
                                        {
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
                                    for &m in
                                        &[TriggerMode::Once, TriggerMode::Loop, TriggerMode::Toggle]
                                    {
                                        if ui
                                            .selectable_label(binding.mode == m, format!("{:?}", m))
                                            .clicked()
                                        {
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
                                // ⚙ 仅渲染于有参数的功能行（params_of 查注册表）
                                let has_params = binding.key.is_some_and(|k| {
                                    self.key_bindings
                                        .params_of(&k)
                                        .is_some_and(|(specs, _)| !specs.is_empty())
                                });
                                if has_params && ui.button("⚙").clicked() {
                                    gear_idx = Some(i);
                                }
                                if !has_params {
                                    ui.add_enabled(false, egui::Button::new("⚙"))
                                        .on_disabled_hover_text("此功能无参数");
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

        // ── 动态参数：齿轮按钮（Actions 列）+ 弹窗编辑 ──
        // 2026-08-22 用户方案：参数属于各自绑定行 — 底部合并大 table 删除，
        // 有参数的功能在 Actions 列渲染 ⚙ 按钮，点击开 popup 就地编辑
        //（live 直写：修改即刻生效，下周期应用；Save 持久化）。
        // 渲染在帧尾统一执行（show_param_window）— 此处仅 toggle 开关。
        if let Some(idx) = gear_idx {
            // 行循环与 gear 执行同帧先后发生，行未被删除（Del 互斥触发）
            if let Some(g) = self.bindings_list.get(idx) {
                // toggle：同 id 再点齿轮关闭
                self.param_popup = if self.param_popup == Some(g.id) {
                    None
                } else {
                    Some(g.id)
                };
            }
        }
        if let Some(idx) = remove_idx {
            // 删除捕获目标行时同步取消捕获，避免 id 悬空
            let row_id = self.bindings_list.get(idx).map(|g| g.id);
            match self.capture.target {
                Some(CaptureTarget::Binding(id)) if row_id == Some(id) => self.cancel_capture(),
                Some(CaptureTarget::KeySlot { binding_id, .. }) if row_id == Some(binding_id) => {
                    self.cancel_capture()
                }
                _ => {}
            }
            if self.param_popup == row_id {
                self.param_popup = None; // 弹窗指向行已删 — 同步清除
            }
            self.bindings_list.remove(idx);
            need_apply = true;
        }
        if let Some(idx) = capture_idx {
            if let Some(id) = self.bindings_list.get(idx).map(|g| g.id) {
                self.start_capture(CaptureTarget::Binding(id));
            }
        }
        if let Some(popup_id) = self.param_popup
            && let Some(g) = self.bindings_list.iter().find(|x| x.id == popup_id)
            && let Some(key) = g.key
        {
            self.show_param_window(ctx, popup_id, &key);
        }
        // New Profile：以当前状态在 profiles/ 创建新副本并切换
        if self.pending_new_profile {
            self.pending_new_profile = false;
            let name = self.pending_profile_name.trim().to_string();
            if name.is_empty() {
                self.error_msg = Some("Profile name is empty".into());
            } else if let Some(path) = profile::profile_path(&name) {
                match self.save_config_to(&path) {
                    Ok(()) => {
                        self.active_profile = name.clone();
                        self.dirty = false;
                        self.log(format!("Profile '{name}' created"));
                    }
                    Err(e) => self.error_msg = Some(e),
                }
            } else {
                self.error_msg = Some(format!("Invalid profile name: '{name}'"));
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
                // 默认功能：第一个未被任何行占用的非停止功能 — 固定
                // "连点器"在已绑定时，加行后不选功能直接按键会产生重复
                // 绑定；"停止退出"永不作默认（直接按键会导致程序退出）
                // （review 4.6）
                let used: Vec<&str> = self.bindings_list.iter().map(|g| g.func.as_str()).collect();
                let default_func = profile::list_function_names()
                    .into_iter()
                    .find(|name| *name != "停止退出" && !used.contains(name))
                    .unwrap_or("连点器")
                    .to_string();
                self.bindings_list.push(GuiBinding {
                    id,
                    key: None,
                    key_name: "...".into(),
                    func: default_func,
                    mode: TriggerMode::Loop,
                });
                self.dirty = true; // 空行已可见 — Esc 放弃也应提示未保存
                // 新增行自动进入按键捕获
                self.start_capture(CaptureTarget::Binding(id));
            }

            // 配置加载失败时禁用保存 — 防止用空列表覆盖损坏的配置文件
            let save_resp = ui.add_enabled(self.config_ok, egui::Button::new("Save"));
            if save_resp.clicked() {
                match self.save_config() {
                    Ok(()) => self.dirty = false,
                    Err(e) => self.error_msg = Some(e),
                }
            }
            if !self.config_ok {
                save_resp.on_hover_text("配置加载失败，保存会覆盖现有文件");
            }
        });
    }

    /// Profile 选择行（绑定表上方）— 下拉实时切换 + New（以文本框命名建副本）。
    fn show_profile_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Profile");
            let profiles = profile::list_profiles();
            // 活动名不在列表（被外部删除）→ 显示为占位
            let selected = if profiles.iter().any(|p| *p == self.active_profile) {
                self.active_profile.clone()
            } else {
                "(missing)".into()
            };
            egui::ComboBox::from_id_salt("profile_selector")
                .selected_text(selected)
                .width(140.0)
                .show_ui(ui, |ui| {
                    for name in &profiles {
                        let is_active = *name == self.active_profile;
                        if ui.selectable_label(is_active, name).clicked()
                            && !is_active
                            && !self.capture.active
                        {
                            // 实时切换：加载目标 profile 并全量重注册
                            match profile::load_full_from(
                                &profile::profile_path(name).expect("listed name"),
                            ) {
                                Ok((bindings, func_params)) => {
                                    self.active_profile = name.clone();
                                    self.func_params = func_params;
                                    self.rebuild_from_bindings(bindings);
                                    self.dirty = false;
                                    self.log(format!("Profile → '{name}'"));
                                }
                                Err(e) => {
                                    self.error_msg = Some(format!("切换失败: {e}"));
                                }
                            }
                        }
                    }
                });
            // 新建名输入（Enter 或 New 按钮提交）
            let name_edit = ui.add_sized(
                [120.0, 20.0],
                egui::TextEdit::singleline(&mut self.pending_profile_name)
                    .hint_text("new profile name"),
            );
            let committed = name_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("New").clicked() || committed {
                self.pending_new_profile = true;
            }
        });
    }

    /// 从 Binding 列表全量重建 GUI 行 + 重注册（Load from File 后）。
    fn rebuild_from_bindings(&mut self, bindings: Vec<profile::Binding>) {
        self.bindings_list = bindings
            .iter()
            .enumerate()
            .map(|(i, b)| GuiBinding {
                id: i,
                key: Some(b.key),
                key_name: profile::key_display_name(b.key),
                func: b.func.clone(),
                mode: b.mode,
            })
            .collect();
        self.next_id = bindings.len();
        self.live_apply();
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
    /// 加载 CJK 补充字体（+ 符号字体），注入 egui 字体系统作为 fallback。
    ///
    /// 来源优先级：`[gui] font_path` 显式配置 → 自动探测系统字体
    /// （msyh.ttc → simsun.ttc）。显式路径加载失败时告警并回退自动探测。
    fn load_cjk_font(&mut self, ctx: &egui::Context) {
        let configured = &self.gui_config.font_path;
        let cjk_font = if configured.is_empty() {
            [r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\simsun.ttc"]
                .iter()
                .find_map(|path| {
                    std::fs::read(path)
                        .ok()
                        .map(|b| egui::FontData::from_owned(b).into())
                })
        } else {
            match std::fs::read(configured) {
                Ok(bytes) => Some(egui::FontData::from_owned(bytes).into()),
                Err(e) => {
                    self.log(format!(
                        "WARNING: font_path '{}' load failed ({e}) — falling back to system fonts.",
                        configured
                    ));
                    None
                }
            }
        };

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
    fn start_capture(&mut self, target: CaptureTarget) {
        if self.capture.active {
            return; // 重入守卫 — 防旧 rx 被静默替换泄漏
        }
        self.capture.active = true;
        self.capture.target = Some(target);
        self.capture.rx = Some(self.key_bindings.enable_capture());
    }

    /// 参数弹窗 — egui::Window 浮窗（2026-08-22 实测替代 Popup：popup 的
    /// from_toggle_button_response + CloseOnClickOutside 与弹窗内按钮点击
    /// 互相干扰（按钮 clicked 永远 false），Window 无该层拦截；齿轮
    /// clicked toggle `param_popup` id，× 或再点齿轮关闭）。
    fn show_param_window(&mut self, ctx: &egui::Context, id: usize, key: &Key) {
        let Some((specs, store)) = self.key_bindings.params_of(key) else {
            self.param_popup = None;
            return;
        };
        let Some(row) = self.bindings_list.iter().find(|g| g.id == id) else {
            self.param_popup = None; // 行已删除
            return;
        };
        let key_name = row.key_name.clone();
        let func = row.func.clone();
        let mut open = true;
        let mut keyslot_capture: Option<CaptureTarget> = None;

        egui::Window::new(egui::RichText::new(format!("{key_name} — {func}")).strong())
            .id(egui::Id::new(("param_window", id)))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_min_width(240.0);
                egui::Grid::new(egui::Id::new(("param_grid", id)))
                    .num_columns(2)
                    .min_col_width(90.0)
                    .show(ui, |ui| {
                        // per-function：写功能级参数表（同功能多键共享）
                        let entry = self.func_params.entry(func.clone()).or_default();
                        for (si, spec) in specs.iter().enumerate() {
                            ui.label(spec.name);
                            match &spec.kind {
                                ParamKind::Float { min, max, step, .. } => {
                                    let mut v = store.get_f64(si);
                                    let resp = ui.add(
                                        egui::DragValue::new(&mut v)
                                            .range(*min..=*max)
                                            .speed(*step)
                                            .fixed_decimals(1),
                                    );
                                    if resp.changed() {
                                        store.set_f64(si, v); // live 直写
                                        entry.insert(spec.name.into(), toml::Value::Float(v));
                                        self.dirty = true;
                                    }
                                }
                                ParamKind::Int {
                                    min: 0,
                                    max: 0x1FFFF,
                                    ..
                                } if spec.name == "key" => {
                                    // 键槽特例（SpamKey）：打包 ScanCode+E0，
                                    // Set Key 捕获按钮（能抓任意键，含异形键）
                                    let v = store.get_i64(si);
                                    let current = gi_utils::functions::spam_key::key_slot_name(v);
                                    if ui.button(current.as_str()).clicked() && !self.capture.active
                                    {
                                        keyslot_capture = Some(CaptureTarget::KeySlot {
                                            binding_id: id,
                                            slot: si,
                                        });
                                    }
                                }
                                ParamKind::Int { min, max, .. } => {
                                    let mut v = store.get_i64(si);
                                    let resp =
                                        ui.add(egui::DragValue::new(&mut v).range(*min..=*max));
                                    if resp.changed() {
                                        store.set_i64(si, v);
                                        entry.insert(spec.name.into(), toml::Value::Integer(v));
                                        self.dirty = true;
                                    }
                                }
                                ParamKind::Bool { .. } => {
                                    let mut v = store.get_bool(si);
                                    if ui.checkbox(&mut v, "").changed() {
                                        store.set_bool(si, v);
                                        entry.insert(spec.name.into(), toml::Value::Boolean(v));
                                        self.dirty = true;
                                    }
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
        if !open {
            self.param_popup = None; // × 关闭
        }

        if let Some(target) = keyslot_capture {
            self.start_capture(target);
        }
    }

    /// 取消按键捕获（弹窗 Cancel 按钮）。
    fn cancel_capture(&mut self) {
        self.key_bindings.disable_capture();
        self.capture.active = false;
        self.capture.target = None;
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

                let name = profile::key_display_name(key);

                match self.capture.target {
                    Some(CaptureTarget::Binding(id)) => {
                        // 按 id 定位行 — 捕获期间增删行不导致写错行
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

                        self.cancel_capture();
                        self.live_apply();
                    }
                    Some(CaptureTarget::KeySlot { binding_id, slot }) => {
                        // 参数键槽：打包值写 Int 槽（live 直写）+ func_params
                        // （Save 持久化）。行内 func 名定位 per-function 表；
                        // L7 同语义：键值未变不置 dirty。
                        let packed = gi_utils::functions::spam_key::pack_key(key);
                        let row = self.bindings_list.iter().find(|g| g.id == binding_id);
                        let store = row
                            .and_then(|g| g.key)
                            .and_then(|k| self.key_bindings.params_of(&k))
                            .and_then(|(specs, store)| specs.get(slot).map(|_| store));
                        if let (Some(store), Some(row)) = (store, row) {
                            if store.get_i64(slot) == packed {
                                self.log(format!("Key '{}' unchanged — no apply", key.name()));
                                self.cancel_capture();
                                return;
                            }
                            store.set_i64(slot, packed);
                            // 持久化 packed 整数（apply_params 的 Int 臂直收；
                            // String 臂仅兼容手写键名 — 存 String 会让重启
                            // 恢复/GUI 重注册走解析路径，无谓增加失败面）
                            self.func_params
                                .entry(row.func.clone())
                                .or_default()
                                .insert("key".into(), toml::Value::Integer(packed));
                            self.dirty = true;
                            self.log(format!(
                                "Parameter key → '{}' ({} / slot {slot})",
                                name, row.func
                            ));
                        }
                        // 目标行已删除 → 静默丢弃（同 Binding 分支语义）
                        self.cancel_capture();
                        // 键槽写入不改绑定结构 — 无需 live_apply；
                        // persist 随下次 Save
                    }
                    None => {}
                }
            }
        }
    }

    /// 校验键唯一性（一键一功能；功能可绑多键 — 2026-08-22 用户决策）。
    /// 未设键的行跳过。与 profile::load 的校验规则一致；live_apply 与
    /// save_config 共用。
    fn validate_bindings(&self) -> Result<(), String> {
        let mut keys = std::collections::HashSet::new();
        for g in &self.bindings_list {
            if let Some(key) = g.key
                && !keys.insert(key)
            {
                return Err(format!(
                    "duplicate key: '{}' is bound to multiple functions",
                    key.name()
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
            self.dirty = true; // 表格已是新状态而引擎仍旧绑定 — 提示未保存
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
                match profile::create_function(&g.func, self.send_ctx.clone()) {
                    Ok(f) => f,
                    Err(e) => {
                        // L8: 聚合所有错误 — 不覆盖只显示最后一个
                        errors.push(format!("'{}': {}", g.func, e));
                        continue;
                    }
                }
            };
            // 动态参数覆写（per-function：从功能级参数表取，换功能不残留）。
            // 失败语义与启动路径一致：WARN + 按默认参数注册 — 不因单个参数
            // 坏值使整行热键失效（参数错误应可见但不破坏绑定）
            let params = self.func_params.get(&g.func).cloned().unwrap_or_default();
            if let Err(e) = profile::apply_params(&func, &params) {
                errors.push(format!("'{}' params: {}（按默认参数注册）", g.func, e));
            }

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
                if profile::key_to_config_name(key).is_none() {
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
                    params: self.func_params.get(&g.func).cloned().unwrap_or_default(),
                })
            })
            .collect();

        profile::save_to(
            &profile::profile_path(&self.active_profile).expect("validated name"),
            &bindings,
            &self.func_params,
            &self.gui_config,
        )
    }

    /// 另存为指定路径 — New Profile 的实现（同 save 校验）。
    fn save_config_to(&self, path: &std::path::Path) -> Result<(), String> {
        self.validate_bindings()?;
        let bindings: Vec<Binding> = self
            .bindings_list
            .iter()
            .filter_map(|g| {
                g.key.map(|key| Binding {
                    key,
                    func: g.func.clone(),
                    mode: g.mode,
                    params: self.func_params.get(&g.func).cloned().unwrap_or_default(),
                })
            })
            .collect();
        profile::save_to(path, &bindings, &self.func_params, &self.gui_config)
    }
}

// ═══════════════════════════════════════════════════════════════════
// Drop — 清理关闭
// ═══════════════════════════════════════════════════════════════════

impl Drop for GuiApp {
    fn drop(&mut self) {
        // 只做无害清理。完整关机序列（托盘收尾/停引擎/stop_all/亲和性恢复）
        // 在 main 的 shutdown_all 显式执行 — 这里绝不能包含任何破坏性
        // 动作（关机次序语义自 glow 时代保留，详见 shutdown_all 注释）。
        self.key_bindings.disable_capture();
    }
}

/// 完整关机序列 — 仅在正常退出与重试耗尽时由 main 显式调用。
/// 顺序：托盘收尾 → 停引擎 → stop_all（先于亲和性恢复，防优化游戏竞态）
/// → 蜂鸣 → 恢复亲和性（决策 21）→ 共享图标销毁（最后，确认托盘线程已退出）。
fn shutdown_all(
    engine_handle: JoinHandle<()>,
    tray_handle: Option<JoinHandle<()>>,
    tray_quit: &AtomicBool,
    tray_icon: Option<tray_icon::SharedIcon>,
    stop_flag: &AtomicBool,
    key_bindings: &Arc<gi_utils::engine::bindings::KeyBindings>,
) {
    // 先停引擎（stop_flag → join），再收尾托盘 — 托盘收尾最坏有 2s 有界等待，
    // 放前面会拖长退出路径（review #14）。engine_handle 为直值（单次运行
    // 语义 — v1.7.0 自愈循环退役后不再需要 Option）。
    stop_flag.store(true, Ordering::Release);
    let _ = engine_handle.join();

    // 有界等待托盘线程退出；返回值 = 线程已确认退出（共享图标引用方清零）。
    let tray_exited = stop_tray_thread(tray_handle, tray_quit);

    // 显式停止所有功能线程 — 必须在亲和性恢复**之前**（决策 21）
    key_bindings.stop_all();

    // 退出蜂鸣 — 异步播放（~300ms），由随后的亲和性恢复耗时覆盖
    gi_utils::utils::beep::beep_async(375, 300);

    // 恢复所有进程的完整 CPU 亲和性（best-effort，静默吞错误）。
    // 线程级 pin 先还原（掩码留在游戏线程上不会随本进程退出消失）。
    let _ = gi_utils::utils::thread_pin::restore();
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
) -> Vec<String> {
    let mut log = Vec::new();
    for b in bindings {
        let func: Arc<dyn KeyFunction> = if b.func == "停止退出" {
            stop_func.clone()
        } else {
            match profile::create_function(&b.func, send_ctx.clone()) {
                Ok(f) => f,
                Err(e) => {
                    log.push(format!(
                        "  ERROR: '{}' -> '{}': {}",
                        b.key.name(),
                        b.func,
                        e
                    ));
                    continue;
                }
            }
        };
        // 动态参数覆写（配置侧初值 — 未知名/类型错仅记日志不阻断启动）
        if let Err(e) = profile::apply_params(&func, &b.params) {
            log.push(format!("  WARN: '{}' params: {}", b.func, e));
        }
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
/// 安装 panic hook：崩溃现场落盘 + 恢复亲和性 + MessageBox + 退出。
///
/// panic=abort 时 Drop 不执行（`restore_all_affinity` 在 Drop 路径中），
/// 若 panic 前已隔离游戏核心，其它进程会停留在受限核心 — hook 兜底恢复。
/// 必须在任何可能 panic 的代码之前安装。
///
/// dev-wgpu：原"GUI 渲染 panic 可恢复"分支（IN_GUI_RETRY/GUI_MAIN_THREAD/
/// catch_unwind 重试）已随 glow 后端一起移除 — wgpu surface Lost 逐帧
/// 自愈，渲染层不再 panic（睡眠唤醒实测通过）。hook 现为纯兜底：
/// 任何线程 panic = 落盘 + 还原 + 弹窗 + fail-fast（防 Loop 功能僵尸注入）。
fn install_panic_hook(
    log: gi_utils::utils::log_collector::LogCollector,
    stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    std::panic::set_hook(Box::new(move |info| {
        // 先停机再交互 — 弹窗是模态阻塞的，期间 engine/Loop 功能线程若
        // 仍运行会持续注入（fail-fast 语义；顺序：停机 → 落盘 → 还原 → 弹窗）
        stop_flag.store(true, std::sync::atomic::Ordering::Release);
        // 崩溃现场落盘（best-effort — panic 路径绝不 panic 或弹二次错误）
        write_crash_log(&format!("{info}"), &log);
        let _ = utils::thread_pin::restore();
        let _ = utils::affinity::restore_all_affinity();
        show_message_box(
            "GI-Utils 错误",
            &format!(
                "GI-Utils 发生致命错误，即将退出：

{}",
                info
            ),
        );
        // fail-fast：unwind 语义下线程静默死亡会让 Loop 功能继续注入、
        // F12 失效（僵尸进程）
        std::process::exit(1);
    }));
}

/// 崩溃现场落盘：panic 消息 + 日志缓冲快照写入 exe 旁 `crash.log`。
/// 全部 best-effort — 失败静默（崩溃路径不引入新失败模式）。
fn write_crash_log(summary: &str, log: &gi_utils::utils::log_collector::LogCollector) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(dir) = exe.parent() else {
        return;
    };
    let mut content = format!(
        "GI-Utils crash log — {}
panic: {}

---- log buffer ----
",
        chrono_now(),
        summary
    );
    for line in log.snapshot() {
        content.push_str(&line);
        content.push('\n');
    }
    let _ = std::fs::write(dir.join("crash.log"), content);
}

/// 崩溃时间戳（无 chrono 依赖 — SystemTime 换算，失败回退空串）。
fn chrono_now() -> String {
    use std::time::UNIX_EPOCH;
    let Ok(dur) = std::time::SystemTime::now().duration_since(UNIX_EPOCH) else {
        return String::new();
    };
    format!("UTC {}", dur.as_secs())
}

// ═══════════════════════════════════════════════════════════════════
// main — GUI 入口
// ═══════════════════════════════════════════════════════════════════

fn main() {
    use windows::core::w;

    // 启动日志（GUI 无控制台，收集到内存后在日志面板显示）
    let mut startup_log: Vec<String> = vec![
        format!("GI-Utils GUI v{}", env!("CARGO_PKG_VERSION")),
        // LGPL §4(c) 合规：运行时显示所用库的版权与许可提示
        "Interception protocol layer: derived from interception.c (oblitum/Interception), LGPL 3.0 — see LICENSE-LGPL.txt"
            .into(),
        "Initializing...".into(),
    ];

    // 全局日志收集：功能线程（优化游戏、坐标颜色等）的 tracing 输出
    // 经全局 subscriber 汇入共享 buffer，GUI 帧循环 drain 到日志面板。
    // 必须在配置加载（config.rs 首次生成提示）之前安装。
    let log_collector = gi_utils::utils::log_collector::LogCollector::install(200);

    // ── 0. 单实例保护（在任何副作用之前）────────

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
    // Profile 迁移：旧单文件（gi-utils-config.toml / config.toml）→ profiles/默认.toml
    if profile::migrate_legacy_config() {
        startup_log.push("Migrated legacy config → profiles/默认.toml".into());
    }
    let (config_bindings, startup_func_params, config_ok) = match profile::load_full() {
        Ok((b, fp)) => {
            startup_log.push(format!("Loaded {} bindings from profile", b.len()));
            (b, fp, true)
        }
        Err(e) => {
            startup_log.push(format!("Config error: {}", e));
            (Vec::new(), profile::FuncParams::new(), false)
        }
    };

    // ── 3. 创建 Engine ──────────────────────────────────────
    let engine = Engine::new();
    let key_bindings = engine.bindings();
    let send_ctx = engine.send_context();
    let stop_flag = engine.stop_flag();

    // panic hook 需持有 stop_flag（panic 时弹窗前置位停机 — 模态弹窗
    // 阻塞期间 engine/Loop 功能线程不得继续注入）。安装晚于 Engine 创建：
    // Engine::new 自身的 panic（驱动缺失）无亲和性需要还原，且其 unwrap
    // 消息经默认 hook 亦可读。
    install_panic_hook(log_collector.clone(), stop_flag.clone());

    // ── 4. 注册初始绑定 ─────────────────────────────────────
    let stop_func: Arc<dyn KeyFunction> = Arc::new(gi_utils::functions::stop::停止退出::new(
        stop_flag.clone(),
    ));
    startup_log.extend(register_all_bindings(
        &key_bindings,
        &stop_func,
        &config_bindings,
        &send_ctx,
    ));

    // ── 5. 托盘图标：主线程预加载一次（健康 GDI/WIC 状态），跨崩溃恢复
    // 轮共享 — 睡眠唤醒的 GL 崩溃会连带污染进程内 WIC 图标加载（恢复轮
    // LoadImageW 永久失败），必须在启动时完成加载。预加载失败回退程序
    // 生成图标（纯 GDI 路径，恢复轮仍可用）。
    let gui_cfg = profile::load_gui_config();
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

    // ── 6. 启动 Engine 后台线程 ──
    // 句柄留在 main（shutdown_all 需要），app 不持有 — 关机仅由
    // main 的 shutdown_all 执行。
    let engine_handle = std::thread::Builder::new()
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
        .expect("Failed to spawn engine thread");
    startup_log.push("Engine running.".into());

    // 托盘可用性 / 窗口隐藏态 — 进程级共享（GuiApp 持 Arc；tray 线程失败
    // 时重置以引导关窗走退出路径）
    let tray_ok_shared = Arc::new(AtomicBool::new(false));
    let hidden_shared = Arc::new(AtomicBool::new(false));

    // ── 7. GUI 事件循环（直接运行 — 无重试）──
    // dev-wgpu：睡眠唤醒的 glow/wgl make_current panic 已随后端切换消失
    // （wgpu surface Lost 由 egui-wgpu 逐帧 RecreateSurface 自愈，实测通过），
    // catch_unwind 重试循环退役。启动失败（非 panic）仍走弹窗 + 落盘出口。
    let function_names = profile::list_function_names();
    let (tray_tx, tray_rx) = mpsc::channel::<TrayAction>();
    let tray_quit = Arc::new(AtomicBool::new(false));
    let tray_handle = match std::thread::Builder::new().name("tray".into()).spawn({
        let icon = preloaded_icon.clone();
        let pixels = tray_pixels.clone();
        let quit = tray_quit.clone();
        move || tray::run_tray_thread(tray_tx, quit, icon, pixels, tray_w, tray_h)
    }) {
        Ok(h) => Some(h),
        Err(e) => {
            startup_log.push(format!("Tray thread spawn failed: {}", e));
            // 无托盘：隐藏态不可恢复 — 重置共享标志（关窗走退出路径）
            tray_ok_shared.store(false, Ordering::Release);
            hidden_shared.store(false, Ordering::Release);
            None
        }
    };

    let gui_bindings: Vec<GuiBinding> = config_bindings
        .iter()
        .enumerate()
        .map(|(i, b)| GuiBinding {
            id: i,
            key: Some(b.key),
            key_name: profile::key_display_name(b.key),
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
            target: None,
            rx: None,
        },
        param_popup: None,
        function_names: function_names.clone(),
        font_loaded: false,
        log_messages: startup_log.clone(),
        log_visible: true,
        log_collector: log_collector.clone(),
        tray_rx,
        should_exit: false,
        tray_ok: tray_ok_shared.clone(),
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
        func_params: startup_func_params.clone(),
        active_profile: "默认".into(),
        pending_profile_name: String::new(),
        pending_new_profile: false,
    };

    // dev-wgpu：wgpu + Vulkan 后端（睡眠唤醒修复 — WGL 上下文跨电源转换
    // 失效是 glow/glutin 的结构问题；wgpu surface Lost 可逐帧重建，见
    // egui-wgpu SurfaceErrorAction::RecreateSurface）。Renderer 显式锁定
    // wgpu（默认优先级已选 wgpu，显式防未来变化）。
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 500.0])
            .with_min_inner_size([600.0, 300.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew({
                // 官方 without_display_handle() 提供其余字段默认值
                // （power_preference=HighPerformance、8K 纹理上限等），
                // 仅覆写 backends 锁 Vulkan
                let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
                setup.instance_descriptor.backends = eframe::egui_wgpu::wgpu::Backends::VULKAN;
                setup
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    match eframe::run_native(
        "GI-Utils Configuration",
        options,
        Box::new(|_cc| Ok(Box::new(app))),
    ) {
        // 正常退出（窗口关闭 / 托盘退出）— 完整关机序列
        Ok(()) => {
            shutdown_all(
                engine_handle,
                tray_handle,
                &tray_quit,
                preloaded_icon.take(),
                &stop_flag,
                &key_bindings,
            );
        }
        // 启动错误（非 panic）— 完整关机后提示退出
        Err(e) => {
            shutdown_all(
                engine_handle,
                tray_handle,
                &tray_quit,
                preloaded_icon.take(),
                &stop_flag,
                &key_bindings,
            );
            let msg = e.to_string();
            show_message_box(
                "GI-Utils 启动失败",
                &format!(
                    "GUI 初始化失败：

{msg}"
                ),
            );
            write_crash_log(&msg, &log_collector);
            std::process::exit(1);
        }
    }
}
