//! 热键绑定配置 — TOML-based key binding configuration.
//!
//! 启动时从 exe 目录读取 `gi-utils-config.toml`。如果文件不存在，自动生成默认配置。
//! Reads `gi-utils-config.toml` from the exe directory at startup.
//! If the file is missing, a default config is generated.

use crate::engine::TriggerMode;
use crate::engine::bindings::{KeyFunction, ParamKind};
use crate::interception::SendContext;
use crate::key::Key;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

// ═══════════════════════════════════════════════════════════════════
// Config structure
// ═══════════════════════════════════════════════════════════════════

#[derive(Deserialize, Serialize)]
struct RawConfig {
    #[serde(default)]
    bindings: Vec<RawBinding>,
    #[serde(default)]
    gui: RawGuiConfig,
    /// 顶层 `[params.<功能名>]` — per-function 持久化（旧格式
    /// `[bindings.params]` 仍在 RawBinding 读取兼容）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    params: Option<FuncParams>,
}

#[derive(Deserialize, Serialize)]
struct RawBinding {
    key: String,
    func: String,
    mode: String,
    /// 旧格式（0.4 首版动态参数）兼容读取 — per-function 语义下新 Save
    /// **永不输出**（skip_serializing）：行内 params 有 TOML 归属歧义
    /// （`[bindings.params]` 落在下一个 `[[bindings]]` 元素内），持久化
    /// 一律走顶层 `[params.<功能名>]`。load 时行内参数提升迁移到全局表。
    #[serde(default, skip_serializing)]
    params: Option<std::collections::BTreeMap<String, toml::Value>>,
}

#[derive(Deserialize, Serialize, Default)]
struct RawGuiConfig {
    #[serde(default)]
    icon_path: String,
    #[serde(default)]
    font_path: String,
}

/// GUI 配置 — GUI-only settings from the `[gui]` section.
#[derive(Debug, Clone, Default)]
pub struct GuiConfig {
    /// 托盘图标 .ico 文件路径。空或文件不存在时回退程序生成图标。
    pub icon_path: String,
    /// CJK 补充字体文件路径（.ttf/.ttc/.otf）。空 = 自动探测系统字体
    /// （msyh.ttc → simsun.ttc 顺序）；非空但加载失败时回退自动探测。
    pub font_path: String,
}

/// 动态参数表（名字 → 值；BTreeMap 序列化稳定有序）。
pub type Params = std::collections::BTreeMap<String, toml::Value>;

/// 功能级参数表 — **per-function 语义**（2026-08-22 修正）：
/// 参数属于功能定义而非绑定行 — 同功能多键共享一份；换键换功能时
/// 参数随功能走，不残留、不互扰（旧实现挂在 Binding 行上，同键换
/// 功能会用旧功能参数校验新功能而报错，实证）。
pub type FuncParams = std::collections::BTreeMap<String, Params>;

/// 解析后的绑定项 — A parsed binding ready to register.
#[derive(Clone)]
pub struct Binding {
    pub key: Key,
    pub func: String,
    pub mode: TriggerMode,
    /// 动态参数初值（空 = 全默认）。
    pub params: Params,
}

// ═══════════════════════════════════════════════════════════════════
// Key name → Key constant  (case-insensitive lookup)
// ═══════════════════════════════════════════════════════════════════

