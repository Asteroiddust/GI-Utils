# GI-Utils — Rust 游戏输入自动化工具 v1.0.0

> **Review cleared**: 48/48 issues resolved (0C 0H 0M 0L 0S)
> **Build**: O3 + LTO fat + panic=abort + target-cpu=native

## 项目概述

基于 [Interception](https://github.com/oblitum/Interception) 内核驱动的游戏辅助工具，从 C++ (Visual Studio) 重构为 Rust。

原 C++ 项目: `E:\Projects\fmttest`，服务于原神/崩铁/鸣潮等游戏。

## 技术栈

- **Rust 1.96** (edition 2021)
- **Interception 驱动** — 内核级键盘/鼠标输入拦截与注入
- **windows-rs 0.62** — Win32 API (GDI、Threading、ToolHelp)
- **toml + serde** — TOML 配置文件解析
- **tracing** — debug 构建的结构化日志

## 项目结构

```
src/
├── main.rs                    # 入口：加载 config → 校准 TSC → 注册 → Engine.run()
├── config.rs                  # TOML 配置解析 + 函数工厂
├── build.rs                   # 链接 interception.lib
├── key.rs                     # Key (ScanCode + is_e0) + 90+ 常量
├── scan_code.rs               # ScanCode(u16) FFI 新类型

├── interception/              # Interception FFI 绑定
│   ├── ffi.rs                 #   extern "C" 声明、常量、repr(C) struct
│   ├── context.rs             #   InterceptionContext (recv) + SendContext (send)
│   └── strokes.rs             #   read/write unaligned 安全转换

├── engine/
│   ├── mod.rs                 #   Engine — 事件循环 + 按键显示
│   ├── event.rs               #   InputEvent + EventSequence 链式 API
│   ├── function.rs            #   KeyFunction trait (1 method)
│   └── bindings.rs            #   KeyBindings + TriggerMode + ActiveGuard

├── utils/
│   ├── delay.rs               #   TSC delay + interruptible + 校准
│   ├── beep.rs                #   蜂鸣 (同步 beep + 异步 beep_async)
│   ├── affinity.rs            #   CPU 亲和性 + 进程迭代
│   └── screen.rs              #   PixelReader (cached DC) + 像素取色

└── functions/
    ├── stop.rs                #   停止退出 (F12, Once)
    ├── auto_clicker.rs        #   连点器 (F13, Loop)
    ├── quick_pickup.rs        #   快速拾取 (F14, Loop)
    ├── ghost_walk.rs          #   鬼畜走路 (F15, Loop)
    ├── mavuika_jump.rs        #   火神跳喷 (F16, Loop)
    ├── ganyu_aim_cancel.rs    #   甘雨走A (F17, Once)
    ├── mavuika_double_cancel.rs # 双玛头 (F18, Loop)
    └── mouse_color.rs         #   坐标颜色 (F19, Loop)
```

## 架构

```
Engine (主循环, blocking)
├── InterceptionContext (recv) + SendContext (send, Arc 共享)
├── 事件循环: wait → receive → forward → dispatch
│   └── KeyBindings.process_key_down/up(key)
└── KeyBindings
      ├── HashMap<Key, Entry>    (绑定注册表)
      ├── HashMap<Key, bool>     (去抖表)
      ├── Entry.active: Arc<AtomicBool>        (运行状态)
      ├── Entry.stop_requested: Arc<AtomicBool> (停止信号)
      ├── Entry.handle: Option<JoinHandle>     (线程句柄)
      └── TriggerMode: Once / Loop / Toggle
```

## 核心设计决策

| 决策 | 理由 |
|------|------|
| **Key = ScanCode + is_e0** | 单一类型消除 PS/2 值冲突，E0 自动注入 state |
| **SendContext 独立类型** | 发送/接收分离，编译器强制禁止并发接收 |
| **stop_requested 正向语义** | `true`=停止，全项目统一，无双重否定 |
| **TOML 动态配置** | `config.toml` 驱动热键映射，无需重编译 |
| **ActiveGuard Drop 防护** | 线程 panic 时自动清理 active 标志 |
| **Mutex 外 join** | stop 不阻塞主事件循环 |
| **delay_ms_interruptible** | 100μs 检查间隔，Loop/Toggle 即时响应 |
| **EventSequence 链式 API** | `seq.tap(K).sleep(50).wheel(DOWN)` |
| **KeyFunction 只有 1 个方法** | `execute(&self, stop_requested: Arc<AtomicBool>)` |
| **printf 作 release 输出** | 避免 Windows stderr 缓冲问题 |

## 构建

```bash
cargo build --release
# 输出: target/release/gi-utils.exe (~200KB)
```

release profile: `opt-level=3, lto=fat, panic=abort, strip=true`

## 运行

**必须以管理员身份运行**。首次运行自动生成 `config.toml`。

```toml
[[bindings]]
key = "F12"
func = "停止退出"
mode = "Once"

[[bindings]]
key = "F13"
func = "连点器"
mode = "Loop"
```

## 移植进度

| 功能 | 状态 |
|------|------|
| 停止退出 | ✅ |
| 连点器 | ✅ |
| 快速拾取 | ✅ |
| 鬼畜走路 | ✅ |
| 火神跳喷 | ✅ |
| 甘雨走A | ✅ |
| 双玛头 | ✅ |
| 坐标颜色 | ✅ |
| 甘雨加特林 | ⬜ |
| 龙王喷水 + 子功能 | ⬜ |
| 克洛琳德 | ⬜ |
| 添加好友 | ⬜ |
| 申请加入 | ⬜ |
| 优化游戏 | ⬜ |
| 2048 系列 | ⬜ |

## 事件类型

| 类型 | 模型 | 代表功能 |
|------|------|---------|
| **Serial** (Sequence based) | `EventSequence` 链式 API | 连点器、快速拾取、甘雨走A、双玛头 |
| **Timestamp** (Time based) | 时间轴调度器 | 鬼畜走路、龙王喷水（未来：多键编排） |
