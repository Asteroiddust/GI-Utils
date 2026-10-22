# GI-Utils — Agent 指令与项目权威文档

> **版本 v1.9.0** · release 基线 `v1.7.7`（`dd519fd`）· 本文档 2026-10-23 全量重写
>
> 本文档是**唯一权威项目文档**。旧的 `CLAUDE.md` 已删除，请勿再创建或引用。
> 用户向说明见 [README.md](README.md) / [README.zh-CN.md](README.zh-CN.md)；本文档面向接手开发的 agent。
>
> **维护规则**：改动行为、结构、配置格式或构建方式时，必须在同一次提交内同步本文档；
> 文档与代码冲突时以代码为准，并当场修正文档。避免旧文档那种「修复被回滚、文档悄悄漂移」的第二次发生。
>
> **当前健康（2026-10-23 实测）**：`cargo check` 0 error / 0 warning；`cargo test` 93 单测
> （lib 89 + GUI bin 4）+ 2 doctest 全过（另 1 个 doctest ignored）；`cargo fmt --check` 干净；
> debug/release 产物与部署链接均未过期。
> 注：健康数据采于 2026-10-23。同日完成 Phase 1 清理与 Phase 2 健壮性；**作者实机验证
> 正常后发布 v1.7.7**（release 13,693,440 B，SHA256 `8CD60E65…FBF3`，构建源 `dd519fd`）。
> `v1.7.6`/`v1.7.7` 与 `bench-getpixel-archive` tag 均为本地。代码基线 `bb3f91b`。

## 0. 给 Agent 的硬性规则

1. **先读本文件**。只改与任务相关的文件；不要顺手重构、不要清理无关 lint。
2. **交付门槛**：`cargo check` 0 error / 0 warning、`cargo test` 全绿、`cargo fmt --check` 干净。
3. **改码后必须 `cargo build`** 刷新 `target/debug/gi-utils-gui.exe`——`cargo test` 不更新它，
   而部署目录的 `gi-utils-dev.exe` 符号链接指向它（曾因此验证到旧二进制）。