/// All known key-name to Key mappings. Single source of truth for both
/// parse (string → Key) and serialize (Key → string).
const KEY_PAIRS: &[(&str, Key)] = &[
    ("F1", Key::F1),
    ("F2", Key::F2),
    ("F3", Key::F3),
    ("F4", Key::F4),
    ("F5", Key::F5),
    ("F6", Key::F6),
    ("F7", Key::F7),
    ("F8", Key::F8),
    ("F9", Key::F9),
    ("F10", Key::F10),
    ("F11", Key::F11),
    ("F12", Key::F12),
    ("F13", Key::F13),
    ("F14", Key::F14),
    ("F15", Key::F15),
    ("F16", Key::F16),
    ("F17", Key::F17),
    ("F18", Key::F18),
    ("F19", Key::F19),
    ("F20", Key::F20),
    ("F21", Key::F21),
    ("F22", Key::F22),
    ("F23", Key::F23),
    ("F24", Key::F24),
    ("Esc", Key::ESCAPE),
    ("Tab", Key::TAB),
    ("CapsLock", Key::CAPS_LOCK),
    ("LShift", Key::LSHIFT),
    ("RShift", Key::RSHIFT),
    ("LCtrl", Key::LCTRL),
    ("RCtrl", Key::RCTRL),
    ("LAlt", Key::LALT),
    ("RAlt", Key::RALT),
    ("LWin", Key::LWIN),
    ("RWin", Key::RWIN),
    ("Apps", Key::APPS),
    ("Space", Key::SPACE),
    ("Enter", Key::ENTER),
    ("Backspace", Key::BACKSPACE),
    ("A", Key::A),
    ("B", Key::B),
    ("C", Key::C),
    ("D", Key::D),
    ("E", Key::E),
    ("F", Key::F),
    ("G", Key::G),
    ("H", Key::H),
    ("I", Key::I),
    ("J", Key::J),
    ("K", Key::K),
    ("L", Key::L),
    ("M", Key::M),
    ("N", Key::N),
    ("O", Key::O),
    ("P", Key::P),
    ("Q", Key::Q),
    ("R", Key::R),
    ("S", Key::S),
    ("T", Key::T),
    ("U", Key::U),
    ("V", Key::V),
    ("W", Key::W),
    ("X", Key::X),
    ("Y", Key::Y),
    ("Z", Key::Z),
    ("N1", Key::N1),
    ("N2", Key::N2),
    ("N3", Key::N3),
    ("N4", Key::N4),
    ("N5", Key::N5),
    ("N6", Key::N6),
    ("N7", Key::N7),
    ("N8", Key::N8),
    ("N9", Key::N9),
    ("N0", Key::N0),
    ("Up", Key::UP),
    ("Down", Key::DOWN),
    ("Left", Key::LEFT),
    ("Right", Key::RIGHT),
    ("Home", Key::HOME),
    ("End", Key::END),
    ("PageUp", Key::PAGEUP),
    ("PageDown", Key::PAGEDOWN),
    ("Insert", Key::INSERT),
    ("Delete", Key::DELETE),
    ("PrintScreen", Key::PRINT_SCREEN),
    ("ScrollLock", Key::SCROLL_LOCK),
    ("NumLock", Key::NUM_LOCK),
    ("SysRq", Key::SYS_RQ),
    ("`", Key::GRAVE),
    ("-", Key::MINUS),
    ("=", Key::EQUALS),
    ("[", Key::LBRACKET),
    ("]", Key::RBRACKET),
    ("\\", Key::BACKSLASH),
    (";", Key::SEMICOLON),
    ("'", Key::QUOTE),
    (",", Key::COMMA),
    (".", Key::PERIOD),
    ("/", Key::SLASH),
    ("MediaPrevTrack", Key::MEDIA_PREV_TRACK),
    ("MediaNextTrack", Key::MEDIA_NEXT_TRACK),
    ("MediaMute", Key::MEDIA_MUTE),
    ("MediaCalculator", Key::MEDIA_CALCULATOR),
    ("MediaPlayPause", Key::MEDIA_PLAY_PAUSE),
    ("MediaStop", Key::MEDIA_STOP),
    ("MediaVolumeDown", Key::MEDIA_VOLUME_DOWN),
    ("MediaVolumeUp", Key::MEDIA_VOLUME_UP),
    ("MediaWWWHome", Key::MEDIA_WWW_HOME),
    ("Power", Key::ACPI_POWER),
    ("Sleep", Key::ACPI_SLEEP),
    ("Wake", Key::ACPI_WAKE),
    ("Oem5", Key::OEM5),
    ("Numpad0", Key::NUMPAD_0),
    ("Numpad1", Key::NUMPAD_1),
    ("Numpad2", Key::NUMPAD_2),
    ("Numpad3", Key::NUMPAD_3),
    ("Numpad4", Key::NUMPAD_4),
    ("Numpad5", Key::NUMPAD_5),
    ("Numpad6", Key::NUMPAD_6),
    ("Numpad7", Key::NUMPAD_7),
    ("Numpad8", Key::NUMPAD_8),
    ("Numpad9", Key::NUMPAD_9),
    ("NumpadAdd", Key::NUMPAD_ADD),
    ("NumpadSubtract", Key::NUMPAD_SUBTRACT),
    ("NumpadMultiply", Key::NUMPAD_MULTIPLY),
    ("NumpadDivide", Key::NUMPAD_DIVIDE),
    ("NumpadEnter", Key::NUMPAD_ENTER),
    ("NumpadPeriod", Key::NUMPAD_PERIOD),
];

