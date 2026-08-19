# GI-Utils

[English](README.md) | 中文

基于 [Interception](https://github.com/oblitum/Interception) 内核驱动协议的
Windows 游戏输入自动化工具，Rust 实现。前身是 C++/Visual Studio 项目，
现已完全重构为 Rust。

面向原神 / 崩坏：星穹铁道 / 鸣潮等游戏设计。

---

## 目录

- [功能特性](#功能特性)
- [工作原理](#工作原理)
- [环境要求](#环境要求)
- [安装驱动](#安装驱动)
- [构建](#构建)
- [配置](#配置)
- [功能一览](#功能一览)
- [触发模式](#触发模式)
- [项目结构](#项目结构)
- [路线图](#路线图)
- [许可证](#许可证)
- [使用注意](#使用注意)

---

## 功能特性

- **GUI 配置面板**（egui）：运行时增删改按键绑定，即时生效，保存到
  `config.toml`，无需重编译
- **托盘图标**：关闭窗口隐藏到托盘，双击唤回，菜单退出
- **崩溃自愈**：GUI 渲染上下文失效（睡眠唤醒后 wgl 丢失）时自动重建
  窗口，最多重试 3 次 — 期间输入引擎全程存活，不中断功能
- **三种触发模式**：`Once` / `Loop` / `Toggle`
- **10 个内置功能**（见[功能一览](#功能一览)）
- **高精度时序**：
  - TSC 忙等延时，启动时校准（微秒级精度）
  - 时间轴调度器：绝对时刻编排，MIDI 编辑器语义（播放中实时编辑、
    停止时挂起键自动补发释放）
- **构建期零 C 依赖**：用户层协议为原生 Rust 移植
  （`src/interception/protocol.rs`）— 不需要 `interception.lib`，运行时
  也不依赖任何 DLL，exe 直接以 `DeviceIoControl` 与内核驱动通信
- **CPU 核心分区**：GUI 渲染独占核心（低优先级），输入引擎与功能线程
  独占另一组核心（实时优先级），互不干扰输入时序

## 工作原理

```
物理键盘/鼠标
        │  （内核过滤器）
        ▼
Interception 驱动（内核态）              ← 需自行安装（见下）
        │  DeviceIoControl 协议          ← 我们的原生 Rust 移植
        ▼
Engine 线程（14,15 核 @ REALTIME）
  拦截 → 转发 → 分发
        │
        ├── GUI 面板线程（12,13 核 @ LOWEST）   ← 绑定编辑/日志/托盘
        └── 功能线程（14,15 核）                ← 各功能执行体
```

引擎以"全键盘过滤器"拦截输入，逐键转发回系统（不影响正常打字），
同时把绑定的热键分发给功能线程；鼠标事件不拦截、原样通过。

## 环境要求

- Windows 10 1903+ / Windows 11，x64
- **已安装 Interception 内核驱动**（不在本仓库内 — 见下）
- 程序需**以管理员身份运行**
- 构建需 Rust **nightly** 工具链（1.100 验证过）

## 安装驱动

内核驱动**不随本仓库分发**（由原作者按其自有许可分发）：

1. 从
   [oblitum/Interception](https://github.com/oblitum/Interception/releases)
   下载发布包
2. 管理员身份运行 `install-interception.exe`
3. 按提示重启

验证方法：GI-Utils 能正常启动即说明驱动在位（否则创建上下文会失败）。

## 构建

```bash
# 日常开发 — dev profile 极速增量编译
cargo build
# 输出: target/debug/gi-utils-gui.exe

# 部署发布 — release + build-std（std 以 panic_unwind + native 调优重编）
# + rust-lld 链接，全量约 1 分钟
cargo build --release --config .cargo/build-std.toml
# 输出: target/release/gi-utils-gui.exe（约 6.8 MB，自包含）
```

构建配置要点：

- `rustflags`：`-C target-cpu=native -Z threads=16`
- 链接器：`rust-lld`（lld-link）
- release：`opt-level=3`、fat LTO、`codegen-units=1`、`strip`、
  `panic=unwind`（unwind 是 GUI 崩溃自愈的基础）
- dev：自身代码 `opt-level=0`，依赖统一 `opt-level=2`（界面流畅）

测试：`cargo test` — 46 单测 + doctest，无需驱动。

## 配置

`config.toml` 位于 exe 同目录，首次运行自动生成。所有修改即时生效 —
GUI 面板是主要编辑入口，文件本身是纯 TOML：

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

# 可选自定义托盘图标（.ico）；留空使用程序生成的兜底图标
[gui]
icon_path = ""
```

功能名为中文（与游戏内术语一致）；按键名支持常量表中 90+ 键
（F1–F24、字母、小键盘、媒体键等）。

## 功能一览

| 配置名 | 默认按键 | 模式 | 行为 |
|---|---|---|---|
| 停止退出 | F12 | Once | 置位引擎停止标志 → 干净退出 |
| 连点器v1 | F13 | Loop | 按住时以 10ms 周期连点鼠标左键 |
| 连点器v2 | — | Loop | 连点变体：按下 8ms / 松开 8ms（独立调参） |
| 快速拾取 | F14 | Loop | 循环 tap F + 滚轮下拉（掉落物收集） |
| 鬼畜走路 | F15 | Loop | WASD 滚动短按（50ms 间隔、1ms 按住） |
| 火神跳喷 | F16 | Loop | 初始跳跃后循环空格连跳 |
| 甘雨走A | F17 | Once | 射箭后摇取消：左/右击 + R 键 |
| 双玛头 | F18 | Loop | 玛薇卡双坠编排（左键长按 + 右键点按 + S） |
| 坐标颜色 | F19 | Loop | 持续输出光标坐标 + 像素 RGB |
| 优化游戏 | NumpadAdd | Once（奇偶切换） | 奇次：提升游戏优先级 + 切前台；偶次：恢复 |

## 触发模式

| 模式 | 按下 | 松开 |
|---|---|---|
| `Once` | 启动，运行至结束 | — |
| `Loop` | 启动循环 | 停止 |
| `Toggle` | 启动 / 停止 | — |

## 项目结构

```
src/
├── bin/gi-utils-gui/     GUI 二进制（面板、托盘、窗口操作）
├── config.rs             TOML 配置 + 函数工厂
├── key.rs                ScanCode 新类型 + Key（扫描码+E0）+ 常量表
├── interception/
│   ├── protocol.rs       Interception 用户层协议的原生 Rust 移植
│   │                     （LGPL 3.0 — 见许可证）
│   └── context.rs        类型化收发上下文
├── engine/
│   ├── mod.rs            Engine 事件循环
│   ├── event.rs          InputEvent + EventSequence + HeldTracker
│   ├── bindings.rs       KeyFunction trait + 绑定注册表
│   └── timeline.rs       绝对时刻时间轴调度器
├── utils/                delay（TSC）/ beep / affinity / screen / 日志桥
└── functions/            每功能一文件
```

## 路线图

- 组合键注册 — 修饰键+功能键绑定（`Ctrl+F13` 等）
- 甘雨加特林、克洛琳德（像素触发）、添加好友 / 申请加入（绝对坐标鼠标）、
  2048 系列

## 许可证

- **MIT** — 项目整体（见 `LICENSE`）
- **LGPL 3.0** — `src/interception/protocol.rs`，oblitum/Interception
  的 `library/interception.c` 之衍生修改版本（Rust 移植）。文本见
  `LICENSE-LGPL.txt` 与 `LICENSE-GPL.txt`；原始版权归
  oblitum/Interception 作者
- Interception **内核驱动**不属于本仓库，仍归原作者按其许可分发

## 使用注意

本工具在 OS 输入栈之下拦截系统级键鼠输入。仅限在自己的机器上、
个人用途使用，并遵守所玩游戏的用户协议。
