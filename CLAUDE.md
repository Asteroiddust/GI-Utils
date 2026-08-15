# GI-Utils — Rust 游戏输入自动化工具 v1.2.0

> **Review**: master 48/48 cleared · gi-utils-gui 2H/12M/15L cleared（含 L12，2026-08）· 时间轴调度器 15/15 cleared（2026-08-14）· GUI/托盘重写 13/13 cleared（2026-08-16）· 25 单测 + 2 doctest 通过
> **Build**: O3 + LTO fat + panic=unwind + target-cpu=native

## 项目概述

基于 [Interception](https://github.com/oblitum/Interception) 内核驱动的游戏辅助工具，从 C++ (Visual Studio) 重构为 Rust。

原 C++ 项目: `E:\Projects\fmttest`，服务于原神/崩铁/鸣潮等游戏。

## 技术栈

- **Rust 1.99 nightly** (edition 2024, -Z plt=no, build-std release-only)
- **Interception 驱动** — 内核级键盘/鼠标输入拦截与注入
- **windows 0.62** — Win32 API（Threading、ToolHelp、Gdi、WindowsAndMessaging、Shell、LibraryLoader、Media、HiDpi、Security）
- **eframe 0.36**（egui，仅 glow + default_fonts）— GUI 配置面板与托盘窗口
- **toml 1.1 + serde 1.0** — TOML 配置解析/序列化
- **tracing 0.1 + tracing-subscriber 0.3**（仅 fmt）— 结构化日志 → GUI 日志面板桥
- **embed-resource 3**（构建期）— exe 图标资源嵌入

## 项目结构

```
src/
├── main.rs                    # headless 入口：加载 config → 校准 TSC → 注册 → Engine.run()
├── lib.rs                     # 库根 — headless 与 GUI 共享全部模块
├── bin/
│   └── gui/
│       ├── main.rs            # GUI 入口 (egui 配置面板 + live-apply + 崩溃自愈重试 → gi-utils-gui.exe)
│       ├── tray.rs            # 托盘线程 (Shell_NotifyIconW + 消息窗口/泵 + quit 标志收尾)
│       ├── tray_icon.rs       # 图标原料 + SharedIcon 共享句柄 (启动预加载, L4 WIC 污染防御)
│       └── window_ops.rs      # HWND 安全包装唯一入口 (IsWindow 重校验 + 跨进程 pid 过滤, L3 幽灵窗口防御)
├── config.rs                  # TOML 配置解析 + 函数工厂 + [gui] 图标配置
├── build.rs                   # 链接 interception.lib + 嵌入 assets/icon.ico
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
│   ├── bindings.rs            #   KeyBindings + TriggerMode + ActiveGuard
│   └── timeline.rs            #   Timeline/RollingKeys — 绝对时刻编排 (Timestamp 范式)

├── utils/
│   ├── delay.rs               #   TSC delay (相对/绝对时刻) + interruptible + 校准
│   ├── beep.rs                #   蜂鸣 (同步 beep + 异步 beep_async)
│   ├── affinity.rs            #   CPU 亲和性 + 进程迭代
│   ├── screen.rs              #   PixelReader (cached DC) + 像素取色
│   └── log.rs                 #   LogCollector — 全局 tracing → GUI 日志面板桥

└── functions/
    ├── stop.rs                #   停止退出 (F12, Once)
    ├── auto_clicker.rs        #   连点器 (F13, Loop)
    ├── quick_pickup.rs        #   快速拾取 (F14, Loop)
    ├── ghost_walk.rs          #   鬼畜走路 (F15, Loop)
    ├── mavuika_jump.rs        #   火神跳喷 (F16, Loop)
    ├── ganyu_aim_cancel.rs    #   甘雨走A (F17, Once)
    ├── mavuika_double_cancel.rs # 双玛头 (F18, Loop)
    ├── mouse_color.rs         #   坐标颜色 (F19, Loop)
    └── optimize_game.rs        #   优化游戏 (NumpadAdd, Once, toggle 奇偶)

assets/
├── icon.ico                   # 自定义图标（构建期嵌入 exe 资源）
└── icon.rc                    # 图标资源脚本 (embed-resource)
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
      ├── capture_tx             (GUI 按键捕获通道，捕获模式拦截分发)
      ├── pending_joins          (已停止线程句柄队列 — GUI 帧 is_finished 惰性 join，绝不阻塞)
      ├── Entry.active / stop_requested / handle
      └── TriggerMode: Once / Loop / Toggle
```

## 核心设计决策

| 决策 | 理由 |
|------|------|
| **Key = ScanCode + is_e0** | 单一类型消除 PS/2 值冲突，E0 自动注入 state |
| **SendContext 独立类型** | 发送/接收分离，编译器强制禁止并发接收 |
| **stop_requested 正向语义** | `true`=停止，全项目统一，无双重否定 |
| **TOML 动态配置** | `config.toml` 驱动热键映射，无需重编译 |
| **ActiveGuard Drop 防护** | 线程 panic 时自动清理 active 标志（panic=unwind 后真正生效） |
| **GUI 崩溃自愈** | 睡眠唤醒 wgl 上下文失效 → eframe make_current panic；catch_unwind 重建 app 重试 ≤3 次；**关机序列在 Drop 之外**（shutdown_all 显式调用 — 回卷 Drop 执行关机曾杀死引擎）；hook 静默仅限主线程渲染 panic（GUI_MAIN_THREAD 区分），其他线程 panic 弹框后立即退出；恢复时从磁盘重载配置 + stop_all/clear_all 重建注册表；tray_ok/hidden 进程级共享继承 |
| **Mutex 外 join** | stop 不阻塞主事件循环 |
| **delay_ms_interruptible** | 100μs 检查间隔，Loop/Toggle 即时响应 |
| **EventSequence 链式 API** | `seq.tap(K).sleep(50).wheel(DOWN)` |
| **时间轴绝对时刻调度** | deadline = 播放起点 TSC + 条目偏移（饱和加法），无累计漂移，前序超时自然追时 |
| **已触发即移除 + 增量同步** | 表恒为未触发条目；播放中追加条目在到期帧被 `partition_point` 捕获，不重复不遗漏（MIDI 实时编辑语义）；回放前缀通知编辑器清理，长会话内存有界 |
| **表空不结束（live-edit）** | 表空以 0.5ms 轮询等待编辑器追加；结束只由 stop_requested 决定（MIDI 编辑器语义：播放器永不自杀） |
| **RollingKeys 节奏滚动** | 按下实时产生、释放动态排程，无静态表边界缝隙（对应 C++ next_press_time + scheduled_releases）；卡顿节拍重锚 — 错过即弃、不突发追拍（有意偏离 C++ 原版） |
| **挂起键兜底清理** | 停止时补发 release（活动音符 note-off，含 At 键盘事件），防卡键 |
| **KeyFunction 只有 1 个方法** | `execute(&self, stop_requested: Arc<AtomicBool>)` |
| **printf 作 release 输出** | 避免 Windows stderr 缓冲问题 |
| **线程级核心分离** | 进程掩码 12-15，GUI 渲染→12,13 (LOWEST)，输入处理→14,15 (REALTIME) |
| **pending_joins 惰性 join** | `is_finished()` 检查保留未结束句柄，GUI 帧永不阻塞 |
| **GUI live-apply + 托盘隐藏** | 修改即时生效；关闭隐藏到托盘，F12/菜单退出 |
| **锁序单向 bindings→pending** | 全部路径同序，无嵌套反转，无死锁 |
| **托盘图标 [gui] icon_path** | config.toml 运行时指定 .ico，LoadImageW 加载，失败/留空回退程序生成蓝 G |
| **exe 图标构建期嵌入** | build.rs embed-resource 嵌入 assets/icon.ico，缺失时警告跳过、构建不失败 |
| **WM_SETICON 窗口图标同步** | eframe 默认用 egui logo 覆盖窗口图标；托盘线程找到主窗口后用同一 HICON 覆盖任务栏/标题栏/Alt-Tab |

## 构建

```bash
# release（含 build-std：std 以 panic_unwind + native/O3 重编，见 .cargo/build-std.toml）
cargo build --release --config .cargo/build-std.toml
# 测试（不带 build-std — 全局 build-std 会让 cargo test 为 dev+test 双 profile
# 各编一份 std，两份 core 链接报 duplicate lang item（cargo 已知问题））
cargo test
# 输出: target/release/gi-utils.exe (~880KB, headless) + gi-utils-gui.exe (~5.8MB, GUI)
```

自定义图标：`assets/icon.ico` 由 build.rs（embed-resource）嵌入两个 exe；文件缺失时跳过并警告，不影响构建。

release profile: `opt-level=3, lto=fat, strip=true, codegen-units=1, panic=unwind`（unwind 是 GUI 崩溃自愈的基础：catch_unwind 捕获渲染 panic 重试；Drop 防护体系全面激活）

rustflags: `-C target-cpu=native -Z plt=no`（target-cpu 已含 native 调优，tune-cpu 冗余已移除）

依赖裁剪：eframe 仅 `default_fonts` + `glow`（无 wgpu/accesskit/links — GUI -46%）；tracing-subscriber 仅 `fmt`。

build-std 说明：不用 `panic_immediate_abort` — 它会跳过 panic hook，破坏 GUI 亲和性恢复兜底。

## 运行

**必须以管理员身份运行**。首次运行自动生成 `config.toml`。

## CPU 核心分配 (8C16T 9800X3D)

```
物理核 0    [0,1  ]  OTHER  (系统 + 其他进程)
物理核 1-5  [2-11 ]  GAME   (游戏)
物理核 6    [12,13]  GUI    (GUI 渲染, 线程级 LOWEST 优先级)
物理核 7    [14,15]  TOOL   (Engine 输入处理 + 功能线程, REALTIME)
```

进程掩码 12-15（GUI 版）。线程级收窄：GUI 主线程 → 12,13 + `THREAD_PRIORITY_LOWEST`；托盘线程 → 12,13（普通优先级，GUI 侧）；Engine 线程与功能线程 → 14,15（`pin_current_thread`，在 spawn 闭包内调用）。headless 版进程掩码保持 14,15，无需扩展。

## 部署

`E:\Program\GI-Utils\` 下两个符号链接，每次构建自动同步：

```
gi-utils.exe      → E:\Projects\Rust\GI-Utils\target\release\gi-utils.exe
gi-utils-gui.exe  → E:\Projects\Rust\GI-Utils\target\release\gi-utils-gui.exe
```

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

# GUI 配置 — icon_path 指向 .ico 托盘图标；留空使用程序生成图标
[gui]
icon_path = ""
```

## 移植进度

| 功能 | 状态 | 优先级 | 难度 | 备注 |
|------|:----:|:------:|:----:|------|
| 停止退出 | ✅ | — | — | |
| 连点器 | ✅ | — | — | |
| 快速拾取 | ✅ | — | — | |
| 鬼畜走路 | ✅ | — | — | |
| 火神跳喷 | ✅ | — | — | |
| 甘雨走A | ✅ | — | — | |
| 双玛头 | ✅ | — | — | |
| 坐标颜色 | ✅ | — | — | |
| 优化游戏 | ✅ | — | — | |
| 甘雨加特林 | ⬜ | 1 | ★★ | R+鼠标移动序列 |
| 克洛琳德 | ⬜ | 2 | ★★★ | 像素颜色检测触发 |
| 添加好友 | ⬜ | 3 | ★★★ | 屏幕坐标+绝对鼠标 |
| 申请加入 | ⬜ | 4 | ★★ | 鼠标序列 |
| 2048 系列 | ⬜ | 5 | ★ | 纯按键序列 |

### 移植流程（C++ 参考模板）

1. 从 `E:\Projects\fmttest\main.cpp` 找到对应类的 EventSequence 构造逻辑
2. 在 `src/functions/` 下新建文件，中文 struct 名照搬原项目
3. 实现 `KeyFunction` trait
4. 在 `src/config.rs` 的 `create_function` 和 `DEFAULT_CONFIG` 各加一行
5. 无需改 `main.rs` — 全部走配置驱动

参考模板: `auto_clicker.rs` (Loop), `ganyu_aim_cancel.rs` (Once), `mavuika_jump.rs` (on_activate+Loop)

## 路线图

### 组合键注册（★★）

支持修饰键+功能键的组合注册，如 `Ctrl+F13`、`Alt+E`：

```toml
[[bindings]]
key = "F13"
modifier = "Ctrl"        # None / Ctrl / Alt / Shift / Win
func = "连点器"
mode = "Loop"
```

**设计要点**：`KeyBindings` 跟踪所有键的实时按下/松开状态；`process_key_down` 时检查修饰键是否已按住；组合键按下时触发功能，修饰键松开时不影响功能运行；兼容现有单键注册（modifier=None）。主要改动在 `bindings.rs`（状态追踪）和 `config.rs`（解析）。

### 通用 held-key 清理（部分落地）

时间轴执行器已内置挂起键清理（`TimelinePlayer::release_pending` / `RollingPlayer` 退出补发 release）。EventSequence 侧的多键 `HeldTracker` 仍待需要时再做 — 触发时机：功能需要多键同时按下（如 MIDI 键盘模拟）。

## 事件类型

| 类型 | 模型 | 代表功能 |
|------|------|---------|
| **Serial** (Sequence based) | `EventSequence` 链式 API | 连点器、快速拾取、甘雨走A、双玛头、火神跳喷 |
| **Timestamp** (Time based) | 时间轴调度器 `Timeline`/`RollingKeys`（`engine/timeline.rs`） | 鬼畜走路 ✅、未来钢琴模式 |
