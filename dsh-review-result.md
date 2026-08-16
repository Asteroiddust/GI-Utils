# GI-Utils Review Result — dsh-dev

> 审查分支：`dsh-dev`
> 审查基准：`e15bf9f`（与 `master` 一致）
> 审查时间：2026-08-16
> 审查方式：人工逐文件阅读 + `cargo test` / `cargo build --release` / `cargo clippy` / `cargo fmt --check`

---

## 0. 处置记录（2026-08-16，dsh-dev）

修复后 `cargo test`：26 单测 + 2 doctest 全部通过。

| 编号 | 处置 | 说明 |
|---|---|---|
| 2.1 | ✅ 已修 | `sync` 守卫/切片只看 `seen`；`base + seen` 仅用于绝对索引换算；新增前缀清理后追加回归测试 |
| 2.2 | ✅ 已修 | `move_absolute` 文档标注归一化语义；新增 `normalize_absolute` 换算辅助 |
| 2.3 | ✅ 已修 | headless `run()` 返回后先 `stop_all` 再蜂鸣/恢复亲和性（对齐 GUI shutdown 顺序） |
| 3.1 | ✅ 已修 | `PixelReader` DC 改 `ManuallyDrop` 泄漏策略（跨线程 ReleaseDC 无合法路径，与托盘图标同策略） |
| 3.2 | ⛔ 不修（有意） | 全核掩码保持。实测缩减游戏可用核心数在部分游戏有较大性能下降和频繁 stutter，缩减 OTHER_CORES_MASK 也有奇怪现象 |
| 3.3 | ✅ 已修 | `Engine::run` 每轮迭代 `drain_pending_joins`（headless 不再累积句柄） |
| 3.4 | ⛔ 不修（有意） | 恢复不降优先级：降级需再次 OpenProcess 游戏句柄（反作弊拦截路径），HIGH 留存无害 |
| 3.5 | ✅ 已修 | NIM_ADD 前 quit 抢先检查 — stop 与图标添加的竞态不再留下孤儿托盘图标 |
| 3.6 | ✅ 已修 | 托盘 spawn 失败分支 + `Ready(false)` 两处复位 hidden；`Ready(false)` 武装 Show 重试拉回窗口 |
| 3.7 | ✅ 已修 | `find_main_window` 增加 `GetWindowThreadProcessId` 本进程过滤 |
| 3.8 | ✅ 已修 | `register` 替换已占用键先 `signal_stop` + 句柄入 pending 队列 |
| 3.9 | ✅ 已修 | `spawn_once` 存储句柄、退役句柄返回调用方入 pending 回收 |
| 4.1 | ✅ 已修 | `delay_ms` 饱和加法 |
| 4.2 | ✅ 已修 | `scroll` 的 `times` 先 clamp 到 `i16::MAX` 再饱和乘 |
| 4.3 | ✅ 已修 | `WM_DESTROY` 先清 userdata 再 `from_raw` |
| 4.4 | ✅ 已修 | `config::save` 同目录临时文件 + rename 原子写 |
| 4.5 | ✅ 已修 | `clear_all` 同步清空 `keys_held` 防抖表 |
| 4.6 | ✅ 已修 | 新增行默认功能 = 功能列表中首个未被占用的非停止功能 |
| 4.7 | ❌ 驳回 | AND mask 缓冲区实现为通用尺寸（当前调用固定 32×32），非硬编码 |
| 4.8 | ✅ 已修 | toggle 仅在分支成功后翻转 — 失败保持原态，下次按下重试同一动作 |

---

## 1. 验证结果

| 检查 | 结果 |
|---|---|
| `cargo test` | ✅ 25 单测 + 2 doctest 全部通过 |
| `cargo build --release --config .cargo/build-std.toml` | ✅ 构建通过 |
| `cargo clippy --all-targets -- -D warnings` | ❌ 13 个风格 lint（无功能错误） |
| `cargo fmt --check` | ❌ 与 rustfmt 不一致，diff 较大（项目当前是手写格式风格） |

---

## 2. Review 发现

### 高严重度

#### 2.1 时间轴 live-edit 追加事件会被永久丢失