static KEY_MAP: OnceLock<HashMap<String, Key>> = OnceLock::new();

fn key_map() -> &'static HashMap<String, Key> {
    KEY_MAP.get_or_init(|| {
        KEY_PAIRS
            .iter()
            .map(|(s, k)| (s.to_lowercase(), *k))
            .collect()
    })
}

pub fn parse_key(name: &str) -> Result<Key, String> {
    key_map()
        .get(&name.to_lowercase())
        .copied()
        .ok_or_else(|| format!("unknown key: '{}'", name))
}

/// 反向查找：Key → 配置名。用于序列化时把 Key 转回 "F13" / "NumpadAdd" 等。
/// Reverse lookup: Key → config name. Used when serializing bindings.
pub fn key_to_config_name(key: Key) -> Option<&'static str> {
    KEY_PAIRS.iter().find(|(_, k)| *k == key).map(|(s, _)| *s)
}

// ═══════════════════════════════════════════════════════════════════
// Mode name → TriggerMode
// ═══════════════════════════════════════════════════════════════════

fn parse_mode(name: &str) -> Result<TriggerMode, String> {
    match name {
        "Once" => Ok(TriggerMode::Once),
        "Loop" => Ok(TriggerMode::Loop),
        "Toggle" => Ok(TriggerMode::Toggle),
        _ => Err(format!(
            "unknown mode: '{}' (use Once / Loop / Toggle)",
            name
        )),
    }
}

// ═══════════════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════════════

/// 配置文件路径（与 exe 同目录）— Path to the config file (next to the exe).
/// 默认配置路径 — exe 旁 `gi-utils-config.toml`（2026-08-22 由
/// config.toml 改名：突出工具专属、避免与常见 config.toml 混淆）。
pub fn default_config_path() -> PathBuf {
    let mut path = std::env::current_exe().expect("failed to get executable path");
    path.set_file_name("gi-utils-config.toml");
    path
}

/// 默认配置内容（首次运行时写入）— Default config content, written on first run.
pub(crate) const DEFAULT_CONFIG: &str = r#"# GI-Utils 热键配置
# 格式: [[bindings]]  key = "按键名"  func = "功能名"  mode = "Once/Loop/Toggle"

[[bindings]]
key = "F12"
func = "停止退出"
mode = "Once"

[[bindings]]
key = "F13"
func = "连点器"
mode = "Loop"

[[bindings]]
key = "F14"
func = "快速拾取"
mode = "Loop"

[[bindings]]
key = "F15"
func = "鬼畜走路"
mode = "Loop"

[[bindings]]
key = "F16"
func = "火神跳喷"
mode = "Loop"

[[bindings]]
key = "F17"
func = "甘雨走A"
mode = "Once"

[[bindings]]
key = "F18"
func = "双玛头"
mode = "Loop"

[[bindings]]
key = "F19"
func = "坐标颜色"
mode = "Loop"

[[bindings]]
key = "NumpadAdd"
func = "优化游戏"
mode = "Once"

# 线程采样 — YuanShen.exe 线程画像（procexp Threads 自动化，分析用途）
[[bindings]]
key = "F20"
func = "线程采样"
mode = "Once"

# 动态参数 — per-function（[params.<功能名>]，GUI ⚙ 面板修改后 Save 同步）
[params."连点器"]
interval_ms = 10.0
hold_ms = 0.0

# GUI 配置 — icon_path 指向 .ico 托盘图标；font_path 指向 CJK 补充字体；
# 两者留空均使用自动回退（程序生成图标 / 系统字体 msyh→simsun）
[gui]
icon_path = "E:/Projects/Rust/GI-Utils/assets/icon.ico"
font_path = ""
"#;

