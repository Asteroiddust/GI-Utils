//! GI-Utils: 游戏输入自动化工具 — Game input automation utility.
//!
//! 使用 Interception 驱动进行内核级键盘/鼠标输入注入。
//! Uses the Interception driver for kernel-level keyboard/mouse input injection.
//!
//! 库 crate，被 headless (gi-utils) 和 GUI (gi-utils-gui) 两个二进制共享。
//! Library crate shared by headless and GUI binaries.

#![allow(dead_code)] // 基础设施常量在子模块中被引用，顶层不可见时静默警告

pub mod config;
pub mod engine;
pub mod functions;
pub mod interception;
pub mod key;
pub mod scan_code;
pub mod utils;
