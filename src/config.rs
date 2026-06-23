//! TOML-based key binding configuration.
//!
//! Reads `config.toml` from the exe directory at startup.
//! If the file is missing, a default config is generated.

use crate::engine::function::KeyFunction;
use crate::engine::TriggerMode;
use crate::interception::SendContext;
use crate::key::Key;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

// ═══════════════════════════════════════════════════════════════════
// Config structure
// ═══════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
struct RawConfig {
    #[serde(default)]
    bindings: Vec<RawBinding>,
}

#[derive(Deserialize)]
struct RawBinding {
    key: String,
    func: String,
    mode: String,
}

/// A parsed binding: ready to register.
pub struct Binding {
    pub key: Key,
    pub func: String,
    pub mode: TriggerMode,
}

// ═══════════════════════════════════════════════════════════════════
// Key name → Key constant  (case-insensitive lookup)
// ═══════════════════════════════════════════════════════════════════

static KEY_MAP: OnceLock<HashMap<String, Key>> = OnceLock::new();

fn key_map() -> &'static HashMap<String, Key> {
    KEY_MAP.get_or_init(|| {
        let pairs: &[(&str, Key)] = &[
            ("F1", Key::F1), ("F2", Key::F2), ("F3", Key::F3),
            ("F4", Key::F4), ("F5", Key::F5), ("F6", Key::F6),
            ("F7", Key::F7), ("F8", Key::F8), ("F9", Key::F9),
            ("F10", Key::F10), ("F11", Key::F11), ("F12", Key::F12),
            ("F13", Key::F13), ("F14", Key::F14), ("F15", Key::F15),
            ("F16", Key::F16), ("F17", Key::F17), ("F18", Key::F18),
            ("F19", Key::F19), ("F20", Key::F20), ("F21", Key::F21),
            ("F22", Key::F22), ("F23", Key::F23), ("F24", Key::F24),
            ("Esc", Key::ESCAPE), ("Tab", Key::TAB),
            ("CapsLock", Key::CAPS_LOCK),
            ("LShift", Key::LSHIFT), ("RShift", Key::RSHIFT),
            ("LCtrl", Key::LCTRL), ("RCtrl", Key::RCTRL),
            ("LAlt", Key::LALT), ("RAlt", Key::RALT),
            ("LWin", Key::LWIN), ("RWin", Key::RWIN),
            ("Apps", Key::APPS),
            ("Space", Key::SPACE), ("Enter", Key::ENTER),
            ("Backspace", Key::BACKSPACE),
            ("A", Key::A), ("B", Key::B), ("C", Key::C), ("D", Key::D),
            ("E", Key::E), ("F", Key::F), ("G", Key::G), ("H", Key::H),
            ("I", Key::I), ("J", Key::J), ("K", Key::K), ("L", Key::L),
            ("M", Key::M), ("N", Key::N), ("O", Key::O), ("P", Key::P),
            ("Q", Key::Q), ("R", Key::R), ("S", Key::S), ("T", Key::T),
            ("U", Key::U), ("V", Key::V), ("W", Key::W), ("X", Key::X),
            ("Y", Key::Y), ("Z", Key::Z),
            ("N1", Key::N1), ("N2", Key::N2), ("N3", Key::N3),
            ("N4", Key::N4), ("N5", Key::N5), ("N6", Key::N6),
            ("N7", Key::N7), ("N8", Key::N8), ("N9", Key::N9), ("N0", Key::N0),
            ("Up", Key::UP), ("Down", Key::DOWN),
            ("Left", Key::LEFT), ("Right", Key::RIGHT),
            ("Home", Key::HOME), ("End", Key::END),
            ("PageUp", Key::PAGEUP), ("PageDown", Key::PAGEDOWN),
            ("Insert", Key::INSERT), ("Delete", Key::DELETE),
            ("PrintScreen", Key::PRINT_SCREEN),
            ("ScrollLock", Key::SCROLL_LOCK), ("NumLock", Key::NUM_LOCK),
            ("SysRq", Key::SYS_RQ),
            ("`", Key::GRAVE), ("-", Key::MINUS), ("=", Key::EQUALS),
            ("[", Key::LBRACKET), ("]", Key::RBRACKET),
            ("\\", Key::BACKSLASH),
            (";", Key::SEMICOLON), ("'", Key::QUOTE),
            (",", Key::COMMA), (".", Key::PERIOD), ("/", Key::SLASH),
            ("MediaPrevTrack", Key::MEDIA_PREV_TRACK),
            ("MediaNextTrack", Key::MEDIA_NEXT_TRACK),
            ("MediaMute", Key::MEDIA_MUTE),
            ("MediaCalculator", Key::MEDIA_CALCULATOR),
            ("MediaPlayPause", Key::MEDIA_PLAY_PAUSE),
            ("MediaStop", Key::MEDIA_STOP),
            ("MediaVolumeDown", Key::MEDIA_VOLUME_DOWN),
            ("MediaVolumeUp", Key::MEDIA_VOLUME_UP),
            ("MediaWWWHome", Key::MEDIA_WWW_HOME),
            ("Power", Key::ACPI_POWER), ("Sleep", Key::ACPI_SLEEP),
            ("Wake", Key::ACPI_WAKE),
            ("Oem5", Key::OEM5),
            ("Numpad0", Key::NUMPAD_0), ("Numpad1", Key::NUMPAD_1),
            ("Numpad2", Key::NUMPAD_2), ("Numpad3", Key::NUMPAD_3),
            ("Numpad4", Key::NUMPAD_4), ("Numpad5", Key::NUMPAD_5),
            ("Numpad6", Key::NUMPAD_6), ("Numpad7", Key::NUMPAD_7),
            ("Numpad8", Key::NUMPAD_8), ("Numpad9", Key::NUMPAD_9),
            ("NumpadAdd", Key::NUMPAD_ADD), ("NumpadSubtract", Key::NUMPAD_SUBTRACT),
            ("NumpadMultiply", Key::NUMPAD_MULTIPLY), ("NumpadDivide", Key::NUMPAD_DIVIDE),
            ("NumpadEnter", Key::NUMPAD_ENTER), ("NumpadPeriod", Key::NUMPAD_PERIOD),
        ];
        pairs.iter().map(|(s, k)| (s.to_lowercase(), *k)).collect()
    })
}

