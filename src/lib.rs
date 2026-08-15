//! GI-Utils: 游戏输入自动化工具 — Game input automation utility.
//!
//! 使用 Interception 驱动进行内核级键盘/鼠标输入注入。
//! Uses the Interception driver for kernel-level keyboard/mouse input injection.
//!
//! 库 crate，被 headless (gi-utils) 和 GUI (gi-utils-gui) 两个二进制共享。
//! Library crate shared by headless and GUI binaries.

#![allow(dead_code)] // 基础设施常量在子模块中被引用，顶层不可见时静默警告
// edition 2024 lint 豁免：Win32 交互区段以整段 unsafe fn 作为 unsafe 边界
// （如 run_tray_thread 的 unsafe { ... } 整体包裹），内部再逐调用嵌套
// unsafe {} 只增噪音不增安全 — 边界审查点在外层函数签名与整体 block。
#![allow(unsafe_op_in_unsafe_fn)]

pub mod config;
pub mod engine;
pub mod functions;
pub mod interception;
pub mod key;
pub mod scan_code;
pub mod utils;
