# GI-Utils Rust 重构技术方案

> **状态（2026-08）**：重构已完成并发布 v1.0.0。本文是重构前的选型方案，
> 部分决策与最终实现不同，以 [CLAUDE.md](CLAUDE.md) 为准：
> - 未用 `interception-sys` crate — 自行编写 FFI 绑定（`interception/` 模块）
> - 未用 `CancellationToken` — 采用 `stop_requested: Arc<AtomicBool>`（更轻，无 async 依赖）
> - 已升级 edition 2024（2026-08，原计划保持 2021）
> - `EventSequence` 链式 API 已按 2.3 实现；键盘/鼠标事件已按 enum 区分
> - 时间轴调度器（`engine/timeline.rs`：Timeline/RollingKeys）已按 NEXT_STEPS §3 实现（重构后新增范式，本文未覆盖）

## 1. 核心依赖选型

### 1.1 输入拦截/注入 — 最关键的选择

原项目依赖 Interception 驱动（内核级输入拦截 + 注入）。Rust 生态有三种路线：

| 路线 | crate | 原理 | 优势 | 劣势 |
|---|---|---|---|---|
| **A: 沿用 Interception** | `interception-sys` + 手动安全包装 | FFI 调用 interception.dll | 内核级，游戏兼容性最好；已验证可行 | 需装驱动；`unsafe` 代码 |
| **B: SendInput API** | `enigo` 或 `winput` | Win32 `SendInput` API | 纯 Rust，无需驱动，跨平台 | 部分游戏会忽略 `SendInput` 注入的输入 |
| **C: Hook 方案** | `inputbot` / `rdev` | `SetWindowsHookEx` | 能监听也能模拟 | 用户态 hook，游戏反作弊容易拦截 |

**结论：选 A（沿用 Interception）**。输入注入类游戏助手，内核级驱动是不可替代的——SendInput 和 Hook 方案在大量游戏中会被反作弊屏蔽。Interception 在驱动层工作，兼容性最好，原项目已验证。

Rust 侧方案：

```
interception-sys (社区已有 FFI binding)
    ↓ 提供
unsafe extern "C" 函数声明
    ↓ 包装为
gi-interception (我们自己写安全抽象层)
    ├── InterceptionContext  (RAII, Send+Sync)
    ├── send_keyboard()
    ├── send_mouse()
    └── receive_stroke()
```

社区已有 `interception-sys` crate（v0.1.3），但版本较老且维护状态不明。更好的做法是**自行编写 FFI 绑定**（约 50 行代码），完全可控。

### 1.2 其他依赖对照表

| C++ 原依赖 | 用途 | Rust 替代 | 说明 |
|---|---|---|---|
| `interception.h` + `interception.lib` | 内核输入注入 | `interception-sys` 或自行 FFI | 见上文 |
| `fmt` | 字符串格式化 | `std::fmt` / `format!()` | Rust 标准库自带，零依赖 |
| `spdlog` | 日志 | `tracing` + `tracing-subscriber` | 更现代的结构化日志，支持 span 追踪 |
| `<Windows.h>` / `<tlhelp32.h>` | Win32 API | `windows` (windows-rs 官方) | 微软官方维护，类型安全 |
| `GetPixel` (GDI) | 屏幕像素取色 | `windows` crate 的 GDI 模块 | 或升级为 DXGI Desktop Duplication（更快） |
| `__rdtsc()` / `_mm_pause()` | 高精度延迟 | `core::arch::x86_64::_rdtsc()` | Rust 标准库内置 intrinsic |
| `Beep()` | 蜂鸣提示 | `windows` crate 调用 `Beep` | 同 API，通过 windows-rs 调用 |

---

## 2. 架构设计

### 2.1 模块划分