/// 加载并解析配置文件 — Load and parse the config file. Generates a default if missing.
///
/// 若文件不存在则自动创建默认配置。验证双向唯一性：每个按键只能绑定一个功能，
/// 每个功能只能绑定一个按键。
/// If the file is missing, a default config is generated. Validates bidirectional
/// uniqueness: each key maps to one function and each function maps to one key.
/// 加载配置（含功能级参数表）— load() 的完整形态。
/// 校验仅键唯一（功能可绑多键，2026-08-22）。
/// 返回 (绑定列表, 功能参数表)；`load()` 为仅取绑定的便捷包装。
pub fn load_full() -> Result<(Vec<Binding>, FuncParams), String> {
    load_full_from(&default_config_path())
}

/// 从指定路径加载配置（含功能级参数表）— Load from File 的实现。
pub fn load_full_from(path: &std::path::Path) -> Result<(Vec<Binding>, FuncParams), String> {
    let path = path.to_path_buf();

    if !path.exists() {
        tracing::info!("  No config file detected.");
        tracing::info!("  Generating default config: {}", path.display());
        std::fs::write(&path, DEFAULT_CONFIG)
            .map_err(|e| format!("failed to write default config: {}", e))?;
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;

    let raw: RawConfig = toml::from_str(&content).map_err(|e| format!("invalid config: {}", e))?;
    let mut func_params = raw.params.unwrap_or_default();

    let mut bindings = Vec::new();
    for (i, b) in raw.bindings.iter().enumerate() {
        let key = parse_key(&b.key).map_err(|e| format!("binding #{}: {}", i + 1, e))?;
        let mode = parse_mode(&b.mode).map_err(|e| format!("binding #{}: {}", i + 1, e))?;
        // fill 语义：Binding.params = 该功能在全局表里的参数（快照），
        // 供注册初值应用；顶层无此功能时回退旧格式 [bindings.params]（兼容）
        let params = func_params
            .get(&b.func)
            .cloned()
            .unwrap_or_else(|| b.params.clone().unwrap_or_default());
        bindings.push(Binding {
            key,
            func: b.func.clone(),
            mode,
            params,
        });
        // 旧格式迁移：顶层未登记但行内带参 → 提升进全局表（save 时持久）
        if !func_params.contains_key(&b.func) && b.params.as_ref().is_some_and(|p| !p.is_empty()) {
            func_params.insert(b.func.clone(), b.params.clone().unwrap_or_default());
        }
    }

    // ── Validate key uniqueness ──────────────────────────────
    // 仅键唯一（一键一功能）；功能可绑多键（2026-08-22 用户决策 —
    // 同功能多键各自独立实例/toggle，参数 per-function 共享）

    let mut keys = HashSet::new();
    for (i, b) in bindings.iter().enumerate() {
        if !keys.insert(b.key) {
            return Err(format!(
                "binding #{}: key '{}' is already bound to another function",
                i + 1,
                b.key.name()
            ));
        }
    }

    Ok((bindings, func_params))
}

/// 加载配置 — 仅取绑定列表（便捷包装，注册用）。
pub fn load() -> Result<Vec<Binding>, String> {
    load_full().map(|(bindings, _)| bindings)
}

/// 加载 `[gui]` 段配置（图标路径等）。解析失败/缺段时返回默认值 —
/// GUI 配置损坏不应阻止程序启动，回退默认行为即可。
/// Load the `[gui]` section. Failures fall back to defaults — a broken
/// GUI section must not block startup.
pub fn load_gui_config() -> GuiConfig {
    load_gui_config_from(&default_config_path())
}

/// 从指定路径读取 `[gui]` 段（Load from File 时同步托盘/字体设置）。
pub fn load_gui_config_from(path: &std::path::Path) -> GuiConfig {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return GuiConfig::default(),
    };
    match toml::from_str::<RawConfig>(&content) {
        Ok(raw) => GuiConfig {
            icon_path: raw.gui.icon_path,
            font_path: raw.gui.font_path,
        },
        Err(_) => GuiConfig::default(),
    }
}

