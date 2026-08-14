//! 热键绑定配置 — TOML-based key binding configuration.
//!
//! 启动时从 exe 目录读取 `config.toml`。如果文件不存在，自动生成默认配置。
//! Reads `config.toml` from the exe directory at startup.
//! If the file is missing, a default config is generated.

use crate::engine::function::KeyFunction;
use crate::engine::TriggerMode;
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
}

#[derive(Deserialize, Serialize)]
struct RawBinding {
    key: String,
    func: String,
    mode: String,
}

#[derive(Deserialize, Serialize, Default)]
struct RawGuiConfig {
    #[serde(default)]
    icon_path: String,
}

/// GUI 配置 — GUI-only settings from the `[gui]` section.
#[derive(Debug, Clone, Default)]
pub struct GuiConfig {
    /// 托盘图标 .ico 文件路径。空或文件不存在时回退程序生成图标。
    pub icon_path: String,
}

/// 解析后的绑定项 — A parsed binding ready to register.
pub struct Binding {
    pub key: Key,
    pub func: String,
    pub mode: TriggerMode,
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

fn parse_key(name: &str) -> Result<Key, String> {
    key_map()
        .get(&name.to_lowercase())
        .copied()
        .ok_or_else(|| format!("unknown key: '{}'", name))
}

/// 反向查找：Key → 配置名。用于序列化时把 Key 转回 "F13" / "NumpadAdd" 等。
/// Reverse lookup: Key → config name. Used when serializing bindings.
pub fn key_to_config_name(key: Key) -> Option<&'static str> {
    KEY_PAIRS
        .iter()
        .find(|(_, k)| *k == key)
        .map(|(s, _)| *s)
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
fn config_path() -> PathBuf {
    let mut path = std::env::current_exe().expect("failed to get executable path");
    path.set_file_name("config.toml");
    path
}

/// 默认配置内容（首次运行时写入）— Default config content, written on first run.
const DEFAULT_CONFIG: &str = r#"# GI-Utils 热键配置
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
key = "F20"
func = "龙王喷水"
mode = "Loop"

# 龙王喷水方向子功能 — 按住 F20 喷射时实时调整方向
[[bindings]]
key = "Up"
func = "上"
mode = "Once"

[[bindings]]
key = "Down"
func = "下"
mode = "Once"

[[bindings]]
key = "Left"
func = "左"
mode = "Once"

[[bindings]]
key = "Right"
func = "右"
mode = "Once"

[[bindings]]
key = "Home"
func = "重置"
mode = "Once"

[[bindings]]
key = "NumpadAdd"
func = "优化游戏"
mode = "Once"

# GUI 配置 — icon_path 指向 .ico 托盘图标；留空使用程序生成图标
[gui]
icon_path = ""
"#;

/// 加载并解析配置文件 — Load and parse the config file. Generates a default if missing.
///
/// 若文件不存在则自动创建默认配置。验证双向唯一性：每个按键只能绑定一个功能，
/// 每个功能只能绑定一个按键。
/// If the file is missing, a default config is generated. Validates bidirectional
/// uniqueness: each key maps to one function and each function maps to one key.
pub fn load() -> Result<Vec<Binding>, String> {
    let path = config_path();

    if !path.exists() {
        tracing::info!("  No config file detected.");
        tracing::info!("  Generating default config: {}", path.display());
        std::fs::write(&path, DEFAULT_CONFIG)
            .map_err(|e| format!("failed to write default config: {}", e))?;
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;

    let raw: RawConfig = toml::from_str(&content).map_err(|e| format!("invalid config: {}", e))?;

    let mut bindings = Vec::new();
    for (i, b) in raw.bindings.iter().enumerate() {
        let key = parse_key(&b.key).map_err(|e| format!("binding #{}: {}", i + 1, e))?;
        let mode = parse_mode(&b.mode).map_err(|e| format!("binding #{}: {}", i + 1, e))?;
        bindings.push(Binding {
            key,
            func: b.func.clone(),
            mode,
        });
    }

    // ── Validate bidirectional uniqueness ────────────────────

    let mut keys = HashSet::new();
    let mut funcs = HashSet::new();
    for (i, b) in bindings.iter().enumerate() {
        if !keys.insert(b.key) {
            return Err(format!(
                "binding #{}: key '{}' is already bound to another function",
                i + 1,
                b.key.name()
            ));
        }
        if !funcs.insert(b.func.clone()) {
            return Err(format!(
                "binding #{}: function '{}' is already bound to another key",
                i + 1,
                b.func
            ));
        }
    }

    Ok(bindings)
}

/// 加载 `[gui]` 段配置（图标路径等）。解析失败/缺段时返回默认值 —
/// GUI 配置损坏不应阻止程序启动，回退默认行为即可。
/// Load the `[gui]` section. Failures fall back to defaults — a broken
/// GUI section must not block startup.
pub fn load_gui_config() -> GuiConfig {
    let content = match std::fs::read_to_string(config_path()) {
        Ok(c) => c,
        Err(_) => return GuiConfig::default(),
    };
    match toml::from_str::<RawConfig>(&content) {
        Ok(raw) => GuiConfig {
            icon_path: raw.gui.icon_path,
        },
        Err(_) => GuiConfig::default(),
    }
}

/// 保存绑定列表到 config.toml — Serialize bindings back to config.toml.
pub fn save(bindings: &[Binding]) -> Result<(), String> {
    let raw_bindings: Vec<RawBinding> = bindings
        .iter()
        .map(|b| RawBinding {
            key: key_to_config_name(b.key)
                .unwrap_or("?")
                .to_string(),
            func: b.func.clone(),
            mode: format!("{:?}", b.mode),
        })
        .collect();
    let toml_str = toml::to_string_pretty(&RawConfig {
        bindings: raw_bindings,
        gui: RawGuiConfig::default(),
    })
    .map_err(|e| format!("failed to serialize config: {}", e))?;
    let content = format!(
        "# GI-Utils 热键配置\n# 由 GUI 面板生成\n\n{}",
        toml_str
    );
    std::fs::write(config_path(), content).map_err(|e| format!("failed to write config: {}", e))
}

/// 返回所有可用功能名称 — List all available function names.
pub fn list_function_names() -> Vec<&'static str> {
    vec![
        "停止退出",
        "连点器",
        "快速拾取",
        "鬼畜走路",
        "火神跳喷",
        "甘雨走A",
        "双玛头",
        "坐标颜色",
        "优化游戏",
        "龙王喷水",
        "上",
        "下",
        "左",
        "右",
        "重置",
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
        "龙王喷水" => Ok(Arc::new(crate::functions::dragon_spin::龙王喷水::new(
            send_ctx,
        ))),
        "上" => Ok(Arc::new(crate::functions::dragon_spin::上::new())),
        "下" => Ok(Arc::new(crate::functions::dragon_spin::下::new())),
        "左" => Ok(Arc::new(crate::functions::dragon_spin::左::new())),
        "右" => Ok(Arc::new(crate::functions::dragon_spin::右::new())),
        "重置" => Ok(Arc::new(crate::functions::dragon_spin::重置::new())),
        _ => Err(format!("unknown function: '{}'", name)),
    }
}