```
src/
├── main.rs                    # 入口：初始化 → 注册功能 → 启动监听
├── interception/              # Interception FFI + 安全包装
│   ├── ffi.rs                 # extern "C" 声明
│   ├── context.rs             # InterceptionContext RAII
│   ├── stroke.rs              # Stroke 类型安全封装
│   └── mod.rs
├── engine/                    # 核心引擎（对应当前 main.cpp 的后半部分）
│   ├── monitor.rs             # KeyMonitor → 主事件循环
│   ├── manager.rs             # KeyboardManager → 按键→功能映射
│   ├── event.rs               # Event / EventSequence
│   └── mod.rs
├── functions/                 # 每个游戏功能独立文件
│   ├── mod.rs                 # KeyFunction trait 定义
│   ├── auto_clicker.rs        # 连点器
│   ├── quick_pickup.rs        # 快速拾取
│   ├── fire_jump.rs           # 火神跳飞
│   ├── ganyu_walk_a.rs        # 甘雨走A
│   ├── double_ma.rs           # 双玛头
│   ├── ganyu_gatling.rs       # 甘雨加特林
│   ├── ghost_walk.rs          # 鬼畜走路
│   ├── clorinde.rs            # 克洛琳德 (像素检测)
│   ├── add_friend.rs          # 添加好友
│   ├── apply_join.rs          # 申请加入
│   ├── mouse_color.rs         # 坐标颜色
│   ├── optimize_game.rs       # 优化游戏
│   └── game_2048.rs           # 2048 小游戏
├── utils/
│   ├── delay.rs               # TSC 高精度延迟
│   ├── affinity.rs            # CPU 亲和性管理
│   ├── screen.rs              # 屏幕像素取色
│   ├── beep.rs                # 蜂鸣提示
│   └── mod.rs
├── scan_code.rs               # 扫描码枚举 (正确的值)
└── config.rs                  # 编译期/运行时配置
```

对比现状：**1092 行单文件** → **~20 个模块文件**，每个功能 50-150 行。

### 2.2 KeyFunction trait 设计

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;

pub trait KeyFunction: Send + Sync {
    fn execute(&self, cancel: CancellationToken);
    fn on_activate(&self) {}     // 默认空实现
    fn on_deactivate(&self) {}   // 默认空实现
}

// 带取消的循环模板 —— 替代 C++ 的 while(active_)
fn run_loop<F>(cancel: CancellationToken, mut body: F)
where
    F: FnMut(),
{
    while !cancel.is_cancelled() {
        body();
    }
}
```

关键改进：
- `CancellationToken` 替代裸 `atomic<bool>` 轮询 → 优雅关闭，不泄漏线程
- `Send + Sync` trait bound 编译期保证线程安全
- 每个功能的 `execute` 不自己管理线程，由 `KeyboardManager` 统一调度

### 2.3 EventSequence 的 Rust 化

```rust
#[derive(Clone)]
pub enum InputEvent {
    KeyPress { code: u16, state: KeyState, delay_ms: f64 },
    MouseMove { dx: i32, dy: i32, delay_ms: f64 },
    MouseButton { button: MouseButton, state: MouseState, delay_ms: f64 },
    MouseWheel { delta: i16, delay_ms: f64 },
    MouseMoveAbsolute { x: u16, y: u16, delay_ms: f64 },
}

pub struct EventSequence {
    events: Vec<InputEvent>,
}
```

对比 C++ 的 `Event` + tag `is_keyboard` + `reinterpret_cast` → Rust enum 自带类型标签，不会出现错误 match。

### 2.4 KeyboardManager 对比

```rust
// Rust: 编译期保证线程安全
pub struct KeyboardManager {
    functions: HashMap<KeyId, Arc<dyn KeyFunction>>,
    key_states: HashMap<KeyId, AtomicBool>,
    cancel_tokens: HashMap<KeyId, CancellationToken>,
    tokio_runtime: Handle,  // 或 std::thread
    lock: RwLock<()>,
}
```

---

## 3. 关键技术映射

### 3.1 高精度延迟

```rust
// C++ 原版:
// UINT64 target = __rdtsc() + static_cast<UINT64>(ms * TSC_FREQ / 1000.0);
// while (__rdtsc() < target) { _mm_pause(); }

// Rust 等价:
use core::arch::x86_64::{_rdtsc, _mm_pause};

