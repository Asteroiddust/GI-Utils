# Next Steps — GI-Utils v1.0.0

## 当前状态

Phase 1-2 完成，v1.0.0 已发布。8 个功能通过 TOML 配置驱动注册。

| 按键 | 功能 | 模式 | 备注 |
|------|------|------|------|
| F12 | 停止退出 | Once | 引擎停止标志 |
| F13 | 连点器 | Loop | 左键连点 |
| F14 | 快速拾取 | Loop | F+滚轮 |
| F15 | 鬼畜走路 | Loop | WASD 交错 |
| F16 | 火神跳喷 | Loop | 初始跳+循环跳 |
| F17 | 甘雨走A | Once | 射箭后摇取消 |
| F18 | 双玛头 | Loop | 复杂鼠标+键盘序列 |
| F19 | 坐标颜色 | Loop | 光标位置+像素RGB |

### 已完成的架构改进

- **Key 统一类型** — `ScanCode` + `is_e0` 合并为 `Key`，E0 标志自动注入 state
- **TOML 动态配置** — `config.toml` 驱动按键映射，热键可编辑无需重编译
- **SendContext 类型安全** — 发送上下文独立类型，编译器强制禁止并发接收
- **stop_requested 正向语义** — `true=停止`，全项目统一
- **delay_ms_interruptible** — 100μs 检查间隔，Loop/Toggle 即时响应
- **ActiveGuard panic 防护** — 线程 panic 时自动清理 active 标志
- **Mutex 外 join** — stop 不阻塞主事件循环
- **profile: O3 + LTO fat + panic=abort** — 速度优先

## 待移植功能

| 优先级 | 功能 | 难度 | 备注 |
|--------|------|------|------|
| 1 | 甘雨加特林 | ★★ | R+鼠标移动序列 |
| 2 | 龙王喷水 | ★★ | 共享 mutable mouse stroke，方向键子功能 |
| 3 | 克洛琳德 | ★★★ | 像素颜色检测触发 |
| 4 | 添加好友 | ★★★ | 屏幕坐标+绝对鼠标 |
| 5 | 申请加入 | ★★ | 鼠标序列 |
| 6 | 优化游戏 | ★★ | 找窗口→设亲和性→切前台 |
| 7 | 2048 系列 | ★ | 纯按键序列 |

## 移植流程

1. 从 `E:\Projects\fmttest\main.cpp` 找到对应类的 EventSequence 构造逻辑
2. 在 `src/functions/` 下新建文件，中文 struct 名照搬原项目
3. 实现 `KeyFunction` trait
4. 在 `src/config.rs` 的 `create_function` 和 `DEFAULT_CONFIG` 各加一行
5. 无需改 `main.rs` — 全部走配置驱动

参考模板: `auto_clicker.rs` (Loop), `ganyu_aim_cancel.rs` (Once), `mavuika_jump.rs` (on_activate+Loop)

---

## 未来计划

### 1. 组合键注册

支持修饰键+功能键的组合注册，如 `Ctrl+F13`、`Alt+E`：

```
// config.toml
[[bindings]]
key = "F13"
modifier = "Ctrl"        # None / Ctrl / Alt / Shift / Win
func = "连点器"
mode = "Loop"
```

**设计要点**：
- `KeyBindings` 跟踪所有键的实时按下/松开状态
- `process_key_down` 时检查修饰键是否已按住
- 组合键按下时触发功能，修饰键松开时不影响功能运行
- 兼容现有单键注册（modifier=None）

**实现难度**：★★。主要改动在 `bindings.rs`（状态追踪）和 `config.rs`（解析）。

### 2. 事件类型：Serial/Sequence vs Timestamp

所有键鼠编排最终分属两类范式：

| 类型 | 模型 | 代表 | 特点 |
|------|------|------|------|
| **Serial** (Sequence based) | `EventSequence` 链式 API | 连点器、快速拾取、甘雨走A、双玛头、火神跳喷 | 事件严格顺序：A → B → C，无重叠 |
| **Timestamp** (Time based) | 时间轴调度器 | 鬼畜走路、龙王喷水、未来钢琴模式 | 每个事件带绝对时间戳，多键可重叠 |

Serial 类用 `EventSequence` 构建，无需改动。Timestamp 类是 Phase 4 的重点——需要全新的运行时调度器。

### 3. 基于时间戳的多键编排

当前 `EventSequence` 是严格顺序的——一个事件接一个事件。无法表达"整个序列中 W 键按下并持续 200ms，期间 A/S/D 交错出现"这类重叠时序。

参考鬼畜走路原 C++ 实现的事件时间戳设计：

```rust
// 概念示例：时间戳调度器
struct KeyScheduler {
    events: Vec<ScheduledEvent>,  // (key, press_time, release_time)
}

// W: t=0ms 按下, t=200ms 松开
// A: t=50ms 按下, t=150ms 松开
// → W 和 A 有 100ms 重叠窗口
```

**不是第四种 TriggerMode**——这是全新的功能编写范式，与 Once/Loop/Toggle 正交。TriggerMode 只管"何时启动/停止"，时间戳编排管"启动后键是怎么按的"。

**设计要点**：
- 保留 `KeyFunction` trait 不变（`stop_requested` 信号仍然适用）
- 新增时间轴执行器，每个事件带绝对时间戳
- 执行循环按时间线推进，处理到期事件（按下/松开）
- 支持多键重叠和精确时序控制
- 可用于模拟钢琴演奏、复杂技能连招等场景

**实现难度**：★★★★。需要重新设计事件模型和运行时调度器。

### 4. 通用 held-key 清理

当前鬼畜走路用单 `held` 变量，适合所有有序序列功能。提取为通用 pattern：

```rust
// engine/event.rs 或 engine/execute.rs
pub struct HeldTracker(Vec<ScanCode>);

impl HeldTracker {
    pub fn track(&mut self, event: &InputEvent) { ... }
    pub fn cleanup(&self, ctx: &SendContext) { ... }
}
```

**触发时机**：当某个功能需要多键同时按下（如 MIDI 键盘模拟）时再做。

---

## 已知问题（review 残留）

- [ ] M10: `config.rs` parse_key 缺少部分不常用特殊字符常量
- [ ] M12: `create_function` 错误信息缺上下文
- [ ] L7: `key.rs` `allow(dead_code)` 抑制 90+ 常量
- [ ] L11: `mouse_color.rs` 忙等 100% CPU 无限速
- [ ] L13: `mavuika_double_cancel.rs` 5x 展开 vs C++ 跑间检查
- [ ] L16: `mod.rs` `read_key_stroke` 无条件调用
- [ ] L17: `config.rs` 键名大小写敏感
- [ ] L18: config 写 exe 目录，Program Files 权限
- [ ] L20: 实时优先级在 config 加载之前
- [ ] L21: `config_path()` 静默吞错误
- [ ] L22: `build.rs` 硬编码路径不可移植
