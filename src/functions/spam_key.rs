//! Spam Key — 按住绑定键时以可调节奏重复敲击任意配置的按键。
//! Loop 模式，按住循环。
//!
//! 动态参数三件套（GUI live 直写，下周期生效）：
//! - `key`：目标键（配置/面板均为键名，如 "F"、"Space" — 字符串参数，
//!   变更需重注册 — 面板下拉选择，dirty 后 Save/live-apply 走重注册路径）
//! - `interval_ms`：敲击周期（1.0–10000）
//! - `hold_ms`：按下时长（0–interval；0 = down+up 一次驱动级原子批）
//!
//! 与连点器的区别：连点器固定敲鼠标左键；本功能敲任意可配置键。
//! 两者共用同一节奏引擎语义（v1/v2 归并时代的双键位形态）。

use crate::engine::bindings::{KeyFunction, ParamKind, ParamSpec, ParamValues};
use crate::engine::event::InputEvent;
use crate::interception::SendContext;
use crate::key::{Key, ScanCode};
use crate::utils::delay;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// ═══════════════════════════════════════════════════════════════════
// 参数声明 — key 为字符串槽（Int 槽存 ScanCode 组合码）
// ═══════════════════════════════════════════════════════════════════

/// ScanCode(u16) + is_e0 打包成一个 i64：低 16 位 scan code，位 16 = E0。
/// 键参数经 Int 槽存取（原子），面板显示/写入经 `key_name`↔打包值转换。
#[inline]
fn pack_key(k: Key) -> i64 {
    (k.code.raw() as i64) | ((k.is_e0 as i64) << 16)
}

#[inline]
fn unpack_key(v: i64) -> Key {
    Key {
        code: ScanCode((v & 0xFFFF) as u16),
        is_e0: (v >> 16) & 1 != 0,
    }
}

/// 默认目标键：Key::F 的打包值（PS/2 set-1 make code 0x21，非 E0）—
/// const 上下文无法调用 `ScanCode::raw()`，字面量 + key.rs:116 对照。
const DEFAULT_KEY: i64 = 0x21;

/// 参数声明 — 索引即槽位（key=0，interval=1，hold=2）。
const SPAM_SPEC: &[ParamSpec] = &[
    // key 槽借 Int 类型原子存储（打包值）；面板对该槽特殊渲染为键名下拉
    ParamSpec {
        name: "key",
        kind: ParamKind::Int {
            default: DEFAULT_KEY,
            min: 0,
            max: 0x1FFFF,
        },
    },
    ParamSpec::float("interval_ms", 50.0, 1.0, 10_000.0, 0.5),
    ParamSpec::float("hold_ms", 0.0, 0.0, 10_000.0, 0.5),
];

// ═══════════════════════════════════════════════════════════════════
// 功能本体
// ═══════════════════════════════════════════════════════════════════

/// Spam Key 功能 — Loop 模式。
///
/// 每周期读参数快照：向 `key` 槽指向的键发送敲击。`hold_ms` 语义与连点器
/// 一致（0 = down+up 原子批；>0 = down → hold → up → 补足 interval）。
pub struct SpamKey {
    send_ctx: Arc<SendContext>,
    params: Arc<ParamValues>,
}

impl SpamKey {
    pub fn new(send_ctx: Arc<SendContext>) -> Self {
        Self {
            send_ctx,
            params: Arc::new(ParamValues::new(SPAM_SPEC)),
        }
    }
}

impl KeyFunction for SpamKey {
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        while !stop_requested.load(Ordering::Acquire) {
            // 键槽：AtomicI64 装打包值 — 直接读原子槽（经 Int 路径）
            let key = unpack_key(self.params.get_i64(0));
            let interval = self.params.get_f64(1);
            let hold = self.params.get_f64(2).clamp(0.0, interval);

            let down = InputEvent::Keyboard {
                code: key.code,
                state: key.down_state(),
            };
            let up = InputEvent::Keyboard {
                code: key.code,
                state: key.up_state(),
            };

            if hold <= 0.0 {
                // 原子批：down+up 一次 IOCTL（v1 语义）
                self.send_ctx.send_events(&[down, up]);
                delay::delay_ms_interruptible(interval, &stop_requested);
            } else {
                // 独立按下/松开节奏（v2 语义）
                self.send_ctx.send_event(&down);
                delay::delay_ms_interruptible(hold, &stop_requested);
                self.send_ctx.send_event(&up);
                let rest = interval - hold;
                if rest > 0.0 {
                    delay::delay_ms_interruptible(rest, &stop_requested);
                }
            }
        }
    }

    fn parameters(&self) -> &[ParamSpec] {
        SPAM_SPEC
    }

    fn param_store(&self) -> Option<Arc<ParamValues>> {
        Some(self.params.clone())
    }
}

// ═══════════════════════════════════════════════════════════════════
// 键槽辅助 — 打包值 ↔ 配置键名（GUI 下拉 / [bindings.params] 持久化）
// ═══════════════════════════════════════════════════════════════════

/// 打包值 → 配置键名（KEY_PAIRS 反查；未知码返回 "0x%04X" 兜底）。
pub fn key_slot_name(v: i64) -> String {
    let k = unpack_key(v);
    crate::config::key_to_config_name(k)
        .map(str::to_string)
        .unwrap_or_else(|| format!("0x{:04X}", k.code.raw()))
}

/// 配置键名 → 打包值（大小写不敏感；未知名返回 None）。
pub fn key_slot_value(name: &str) -> Option<i64> {
    crate::config::parse_key(name).ok().map(pack_key)
}

/// 键槽直写打包值（面板切换键时用 — 走 Int 槽 i64 路径）。
pub fn set_key_slot(store: &ParamValues, k: Key) {
    store.set_i64(0, pack_key(k));
}