pub fn delay_ms(ms: f64) {
    let target = unsafe { _rdtsc() } + (ms * TSC_FREQ / 1000.0) as u64;
    while unsafe { _rdtsc() } < target {
        unsafe { _mm_pause() };
    }
}
```

或直接用 `minstant` crate（社区 TSC Instant 实现，~10ns 开销）获得更友好的 API。

### 3.2 屏幕像素取色

```rust
// 方案A: GDI GetPixel（原样移植，简单但慢）
use windows::Win32::Graphics::Gdi::{GetPixel, GetDC, ReleaseDC};

// 方案B: DXGI Desktop Duplication（更推荐，快 100 倍以上）
// 用 dxgi-capture-rs 或 windows-rs 的 DXGI 模块
```

当前 `克洛琳德` 功能只需要读一个像素，GDI GetPixel 的性能足够（单像素 ~0.01ms），可以直接用方案 A。

### 3.3 CPU 亲和性

```rust
use windows::Win32::System::Threading::{
    SetProcessAffinityMask, SetPriorityClass,
    OpenProcess, PROCESS_SET_INFORMATION,
    REALTIME_PRIORITY_CLASS,
};
use windows::Win32::System::ToolHelp::{
    CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32,
};
```

`windows-rs` 官方 crate 覆盖了所有需要的 Win32 API，且类型安全（不像 C++ 里裸 `HANDLE` 容易忘记关闭）。

### 3.4 日志

```toml
[dependencies]
tracing = "0.1"
tracing-subscriber = "0.3"
```

```rust
use tracing::{info, error, warn, debug, instrument};

// Release 版本只输出 error/warn，Debug 版本全输出
tracing_subscriber::fmt()
    .with_max_level(if cfg!(debug_assertions) { Level::DEBUG } else { Level::WARN })
    .init();
```

---

## 4. 风险点与缓解

| 风险 | 缓解方案 |
|---|---|
| Interception FFI 的 unsafe 代码 | 收拢到 `interception/` 模块，上层 100% safe |
| `windows-rs` 学习曲线 | 只需要调用 5-6 个 API，官方文档够用 |
| TSC 频率校准需要硬件配合 | 先在 9800X3D 上硬编码常数，后续加自动校准 |
| `CancellationToken` 引入 async 依赖 | 用 `tokio_util::sync::CancellationToken`（不依赖 tokio runtime），或直接用 `std::sync::mpsc` channel |
| C++ `detach` 线程变成有生命周期 | 每个功能在 `KeyboardManager` 中持有 `JoinHandle`，释放时先 cancel 再 join |

---

## 5. 开发步骤建议

```
Phase 1: 基础设施
  □ interception FFI 绑定 + 安全包装
  □ utils (delay, beep)
  □ scan_code 枚举
  □ EventSequence + KeyFunction trait

Phase 2: 核心引擎
  □ SendContext
  □ KeyboardManager
  □ KeyMonitor 主循环
  □ 用 1 个简单功能（连点器）端到端验证

Phase 3: 逐个移植功能
  □ 连点器 → 快速拾取 → 火神跳飞 → 甘雨走A → 甘雨加特林
  □ 鬼畜走路 → 双玛头
  □ 克洛琳德（像素检测）→ 2048 → 添加好友 → 申请加入
  □ 优化游戏（CPU 亲和性）

Phase 4: 打磨
  □ Release 构建优化（LTO, strip）
  □ 错误处理完善
  □ 配置化（外部文件定义按键映射）
```

---

## 6. Cargo.toml 预期

```toml
[package]
name = "gi-utils"
version = "0.1.0"
edition = "2024"

[dependencies]
windows = { version = "0.58", features = [
    "Win32_System_Threading",
    "Win32_System_ToolHelp",
    "Win32_Graphics_Gdi",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Media",
]}                              # Win32 API（按需开 feature）
tracing = "0.1"                  # 日志框架
tracing-subscriber = "0.3"       # 日志输出
# interception-sys = "0.1"       # 或者自行 FFI，不依赖第三方

[profile.release]
opt-level = "z"                  # 优化体积
lto = "fat"                      # 全链接时优化
strip = true                     # 去除符号
codegen-units = 1                # 单编译单元（更好优化）
```
