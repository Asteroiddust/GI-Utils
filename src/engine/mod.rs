//! Engine — 主事件循环 (Main Event Loop).
//!
//! 创建 Interception 上下文，设置过滤器，运行阻塞式事件循环以接收、转发和分发输入事件。
//! Creates Interception contexts, sets up filters, and runs the
//! blocking event loop that receives, forwards, and dispatches input events.

pub mod bindings;
pub mod event;
pub mod function;
pub mod timeline;

pub use bindings::TriggerMode;

use crate::interception::ffi::*;
use crate::interception::{InterceptionContext, SendContext};
use crate::key::Key;
use crate::scan_code::ScanCode;
use bindings::KeyBindings;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 应用程序引擎 (Application Engine)。
/// 分离的 recv 上下文用于接收，共享的 send 上下文用于转发事件。
/// Separate recv context for receiving, shared send context for forwarding events.
pub struct Engine {
    recv_ctx: InterceptionContext,
    send_ctx: Arc<SendContext>,
    bindings: Arc<KeyBindings>,
    stop_requested: Arc<AtomicBool>,
    /// 为 true 时将所有按键输出到 stdout。
    verbose: bool,
}

impl Engine {
    /// 创建引擎，含独立的 recv/send 上下文。
    pub fn new() -> Self {
        Self::with_verbose(false)
    }

    /// 创建引擎，将所有按键输出到 stdout 用于调试。
    pub fn verbose() -> Self {
        Self::with_verbose(true)
    }

    fn with_verbose(verbose: bool) -> Self {
        let recv_ctx = InterceptionContext::create();
        let send_ctx = Arc::new(SendContext::create());

        // Set keyboard filter on the receive context: capture all key events
        recv_ctx.set_filter(
            interception_is_keyboard as InterceptionPredicate,
            INTERCEPTION_FILTER_KEY_ALL,
        );

        Self {
            recv_ctx,
            send_ctx,
            bindings: Arc::new(KeyBindings::new()),
            stop_requested: Arc::new(AtomicBool::new(false)),
            verbose,
        }
    }

    /// 获取按键绑定共享引用（用于注册函数，可跨线程）。
    pub fn bindings(&self) -> Arc<KeyBindings> {
        self.bindings.clone()
    }

    /// 获取共享的 send 上下文（供需要发送输入的函数使用）。
    pub fn send_context(&self) -> Arc<SendContext> {
        self.send_ctx.clone()
    }

    /// 从其他线程请求引擎停止。
    pub fn stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
    }

    /// 获取 stop 标志的 Arc 克隆，供 stop 函数使用。
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop_requested.clone()
    }

    /// 运行主事件循环。阻塞直到调用 [`stop`]。
    pub fn run(&self) {
        let send_ctx = &self.send_ctx;

        // L12: Engine 线程固定到输入处理核心（14,15）— GUI 版进程掩码为
        // 12-15，新线程继承进程掩码，需显式收窄；headless 版为无操作
        // （进程掩码本就是 14,15）。失败不致命 — 仅降低时序隔离性。
        let _ = crate::utils::affinity::pin_current_thread(
            crate::utils::affinity::ENGINE_CORES_MASK,
        );

        while !self.stop_requested.load(Ordering::Acquire) {
            // headless 版无 GUI 帧循环 — 已结束功能线程的句柄在此回收
            // （GUI 版由帧循环调用同一方法；is_finished 检查绝不阻塞，
            //  谁先看到结束谁 join — review 3.3）
            self.bindings.drain_pending_joins();
            let device = self.recv_ctx.wait_timeout(100);
            if device == 0 {
                continue; // timeout, recheck stop_requested
            }
            let mut stroke_buf: InterceptionStroke = [0u8; STROKE_SIZE];

            while self.recv_ctx.receive(device, &mut stroke_buf) > 0 {
                // 1. 转发原始事件 — Forward raw event to the system
                send_ctx.send_stroke(device, &stroke_buf);

                // 2. 解析按键 stroke — Deserialize the raw buffer into a key stroke struct
                let ks = crate::interception::strokes::read_key_stroke(&stroke_buf);

                // 3. 消除歧义 — Resolve E0 prefix and press/release into a unified Key
                let is_e0 = (ks.state & INTERCEPTION_KEY_E0) != 0;
                let is_pressing = (ks.state & INTERCEPTION_KEY_UP) == 0;
                let is_e1 = (ks.state & INTERCEPTION_KEY_E1) != 0;
                let key = Key {
                    code: ScanCode(ks.code),
                    is_e0,
                };

                // 4. 调试输出 — Print verbose keystroke info if enabled
                if self.verbose {
                    print_keystroke(device, key, ks.state, is_e1, ks.information);
                }

                // 5. 分发 — Route press/release to the binding manager
                if is_pressing {
                    self.bindings.process_key_down(key);
                } else {
                    self.bindings.process_key_up(key);
                }
            }
        }
    }
}

// ── 按键调试显示 — Keystroke debug display ────────────────────

fn print_keystroke(device: i32, key: Key, state: u16, e1: bool, info: u32) {
    let pressing = (state & INTERCEPTION_KEY_UP) == 0;
    let dir = if pressing { "\u{2193}" } else { "\u{2191}" };
    let tags = match (key.is_e0, e1) {
        (false, false) => "",
        (true, false) => " E0",
        (false, true) => " E1",
        (true, true) => " E0 E1",
    };
    let dev_type = if device <= INTERCEPTION_MAX_KEYBOARD {
        "KBD"
    } else {
        "MSE"
    };
    println!(
        "[{}] {:<3} #{:<2} {:<16} {:>4}  code={:#04X}  state={:#04X}  info={:#08X}",
        dir,
        dev_type,
        device,
        tags,
        key.name(),
        key.code.raw(),
        state,
        info
    );
}
