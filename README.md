# GI-Utils

Game input automation tool for Windows, built in Rust on the
[Interception](https://github.com/oblitum/Interception) kernel driver protocol.

游戏输入自动化工具（原神 / 崩铁 / 鸣潮等），基于 Interception 内核驱动协议，
Rust 实现。

## Features

- GUI configuration panel (egui): bind any function to any key at runtime,
  live-apply, no rebuild needed (`config.toml` driven)
- Trigger modes: `Once` / `Loop` / `Toggle` (Logitech G-Hub style)
- Built-in functions: auto-clicker v1/v2, quick pickup, ghost walk,
  Mavuika jump, Ganyu aim-cancel, Mavuika double-cancel, pixel color
  reader, game optimizer (priority / foreground)
- High-precision timing: TSC busy-wait delays (µs-level), timeline
  scheduler with absolute-time orchestration (MIDI-editor semantics)
- Tray icon with hide-to-tray, crash self-healing GUI
- **Zero C dependencies at build time** — the user-mode protocol layer is
  a native Rust port (`src/interception/protocol.rs`); no `interception.lib`
  or DLL is required

## Requirements

- Windows 10/11 x64
- **The Interception kernel driver must be installed** (install it from
  [oblitum/Interception](https://github.com/oblitum/Interception) —
  it is **not** included in this repository and is distributed under its
  own license by its authors). Our user-mode code is wire-compatible with
  the original driver (and with API-compatible alternatives).
- Run as **administrator**

## Build

```bash
# daily development (fast dev profile)
cargo build

# deployment build (release + build-std, rust-lld)
cargo build --release --config .cargo/build-std.toml
# output: target/release/gi-utils-gui.exe
```

Requires Rust nightly (tested on 1.100).

## Configuration

`config.toml` is auto-generated next to the exe on first run:

```toml
[[bindings]]
key = "F12"
func = "停止退出"
mode = "Once"

[[bindings]]
key = "F13"
func = "连点器v1"
mode = "Loop"
```

The GUI panel edits this file; all changes apply immediately.

## License

- **MIT** for the project (see `LICENSE`)
- **LGPL 3.0** for `src/interception/protocol.rs` — a modified version
  (Rust port) of `library/interception.c` from oblitum/Interception
  (see `LICENSE-LGPL.txt` and `LICENSE-GPL.txt`; original copyright:
  the oblitum/Interception authors)

## Notice

Input interception operates below the OS input stack and can observe and
inject system-wide keyboard/mouse input. Use it only on your own machine
and in accordance with the terms of service of the games you play.
