//! GI-Utils: 游戏输入自动化工具 — Game input automation utility.
//!
//! 使用 Interception 驱动进行内核级键盘/鼠标输入注入。
//! Uses the Interception driver for kernel-level keyboard/mouse input injection.
//!
//! 库 crate，承载全部功能模块，由 GUI 二进制 (gi-utils-gui) 与测试共享。
//! Library crate holding all feature modules, shared by the GUI binary and tests.

#![allow(dead_code)] // 基础设施常量在子模块中被引用，顶层不可见时静默警告

pub mod config;
pub mod engine;
pub mod functions;
pub mod interception;
pub mod key;
pub mod utils;