- **位置**：`src/engine/timeline.rs:288-306`、`:331-343`
- **问题**：前缀清理后 `base` 被平移，但 `sync()` 仍用 `base + seen` 作为“是否还有新条目”的判断和切片起点。只要发生过前缀清理（至少一个 Note 完全回放），之后播放中追加的新事件会被跳过。
- **修复方向**：判断/切片应基于当前 `Vec` 位置 `seen`，`from` 再用 `base + seen` 生成绝对 entry 索引。
- **影响**：当前 `鬼畜走路` 走 `RollingKeys` 不受影响，但这是时间轴/MIDI live-edit 的核心路径，属于潜伏的高危 bug。

#### 2.2 `move_absolute` 未按 Interception 绝对坐标归一化

- **位置**：`src/engine/event.rs:167-175`
- **问题**：Interception 绝对鼠标坐标是 0..65535 的归一化值，不是屏幕像素坐标。当前 `move_abs(1920,1080)` 只会移到屏幕约 3% 处；未来“添加好友/申请加入”等绝对定位功能会错位。

#### 2.3 headless 退出顺序错误：先恢复亲和性，后停止功能线程

- **位置**：`src/main.rs:131-141`
- **问题**：`engine.run()` 返回后直接 `beep` + `restore_all_affinity()`，`stop_all()` 要等 main 末尾 drop `Engine` 才执行。按住 Loop 或 Toggle 运行时按 F12，退出蜂鸣和亲和性恢复期间功能线程仍会继续注入输入。
- **修复方向**：headless 应对齐 GUI `shutdown_all` 的顺序：先 `stop_all`，再蜂鸣/恢复亲和性。

---

### 中严重度

#### 3.1 `ScreenDC`/`PixelReader` 违反 `ReleaseDC` 同线程契约

- **位置**：`src/utils/screen.rs:54-55`、`src/functions/mouse_color.rs`、`src/engine/bindings.rs:239-249`
- **问题**：`坐标颜色` 的 `GetDC(NULL)` 在 GUI/主线程创建，`GetPixel` 在功能线程使用；live-apply `clear_all()` 先 drop Entry 的 Arc，旧功能线程退出时若成为最后一个 Arc，`ScreenDC::drop` 会在功能线程调用 `ReleaseDC`，与 MSDN“ReleaseDC 必须与 GetDC 同线程”冲突。

#### 3.2 CPU 核心隔离实际是 no-op

- **位置**：`src/utils/affinity.rs:29-48`
- **问题**：`GAME_CORES_MASK` 与 `OTHER_CORES_MASK` 都是 `ALL_CORES_MASK`，`isolate_game_cores()` 把其他进程设成“全核”，等于什么都没做，与 CLAUDE.md 的核心分配表自相矛盾。

#### 3.3 headless 下 `pending_joins` 从不 drain

- **位置**：`src/engine/bindings.rs:192-203`、`:353-356`
- **问题**：唯一 `drain_pending_joins()` 在 GUI 帧循环；headless 每次 Loop 松开/Toggle 关闭都会累积一个已结束线程的 `JoinHandle`，长时间运行会泄漏线程句柄/内核对象。

#### 3.4 “优化游戏”偶数次恢复不恢复优先级

- **位置**：`src/functions/optimize_game.rs:51-64`
- **问题**：关闭优化时只调 `restore_all_affinity()`，没有把游戏进程从 `HIGH_PRIORITY_CLASS` 降回 `NORMAL_PRIORITY_CLASS`。

#### 3.5 GUI panic 恢复丢失旧托盘线程句柄，可能销毁仍在使用的 HICON

- **位置**：`src/bin/gui/main.rs:1262-1273`、`1278-1285`、`src/bin/gui/tray_icon.rs:181-184`
- **问题**：`stop_tray_thread()` 2 秒超时后 `JoinHandle` 被 drop 即 detach；最终 `shutdown_all` 对 `None` 返回“已退出”，随后 `destroy()` 共享图标，而旧托盘线程可能仍存活并使用该 HICON。

#### 3.6 隐藏态在托盘不可用时未复位

- **位置**：`src/bin/gui/main.rs:1104-1105`、`1162-1167`、`202-214`
- **问题**：崩溃恢复轮若托盘线程 spawn 失败或 `Ready(false)`，只重置了 `tray_ok`，没清 `hidden`。若之前已隐藏到托盘，恢复后的窗口会被继续隐藏且无托盘图标可唤回，变成不可见应用。