/// 保存绑定列表到默认配置文件 — Serialize bindings back to config file.
///
/// [gui] 段由调用方传入的 `gui` 原样写回 — **fail-closed**：绝不读磁盘
/// 回填（读回失败静默回退默认值会清空用户 icon_path — review #3）。
/// 无法序列化的键返回错误（不写 "?" — "?" 下次启动解析失败会拖垮全部绑定）。
pub fn save(bindings: &[Binding], func_params: &FuncParams, gui: &GuiConfig) -> Result<(), String> {
    save_to(&default_config_path(), bindings, func_params, gui)
}

/// 另存为指定路径 — Save As 的实现（原子写同默认路径）。
pub fn save_to(
    path: &std::path::Path,
    bindings: &[Binding],
    func_params: &FuncParams,
    gui: &GuiConfig,
) -> Result<(), String> {
    let raw_bindings: Vec<RawBinding> = bindings
        .iter()
        .map(|b| {
            Ok(RawBinding {
                key: key_to_config_name(b.key)
                    .ok_or_else(|| {
                        format!("key '{}' cannot be serialized to config", b.key.name())
                    })?
                    .to_string(),
                func: b.func.clone(),
                mode: format!("{:?}", b.mode),
                params: if b.params.is_empty() {
                    None
                } else {
                    Some(b.params.clone())
                },
            })
        })
        .collect::<Result<_, String>>()?;
    let toml_str = toml::to_string_pretty(&RawConfig {
        bindings: raw_bindings,
        gui: RawGuiConfig {
            icon_path: gui.icon_path.clone(),
            font_path: gui.font_path.clone(),
        },
        params: if func_params.is_empty() {
            None
        } else {
            Some(func_params.clone())
        },
    })
    .map_err(|e| format!("failed to serialize config: {}", e))?;
    let content = format!("# GI-Utils 热键配置\n# 由 GUI 面板生成\n\n{}", toml_str);
    // 原子写：先写同目录临时文件再 rename 覆盖 — 直接 write 中途崩溃/
    // 断电会留下截断的配置文件，下次启动解析失败拖垮全部绑定
    // （review 4.4）。同目录保证 rename 同卷（Windows rename 即
    // MoveFileExW REPLACE_EXISTING，可覆盖已存在的目标）。
    let tmp_path = path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, &content).map_err(|e| format!("failed to write config: {}", e))?;
    std::fs::rename(&tmp_path, &path).map_err(|e| format!("failed to replace config: {}", e))
}

/// 键的显示名 — 优先配置名（"F13"/"NumpadAdd"），未收录回退扫描码名。
/// GUI 显示路径共用的唯一实现（review：三份拷贝漂移风险）。
pub fn key_display_name(key: Key) -> String {
    key_to_config_name(key)
        .map(str::to_string)
        .unwrap_or_else(|| key.name().to_string())
}

/// 返回所有可用功能名称 — List all available function names.
pub fn list_function_names() -> Vec<&'static str> {
    vec![
        "停止退出",
        "连点器",
        "SpamKey",
        "快速拾取",
        "鬼畜走路",
        "火神跳喷",
        "甘雨走A",
        "双玛头",
        "坐标颜色",
        "优化游戏",
        "优化游戏标准",
        "优化游戏简易",
        "线程采样",
    ]
}

// ═══════════════════════════════════════════════════════════════════
// Function factory
// ═══════════════════════════════════════════════════════════════════

