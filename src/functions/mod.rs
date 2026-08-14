//! 功能模块 — 具体的游戏自动化功能实现。
//!
//! 每个子模块定义一个 struct 并实现 `KeyFunction` trait，
//! 通过 `TriggerMode` (Once / Loop / Toggle) 控制执行方式。

pub mod auto_clicker;
pub mod ganyu_aim_cancel;
pub mod ghost_walk;
pub mod mavuika_double_cancel;
pub mod mavuika_jump;
pub mod mouse_color;
pub mod optimize_game;
pub mod quick_pickup;
pub mod stop;