#### 3.7 `find_main_window` 缺少文档声称的跨进程 PID 过滤

- **位置**：`src/bin/gui/window_ops.rs:15-22`
- **问题**：只按固定标题 + `IsWindow` 查找，未校验窗口属于当前进程。若存在同名外部窗口，`post_close`/`show_and_activate`/`set_window_icon` 可能操作到无关进程的窗口。

#### 3.8 `register`/`unregister` 直接 drop 运行中的 Entry，可产生孤儿 Loop/Toggle 线程

- **位置**：`src/engine/bindings.rs:207-223`
- **问题**：当前 GUI 总是先 `clear_all()` 再注册所以未触发；但作为公开 API，若直接替换已占用 key，旧线程的 `stop_requested` 引用会丢失，无人能停止它。

#### 3.9 `spawn_once` 丢弃 `JoinHandle`

- **位置**：`src/engine/bindings.rs:106-131`
- **问题**：Once 线程不可 join、不可回收；live-apply 期间正在执行的“优化游戏”（最长约 2s）无法被停止或等待，可能继续修改进程优先级/亲和性/前台。

---

### 低严重度

#### 4.1 `delay_ms`/`delay_ms_interruptible` 未用饱和加法

- **位置**：`src/utils/delay.rs:78,100`
- **问题**：超大 `ms` 换算成 `u64::MAX` 后加法可能回绕，延时退化为 0。建议与 timeline 一样用 `saturating_add`。

#### 4.2 `EventSequence::scroll` 的 `times as i16` 截断

- **位置**：`src/engine/event.rs:362-363`
- **问题**：超大次数可能反转滚动方向。

#### 4.3 `WM_DESTROY` 中先释放 ctx 再清 userdata

- **位置**：`src/bin/gui/tray.rs:171-176`
- **问题**：防御性顺序问题，应先清零再释放。

#### 4.4 `config::save` 非原子写

- **位置**：`src/config.rs:372-401`
- **问题**：写 `config.toml` 中途被杀会损坏配置，建议临时文件 + rename。

#### 4.5 `keys_held` 可能永久卡键

- **位置**：`src/engine/bindings.rs:285-292`
- **问题**：若某 key-up 被漏掉（如键盘拔出），该键后续按下会被防抖永久忽略；`clear_all` 也不清理。

#### 4.6 GUI 新增绑定默认“连点器”会触发重复功能校验

- **位置**：`src/bin/gui/main.rs:483-519`、`675-697`
- **问题**：当“连点器”已绑定时，新增行按下按键后必弹 duplicate function 错误，需手动改功能。

#### 4.7 `tray_icon` AND mask 缓冲区只适配 32×32

- **位置**：`src/bin/gui/tray_icon.rs:84`
- **问题**：对非 32 宽度可能缓冲不足；当前调用固定 32，属潜伏问题。

#### 4.8 “优化游戏”失败仍消耗 toggle 次数

- **位置**：`src/functions/optimize_game.rs:51-64`
- **问题**：第一次执行找不到窗口报错，第二次按会变成“恢复”。

---

## 3. 代码卫生（非功能性）

- `cargo clippy -D warnings` 的 13 项全部是风格：
  - 缺 `Default` impl（`KeyBindings`、`RollingKeys`、`Engine`、`坐标颜色`、`优化游戏`）
  - `collapsible_if`
  - `manual_is_multiple_of`
  - `manual_range_contains`
  - doc link 格式
  - `new_without_default`
- `cargo fmt --check` 不干净，项目当前不是 rustfmt 风格；如果打算引入 rustfmt，会产生一次大 diff。

---

## 4. 建议修复优先级

1. 时间轴 `sync` 索引 bug（2.1）
2. headless 退出顺序（2.3）
3. `move_absolute` 归一化（2.2）
4. CPU 掩码/文档一致性（3.2）
5. `pending_joins` headless 泄漏（3.3）
6. `ScreenDC` 线程归属（3.1）
7. 优化游戏优先级恢复（3.4）
8. GUI 托盘/隐藏恢复路径（3.5、3.6、3.7）
