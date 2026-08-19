//! Interception 内核驱动的原生 Rust 用户层实现
//!
//! - [`protocol`]: 用户层 API 的原生移植（DeviceIoControl 协议端，
//!   替代原预编译 interception.lib；内核驱动侧不变）
//! - [`context`]: `InterceptionContext`（接收）和 `SendContext`（发送）的类型化封装

pub mod context;
pub mod protocol;

/// 重新导出上下文类型，方便从 `interception::` 直接使用。
pub use context::{InterceptionContext, SendContext};
