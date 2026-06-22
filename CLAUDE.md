# GI-Utils — Rust 游戏输入自动化工具

## 项目概述

基于 [Interception](https://github.com/oblitum/Interception) 内核驱动的游戏辅助工具，从 C++ (Visual Studio) 重构为 Rust。

原 C++ 项目位于 `E:\Projects\fmttest`，服务于原神/崩铁/鸣潮等游戏。

## 技术栈

- **Rust edition 2021** (稳定版)
- **Interception 驱动** — 内核级键盘/鼠标输入拦截与注入，FFI 调用 `interception.dll`
- **windows-rs 0.58** — Win32 API (GDI 像素取色、进程管理、线程优先级)
- **tracing** — debug 构建的结构化日志（release 用 `println!`/`eprintln!`）

## 项目结构

```
src/
├── main.rs                    # 入口：提权 → TSC校准 → 注册功能 → 启动 KeyMonitor
├── build.rs                   # 链接 interception.lib
│
├── interception/              # Interception FFI 绑定 (完整覆盖 interception.h)
│   ├── ffi.rs                 #   extern "C" 声明、常量、repr(C) struct
│   ├── context.rs             #   RAII InterceptionContext + safe send/receive API
│   └── strokes.rs             #   类型安全 Stroke 转换（零拷贝）
│
├── scan_code.rs               # ScanCode(u16) newtype + 完整 Set 1 关联常量 + name()
│
├── utils/                     # 工具模块
│   ├── delay.rs               #   read_tsc() + cpu_relax() 安全包装 + TSC 校准
│   ├── beep.rs                #   蜂鸣反馈 (直接 FFI kernel32 Beep)
│   ├── affinity.rs            #   RAII OwnedHandle + ProcessIterator + Result API
│   └── screen.rs              #   RAII ScreenDC + 像素取色 + 光标位置
│
├── engine/                    # 核心引擎
│   ├── event.rs               #   InputEvent (Keyboard/Mouse/Sleep) + EventSequence
│   ├── function.rs            #   KeyFunction trait (1 method) + Arc<AtomicBool> 取消
│   ├── bindings.rs            #   KeyBindings + KeyId + TriggerMode (3种触发模式)
│   └── monitor.rs             #   KeyMonitor 主事件循环 + verbose keystroke 显示
│
└── functions/                 # 游戏功能实现
    ├── mod.rs
    └── auto_clicker.rs        #   连点器 (F13, WhileHeld)
```

## 架构

```
KeyMonitor (主循环, blocking)
  ├── InterceptionContext: recv (接收) + send (转发/注入)
  ├── 事件循环: wait → receive → forward → dispatch
  │     ├── F12 → 退出
  │     ├── verbose? → print_keystroke()
  │     └── KeyBindings.process_key_down/up(key)
  └── KeyBindings
        ├── HashMap<KeyId, Entry>  (绑定注册表)
        ├── HashMap<KeyId, bool>   (去抖表)
        ├── Entry.active: Arc<AtomicBool>      (统一运行状态)
        ├── Entry.cancel: Option<Arc<...>>    (loop 取消信号)
        ├── Entry.handle: Option<JoinHandle>  (loop 线程句柄)
        ├── spawn_once() / spawn_loop() / stop_loop()
        └── TriggerMode 分支: Once / WhileHeld / Toggle
```

## 核心设计决策

| 决策 | 理由 |
|------|------|
| **沿用 Interception** | 内核驱动是唯一可靠的在游戏中注入输入的方式 |
| **不用 tokio/async** | `std::thread` + `Arc<AtomicBool>` 足够，零额外依赖 |
| **ScanCode 用 newtype** | 公共 API 类型安全 (`KeyId::new(ScanCode::F13, false)`)，允许重复值 |
| **Sleep 是 InputEvent 的独立 variant** | 延迟和动作正交，执行循环只需顺序处理 events |
| **EventSequence 链式 API** | 所有方法返回 `&mut Self`，`press.sleep.release.wheel` |
| **TriggerMode 三种模式** | Once (单次) / WhileHeld (按住循环) / Toggle (开关) |
| **KeyFunction trait 只有 1 个方法** | `execute(&self, running: Arc<AtomicBool>)`，启动/停止/生命周期全在 manager |
| **Entry 用 active + cancel 双 flag** | Once 只设 active (线程自清)，WhileHeld/Toggle 通过 cancel 通知停止 |
| **send_ctx 用 Arc 共享** | 多线程安全共享 Interception 发送上下文 |
| **println! 替代 tracing 作 release 输出** | 避免 Windows 上 stderr 缓冲导致的输出不可见 |

## 构建

```bash
cargo build --release
# 输出: target/release/gi-utils.exe
```

依赖 `interception.lib`，路径在 `build.rs` 中配置：
```
E:\Program\Interception\library\x64\interception.lib
```

运行时需要 `interception.dll` 在 exe 同目录。

## 运行

**必须以管理员身份运行**（Interception 内核驱动要求）。

```
F13 = Auto Clicker (按住激活，松开停止，WhileHeld 模式)
F12 = 退出程序
```

## 移植进度

| 功能 | C++ 类名 | Rust 状态 |
|------|---------|----------|
| 连点器 | `连点器` | ✅ 已移植 |
| 快速拾取 | `快速拾取` | ⬜ 待移植 |
| 龙王喷水 | `龙王喷水` + `上下左右重置` | ⬜ 待移植 |
| 火神跳飞 | `火神跳飞` | ⬜ 待移植 |
| 甘雨走A | `甘雨走A` | ⬜ 待移植 |
| 甘雨加特林 | `甘雨加特林` | ⬜ 待移植 |
| 双玛头 | `双玛头` | ⬜ 待移植 |
| 鬼畜走路 | `鬼畜走路` | ⬜ 待移植 |
| 克洛琳德 | `克洛琳德` | ⬜ 待移植（需像素检测） |
| 添加好友 | `添加好友` | ⬜ 待移植 |
| 申请加入 | `申请加入` | ⬜ 待移植 |
| 坐标颜色 | `坐标颜色` | ⬜ 待移植 |
| 优化游戏 | `优化游戏` | ⬜ 待移植（CPU 亲和性已写） |
| 2048 系列 | `二零四八_*` | ⬜ 待移植 |
