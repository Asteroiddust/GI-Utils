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

use crate::interception::native::{
    Device, INTERCEPTION_FILTER_KEY_ALL, INTERCEPTION_KEY_E0, INTERCEPTION_KEY_UP,
    InterceptionKeyStroke, MAX_STROKES_PER_IOCTL,
};
use crate::interception::{InterceptionContext, SendContext};
use crate::key::Key;
use crate::scan_code::ScanCode;
use bindings::KeyBindings;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 应用程序引擎 (Application Engine)。
/// 分离的 recv 上下文用于接收，共享的 send 上下文用于转发事件。
/// Separate recv context for receiving, shared send context for forwarding events.
pub struct Engine {
    recv_ctx: InterceptionContext,
    send_ctx: Arc<SendContext>,
    bindings: Arc<KeyBindings>,
    stop_requested: Arc<AtomicBool>,
}

impl Engine {
    /// 创建引擎，含独立的 recv/send 上下文。
    pub fn new() -> Self {
        let recv_ctx = InterceptionContext::create();
        let send_ctx = Arc::new(SendContext::create());

        // Set keyboard filter on the receive context: capture all key events
        recv_ctx.set_filter(Device::is_keyboard, INTERCEPTION_FILTER_KEY_ALL);

        Self {
            recv_ctx,
            send_ctx,
            bindings: Arc::new(KeyBindings::new()),
            stop_requested: Arc::new(AtomicBool::new(false)),
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

        // L12: Engine 线程固定到输入处理核心（14,15）— 进程掩码为 12-15，
        // 新线程继承进程掩码，需显式收窄。失败不致命 — 仅降低时序隔离性。
        let _ =
            crate::utils::affinity::pin_current_thread(crate::utils::affinity::ENGINE_CORES_MASK);

        while !self.stop_requested.load(Ordering::Acquire) {
            // Engine 线程也回收已结束功能线程的句柄（与 GUI 帧循环调用同一
            // 方法；is_finished 检查绝不阻塞，谁先看到结束谁 join —
            // review 3.3）
            self.bindings.drain_pending_joins();
            let Some(device) = self.recv_ctx.wait_timeout(100) else {
                continue; // timeout, recheck stop_requested
            };

            match device {
                // ── 键盘设备：批量接收 + 整批转发 + 逐条解析分发 ──
                Device::Keyboard(dev) => {
                    // 批缓冲：一次 IOCTL_READ 取回突发输入（C 版语义本就
                    // 支持 nstroke，旧实现恒为 1 — 重写深化的系统调用削减）
                    let mut strokes = [InterceptionKeyStroke::default(); MAX_STROKES_PER_IOCTL];
                    loop {
                        // receive 返回实际读到的前缀切片 — 只遍历真实条目
                        let got = self.recv_ctx.receive_keyboard(dev, &mut strokes);
                        if got.is_empty() {
                            break;
                        }

                        // 1. 整批转发 — 一次 IOCTL_WRITE（与批量接收对称，
                        //    减少与功能线程发送的交错 — review V5）
                        send_ctx.forward_keyboard(dev, got);

                        // 2./3. 逐条消除歧义并分发
                        for ks in got {
                            let is_e0 = (ks.state & INTERCEPTION_KEY_E0) != 0;
                            let is_pressing = (ks.state & INTERCEPTION_KEY_UP) == 0;
                            let key = Key {
                                code: ScanCode(ks.code),
                                is_e0,
                            };
                            if is_pressing {
                                self.bindings.process_key_down(key);
                            } else {
                                self.bindings.process_key_up(key);
                            }
                        }
                    }
                }
                // ── 鼠标设备：过滤器仅设键盘（鼠标 pass-through 不进
                //    队列），此分支不可达 — 若未来拦截鼠标，在此转发
                //    （review：旧注释所称"鼠标被误按键盘解析"在旧版同样
                //    不会发生，过滤器历来只设键盘）──────────────────
                Device::Mouse(_) => {}
            }
        }
    }
}