/// 按名称创建功能实例 — Create a function instance by name.
///
/// 新增功能只需在此处增加一个分支。
/// New functions only need a branch added here.
pub fn create_function(
    name: &str,
    send_ctx: Arc<SendContext>,
) -> Result<Arc<dyn KeyFunction>, String> {
    match name {
        "连点器" => Ok(Arc::new(crate::functions::auto_clicker::连点器::new(
            send_ctx,
        ))),
        "SpamKey" => Ok(Arc::new(crate::functions::spam_key::SpamKey::new(send_ctx))),
        "快速拾取" => Ok(Arc::new(crate::functions::quick_pickup::快速拾取::new(
            send_ctx,
        ))),
        "鬼畜走路" => Ok(Arc::new(crate::functions::ghost_walk::鬼畜走路::new(
            send_ctx,
        ))),
        "火神跳喷" => Ok(Arc::new(crate::functions::mavuika_jump::火神跳喷::new(
            send_ctx,
        ))),
        "甘雨走A" => Ok(Arc::new(
            crate::functions::ganyu_aim_cancel::甘雨走A::new(send_ctx),
        )),
        "双玛头" => Ok(Arc::new(
            crate::functions::mavuika_double_cancel::双玛头::new(send_ctx),
        )),
        "坐标颜色" => Ok(Arc::new(crate::functions::mouse_color::坐标颜色::new())),
        "优化游戏" => Ok(Arc::new(crate::functions::optimize_game::优化游戏::new())),
        "优化游戏标准" => Ok(Arc::new(
            crate::functions::optimize_game::优化游戏标准::new(),
        )),
        "优化游戏简易" => Ok(Arc::new(
            crate::functions::optimize_game::优化游戏简易::new(),
        )),
        "线程采样" => Ok(Arc::new(crate::functions::thread_sampler::线程采样::new())),
        _ => Err(format!("unknown function: '{}'", name)),
    }
}