4. **不要主动跑 release 构建**（`cargo build --release` 或带 `-Z build-std`）。只有用户明确说
   「发版」时才执行，见[构建](#6-构建)。日常一律 dev profile。
5. **不要从 agent 会话里启动 `gi-utils-gui.exe`**：它需要管理员权限、会安装输入钩子/拦截键盘、
   并可能抢占焦点影响当前会话。实机验证交给用户手动做。
6. **不要改驱动协议层**：`src/interception/protocol.rs` 的 IOCTL/结构体布局与 Interception
   内核驱动逐字节兼容，改错会导致驱动行为异常且难以回滚。用户层封装（`context.rs`）可改。
7. **除非用户明确要求，不要 `git commit` / `push`**。提交风格与版本纪律见[第 10 节](#10-开发流程与版本纪律)。
8. **定位前提**：个人工具，按作者本机（9800X3D 8C16T / 4K 屏）写定；不做其他机器/用户兼容承诺，
   不假设多实例并发。任何「兼容性泛化」改动先问用户。
9. **别追的「假 bug」**：GPU 超频调试期间出现 `egui-wgpu ... Failed to create staging buffer`
   导致的退出，作者已确认是超频/显存分配失败的正常反应（2026-10-23），不是项目缺陷。
   部署目录的 `crash.log` 是 2026-10-22 那次留下的，不要据此立项。
10. **工具链不锁定**（仓库无 `rust-toolchain.toml`，历史上是刻意删的）：当前为
    rustc/cargo **1.101.0-nightly**。换工具链后首次编译需要重编全部依赖（冷 check ≈ 30s），属正常。

## 1. 项目概述

基于 [Interception](https://github.com/oblitum/Interception) 内核驱动的 Windows 游戏输入自动化工具，
从 C++（Visual Studio）项目 `E:\Projects\fmttest` 完整重构为 Rust。服务原神 / 崩铁 / 鸣潮 / 绝区零等。

- 单 GUI 二进制 + 一个库 crate；配置为 exe 旁 `profiles/*.toml`，GUI 面板实时增删改绑定。
- 14 个内置功能（含 3 档「优化游戏」、线程采样工具与时间轴连点器），三种触发模式 Once / Loop / Toggle。
- 输入走 Interception 内核驱动：**只拦截键盘**并全量转发，鼠标不拦截（仅注入）。
- 用户向功能说明、驱动安装、配置示例：见 README（英文）/ README.zh-CN（中文）。

## 2. 技术栈（2026-10-23 实况）

| 依赖 | 版本/特性 | 用途 |
|---|---|---|
| Rust | nightly，edition 2024；实机 1.101.0-nightly（历史长期 1.100，未锁版本） | 全项目 |
| `windows` | 0.62，features：Security / Storage_FileSystem / System_IO / System_Threading / Diagnostics_ToolHelp / Graphics_Gdi / UI_WindowsAndMessaging / UI_HiDpi / UI_Shell / System_LibraryLoader / Media / **Wdk_System_SystemInformation / Wdk_System_Threading / Win32_System_WindowsProgramming / Win32_System_Kernel** | Win32 + NT 查询 |
| `eframe` | 0.36（锁定 0.36.1），`default-features = false`，features **`default_fonts` + `wgpu`** | GUI / 托盘窗口 |
| `wgpu` | 30.0.1（由 eframe 引入），显式锁 **Vulkan** 后端 | GUI 渲染 |
| `toml` + `serde` | 1 / 1 | 配置解析/序列化 |
| `tracing` + `tracing-subscriber` | 0.1 / 0.3（仅 `fmt`） | 日志 → GUI 日志面板桥 |
| `embed-resource` | 3（build-dependency） | 构建期嵌入 `assets/icon.ico` |

编译期注意：`profile.dev.package.libm` 与 `profile.release.package.libm` 都设 `opt-level = 0`，
绕开 LLVM 21 的 SEH bug（llvm/llvm-project#199290，naga→libm 数学链）；上游修复后可移除。

## 3. 目录结构

```
Cargo.toml / Cargo.lock        # 依赖 + dev（极速）/ release（激进）profile
build.rs                        # 嵌入 assets/icon.ico（缺失时警告跳过，构建不失败）
assets/{icon.ico,icon.rc}       # 托盘/窗口/exe 图标原料
README.md / README.zh-CN.md     # 用户向文档（双语，发版时同步）
AGENTS.md                       # 本文档（唯一权威，agent 入口）
LICENSE                         # MIT（正文）+ GPL/LGPL 声明仅覆盖 protocol.rs 移植
LICENSE-GPL.txt / LICENSE-LGPL.txt
.cargo/config.toml              # rust-lld + target-cpu=native + -Z threads=16
.cargo/build-std.toml           # 仅 release 用：build-std = std + panic_unwind
.vscode/settings.json           # 入库（无害）
tools/*.py                      # 一次性分析脚本；被 /tools 忽略但仍被 Git 跟踪（历史遗留）
                                 #   loc.py、thread_sample_analyzer.py
src/
├── lib.rs                      # 库根（GUI 二进制与测试共享全部模块）
├── bin/gi-utils-gui/           # 唯一二进制（目录名 = 二进制名，无 [[bin]] 配置）
│   ├── main.rs                 # 入口 / GUI 面板 / 单实例 / 启动关机 / panic hook (~1.7k 行)
│   ├── tray.rs                 # 托盘线程：消息窗口 + Shell_NotifyIconW + ~200ms 泵 + TrayAction
│   ├── tray_icon.rs            # 图标原料 + SharedIcon（启动预载，防 WIC 污染/销毁竞态）
│   └── window_ops.rs           # HWND 安全包装唯一入口（IsWindow 重校验 + 跨进程 pid 过滤）
├── profile.rs                  # profiles/ 目录、TOML 解析/原子写、功能工厂、参数合成 (~1k 行)
├── key.rs                      # ScanCode(u16) + Key{ScanCode,is_e0} + 90+ 键名常量
├── interception/
│   ├── protocol.rs             # 内核协议端原生移植：20 设备上下文、IOCTL、栈分批零堆分配
│   └── context.rs              # InterceptionContext(recv) + SendContext(send) 类型化封装
├── engine/
│   ├── mod.rs                  # Engine 事件循环（接收→转发→分发）
│   ├── event.rs                # InputEvent + EventSequence 链式 API + HeldTracker
│   ├── bindings.rs             # KeyFunction trait / ParamSpec / KeyBindings / TriggerMode / ActiveGuard
│   └── timeline.rs             # Timeline / RollingKeys 绝对时刻编排（~1k 行，测试最多）
├── utils/
│   ├── delay.rs                # TSC 校准 + 相对/绝对时刻 delay + interruptible
│   ├── beep.rs                 # 同步/异步蜂鸣（退出提示音）
│   ├── affinity.rs             # CPU 亲和性掩码/进程遍历/线程 pin 与优先级
│   ├── thread_info.rs          # NT 快照与线程信息查询（NtQuerySystemInformation 等）
│   ├── thread_pin.rs           # 热线程 pinning 策略表与采样（原神/崩铁/绝区零）
│   ├── screen.rs               # PixelReader（缓存 DC）+ 单点取色（GetPixel）
│   ├── log_collector.rs        # 全局 tracing → GUI 日志面板（有界 FIFO，上限 200 丢最旧）
│   └── mod.rs                  # 初始化（TSC 校准、日志、启动信息）
└── functions/
    ├── mod.rs                  # GAME_PROCESS_NAMES + 共用敲击节奏引擎 click_plan/click_rhythm
    ├── stop.rs                 # 停止退出（Once，设置 engine stop_flag）
    ├── auto_clicker.rs         # 连点器（Loop，interval_ms/hold_ms）
    ├── spam_key.rs             # SpamKey（Loop，key/interval_ms/hold_ms）
    ├── quick_pickup.rs         # 快速拾取（Loop）
    ├── ghost_walk.rs           # 鬼畜走路（Loop，RollingKeys 时间轴）
    ├── mavuika_jump.rs         # 火神跳喷（Loop）
    ├── ganyu_aim_cancel.rs     # 甘雨走A（Once）
    ├── mavuika_double_cancel.rs  # 双玛头（Loop）
    ├── mouse_color.rs          # 坐标颜色（Loop，屏幕取色显示）
    ├── optimize_game.rs        # 优化游戏三档（Once，奇偶 toggle）
    ├── thread_sampler.rs       # 线程采样（Once，诊断工具：写 thread_sample.txt）
    └── timeline_clicker.rs     # 连点器（时间轴）— Timestamp 绝对 TSC 网格（Loop/Toggle）
```

> `.zcode/`、`.workbuddy/`、`.claude/` 是客户端/本地会话产物，不入库（`.workbuddy/memory`
> 的历史会话笔记曾被用户级 ignore 排除，已被作者 2026-09-11 声明「丢了就丢了」）。
> `.zcodeignore` 曾误入库，已在 `bb3f91b` 移除并加入 `.gitignore`。

## 4. 架构

### 4.1 线程模型

| 线程 | 职责 | 亲和性 / 优先级 |
|---|---|---|
| GUI 主线程（winit/eframe） | 渲染、配置面板、托盘动作消费、帧监视器 | LP 12,13；`THREAD_PRIORITY_LOWEST`（22） |
| Engine 线程 | `Engine::run` 阻塞循环：收→转发→分发 | LP 14,15；REALTIME class + 优先级 24 |
| 功能线程（Once/Loop/Toggle） | `KeyFunction::execute(stop_requested)` | spawn 后 pin LP 14,15，优先级 24 |
| 托盘线程 | 隐藏消息窗口 + `Shell_NotifyIconW` + ~200ms 消息泵 | LP 12,13；REALTIME class + IDLE（映射=16） |
| `beep_async` 瞬态线程 | 退出蜂鸣 | 不 pin，可忽略 |

进程掩码 `PROCESS_CORES_MASK = GUI(12,13) | ENGINE(14,15)`；GUI 先 pin 再 spawn 引擎/功能线程，
新线程继承进程掩码后各自收窄。优先级全序：**输入 24 > 渲染 22 > 托盘 16**（2026-08-22 定）。

### 4.2 启动与关机

`main` 顺序（`main.rs`）：

1. `LogCollector::install(200)`（必须早于任何会打日志的步骤）；
2. **单实例保护**：命名 mutex `GIUtilsSingleInstance`——已存在则激活既有窗口（`--silent` 时直接退出、
   不抢焦点），mutex 句柄故意不释放（OS 退出时回收）；
3. `utils::init()`：先 `configure_self`（REALTIME class + 进程掩码 14,15，须早于校准）→
   TSC 校准（20×100ms，约 2s，阻塞启动）→ 日志；
4. GUI 线程 `configure_gui_self()`：pin 12,13 + LOWEST；
5. 配置：legacy 迁移 → `profiles/.last` 记忆 → `load_full_from`（含模板表）；
6. `Engine::new()`：建 recv 上下文（只设键盘过滤器）+ send 上下文。此处在 panic hook 安装之前，
   故用 `catch_unwind` 就地守卫：失败 → `crash.log` + MessageBox（`--silent` 只落盘）+ exit(1)；
7. 安装 panic hook（见下）；注册绑定（`create_function` + `apply_params`）；预载托盘图标；
8. spawn Engine 线程 → spawn 托盘线程 → `eframe::run_native`（wgpu/Vulkan）。

关机（`shutdown_all`，由 `run_native` 返回后显式调用，**不在 Drop 里**）：

```
stop_flag=true → join Engine → 有界等待托盘线程（最多 2s）→ key_bindings.stop_all()
→ pressed::release_into（best-effort，补发残留挂起键）→ beep_async
→ thread_pin::restore() → restore_all_affinity() → 销毁共享托盘图标（仅当托盘已退出）
```

顺序有原因：先停引擎再收托盘避免退出被 2s 等待拖长；`stop_all` 必须在亲和性恢复之前
（防「优化游戏」线程竞态恢复）；线程级 pin 不随本进程退出消失，必须显式还原。

### 4.3 输入路径

- **接收**：Engine `wait_timeout(100)` 阻塞等待，收到键盘设备后循环 `receive_keyboard` 取批
  （`MAX_STROKES_PER_IOCTL` 栈缓冲，返回实际读到的前缀切片）。
- **转发**：每批先整批 `forward_keyboard`（一次 IOCTL_WRITE，减少与功能线程发送的交错），
  再逐条解析 E0/UP 位 → `Key{ScanCode,is_e0}` → `process_key_down/up`。
- **鼠标**：拦截过滤器只设键盘，鼠标事件不进入队列（`Device::Mouse` 分支不可达）；鼠标注入走
  `SendContext` 直接发送。
- **发送**：`SendContext` 是独立类型（`unsafe impl Sync`，可被多个功能线程同时持有/发送），
  `send_event`/`send_events` 当前为**逐事件一次 DeviceIoControl**（v1.7.6「取消切片」）。
  因此不同功能线程的发送**可以交错**，down/up 之间不保证驱动级原子。
- **panic 挂起键兜底**：`src/interception/pressed.rs` 原子位图——每次注入前登记 DOWN、UP 写成功
  才清；panic hook 在停机后与 MessageBox 后各补发一遍 release；物理按键转发（`forward_keyboard`）
  不登记。
- **挂起键清理**：`EventSequence::play` 内置 `HeldTracker`（按下记录/松开移除/stop 时合并补发）；
  时间轴执行器退出时 `release_pending` 补发 release，防卡键。

### 4.4 绑定与触发

`KeyBindings`：`HashMap<Key, Entry>` 注册表 + 去抖表 + 捕获通道（GUI Set Key）+ `pending_joins`
（已停止线程句柄队列，`is_finished` 惰性 join，GUI 帧绝不阻塞）。

- `Entry` 持有 `Arc<dyn KeyFunction>`、`active`、`stop_requested`、`handle`。
- `TriggerMode`：`Once`（按一次执行一次）、`Loop`（按住循环，松开停）、`Toggle`（按一下开、
  再按一下停）。
- `ActiveGuard`：Drop 时清 `active`，线程 panic 也不会留下「永远 active」的僵尸条目。
- **stop 语义正向**：`stop_requested = true` 表示停止，全项目统一。
- 锁序单向 `bindings → pending_joins`，禁止嵌套反转（无死锁）。
- 同一功能可绑多个键（v1.5.0 起，每个绑定行是独立实例/toggle）；每行独立参数自 v1.7.3；键必须唯一。

### 4.5 时间轴

- `Timeline`：每条目是「播放起点 TSC + 偏移」的绝对时刻，无累计漂移；已触发条目立即移除，
  播放中可增量追加（`partition_point` 捕获），语义对齐 MIDI 实时编辑。
- 表空不结束：以 0.5ms 轮询等待编辑器追加，结束只由 `stop_requested` 决定。
- `RollingKeys`：按下实时产生、释放动态排程；卡顿后节拍重锚（错过即弃，不突发追拍——有意偏离 C++）。
- 代表：鬼畜走路；未来钢琴模式。Serial 类功能用 `EventSequence` 链式 API（`tap/sleep/wheel` 等）。

### 4.6 GUI 与托盘

- GUI 面板：绑定表格（增删/改键/捕获/参数 ⚙）、Profile 下拉 + New/Save、功能参数面板、日志面板。
- 修改 live-apply：Save/改键会 `clear_all` + 全量重注册；**参数**是例外——GUI 直写原子参数槽，
  下个周期生效，不重注册、不打断运行中的功能。
- 日志面板 header 有 `Clear` 按钮：一次清空 GUI 已显示行与 `LogCollector` 尚未 drain 的行
  （否则下一帧会把缓冲旧日志补回来）；`✕` 只隐藏面板。
- 关闭窗口 = `ShowWindow(SW_HIDE)` 隐藏到托盘（不退出）；托盘菜单：显示窗口 / Profile 二级菜单
  （当前项打勾）/ 退出。F12「停止退出」= 置 `stop_flag` → 帧监视器 → `post_close` 走完整关机。
- 托盘动作经 `mpsc<TrayAction>` 交给 `eframe::App::logic` 消费（隐藏态 `request_repaint_after(500ms)`）。
  注意：隐藏用 `ShowWindow(SW_HIDE)` 不改 winit 的 visible 标志，隐藏态 `ui()` 是否仍执行**未单独测量**；
  两条路径都会消费托盘动作，行为已实机验证（2026-09-20），机制细节存疑但无害。
- 渲染：wgpu + Vulkan，eframe 显式 `Renderer::Wgpu`；睡眠唤醒时 surface Lost 由 egui-wgpu 逐帧
  `RecreateSurface` 自愈。**旧的 glow/WGL 的 catch_unwind 自愈循环已退役（v1.7.0）**。

## 5. 核心设计决策

| 决策 | 理由 / 现状 |
|---|---|
| **Key = ScanCode + is_e0** | 单一类型消除 PS/2 值冲突，E0 自动注入 state；键名映射在 `key.rs` |
| **SendContext 独立类型** | 发送/接收分离，编译器禁止并发接收；发送侧可多线程共享 |
| **用户层协议原生移植** | 替代预编译 `interception.lib`：类型化收发切片、栈分批零堆分配、`Result` 错误传播、`set_filter` 闭包、Drop 清理——**内核驱动协议未改** |
| **wgpu + Vulkan 后端** | v1.7.0（2026-09-05）睡眠唤醒修复：WGL 上下文跨电源转换永久失效是 glow/glutin 的结构问题；wgpu surface 可逐帧重建。**渲染自愈体系随之退役**，panic hook 退化为纯 fail-fast |
| **取消输出切片（no-slicing，v1.7.6，最终形态）** | `send_events` 逐事件一次 IOCTL（Sleep 空操作）；不同功能线程的发送可交错，连点 down/up 无驱动级原子性；性能影响可忽略（10ms 周期连点 ≈ 200 请求/s）。**2026-10-23 作者确认现行使用无问题，保留为最终形态，不再回退**；历史切片实现见 `f621bf3^`（仅存档）。`ClickPlan::AtomicBatch` 已更名 `BackToBack` 以消除「原子批」歧义 |
| **stop_requested 正向语义** | `true`=停止，全项目无双重否定 |
| **ActiveGuard Drop 防护** | 线程 panic 时自动清 active（panic=unwind 下真正生效；build-std 也保持 unwind） |
| **Mutex 外 join / pending_joins 惰性 join** | stop 不阻塞事件循环；GUI 帧只 `is_finished`，绝不阻塞 |
| **delay_ms_interruptible** | 100μs 检查间隔，Loop/Toggle 即时响应停止 |
| **EventSequence 链式 API** | `seq.tap(K).sleep(50).wheel(DOWN)`；`play()` 内建 HeldTracker |
| **时间轴绝对时刻调度** | deadline = 起点 TSC + 偏移（饱和加法），无累计漂移，前序超时自然追时 |
| **已触发即移除 + 增量同步** | 表恒为未触发条目；追加条目在到期帧被 `partition_point` 捕获，不重复不遗漏 |
| **表空不结束（live-edit）** | 0.5ms 轮询等追加；结束只由 stop_requested 决定（MIDI 编辑器语义） |
| **RollingKeys 节奏滚动** | 按下实时产生、释放动态排程；卡顿节拍重锚（错过即弃、不突发追拍，有意偏离 C++） |
| **KeyFunction 1 必需 + 2 默认方法** | `execute(stop_requested)` 必需；`parameters()`/`param_store()` 默认空实现（无参功能零改动） |
| **动态参数直写** | 参数槽数量/类型实现期固定（Float/Int/Bool）；`ParamValues` 全原子（f64 存储位模式于 AtomicU64）；GUI ⚙ 直写 store，下周期生效；`apply_params` 显式拒绝 NaN/Inf（clamp 不滤 NaN，会穿透到下游 assert panic） |
| **参数持久化 = 模板 + 差量** | 顶层 `[params.<功能名>]` = 模板（活基线，同功能所有行共享）；行内 `[bindings.params]` = 差量（只存与模板不同的槽）。**Save 成对写回模板段 + 差量**；只写差量会在重启后丢参数（v1.7.4 高危修复）。GUI 切换功能/新建行以模板为初值（A+ 语义） |
| **Profile 体系 + 记忆** | exe 旁 `profiles/<名>.toml`；`profiles/.last` 记录上次活动 profile，启动恢复（失效回退 `默认` 并回写）；旧 `gi-utils-config.toml`/`config.toml` 自动迁移、旧文件保留 |
| **切 profile 共用 switch_profile** | 窗口下拉与托盘同一实现 + 三守卫：同名幂等（避免无谓 `clear_all` 打断 Loop）、dirty 拒绝、按键捕获中拒绝（rebuild 按行索引会错位） |
| **GUI live-apply + 托盘隐藏** | 修改即时生效；关闭隐藏到托盘，F12/托盘菜单退出 |
| **锁序 bindings → pending_joins** | 全部路径同序，无嵌套反转 |
| **托盘图标 / exe 图标 / WM_SETICON** | `[gui] icon_path` 指定 .ico（失败回退程序生成蓝 G）；build.rs 嵌入 exe 图标；托盘线程找到主窗口后用同一 HICON 覆盖任务栏/标题栏/Alt-Tab |
| **托盘动作走 App::logic** | 隐藏态 500ms 周期唤醒；Exit 分支另有 `post_close` 兜底。实机验证通过（2026-09-20），机制细节未单独测量 |
| **时间轴连点器** | `timeline_clicker.rs`：Timestamp 范式鼠标左键连点，`next_down` 绝对 TSC 网格（+interval）推进、释放独立排程；按下间隔严格锚定 interval（发送抖动 µs 级），卡顿越过下一拍重锚（错过即弃、不突发）；`hold` 钳到 `[0, interval]`、hold==interval 时先补发上一拍 release；等待期 ≤1ms 轮询 interval，缩短时立即重锚下一拍；`debug_sample`（Bool 默认关）每 100 个间隔输出一次统计（仅调试）；推荐 Toggle/Loop，Once 不适用 |
| **敲击节奏引擎共用** | `functions/mod.rs`：`click_plan`（纯决策）+ `click_rhythm`（唯一执行）——连点器与 SpamKey 此前各一份复制（项目典型「两处复制→修复漂移」风险面），2026-09-20 抽取；`hold` 钳到 `[0, interval]` |
| **wait_until_interruptible 单次 TSC 读** | 到期判定与检查点复用同一读数；本轮循环是 Loop/时间轴热路径，实测每轮 65→61 周期（固定窗口自旋 +7%，6/6 轮一致）；最坏晚一轮（~15ns）触发 stop 检查 |
| **线程级核心分离** | 进程掩码 12-15；GUI 12,13 LOWEST；输入/功能 14,15 REALTIME 24；托盘 16 |
| **热线程 pinning 金银核** | 进程名 → 模块白名单（`"模块名+0x"` 前缀）双采样 Δcycles 降序 Top-2 → 金核 A/B 的 LP 对；现注册原神/崩铁/绝区零（详见 `thread_pin.rs`）；未注册游戏只 HIGH + OTHER 隔离、不 pin；**终末地有意不注册**（反作弊封锁模块枚举，白名单不可用）；SET 权限被拒逐条降级。退出/恢复/panic 三路径兜底还原 |
| **优化游戏三档** | Advanced = 游戏亲和性 + OTHER 隔离 + HIGH + 前台 + pin；Standard = OTHER 隔离 + HIGH + 前台（不改游戏亲和性、不 pin）；Minimal = 仅 HIGH + 前台。奇偶 toggle 按模式独立（`static [AtomicBool; 3]`）；偶数次恢复**有意不降游戏优先级**（降级要再开游戏句柄，反作弊路径；HIGH 留存无害） |
| **优化游戏找窗三级序** | ① 已登记名单（`GAME_PROCESS_NAMES` 7 游戏，进程名→pid→可见主窗口，有标题优先）→ ② 窗口类兜底（UnityWndClass / UnrealWindow）→ ③ 失败；名单优先覆盖窗口类不明的游戏。换游戏检测：hwnd/pid 过时即重捕获并走优化方向 |
| **TSC 校准 20×100ms 阻塞启动 ~2s** | 有意保持：启动一次性成本换最大样本稳健性（实测散布 ±1.45ppm，最差样本对 10ms 时序误差 15ns 级）；QPC 缩窗方案（~100ms）评估过不采用 |
| **单实例 mutex** | 命名 mutex，第二实例激活既有窗口（`--silent` 则静默退出）；句柄泄漏到进程结束，保排他 |
| **panic hook = fail-fast + 补发** | 任意线程 panic：置 stop → 补发挂起键#1 → `crash.log`（`try_snapshot` 非阻塞，防日志锁死锁）→ 还原 thread_pin（**try_lock 版，自身绝不死锁**）/affinity → MessageBox → 补发#2 → `exit(1)`（unwind 下线程静默死亡会让 Loop 继续注入、F12 失效）。启动期 `Engine::new` 由 catch_unwind 守卫。 |
| **panic 挂起合成键补发（原子位图）** | `pressed.rs`：64B 对齐 512-bit 键盘位图（256 ScanCode × E0）+ 5 鼠标键位；`before_send` 登记、`after_send` 写成功才清；`release_into` = swap 快照 → 逐条 `send_untracked`（不触发登记/误清并发新 press）→ 失败位重新登记供第二遍/关机重试；hook 两遍补发，空表零 IOCTL。局限：仅 unwind、快照 vs in-flight 竞态、同键多持有者单 bit、>0xFF 扫描码忽略 |

## 6. 构建

```bash
# 日常（dev profile，增量秒级）—— 默认路径
cargo check                 # 类型检查（0 warning 是当前基线）
cargo test                  # 93 单测 + doctest
cargo build                 # target/debug/gi-utils-gui.exe（部署目录 gi-utils-dev.exe 指向它）
cargo fmt --check           # 必须干净（2026-08-19 起全项目格式化）

# 发版（仅用户明示「发版」时）
cargo build --release --config .cargo/build-std.toml
# 输出 target/release/gi-utils-gui.exe（当前约 13.7MB）
```

- `build-std` 用 `panic_unwind` 变体 + 本机 `target-cpu=native`/O3 重编 std，**只能通过 `--config`
  按需引入**：放进全局 `config.toml` 会让 `cargo test` 为 dev+test 各编一份 std，链接报
  duplicate lang item（cargo 已知问题）。
- 不用 `panic_immediate_abort`：会跳过 panic hook，破坏亲和性恢复兜底。
- release profile：`opt-level=3, lto=fat, strip=true, codegen-units=1, panic=unwind, incremental=false`。
- dev profile：`opt-level=0, debug=0, codegen-units=256, incremental`；依赖统一 `opt-level=2`（GUI 流畅）。
- 链接器 `rust-lld.exe`；rustflags `-C target-cpu=native -Z threads=16`。
- 依赖裁剪：eframe 仅 `default_fonts`+`wgpu`（无 accesskit/links/persistence/x11/wayland）；
  tracing-subscriber 仅 `fmt`。
- `cargo clippy` 不是门禁：历史约 21 项风格 lint（缺 Default、collapsible_if 等）**有意不修**，
  无功能错误；不要为了清零而改语义。
- 产物**不含版本资源**（无 FileVersion/ProductVersion），窗口标题用 `CARGO_PKG_VERSION` 显示版本。
- 本机 cargo 1.101-nightly 的 build-dir 布局与旧版不同（`target/debug` 下无 deps/.fingerprint，
  测试产物落在 `target/debug/build/gi-utils/<hash>/out/`）——已用空 crate 复现，**不是项目问题**。

## 7. 运行与部署

- **必须管理员身份运行**；需已安装 Interception 驱动（安装步骤见 README）。
- 命令行参数：`--silent` = 静默启动（不显示配置窗口，仅托盘 + 引擎；登录自启用；
  不抢焦点；第二实例 `--silent` 直接退出不打断已有实例）。
- 部署目录 `E:\Program\GI-Utils` 两个符号链接（链接本身固定，指向最新构建产物）：

```
gi-utils-gui.exe → E:\Projects\Rust\GI-Utils\target\release\gi-utils-gui.exe   # 发版
gi-utils-dev.exe → E:\Projects\Rust\GI-Utils\target\debug\gi-utils-gui.exe     # 日常验证
```

- 配置与用户数据（`profiles/`、`crash.log`、`thread_sample.txt`）都写在 **exe 旁**，即部署目录。
  部署目录里的 `verbose.exe`/`zzz-utils.exe`/`gi-utils-config.toml` 是历史遗留，不属仓库。
- 2026-10-23 发布 **v1.7.7**：release 13,693,440 B（13.69MB，SHA256 `8CD60E65…FBF3`）、
  debug 19.39MB；部署符号链接自动指向新 release。开发后记得刷新 debug（见规则 3）。

## 8. 配置系统

### 8.1 文件与合成规则

- 位置：exe 旁 `profiles/`，一个 `*.toml` 一个 profile（文件名即 profile 名）；`profiles/.last`
  记录上次活动项。profile 名禁止路径分隔符/控制字符（防目录穿越）。
- 加载：`load_full_from` 合并「模板 + 行内差量」→ 每行生效参数；返回模板表供 GUI 使用。
- 保存：`save_to` 原子写（临时文件 + rename），成对写回**模板段**与**行内差量**；差量只包含与
  模板不同的槽（改回模板值 → 差量消失 → 跟随模板）。
- 无参功能上残留参数：静默忽略（历史数据/手写）；**功能有参数但键名写错**：报错（typo 检测）。
- `apply_params` 对 Float/Int/Bool 做类型匹配；**Float 槽接受 Integer**（手写 `interval_ms = 10`
  宽容转为 10.0），Int 槽只接受 Integer（键槽另接受键名字符串）；NaN/Inf 显式拒绝。
- 功能名含全角括号（如 `连点器（时间轴）`）时，手写 TOML 的参数表名必须加引号：
  `[params."连点器（时间轴）"]`（GUI Save 会自动加引号）。

### 8.2 格式示例

```toml
[[bindings]]
key = "F13"
func = "连点器"
mode = "Loop"

# 行内差量：仅与模板不同的槽
[bindings.params]
hold_ms = 5.0

# 功能参数模板（活基线，同功能所有行共享；GUI Save 一并写回）
[params."连点器"]
interval_ms = 10.0
hold_ms = 0.0

[gui]
icon_path = "E:/Projects/Rust/GI-Utils/assets/icon.ico"  # 留空回退程序生成图标
font_path = ""                                            # 留空回退系统 CJK 字体 msyh→simsun
```

### 8.3 参数类型

- `Float`：f64，带 min/max/step，GUI DragValue 按范围钳制。
- `Int`：i64，带 min/max；**键槽**是特例——低 16 位 ScanCode、位 16 为 E0，合并为整数持久化；
  GUI 对 `key` 槽渲染为键名下拉 + Set Key 捕获。
- `Bool`：开关。
- 参数默认值来自 `ParamSpec`；未写进配置的槽取默认。

## 9. 功能清单（14 项）

| 功能 | 触发 | 模型/备注 |
|---|---|---|
| 停止退出 | Once | 置 engine `stop_flag` → 完整关机；**不是** `create_function` 分支（main 特判，注入 stop_flag） |
| 连点器 | Loop | 鼠标左键；`interval_ms`(1–10000, 默认10) / `hold_ms`(0–interval, 默认0)；Serial |
| 连点器（时间轴） | Loop / Toggle（推荐 Toggle） | 鼠标左键；`interval_ms` / `hold_ms` / `debug_sample`（Bool 默认 false，每 100 次输出间隔统计）；Timestamp 绝对 TSC 网格——按下间隔严格 = interval（无累计漂移、卡顿即弃不追拍）；与 Serial 连点器并行 |
| SpamKey | Loop | 任意键；`key`(打包 Int) / `interval_ms`(默认50) / `hold_ms`；Serial |
| 快速拾取 | Loop | F + 滚轮下拉；Serial |
| 鬼畜走路 | Loop | WASD 交错短按；**RollingKeys 时间轴** |
| 火神跳喷 | Loop | 初始跳跃 + 空格连跳；Serial |
| 甘雨走A | Once | 左右键连点 + R 取消后摇；Serial |
| 双玛头 | Loop | 复杂鼠标序列 + S；Serial |
| 坐标颜色 | Loop | 光标坐标 + 屏幕像素颜色显示；按住有效 |
| 优化游戏 | Once（奇偶 toggle） | Advanced：亲和性 + OTHER 隔离 + HIGH + 前台 + 热线程 pin |
| 优化游戏标准 | Once（奇偶 toggle） | OTHER 隔离 + HIGH + 前台（不改游戏亲和性、不 pin） |
| 优化游戏简易 | Once（奇偶 toggle） | 仅 HIGH + 前台 |
| 线程采样 | Once | 诊断工具：两次采样差分 → 写 exe 旁 `thread_sample.txt` + 日志摘要；目标按 `GAME_PROCESS_NAMES` 扫描 |

默认配置（首次运行生成）绑 10 条：F12 停止、F13 连点器、F14 快速拾取、F15 鬼畜走路、F16 火神跳喷、
F17 甘雨走A、F18 双玛头、F19 坐标颜色、NumpadAdd 优化游戏、F20 线程采样；`[params."连点器"]`
带默认值。SpamKey / 优化游戏标准 / 优化游戏简易 默认不绑定，GUI 里可选。

## 10. 开发流程与版本纪律

### 10.1 提交与版本

- 提交风格：`feat:` / `fix:` / `chore:` / `docs:` / `refactor:` / `perf:`（可 `feat(scope):`），
  中文说明为主。历史纪律：**每个功能/修复后紧跟一个版本号提交**
  `chore: 版本升至 vX.Y.Z — 主题`（minor=功能、patch=修复）——沿用。
- 版本号唯一来源 `Cargo.toml`；升版本后 `Cargo.lock` 同步（`cargo build` 会更新）。
- **tag**：v1.3.0 后长期缺 tag；2026-10-23 已补 `v1.7.6`（release 状态 `bb3f91b`）、
  `v1.7.7`（`dd519fd`，v1.7.7 release 构建源）与 `bench-getpixel-archive`（本地存档
  `2c949b5`）。今后发版：升版本 → 构建 → 打 tag。
  注意历史上文档曾引用不存在的「v1.5.2」——**不要在不存在的版本上写记录**。
- 发版清单（用户明确要求时）：升版本 → `cargo build --release --config .cargo/build-std.toml` →
  更新 README 的体积/测试数/功能数 → 更新本文档变更 → 建议打 tag。
- 测试：93 个单测（lib 89：timeline 23 / profile 13 / bindings 9 / protocol 6 / event 4 / delay 3 /
  functions/mod 3 / thread_pin 2 / pressed 15 / timeline_clicker 9 / log_collector 2；GUI bin 4）+ 2 doctest（1 ignored）。**测试缺口**：
  SendContext 发送路径（需驱动）、thread_info NT 解析、optimize_game、log_collector；新增改动
  优先补可测纯函数。
- 未跟踪分支 `bench-getpixel`（本地、未推送，2026-09-29，4 提交 +1362 行）：只含一次性诊断 bin
  （`getpixel-bench` / `getpixel-compare`），**不碰生产 `screen.rs`、不建议 merge**（windows
  feature 膨胀 + 死代码）。其结论已归档在[第 12 节](#12-路线图)，作为性能决策依据；分支保留作存档
  （2026-10-23 已打本地 tag `bench-getpixel-archive`）。

### 10.2 实机验证

Agent 不启动程序（见规则 5）。需要验证时：用户以管理员运行 `E:\Program\GI-Utils\gi-utils-dev.exe`，
关注 GUI 日志面板 / `crash.log` / `thread_sample.txt`。改动涉及输入时序/线程 pin/亲和性时，
必须请用户实测确认，不要仅凭单测下结论。

## 11. 已知问题与技术债

> 这些是**已知且已评估**的项，接手时不要重复发现、也不要未经确认就大改。

1. **无切片语义已定稿**（2026-10-23 作者确认）：保留现状、不恢复切片；全部过时注释已同步，
   `ClickPlan::AtomicBatch` → `BackToBack`。发送路径仍无驱动级回归测试（IOCTL 依赖驱动，
   无驱动不可测）——发送语义变更需人工实机验证。
2. **panic 补发限界**（详见 `pressed.rs` 头注释）：仅覆盖 panic=unwind 路径（kill/断电不补发）；
   快照与其它线程 in-flight 发送的竞态由两遍补发降低、不能根除；同键多持有者只记 1 位；
   >0xFF 扫描码不跟踪；hook 内发送失败时 `warn!` 仍可能触碰日志锁（`try_snapshot` 只护 crash.log）。
3. **启动守卫覆盖范围**：`catch_unwind` 只包 `Engine::new`；同阶段仍有 `LogCollector::install`
   的 expect、legacy 迁移的 `current_exe()` expect、`profile_path` expect 等静默 panic 面
   （后两者需手改 `.last` 才可达）。根治 = 最小 panic hook 提前到日志安装后（低危，未做）。
4. **Toggle 快速连按**：active 未清的极短窗口内第三次按下可能既不 spawn 也不 stop（低危，未复现）。
5. **注释残留（剩余）**：`main.rs` 若干「崩溃恢复/恢复轮」历史注释（当前约 93-120、1343-1345、
   1550、1585、1659-1699 行）属已退役自愈体系，改这些文件时顺手修。其余已知漂移已于
   2026-10-23 清理。
6. **依赖升级未评估**：eframe 0.36.2（2026-09-08）/ 0.37.0（2026-10-15）存在，仓库仍锁 0.36.1；
   升级涉及 wgpu 版本跃迁 + API 变化，需单独评估（不要顺手 bump）。
7. **仓库卫生**：无 CI、无 `rust-toolchain.toml`；`tools/*.py` 被 `/tools` 忽略但仍被跟踪
   （改 ignore 规则时注意）；`/target` 不跟踪。tag 情况见 10.1。
8. **本机 git 维护被关闭**（仅此机器，不随仓库传播）：2026-09-11 对象库损坏事故后设置了
   `maintenance.auto=false`、`gc.auto=0`，备份已删，根因未证实（疑 FS/AV 干扰）。不要在未确认时
   跑 `git gc --aggressive`。
9. **GPU 超频崩溃**：2026-10-22 `crash.log`（egui-wgpu staging buffer panic）是超频调试期的
   显存分配失败（作者 2026-10-23 确认），不是代码缺陷；稳定运行下无需处理。

## 12. 路线图

### 12.1 未移植功能（C++ 参考 `E:\Projects\fmttest\main.cpp`）

| 功能 | 优先级 | 难度 | C++ 参考行 | 备注 / 设计提示 |
|---|---|---|---|---|
| 甘雨加特林 | 1 | ★★ | 448 | R + 鼠标移动序列 |
| 克洛琳德 | 2 | ★★★ | 624 | 单点 GetPixel + R 通道 0xFF 预过滤 + 余弦相似度 >0.98（C++ 677-693）；单点触发用现有 `screen.rs` 足够 |
| 添加好友 | 3 | ★★★ | 321 | 屏幕坐标 + 绝对鼠标（C++ 用 UINT16_MAX 虚拟屏幕映射，360-400 行） |
| 申请加入 | 4 | ★★ | 232 | 鼠标序列 |
| 2048 系列 | 5 | ★ | 835 / 863 / 889 | 纯按键序列（左下/右下/震荡） |

已淘汰：龙王喷水组（C++ 113-229，曾于 2026-08-10 落地后 `ca96559` revert）、火神跳飞（493）。

移植流程：① 读 C++ `EventSequence` 构造 → ② `src/functions/` 新建文件、中文 struct 名照搬 →
③ 实现 `KeyFunction` → ④ `profile.rs` 的 `create_function` 与 `list_function_names`（必要时
`DEFAULT_CONFIG`）各加一处 → ⑤ 走配置驱动，无需改入口。参考模板：连点器（Loop+参数）、
甘雨走A（Once）、火神跳喷（on_activate+Loop）。

### 12.2 组合键注册（★★，未开始）

支持 `Ctrl+F13`/`Alt+E` 等修饰键组合。要点：`KeyBindings` 跟踪所有键实时按下状态；
`process_key_down` 时检查修饰键是否已按住；修饰键松开不影响已触发的功能；兼容单键注册。
主要改动 `bindings.rs`（状态追踪）与 `profile.rs`（解析 + 持久化）。

### 12.3 取色性能结论（bench-getpixel 归档）

VSync/G-SYNC 下 `GetPixel` 恒约 **6.25ms/读**（≈160 QPS，合成层节流，非调用开销）；缓存 DC/点位
形态无差异。200×200 小块快照 GDI≈DXGI（噪声内）；**4K 全屏快照 GDI 25.5ms vs DXGI 6.6ms（3.8×）**。
结论：单点/单帧触发（如克洛琳德）用现有 `screen.rs` 足够；**多点/全屏分析必须走 DXGI 全屏快照，
不要逐点 `GetPixel`**（20 点逐点 125ms vs 一次抓取 6.25ms）。注意 `screen.rs` 头部注释
「~0.01ms/次」在 VSync 下失真约 600 倍（该注释已于 2026-10-23 更正）。分支已打本地存档 tag
`bench-getpixel-archive`（2026-10-23），不 merge。

### 12.4 其他候选

- 钢琴模式（时间轴 + 绝对时刻），设计上已具备基础，未实现。
- 组合键之外的宏录制/回放：无计划，除非用户提出。

## 13. 历史与选型附录

### 13.1 选型（原 TECHNICAL_PLAN.md，已删除）

- 输入拦截三路线：**A) Interception 内核驱动（采用）**——游戏兼容性最好，C++ 原项目已验证；
  B) Win32 SendInput（enigo/winput）——大量游戏忽略注入；C) 用户态 hook（inputbot/rdev）——
  反作弊易拦截。内核级驱动对游戏输入助手不可替代。
- 用户层 API 自行实现（不绑社区 interception-sys），2026-08-19 进一步演化为**原生 Rust 移植**
  `interception/protocol.rs`，删掉 `interception.lib`，零 C 依赖。
- **OpenInputBridge**（2026-08-19 调研）：社区干净室重写的协议兼容驱动（MIT 源码、WHQL 签名但
  分发付费），协议逐字节兼容已验证；**不切换**（原版免费稳定），未来原版不可用时成本仅装驱动。
- 与早期方案的偏差（以代码为准）：未用 CancellationToken（`Arc<AtomicBool>` 更轻）；edition 2024；
  EventSequence 用 enum 设计落地；时间轴调度器为重构后新增范式。

### 13.2 版本时间线（164 提交，2026-06-23 → 09-30，19 个活跃日）

- 06-23 **v1.0.0**：C++→Rust 移植落地（8 功能 / TOML / 48 项 review），当天 11 提交。
- 08-10~14 **v1.1.0**：时间轴引擎 + 龙王喷水组（同日 revert）。
- 08-15 **v1.1.1**：GUI 崩溃自愈（glow/wgl 路线，v1.7.0 退役）。
- 08-16 **v1.2.0/1**：GUI+托盘重写、GUI/输入核心分离、edition 2024。
- 08-16~19 **v1.2.2~v1.2.5**：DeepSeek 审查、连点器双版归一、Interception 原生移植。
- 08-20 **v1.3.0**：文档终审 / 目录规范 / LICENSE（之后不再打 tag）。
- 08-22 **v1.4.0**：线程采样 + 三游戏热线程 pinning + OTHER 0xF0F0 隔离。
- 09-05 **v1.5.0 → v1.7.2**：动态参数 + SpamKey、一功能多键、Profile 体系、
  **wgpu+Vulkan（自愈退役）**、25 项审查。
- 09-06 **v1.7.3**：参数模板 + 差量模型（顶层模板 + 行内差量）+ 每行独立参数（per-binding）。
- 09-16 **v1.7.4**：A+ 参数语义 / 差量落盘 / Profile 记忆。
- 09-20：工作流审查 26 项处置（`b19de74`），托盘 Profile 菜单。
- 09-22 **v1.7.5**：`--silent` 静默启动 + 双语 README 刷新。
- 09-30 **v1.7.6**：**取消输出事件切片**（`f621bf3` 试验 → `1e1413a` 转正）+ Cargo.lock 同步。

### 13.3 审查历史（压缩；报告正文未入库，只留本台账）

- 2026-08-16 DeepSeek 审查 20 项：17 修、2 有意不修（全核掩码 / 恢复不降游戏优先级）、1 驳回。
- 2026-08-19 原生移植审查 14 项全处置。
- 2026-09-20 五阶段工作流审查 26 项：2 高危（同一根因：Save 未写回模板段 → 重启丢参数）已修；
  3 轮优化（节奏引擎共用 / 单次 TSC 读 / 托盘动作走 logic）；10 项低危**有意不修**：
  手写 `.last` 指向保留设备名才可达的 expect、无参功能残留参数 debug 日志、`params_diff`
  结构相等、键槽捕获硬编码 `"key"`、托盘被拒反馈仅 UI pass 可见、`config_ok` 未复位、
  `wide()` 重复实现、`main.rs` 过时注释、`profile::save/load_full` 无调用者（2026-10-23 已删）、profile 测试覆盖缺口。
- 2026-09-11 事故：仓库对象库损坏 → 恢复中 `git gc` 被 SIGTERM → 原地重建；本地关闭自动维护
  （见技术债 9）。**教训**：09-08 的 CLAUDE.md 漂移修复当时只是**未提交改动**，09-11 仓库重建后
  工作区回落到 `f1a7d8d`（09-07 提交）而丢失，成为旧文档大面积过期的直接原因 →
  本次重写为 AGENTS.md 并确立「改行为同提交改文档」规则。

### 13.4 许可

`LICENSE` = MIT（项目主体）；`LICENSE-GPL.txt` + `LICENSE-LGPL.txt` 仅覆盖
`src/interception/protocol.rs` 的 Interception 用户层移植（见 `LICENSE` 第 25-33 行）。
Interception 内核驱动的商业授权不在本仓库，使用前自行确认（README 已声明）。

---

**文档变更**：2026-10-23 v1.7.6 全量重写（替代 CLAUDE.md，修正 glow/自愈/测试数/体积/结构树/
幽灵版本等漂移，补 no-slicing、取色 bench 结论、技术债清单与 agent 规则）。
2026-10-23 Phase 1：无切片定稿并同步注释、ClickPlan 更名、删除 profile 死 API、补 tag。
2026-10-23 Phase 2：启动失败可视化（catch_unwind 守卫 + `--silent` 只落盘）；panic 挂起合成键
补发（`pressed.rs` 原子位图 + 逐条无登记发送 + 失败重登记 + hook 两遍补发 + 正常关机兜底 +
thread_pin try_lock 还原）；测试 63→82 单测（lib 78 + GUI bin 4）。
2026-10-23 发版 v1.7.7：作者实机验证正常；release 13,693,440 B（SHA256 8CD60E65…FBF3）。
2026-10-23 新增连点器（时间轴）：Timestamp 绝对 TSC 网格（按下间隔严格 = interval），
与 Serial 版并行；测试 82→89 单测（lib 85 + GUI bin 4）。
2026-10-23 v1.8.1：连点器（时间轴）增加仅调试用采样（`debug_sample`，默认关；每 100 个
按下间隔输出目标/实际均值、min/max、max|偏差|）；测试 89→91 单测（lib 87 + GUI bin 4）。
2026-10-23 v1.9.0：GUI 日志面板增加 `Clear` 按钮（清空显示行 + collector 缓冲；`✕` 仅隐藏）；
测试 91→93 单测（lib 89 + GUI bin 4）。
