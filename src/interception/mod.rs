//! Interception 内核驱动 FFI 绑定及安全封装
//!
//! 包含原始 FFI 声明 (`ffi`)、RAII 上下文包装 (`context`)、
//! 以及缓冲区读写工具 (`strokes`)。
//!
//! - [`ffi`]: 镜像 `interception.h` 的原始 C 绑定
//! - [`context`]: `InterceptionContext`（接收）和 `SendContext`（发送）的安全 RAII 封装
//! - [`strokes`]: 扁平缓冲区与类型化结构体之间的安全转换

pub mod ffi;
pub mod context;
pub mod strokes;

/// 重新导出上下文类型，方便从 `interception::` 直接使用。
pub use context::{InterceptionContext, SendContext};
