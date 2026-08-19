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
  runtime — changes apply immediately, saved to `config.toml`
- **Tray icon** with hide-to-tray behavior; double-click to restore
- **Crash self-healing**: the GUI survives render-context loss
  (e.g. after sleep/wake) by rebuilding the app up to 3 times; the input
  engine keeps running throughout
- **Three trigger modes**: `Once` / `Loop` / `Toggle`
- **10 built-in functions** (see [Function Reference](#function-reference))
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
# output: target/release/gi-utils-gui.exe (~6.8 MB, self-contained)
```

Build configuration highlights:

- `rustflags`: `-C target-cpu=native -Z threads=16`
- linker: `rust-lld` (lld-link)
- release: `opt-level=3`, fat LTO, `codegen-units=1`, `strip`, `panic=unwind`
  (unwind powers the GUI crash self-healing)
- dev: `opt-level=0` for own code, deps at `opt-level=2` (smooth UI)

Tests: `cargo test` — 46 unit tests + doctests, no driver required.

## Configuration

`config.toml` lives next to the exe and is auto-generated on first run.
Every change is hot — the GUI panel is the primary editor, but the file is
plain TOML:

```toml
[[bindings]]
key = "F12"
func = "停止退出"
mode = "Once"

[[bindings]]
key = "F13"
func = "连点器v1"
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

# Optional custom tray icon (.ico); empty = generated fallback
[gui]
icon_path = ""
```

Function names are Chinese (matching the in-game terminology); key names
accept any key in the 90+ constants table (F1–F24, letters, numpad, media
keys…).

## Function Reference

| Config name | Default key | Mode | Behavior |
|---|---|---|---|
| 停止退出 | F12 | Once | Sets the engine stop flag → clean shutdown |
| 连点器v1 | F13 | Loop | Auto-click LMB at 10 ms cycles while held |
| 连点器v2 | — | Loop | Auto-click variant, 8 ms down / 8 ms up (independently tunable) |
| 快速拾取 | F14 | Loop | Taps F + scrolls wheel down repeatedly (loot pickup) |
| 鬼畜走路 | F15 | Loop | WASD rolling taps (50 ms interval, 1 ms hold) |
| 火神跳喷 | F16 | Loop | Initial jump, then repeating space taps |
| 甘雨走A | F17 | Once | Aim-cancel combo: L/R clicks + R key |
| 双玛头 | F18 | Loop | Mavuika double-cancel choreography (L hold + R clicks + S) |
| 坐标颜色 | F19 | Loop | Prints cursor position + pixel RGB continuously |
| 优化游戏 | NumpadAdd | Once (toggle) | Odd press: raise game priority + foreground; even press: restore |

## Trigger Modes

| Mode | Key-down | Key-up |
|---|---|---|
| `Once` | spawn, run to completion | — |
| `Loop` | spawn loop | stop |
| `Toggle` | start / stop | — |

## Project Layout

```
src/
├── bin/gi-utils-gui/     GUI binary (panel, tray, window ops)
├── config.rs             TOML config + function factory
├── key.rs                ScanCode newtype + Key (scan code + E0) + constants
├── interception/
│   ├── protocol.rs       Native Rust port of the Interception user-mode
│   │                     protocol (LGPL 3.0 — see License)
│   └── context.rs        Typed receive/send contexts
├── engine/
│   ├── mod.rs            Engine event loop
│   ├── event.rs          InputEvent + EventSequence + HeldTracker
│   ├── bindings.rs       KeyFunction trait + binding registry
│   └── timeline.rs       Absolute-time timeline scheduler
├── utils/                delay (TSC), beep, affinity, screen, log collector
└── functions/            One file per game function
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