/// 把配置参数按名应用到功能实例（注册前调用 — 工厂产出默认值后覆写）。
/// 未知名 / 类型不匹配 → Err（配置错误显式暴露，不静默吞）。
/// 整数槽接受 toml Integer（f64 若整值也接受 — TOML 区分但用户手写
/// `interval = 10`（整数）配 float 槽属常见笔误，宽容转换）。
pub fn apply_params(func: &Arc<dyn KeyFunction>, params: &Params) -> Result<(), String> {
    let Some(store) = func.param_store() else {
        if params.is_empty() {
            return Ok(());
        }
        return Err(format!(
            "功能无参数，却配置了 params: {:?}",
            params.keys().collect::<Vec<_>>()
        ));
    };
    let specs = func.parameters();
    for (name, value) in params {
        let Some((idx, spec)) = specs
            .iter()
            .enumerate()
            .find(|(_, s)| s.name == name.as_str())
        else {
            let avail: Vec<&str> = specs.iter().map(|s| s.name).collect();
            return Err(format!("未知参数 '{name}'（可用: {avail:?}）"));
        };
        match (&spec.kind, value) {
            (ParamKind::Float { min, max, .. }, toml::Value::Float(v)) => {
                // NaN/Inf 显式拒绝 — clamp 不滤 NaN，穿透会致下游
                // clamp(min,max) 断言 panic（功能线程 panic = 全进程退出）
                if !v.is_finite() {
                    return Err(format!("参数 '{name}' 非有限值: {v}"));
                }
                store.set_f64(idx, v.clamp(*min, *max));
            }
            (ParamKind::Float { min, max, .. }, toml::Value::Integer(v)) => {
                // 整值宽容转换（手写 "10" 意图即 10.0）
                store.set_f64(idx, (*v as f64).clamp(*min, *max));
            }
            (ParamKind::Int { .. }, toml::Value::String(name_val)) if spec.name == "key" => {
                // 键槽（SpamKey）：持久化为键名 — GUI 捕获写 String；
                // 解析回打包值（解析失败显式报错，不静默回落）
                let Some(key) = crate::config::parse_key(name_val).ok() else {
                    return Err(format!("参数 '{name}' 键名无效: '{name_val}'"));
                };
                store.set_i64(idx, crate::functions::spam_key::pack_key(key));
            }
            (ParamKind::Int { min, max, .. }, toml::Value::Integer(v)) => {
                store.set_i64(idx, (*v).clamp(*min, *max));
            }
            (ParamKind::Bool { .. }, toml::Value::Boolean(v)) => {
                store.set_bool(idx, *v);
            }
            (kind, v) => {
                return Err(format!(
                    "参数 '{name}' 类型不匹配: 值 {v:?} 无法用于 {kind_name}",
                    kind_name = match kind {
                        ParamKind::Float { .. } => "Float",
                        ParamKind::Int { .. } => "Int",
                        ParamKind::Bool { .. } => "Bool",
                    }
                ));
            }
        }
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// apply_params 单测 — 按名/类型匹配与错误语义
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod default_config_tests {
    /// DEFAULT_CONFIG 必须能被自家解析器全量消化（fallback 文件 = 用户
    /// 第一份配置，解析失败即首启报错 — 回归网）
    #[test]
    fn default_config_parses_with_params_table() {
        let raw: toml::Value = toml::from_str(crate::config::DEFAULT_CONFIG)
            .expect("DEFAULT_CONFIG must be valid TOML");
        // 顶层 per-function 参数表存在且归属正确
        assert!(raw.get("params").is_some());
        assert!(raw["params"]["连点器"]["interval_ms"].as_float() == Some(10.0));
        // 行内旧格式不得再出现（skip_serializing 只管写出 — 此处保证模板源头干净）
        let text = crate::config::DEFAULT_CONFIG;
        assert!(!text.contains("[bindings.params]"), "行内 params 已废弃");
    }
}

#[cfg(test)]
mod param_tests {
    use super::*;
    use crate::engine::bindings::{KeyFunction, ParamSpec, ParamValues};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    struct ParamFunc {
        store: Arc<ParamValues>,
    }
    const SPEC: &[ParamSpec] = &[
        ParamSpec::float("interval_ms", 10.0, 1.0, 100.0, 0.5),
        ParamSpec::boolean("turbo", false),
    ];
    impl KeyFunction for ParamFunc {
        fn execute(&self, _: Arc<AtomicBool>) {}
        fn parameters(&self) -> &[ParamSpec] {
            SPEC
        }
        fn param_store(&self) -> Option<Arc<ParamValues>> {
            Some(self.store.clone())
        }
    }
    struct NoParamFunc;
    impl KeyFunction for NoParamFunc {
        fn execute(&self, _: Arc<AtomicBool>) {}
    }

    fn mk(params: &[(&str, toml::Value)]) -> (Arc<ParamFunc>, Params) {
        let mut map = Params::new();
        for (k, v) in params {
            map.insert((*k).into(), v.clone());
        }
        (
            Arc::new(ParamFunc {
                store: Arc::new(ParamValues::new(SPEC)),
            }),
            map,
        )
    }

    #[test]
    fn apply_by_name_and_typed_write() {
        let (f, params) = mk(&[
            ("interval_ms", toml::Value::Float(25.0)),
            ("turbo", toml::Value::Boolean(true)),
        ]);
        apply_params(&(f.clone() as Arc<dyn KeyFunction>), &params).unwrap();
        assert_eq!(f.store.get_f64(0), 25.0);
        assert!(f.store.get_bool(1));
    }

    #[test]
    fn integer_tolerance_for_float_slot() {
        // 手写 interval_ms = 10（整值）配 float 槽 → 宽容转换
        let (f, params) = mk(&[("interval_ms", toml::Value::Integer(10))]);
        apply_params(&(f.clone() as Arc<dyn KeyFunction>), &params).unwrap();
        assert_eq!(f.store.get_f64(0), 10.0);
    }

    #[test]
    fn unknown_name_rejected() {
        let (f, params) = mk(&[("nope", toml::Value::Float(1.0))]);
        assert!(apply_params(&(f as Arc<dyn KeyFunction>), &params).is_err());
    }

    #[test]
    fn wrong_type_rejected_and_clamped() {
        // 类型不符
        let (f, params) = mk(&[("interval_ms", toml::Value::String("x".into()))]);
        assert!(apply_params(&(f as Arc<dyn KeyFunction>), &params).is_err());
        // 超范围 clamp
        let (f, params) = mk(&[("interval_ms", toml::Value::Float(999.0))]);
        apply_params(&(f.clone() as Arc<dyn KeyFunction>), &params).unwrap();
        assert_eq!(f.store.get_f64(0), 100.0); // max
    }

    #[test]
    fn no_param_function_rejects_params_and_accepts_empty() {
        let f: Arc<dyn KeyFunction> = Arc::new(NoParamFunc);
        assert!(apply_params(&f, &Params::new()).is_ok());
        let mut p = Params::new();
        p.insert("x".into(), toml::Value::Boolean(true));
        assert!(apply_params(&f, &p).is_err());
    }
}
