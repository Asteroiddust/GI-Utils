# GI-Utils — Rust 游戏输入自动化工具 v1.3.0

> **Review**: master 48/48 cleared · gi-utils-gui 2H/12M/15L cleared（含 L12，2026-08）· 时间轴调度器 15/15 cleared（2026-08-14）· GUI/托盘重写 13/13 cleared（2026-08-16）· 32 单测 + 2 doctest 通过 · DeepSeek 审查 20 项：17 修 / 2 有意不修（3.2/3.4）/ 1 驳（4.7），2026-08-16 · 原生移植审查 14 项全处置，2026-08-19
> **Build**: O3 + LTO fat + panic=unwind + rust-lld + target-cpu=native

## 项目概述

基于 [Interception](https://github.com/oblitum/Interception) 内核驱动的游戏辅助工具，从 C++ (Visual Studio) 重构为 Rust。

原 C++ 项目: `E:\Projects\fmttest`，服务于原神/崩铁/鸣潮等游戏。

## 技术栈

- **Rust 1.100 nightly** (edition 2024, -Z threads=16, build-std release-only)
- **Interception 驱动** — 内核级键盘/鼠标输入拦截与注入；用户层 API 为**原生 Rust 移植**（`src/interception/protocol.rs`，DeviceIoControl 协议端，替代原预编译 interception.lib，2026-08 移植）
- **windows 0.62** — Win32 API（Threading、ToolHelp、Gdi、WindowsAndMessaging、Shell、LibraryLoader、Media、HiDpi、Security、Storage_FileSystem、System_IO）
- **eframe 0.36**（egui，仅 glow + default_fonts）— GUI 配置面板与托盘窗口
- **toml 1 + serde 1** — TOML 配置解析/序列化
- **tracing 0.1 + tracing-subscriber 0.3**（仅 fmt）— 结构化日志 → GUI 日志面板桥
- **embed-resource 3**（构建期）— exe 图标资源嵌入

## 项目结构

```
# 根级
├── Cargo.toml                 # 依赖 + profile（dev 极速 / release 激进）
├── build.rs                   # 嵌入 assets/icon.ico
├── assets/
│   ├── icon.ico               # 自定义图标（构建期嵌入 exe 资源）
│   └── icon.rc                # 图标资源脚本 (embed-resource)
├── CLAUDE.md                  # 本文档（单一权威）
└── AGENTS.md                  # 子 agent 指令

# 源码
src/
├── lib.rs                     # 库根 — GUI 二进制与测试共享全部模块
├── bin/
│   └── gi-utils-gui/          # 唯一二进制（目录名 = 二进制名，Cargo 自动发现，无 [[bin]] 配置）
│       ├── main.rs            # GUI 入口 (egui 配置面板 + live-apply + 崩溃自愈重试 → gi-utils-gui.exe)
│       ├── tray.rs            # 托盘线程 (Shell_NotifyIconW + 消息窗口/泵 + quit 标志收尾)
│       ├── tray_icon.rs       # 图标原料 + SharedIcon 共享句柄 (启动预加载, L4 WIC 污染防御)
│       └── window_ops.rs      # HWND 安全包装唯一入口 (IsWindow 重校验 + 跨进程 pid 过滤, L3 幽灵窗口防御)
├── config.rs                  # TOML 配置解析 + 函数工厂 + [gui] 图标配置
├── key.rs                     # ScanCode(u16) 新类型 + Key (ScanCode + is_e0) + 90+ 常量

├── interception/              # Interception 用户层原生实现（替代预编译 lib）
│   ├── protocol.rs            #   DeviceIoControl 协议端移植：20 设备上下文、类型化收发、
│   │                          #   IOCTL/常量（与 interception.h 同名）、栈分批零堆分配
│   └── context.rs             #   InterceptionContext (recv) + SendContext (send) 类型化封装

├── engine/
│   ├── mod.rs                 #   Engine — 事件循环
│   ├── event.rs               #   InputEvent + EventSequence 链式 API + HeldTracker
│   ├── bindings.rs            #   KeyFunction trait + KeyBindings + TriggerMode + ActiveGuard
│   └── timeline.rs            #   Timeline/RollingKeys — 绝对时刻编排 (Timestamp 范式)

├── utils/
│   ├── delay.rs               #   TSC delay (相对/绝对时刻) + interruptible + 校准
│   ├── beep.rs                #   蜂鸣 (同步 beep + 异步 beep_async)
│   ├── affinity.rs            #   CPU 亲和性 + 进程迭代
│   ├── screen.rs              #   PixelReader (cached DC) + 像素取色
│   └── log_collector.rs       #   LogCollector — 全局 tracing → GUI 日志面板桥

└── functions/
    ├── stop.rs                #   停止退出 (F12, Once)
    ├── auto_clicker.rs        #   连点器v1 (F13, Loop) + 连点器v2（同文件独立复制版）
    ├── quick_pickup.rs        #   快速拾取 (F14, Loop)
    ├── ghost_walk.rs          #   鬼畜走路 (F15, Loop)
    ├── mavuika_jump.rs        #   火神跳喷 (F16, Loop)
    ├── ganyu_aim_cancel.rs    #   甘雨走A (F17, Once)
    ├── mavuika_double_cancel.rs # 双玛头 (F18, Loop)
    ├── mouse_color.rs         #   坐标颜色 (F19, Loop)
    └── optimize_game.rs        #   优化游戏 (NumpadAdd, Once, toggle 奇偶)
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
| **用户层 API 原生移植** | 替代预编译 interception.lib：类型化收发切片、栈分批零堆分配（消灭 C 版每调用 HeapAlloc）、错误传播 Result、set_filter 闭包、Drop 自动清理 — 内核驱动协议不变 |
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
| **挂起键兜底清理** | 停止时补发 release（活动音符 note-off，含 At 键盘事件），防卡键；EventSequence::play 内置 HeldTracker（双玛头手工粘滞键追踪已退役） |
| **KeyFunction 只有 1 个方法** | `execute(&self, stop_requested: Arc<AtomicBool>)` |
| **线程级核心分离** | 进程掩码 12-15，GUI 渲染→12,13 (LOWEST)，输入处理→14,15 (REALTIME) |
| **pending_joins 惰性 join** | `is_finished()` 检查保留未结束句柄，GUI 帧永不阻塞 |
| **GUI live-apply + 托盘隐藏** | 修改即时生效；关闭隐藏到托盘，F12/菜单退出 |
| **锁序单向 bindings→pending** | 全部路径同序，无嵌套反转，无死锁 |
| **托盘图标 [gui] icon_path** | config.toml 运行时指定 .ico，LoadImageW 加载，失败/留空回退程序生成蓝 G |
| **exe 图标构建期嵌入** | build.rs embed-resource 嵌入 assets/icon.ico，缺失时警告跳过、构建不失败 |
| **WM_SETICON 窗口图标同步** | eframe 默认用 egui logo 覆盖窗口图标；托盘线程找到主窗口后用同一 HICON 覆盖任务栏/标题栏/Alt-Tab |

## 构建

```bash
# ── 日常开发（dev profile，增量编译秒级 — 默认路径）──────────────────
cargo check    # 类型检查
cargo test     # dev profile 跑单测 + doctest
cargo build    # target/debug/gi-utils-gui.exe（功能验证用）
# dev 产物不影响部署（部署符号链接指向 target/release/）

# ── 部署/发版（release + build-std，全量 ~1 分钟 — 仅此时运行）────────
# build-std：std 以 panic_unwind + native/O3 重编，见 .cargo/build-std.toml
cargo build --release --config .cargo/build-std.toml
# 输出: target/release/gi-utils-gui.exe (~6.8MB)
```

> **策略（2026-08-19 定）**：日常编译/测试/功能验证一律走 dev profile —
> release 版 O3 + fat LTO + build-std 重编 std 每次 30-60s+，日常迭代太慢。
> release 构建仅在部署/发版时运行。测试不带 build-std 的原因：全局 build-std
> 会让 cargo test 为 dev+test 双 profile 各编一份 std，两份 core 链接报
> duplicate lang item（cargo 已知问题）。

自定义图标：`assets/icon.ico` 由 build.rs（embed-resource）嵌入 exe；文件缺失时跳过并警告，不影响构建。

release profile: `opt-level=3, lto=fat, strip=true, codegen-units=1, panic=unwind, incremental=false`（unwind 是 GUI 崩溃自愈的基础：catch_unwind 捕获渲染 panic 重试；Drop 防护体系全面激活）

dev profile（极速迭代）: `opt-level=0, debug=0, codegen-units=256, incremental`；依赖统一 `opt-level=2`（eframe/glow 界面流畅 + 依赖只编一次）

链接器: `rust-lld.exe`（lld-link，目标域配置 `[target.x86_64-pc-windows-msvc]`，比 MSVC link.exe 快）

rustflags: `-C target-cpu=native -Z threads=16`（target-cpu 已含 native 调优，tune-cpu 冗余已移除；`-Z plt=no` 已移除 — PLT 是 ELF 概念，Windows PE 上无效）

依赖裁剪：eframe 仅 `default_fonts` + `glow`（无 wgpu/accesskit/links — GUI -46%）；tracing-subscriber 仅 `fmt`。

build-std 说明：不用 `panic_immediate_abort` — 它会跳过 panic hook，破坏 GUI 亲和性恢复兜底。

已知非目标（有意保持）：`cargo clippy` 有 21 项风格 lint（lib 13 + gui 8：缺 `Default` impl、`collapsible_if`、`manual_range_contains`、`div_ceil`、`is_multiple_of` 等，无功能错误）。rustfmt：**2026-08-19 起已全面采用**（用户拍板，全项目一次格式化），`cargo fmt --check` 应保持干净。

## 运行

**必须以管理员身份运行**。首次运行自动生成 `config.toml`。

## CPU 核心分配 (8C16T 9800X3D)

```
物理核 0    [0,1  ]  OTHER  (系统 + 其他进程)
物理核 1-5  [2-11 ]  GAME   (游戏)
物理核 6    [12,13]  GUI    (GUI 渲染, 线程级 LOWEST 优先级)
物理核 7    [14,15]  TOOL   (Engine 输入处理 + 功能线程, REALTIME)
```

> ⚠️ **GAME/OTHER 分区是目标设计，当前未落地**（DeepSeek 审查 3.2，有意保持）：
> `GAME_CORES_MASK` 与 `OTHER_CORES_MASK` 均为全核掩码，「优化游戏」的
> 核心隔离实际是 no-op，仅优先级提升 + 前台切换生效。实测缩减游戏可用
> 核心数在部分游戏有较大性能下降和频繁 stutter，缩减 OTHER_CORES_MASK
> 也有奇怪现象，故暂不改。TOOL/GUI 分区**已落地**（下方进程掩码 + 线程收窄）。
> 「优化游戏」偶数次恢复**有意不降游戏优先级**（3.4）：降级需再次
> OpenProcess 游戏句柄（反作弊拦截路径），且 HIGH 留存无害（进程退出即消亡）。

进程掩码 12-15。线程级收窄：GUI 主线程 → 12,13 + `THREAD_PRIORITY_LOWEST`；托盘线程 → 12,13（普通优先级，GUI 侧）；Engine 线程与功能线程 → 14,15（`pin_current_thread`，在 spawn 闭包内调用）。

## 部署

`E:\Program\GI-Utils\` 下两个符号链接，每次构建自动同步：

```
gi-utils-gui.exe  → E:\Projects\Rust\GI-Utils\target\release\gi-utils-gui.exe   (发版产物)
gi-utils-dev.exe  → E:\Projects\Rust\GI-Utils\target\debug\gi-utils-gui.exe     (dev 日常验证，dev profile 秒级构建)
```

**必须以管理员身份运行**。首次运行自动生成 `config.toml`。

```toml
[[bindings]]
key = "F12"
func = "停止退出"
mode = "Once"

[[bindings]]
key = "F13"
func = "连点器v1"
mode = "Loop"

# GUI 配置 — icon_path 指向 .ico 托盘图标；留空使用程序生成图标
[gui]
icon_path = ""
```

## 移植进度

| 功能 | 状态 | 优先级 | 难度 | 备注 |
|------|:----:|:------:|:----:|------|
| 停止退出 | ✅ | — | — | |
| 连点器v1 | ✅ | — | — | |
| 连点器v2 | ✅ | — | — | v1 同文件复制版（调参互不影响） |
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
5. 无需改入口 — 全部走配置驱动

参考模板: `auto_clicker.rs` (Loop), `ganyu_aim_cancel.rs` (Once), `mavuika_jump.rs` (on_activate+Loop)

## 选型背景 — 原 TECHNICAL_PLAN.md 并入（2026-08-19）

重构前技术选型的存续结论（原文为实现前方案，细节已过时，此处仅留决策依据）：

- **输入拦截三路线评估**：A) Interception 内核驱动 — 游戏兼容性最好，原 C++ 项目已验证（✅ 采用）；B) Win32 SendInput（enigo/winput）— 纯 Rust 无驱动，但大量游戏会忽略其注入；C) SetWindowsHookEx（inputbot/rdev）— 用户态 hook，反作弊易拦截。内核级驱动对游戏输入助手不可替代
- 自行编写 FFI 绑定而非社区 interception-sys（维护状态不明）— 2026-08-19 已进一步演进为**用户层原生 Rust 移植**（`interception/protocol.rs`）
- 依赖对照：fmt→`std::fmt`、spdlog→tracing、tlhelp32→windows crate、RDTSC→`core::arch`（均与最终实现一致）
- **OpenInputBridge**（2026-08-19 调研）：社区干净室重写的协议兼容驱动（MIT 源码 + WHQL 已签但分发付费）。协议逐字节兼容已验证 — `interception/protocol.rs` 零改动可跑（附 2 个扩展 IOCTL：槽位配分/驱动识别）。**决策：不切换**（原版驱动免费稳定）；未来原版不可用时按此切换，成本仅装驱动
- 与早期方案的偏差（以代码为准）：未用 CancellationToken（`stop_requested: Arc<AtomicBool>` 更轻）；edition 2024；EventSequence 按 enum 设计落地；时间轴调度器为重构后新增范式

## DeepSeek 审查处置 — 原 dsh-review-result.md 并入（2026-08-19）

2026-08-16 对 dsh-dev 分支的 DeepSeek harness 审查（20 项），处置记录：

| 编号 | 处置 | 说明 |
|---|---|---|
| 2.1 | ✅ | timeline sync 索引 bug + 回归测试 |
| 2.2 | ✅ | move_absolute 归一化文档 + `normalize_absolute` |
| 2.3 | ✅ | headless 退出顺序（先 stop_all） |
| 3.1 | ✅ | PixelReader DC ManuallyDrop 泄漏策略 |
| 3.2 | ⛔ 有意 | 全核掩码保持（缩减核心实测性能下降 + stutter） |
| 3.3 | ✅ | Engine 循环 drain_pending_joins |
| 3.4 | ⛔ 有意 | 恢复不降游戏优先级（反作弊 OpenProcess 路径） |
| 3.5 | ✅ | NIM_ADD 前 quit 抢先检查 |
| 3.6 | ✅ | 隐藏态两处复位 |
| 3.7 | ✅ | 主窗口 pid 过滤 |
| 3.8 | ✅ | register 替换先停旧线程 |
| 3.9 | ✅ | Once 句柄回收 |
| 4.1-4.6, 4.8 | ✅ | 低危 7 项（饱和加法 / scroll clamp / WM_DESTROY 顺序 / 配置原子写 / 防抖表清理 / 默认功能避重 / toggle 成功才翻转） |
| 4.7 | ❌ 驳回 | AND mask 已是通用尺寸 |

原生移植后续两轮审查（14 项发现全处置 + 空队列 WARN 修复）见 native-interception 分支提交记录。

## 路线图

### 组合键注册（★★）

支持修饰键+功能键的组合注册，如 `Ctrl+F13`、`Alt+E`：

```toml
[[bindings]]
key = "F13"
modifier = "Ctrl"        # None / Ctrl / Alt / Shift / Win
func = "连点器v1"
mode = "Loop"
```

**设计要点**：`KeyBindings` 跟踪所有键的实时按下/松开状态；`process_key_down` 时检查修饰键是否已按住；组合键按下时触发功能，修饰键松开时不影响功能运行；兼容现有单键注册（modifier=None）。主要改动在 `bindings.rs`（状态追踪）和 `config.rs`（解析）。

### 通用 held-key 清理（已落地）

时间轴执行器（`TimelinePlayer::release_pending` / `RollingPlayer` 退出补发 release）与 EventSequence 侧（`play()` 内置 `HeldTracker`：按下记录/松开移除/stop 中断时合并补发全部挂起键 release，2026-08-19 随双玛头 play() 化落地）双路齐备。

## 事件类型

| 类型 | 模型 | 代表功能 |
|------|------|---------|
| **Serial** (Sequence based) | `EventSequence` 链式 API | 连点器v1/v2、快速拾取、甘雨走A、双玛头、火神跳喷 |
| **Timestamp** (Time based) | 时间轴调度器 `Timeline`/`RollingKeys`（`engine/timeline.rs`） | 鬼畜走路 ✅、未来钢琴模式 |