fn parse_key(name: &str) -> Result<Key, String> {
    key_map()
        .get(&name.to_lowercase())
        .copied()
        .ok_or_else(|| format!("unknown key: '{}'", name))
}

// ═══════════════════════════════════════════════════════════════════
// Mode name → TriggerMode
// ═══════════════════════════════════════════════════════════════════

fn parse_mode(name: &str) -> Result<TriggerMode, String> {
    match name {
        "Once" => Ok(TriggerMode::Once),
        "Loop" => Ok(TriggerMode::Loop),
        "Toggle" => Ok(TriggerMode::Toggle),
        _ => Err(format!("unknown mode: '{}' (use Once / Loop / Toggle)", name)),
    }
}

// ═══════════════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════════════

/// Path to the config file (next to the exe).
fn config_path() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("failed to get executable path");
    path.set_file_name("config.toml");
    path
}

/// Default config content — written on first run.
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
"#;

/// Load and parse the config file. Generates a default if missing.
pub fn load() -> Result<Vec<Binding>, String> {
    let path = config_path();

    if !path.exists() {
        println!("  No config file detected.");
        println!("  Generating default config: {}", path.display());
        std::fs::write(&path, DEFAULT_CONFIG)
            .map_err(|e| format!("failed to write default config: {}", e))?;
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;

    let raw: RawConfig = toml::from_str(&content)
        .map_err(|e| format!("invalid config: {}", e))?;

    let mut bindings = Vec::new();
    for (i, b) in raw.bindings.iter().enumerate() {
        let key = parse_key(&b.key)
            .map_err(|e| format!("binding #{}: {}", i + 1, e))?;
        let mode = parse_mode(&b.mode)
            .map_err(|e| format!("binding #{}: {}", i + 1, e))?;
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
                i + 1, b.key.name()
            ));
        }
        if !funcs.insert(b.func.clone()) {
            return Err(format!(
                "binding #{}: function '{}' is already bound to another key",
                i + 1, b.func
            ));
        }
    }

    Ok(bindings)
}

// ═══════════════════════════════════════════════════════════════════
// Function factory
// ═══════════════════════════════════════════════════════════════════

/// Create a function instance by name.
/// New functions only need a branch added here.
pub fn create_function(name: &str, send_ctx: Arc<SendContext>) -> Result<Arc<dyn KeyFunction>, String> {
    match name {
        "连点器" => Ok(Arc::new(crate::functions::auto_clicker::连点器::new(send_ctx))),
        "快速拾取" => Ok(Arc::new(crate::functions::quick_pickup::快速拾取::new(send_ctx))),
        "鬼畜走路" => Ok(Arc::new(crate::functions::ghost_walk::鬼畜走路::new(send_ctx))),
        "火神跳喷" => Ok(Arc::new(crate::functions::mavuika_jump::火神跳喷::new(send_ctx))),
        "甘雨走A" => Ok(Arc::new(crate::functions::ganyu_aim_cancel::甘雨走A::new(send_ctx))),
        "双玛头" => Ok(Arc::new(crate::functions::mavuika_double_cancel::双玛头::new(send_ctx))),
        "坐标颜色" => Ok(Arc::new(crate::functions::mouse_color::坐标颜色::new())),
        _ => Err(format!("unknown function: '{}'", name)),
    }
}
