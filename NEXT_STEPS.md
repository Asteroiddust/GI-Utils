# Next Steps — GI-Utils

## 从哪里继续

Phase 1+ 完成。基础设施已稳定。下一步是 **Phase 3：逐个移植游戏功能**。

## 移植一个功能的标准流程

1. 从原 C++ 项目的 `E:\Projects\fmttest\main.cpp` 找到对应类的 `EventSequence` 构造逻辑
2. 在 `src/functions/` 下新建文件
3. 实现 `KeyFunction` trait（只需 `execute` 一个方法）
4. 用 `EventSequence` 链式 API 翻译事件序列
5. 在 `main.rs` 注册（`bindings.register(...)`）

参考模板: `src/functions/auto_clicker.rs`

## 推荐移植顺序（由简到难）

| 优先级 | 功能 | 难度 | 文件 | 备注 |
|--------|------|------|------|------|
| 1 | 快速拾取 | 简单 | quick_pickup.rs | F+滚轮序列，纯 EventSequence |
| 2 | 火神跳飞 | 简单 | fire_jump.rs | 两段序列（初始跳+循环跳），用 execute 的 before-loop 模式 |
| 3 | 甘雨走A | 简单 | ganyu_walk_a.rs | 单次序列，Once 模式 |
| 4 | 甘雨加特林 | 中等 | ganyu_gatling.rs | R 取消+鼠标移动序列 |
| 5 | 双玛头 | 中等 | double_ma.rs | 复杂按键序列 |
| 6 | 龙王喷水 | 中等 | dragon_spin.rs | 需共享状态（方向键修改 mouse stroke） |
| 7 | 鬼畜走路 | 中等 | ghost_walk.rs | 多序列循环 |
| 8 | 克洛琳德 | 较难 | clorinde.rs | 需像素颜色检测（`utils/screen.rs` 已写好） |
| 9 | 添加好友 | 较难 | add_friend.rs | 需屏幕坐标 + 绝对鼠标移动 |
| 10 | 2048 系列 | 简单 | game_2048.rs | 纯按键序列 |

## G Hub Sequence 模式示例

有些功能需要"按下执行 A，按住循环 B，松开执行 C"：

```rust
fn execute(&self, running: Arc<AtomicBool>) {
    // === on press ===
    do_initial_action();
    
    // === while held ===
    while running.load(Ordering::Acquire) {
        do_loop_action();
    }
    
    // === on release ===
    do_cleanup_action();
}
```

火神跳飞、克洛琳德、添加好友都符合这个模式。

## 已知待办

- [ ] `src/main.rs.bak` 是旧的 verbose 模式备份，确认不需要后可删除
- [ ] DEBUG 模式下的 keystroke 打印目前被 `cfg(debug_assertions)` 跳过（改为 verbose 控制），可考虑统一
- [ ] EventSequence 的 `while` 循环在多个功能中重复，可提取为 `execute_loop(ctx, events, running)` 辅助函数
