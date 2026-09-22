# GI-Utils

[中文文档](README.zh-CN.md) | English

Game input automation tool for Windows, built in Rust on top of the
[Interception](https://github.com/oblitum/Interception) kernel driver
protocol. Originally a C++/Visual Studio project, fully rewritten in Rust.

Designed for games such as Genshin Impact, Honkai: Star Rail, and
Wuthering Waves.

---

## Table of Contents

- [Features](#features)
- [How It Works](#how-it-works)
- [Requirements](#requirements)
- [Installing the Driver](#installing-the-driver)
- [Build](#build)
- [Running](#running)
- [Configuration](#configuration)
- [Function Reference](#function-reference)
- [Trigger Modes](#trigger-modes)
- [Project Layout](#project-layout)
- [Roadmap](#roadmap)
- [License](#license)
- [Notice](#notice)

---

## Features

- **GUI configuration panel** (egui): add/remove/rebind functions at
  runtime — changes apply immediately, saved to the active profile
- **Profiles**: several TOML profiles side by side (`profiles/` next to the
  exe) — switch live from the panel's Profile bar or the tray menu; the last
  used one is remembered across runs
- **Tray icon** with hide-to-tray behavior; double-click to restore.
  `--silent` starts tray-only for a logon autostart entry (see
  [Running](#running))
- **Sleep/wake proof rendering**: wgpu + Vulkan backend — the surface is
  checked every frame and rebuilt when the driver drops it, so the sleep/wake
  context loss that used to crash the old OpenGL (glow/WGL) backend no longer
  occurs
- **Dynamic parameters**: functions declare typed parameters (interval, hold,
  key…) editable live from the ⚙ popup — no rebuild, no thread restart
- **Three trigger modes**: `Once` / `Loop` / `Toggle`
- **13 built-in functions** (see [Function Reference](#function-reference))
- **High-precision timing**:
  - TSC busy-wait delays calibrated at startup (µs-level accuracy)
  - Timeline scheduler: absolute-time orchestration with MIDI-editor
    semantics (live-edit, pending-key cleanup on stop)
- **Zero C dependencies at build time**: the user-mode protocol layer is a
  native Rust port (`src/interception/protocol.rs`) — no `interception.lib`,
  no DLL. The exe talks to the kernel driver via `DeviceIoControl` directly
- CPU core partitioning: GUI renders on dedicated cores at low priority,
  the input engine and function threads run on their own cores in realtime

## How It Works

```
Physical keyboard/mouse
        │  (kernel filter)
        ▼
Interception driver (kernel)          ← installed separately (see below)
        │  DeviceIoControl protocol   ← our native Rust port
        ▼
Engine thread (14,15 @ REALTIME)
  intercept → forward → dispatch
        │
        ├── GUI panel thread (12,13 @ LOWEST)   ← bindings, log, tray
        └── function threads (14,15)            ← once/loop/toggle tasks
```

The engine intercepts keyboard input with an all-key filter, forwards every
stroke back to the system, and dispatches bound hotkeys to function
threads. Mouse input passes through untouched.

## Requirements

- Windows 10 1903+ / Windows 11, x64
- **Interception kernel driver installed** (not bundled — see below)
- Run the application **as administrator**
- Rust **nightly** toolchain for building (tested on 1.100)

## Installing the Driver

The driver is **not included** in this repository (it is distributed by its
authors under their own license).

1. Download a release from
   [oblitum/Interception](https://github.com/oblitum/Interception/releases)
2. Run `install-interception.exe` as administrator
3. Reboot if prompted

Verification: launching GI-Utils without errors means the driver is
present (context creation would otherwise fail at startup).

## Build

```bash
# daily development — fast incremental dev profile
cargo build
# output: target/debug/gi-utils-gui.exe

# deployment — release + build-std (std rebuilt with panic_unwind + native
# tuning) + rust-lld linker, full rebuild ~1 min
cargo build --release --config .cargo/build-std.toml
# output: target/release/gi-utils-gui.exe (~13 MB, self-contained)
```

Build configuration highlights:

- `rustflags`: `-C target-cpu=native -Z threads=16`
- linker: `rust-lld` (lld-link)
- release: `opt-level=3`, fat LTO, `codegen-units=1`, `strip`, `panic=unwind`
  (keeps `Drop` guards and the crash-log panic hook working)
- dev: `opt-level=0` for own code, deps at `opt-level=2` (smooth UI)

Tests: `cargo test` — 66 unit tests + 2 doctests, no driver required.

## Running

Run **as administrator** (the Interception driver interface requires it). The
first launch generates `profiles/默认.toml` next to the exe; switch, create and
edit profiles from the **Profile** bar above the binding table — Save writes the
active profile. Closing the window hides to the tray; F12 exits.

Command-line flags:

| Flag | Effect |
|---|---|
| `--silent` | Silent start — no configuration window, tray icon only (the engine runs as usual). Meant for a logon autostart entry. Launching a second copy with `--silent` leaves an already-running instance untouched |

## Configuration

Profiles live in `profiles/` next to the exe, one plain TOML file per profile
(`profiles/默认.toml` is generated on first run). Every change is hot — the GUI
panel is the primary editor:

```toml
[[bindings]]
key = "F12"
func = "停止退出"
mode = "Once"

[[bindings]]
key = "F13"
func = "连点器"
mode = "Loop"

# … F14–F19: 快速拾取 / 鬼畜走路 / 火神跳喷 / 甘雨走A / 双玛头 / 坐标颜色

[[bindings]]
key = "NumpadAdd"
func = "优化游戏"
mode = "Once"

[[bindings]]
key = "F20"
func = "线程采样"
mode = "Once"

# Function parameter template — the baseline every 连点器 row inherits.
# A binding row only stores its *diff* against this template, so editing the
# template moves every row that has not overridden that slot.
[params."连点器"]
interval_ms = 10.0
hold_ms = 0.0

# Optional custom tray icon (.ico) + CJK font; empty = automatic fallback
[gui]
icon_path = ""
font_path = ""
```

Parameters are per-binding: bind the same function to two keys (e.g. two
`SpamKey` rows) and each row gets its own `[bindings.params]` diff — so they
can strike different keys at different rhythms.

Function names are mostly Chinese (plus English `SpamKey`) (matching the in-game terminology); key names
accept any key in the 90+ constants table (F1–F24, letters, numpad, media
keys…).

## Function Reference

| Config name | Default key | Mode | Behavior |
|---|---|---|---|
| 停止退出 | F12 | Once | Sets the engine stop flag → clean shutdown |
| 连点器 | F13 | Loop | Auto-click LMB — dynamic params (interval_ms / hold_ms, ⚙ panel live-edit) |
| 快速拾取 | F14 | Loop | Taps F + scrolls wheel down repeatedly (loot pickup) |
| 鬼畜走路 | F15 | Loop | WASD rolling taps (50 ms interval, 1 ms hold) |
| 火神跳喷 | F16 | Loop | Initial jump, then repeating space taps |
| 甘雨走A | F17 | Once | Aim-cancel combo: L/R clicks + R key |
| 双玛头 | F18 | Loop | Mavuika double-cancel choreography (L hold + R clicks + S) |
| 坐标颜色 | F19 | Loop | Prints cursor position + pixel RGB continuously |
| 优化游戏 | NumpadAdd | Once (toggle) | Advanced: game affinity + OTHER isolation + priority + hot-thread pinning |
| 优化游戏标准 | — | Once (toggle) | Standard: OTHER isolation + priority (no game affinity / pinning) |
| 优化游戏简易 | — | Once (toggle) | Minimal: priority + foreground only (no affinity changes) |
| 线程采样 | F20 | Once | Thread-profile sampler → `thread_sample.txt` (analysis tool, no injection) |
| SpamKey | — | Loop | Repeat-strike any configured key (key / interval_ms / hold_ms dynamic params) |

## Trigger Modes

| Mode | Key-down | Key-up |
|---|---|---|
| `Once` | spawn, run to completion | — |
| `Loop` | spawn loop | stop |
| `Toggle` | start / stop | — |

## Project Layout

```
src/
├── bin/gi-utils-gui/     GUI binary (panel, tray, tray icon, window ops)
├── profile.rs            Config & profile system (profiles/ dir, TOML,
│                         function factory, param templates)
├── key.rs                ScanCode newtype + Key (scan code + E0) + constants
├── interception/
│   ├── protocol.rs       Native Rust port of the Interception user-mode
│   │                     protocol (LGPL 3.0 — see License)
│   └── context.rs        Typed receive/send contexts
├── engine/
│   ├── mod.rs            Engine event loop
│   ├── event.rs          InputEvent + EventSequence + HeldTracker
│   ├── bindings.rs       KeyFunction trait + binding registry + params
│   └── timeline.rs       Absolute-time timeline scheduler
├── utils/                delay (TSC), beep, affinity, screen, log collector,
│                         thread_info + thread_pin (thread sampling/pinning)
└── functions/            One file per function (incl. thread_sampler,
                          spam_key, optimize_game's three modes)
```

## Roadmap

- 组合键注册 — modifier + key bindings (`Ctrl+F13` etc.)
- 甘雨加特林, 克洛琳德 (pixel-triggered), 添加好友 / 申请加入 (absolute
  mouse positioning), 2048 series

## License

- **MIT** — the project as a whole (see `LICENSE`)
- **LGPL 3.0** — `src/interception/protocol.rs`, a modified version
  (Rust port) of `library/interception.c` from oblitum/Interception.
  See `LICENSE-LGPL.txt` and `LICENSE-GPL.txt`; original copyright:
  the oblitum/Interception authors.
- The Interception **kernel driver** is not part of this repository and
  remains under its authors' license.

## Notice

This tool intercepts system-wide keyboard/mouse input below the OS input
stack. Use it only on your own machine, for personal use, and in
accordance with the terms of service of the games you play.
