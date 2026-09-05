//! 优化游戏 — 三档模式（2026-08-22 拆档）：
//!
//! - **优化游戏**（Advanced）：全量 — 游戏进程亲和性（GAME_CORES_MASK）
//!   + OTHER 进程隔离（isolate_game_cores）+ 优先级 HIGH + 前台切换
//!   + 热线程 pinning（按进程名策略，见 thread_pin::STRATEGIES）
//! - **优化游戏标准**（Standard）：minimal + OTHER 进程隔离
//!   （不改游戏自身亲和性、不 pin）
//! - **优化游戏简易**（Minimal）：仅提升游戏进程优先级 + 前台切换
//!   （不碰任何亲和性 — no-op 场景：不想动系统级核心布局时）
//!
//! 三种模式共享：找窗三级序（名单 → 窗口类 → 失败）、换游戏检测
//! （hwnd+pid 双校验，过时不论奇偶重捕获走优化）、`HIGH` 留存无害
//! （3.4 决策）；奇偶 toggle 按模式独立（防多键绑定间串扰）。

use crate::engine::bindings::KeyFunction;
use crate::utils;
use crate::utils::delay;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use tracing::{error, info};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Threading::{
    HIGH_PRIORITY_CLASS, OpenProcess, PROCESS_SET_INFORMATION, SetPriorityClass,
    SetProcessAffinityMask,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindow, IsWindowVisible, SwitchToThisWindow,
};

// ═══════════════════════════════════════════════════════════════════
// 模式定义
// ═══════════════════════════════════════════════════════════════════

/// 优化档位 — 决定 optimize/restore 各做多少。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizeMode {
    /// 全量：游戏亲和性 + OTHER 隔离 + 优先级 + pinning。
    Advanced,
    /// minimal + OTHER 隔离（不动游戏自身、不 pin）。
    Standard,
    /// 仅优先级 + 前台（不碰任何亲和性）。
    Minimal,
}

impl OptimizeMode {
    fn idx(self) -> usize {
        match self {
            OptimizeMode::Advanced => 0,
            OptimizeMode::Standard => 1,
            OptimizeMode::Minimal => 2,
        }
    }
}

const MODE_COUNT: usize = 3;

/// 奇偶切换状态 — 每模式独立（进程级共享）。
/// 实例会被 live-apply 与崩溃恢复重建：状态若随实例走，重建后奇偶归零，
/// 已隔离的游戏会被二次隔离而非恢复（review 发现）。
/// 三模式各自独立 — 多键绑定不同档位时互不干扰奇偶。
static TOGGLE_STATE: [AtomicBool; MODE_COUNT] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];

// ═══════════════════════════════════════════════════════════════════
// Core — 三种模式共享的实现（找窗/换游戏检测/流程分派）
// ═══════════════════════════════════════════════════════════════════

struct Core {
    /// 上次捕获的游戏窗口（原子存 HWND 原始值 — 实例跨线程共享，
    /// 换游戏后可刷新）。
    hwnd: AtomicIsize,
    /// 上次捕获的游戏 pid — HWND 复用双重校验：换游戏后旧句柄可能被
    /// 无关窗口复用，仅 IsWindow 不充分。
    pid: AtomicU32,
}

impl Core {
    fn new() -> Self {
        let hwnd = find_game_window();
        let mut pid = 0u32;
        if is_valid_window(hwnd) {
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        }
        Self {
            hwnd: AtomicIsize::new(hwnd.0 as isize),
            pid: AtomicU32::new(pid),
        }
    }

    fn current_hwnd(&self) -> HWND {
        HWND(self.hwnd.load(Ordering::Acquire) as *mut std::ffi::c_void)
    }

    /// 持有信息是否仍有效：窗口存活 **且** pid 未变。
    fn info_valid(&self) -> bool {
        let hwnd = self.current_hwnd();
        if !is_valid_window(hwnd) {
            return false;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        pid != 0 && pid == self.pid.load(Ordering::Acquire)
    }

    fn store_info(&self, hwnd: HWND) {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        self.hwnd.store(hwnd.0 as isize, Ordering::Release);
        self.pid.store(pid, Ordering::Release);
    }

    fn execute(&self, mode: OptimizeMode, _stop_requested: Arc<AtomicBool>) {
        // 换游戏检测（2026-08-22）：持有的 hwnd/pid 过时（游戏退出/句柄被
        // 复用）→ 不论奇偶，重新捕获并走**优化**方向 — 新游戏没有被优化过，
        // "恢复"对它无语义；toggle 置相应档位（原本 true 则不变）。找不到
        // 新游戏则按奇偶原逻辑。
        if !self.info_valid() {
            let hwnd = find_game_window();
            if is_valid_window(hwnd) {
                self.store_info(hwnd);
                if self.optimize(mode) {
                    TOGGLE_STATE[mode.idx()].store(true, Ordering::Release);
                }
                return;
            }
        }

        // 切换奇偶：odd → 优化，even → 恢复（每模式独立）。
        // 仅在分支成功后翻转 — 先翻后执行时失败路径会永久卡在错误的
        // 奇偶相（如找不到窗口，下次按下误走恢复 — review 4.8）。
        let optimize = !TOGGLE_STATE[mode.idx()].load(Ordering::Acquire);
        let succeeded = if optimize {
            self.optimize(mode)
        } else {
            self.restore(mode)
        };
        if succeeded {
            TOGGLE_STATE[mode.idx()].fetch_xor(true, Ordering::AcqRel);
        }
    }

    /// 恢复 — 按模式撤销各自动过的状态。
    /// 成功返回 true（供 execute 翻转奇偶）。
    fn restore(&self, mode: OptimizeMode) -> bool {
        info!("优化游戏: 恢复（{:?} 模式）", mode);
        match mode {
            OptimizeMode::Advanced => {
                // 线程级 pin 先还原（线程级掩码不被进程级 restore_all 覆盖）
                utils::thread_pin::restore();
                if let Err(e) = utils::affinity::restore_all_affinity() {
                    error!("优化游戏: 恢复失败: {}", e);
                    return false;
                }
            }
            OptimizeMode::Standard => {
                // OTHER 隔离的撤销：恢复全表进程掩码
                if let Err(e) = utils::affinity::restore_all_affinity() {
                    error!("优化游戏: 恢复失败: {}", e);
                    return false;
                }
            }
            OptimizeMode::Minimal => {
                // 未动任何状态 — HIGH 留存无害（3.4：降级需再 OpenProcess
                // 游戏句柄（反作弊拦截路径），且 HIGH 随进程退出即消亡）
                info!("优化游戏: 简易模式无需要恢复的状态");
            }
        }
        if mode != OptimizeMode::Minimal {
            info!("优化游戏: 已恢复");
            utils::beep::beep_async(375, 300);
        }
        true
    }

    /// 执行优化流程。任何一步失败返回 false（execute 不翻转奇偶，
    /// 下次按下重试同一动作）。
    fn optimize(&self, mode: OptimizeMode) -> bool {
        // ── 1. 窗口验证 / 重新查找 ────────────────────
        let mut hwnd = self.current_hwnd();
        if !is_valid_window(hwnd) {
            hwnd = find_game_window();
        }

        if !is_valid_window(hwnd) {
            error!("优化游戏: 找不到游戏窗口！");
            utils::beep::beep_async(1000, 500);
            return false;
        }

        // ── 2. 获取进程 PID + 窗口标题 ─────────────────
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 {
            error!("优化游戏: 获取进程 ID 失败！");
            utils::beep::beep_async(1000, 500);
            return false;
        }
        let title = get_window_title(hwnd);

        info!(
            "优化游戏: {} (PID: {}, HWND: {:?}) — {:?}",
            if title.is_empty() {
                "(unknown)"
            } else {
                &title
            },
            pid,
            hwnd.0,
            mode
        );

        // ── 3. 打开进程 ───────────────────────────────
        let h_process = match unsafe { OpenProcess(PROCESS_SET_INFORMATION, false, pid) } {
            Ok(h) => h,
            Err(e) => {
                error!("优化游戏: 打开进程失败: {}", e);
                // 对应 C++ 原版 SetProcessAffinityMaskAndPriorityClass 失败蜂鸣 —
                // 游戏反作弊（如 mhyprot）会拦截外部 OpenProcess，此路径不可静默
                utils::beep::beep_async(1000, 500);
                return false;
            }
        };

        // ── 4. 亲和性与隔离（按模式；advanced 才动游戏自身）──
        match mode {
            OptimizeMode::Advanced => {
                if let Err(e) =
                    unsafe { SetProcessAffinityMask(h_process, utils::affinity::GAME_CORES_MASK) }
                {
                    error!("优化游戏: 设置 CPU 亲和性失败: {}", e);
                }
                if let Err(e) = utils::affinity::isolate_game_cores(pid) {
                    error!("优化游戏: 隔离其他进程失败: {}", e);
                }
            }
            OptimizeMode::Standard => {
                if let Err(e) = utils::affinity::isolate_game_cores(pid) {
                    error!("优化游戏: 隔离其他进程失败: {}", e);
                }
            }
            OptimizeMode::Minimal => {}
        }

        // ── 5. 提升优先级（三模式共有；失败告警不阻断 — 与亲和性步骤同粒度）──
        if let Err(e) = unsafe { SetPriorityClass(h_process, HIGH_PRIORITY_CLASS) } {
            error!("优化游戏: 提升优先级失败: {}", e);
        }
        unsafe { windows::Win32::Foundation::CloseHandle(h_process) }.ok();
        info!("优化游戏: 进程优先级已提升为高");

        // ── 6. 切换到前台（三模式共有）─────────────────
        // 最多重试 40 次 (× 50ms = 2s)，防止前台锁定导致死循环
        let foreground = unsafe { GetForegroundWindow() };
        if foreground != hwnd {
            const MAX_RETRIES: u32 = 40;
            let mut attempts: u32 = 0;
            while unsafe { GetForegroundWindow() } != hwnd && attempts < MAX_RETRIES {
                unsafe { SwitchToThisWindow(hwnd, true) };
                delay::delay_ms(50.0);
                attempts += 1;
            }
            if attempts >= MAX_RETRIES {
                error!("优化游戏: 切换前台超时 ({}ms) — 跳过", MAX_RETRIES * 50);
                utils::beep::beep_async(500, 200);
                return false;
            }
        }
        info!("优化游戏: 完成");
        // ── 7. 热线程 pinning（仅 advanced — 前台已切换，采样窗口测得真实负载）──
        if mode == OptimizeMode::Advanced {
            utils::thread_pin::apply_for_pid(pid);
        }
        // 成功后刷新捕获基准 — 后续换游戏检测以此为对照
        self.store_info(hwnd);
        utils::beep::beep_async(750, 300);
        true
    }
}

// ═══════════════════════════════════════════════════════════════════
// 三个公开功能 struct — 工厂按名分派，共享 Core
// ═══════════════════════════════════════════════════════════════════

/// 优化游戏（Advanced）— 全量档：游戏亲和性 + OTHER 隔离 + 优先级
/// + 前台 + 热线程 pinning。Once 模式，按下 NumpadAdd 执行一次。
pub struct 优化游戏 {
    core: Core,
}

impl 优化游戏 {
    pub fn new() -> Self {
        Self { core: Core::new() }
    }
}

impl KeyFunction for 优化游戏 {
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        self.core.execute(OptimizeMode::Advanced, stop_requested);
    }
}

/// 优化游戏标准（Standard）— 仅加 OTHER 进程隔离，不动游戏自身、不 pin。
pub struct 优化游戏标准 {
    core: Core,
}

impl 优化游戏标准 {
    pub fn new() -> Self {
        Self { core: Core::new() }
    }
}

impl KeyFunction for 优化游戏标准 {
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        self.core.execute(OptimizeMode::Standard, stop_requested);
    }
}

/// 优化游戏简易（Minimal）— 仅提升优先级 + 前台，不碰任何亲和性。
pub struct 优化游戏简易 {
    core: Core,
}

impl 优化游戏简易 {
    pub fn new() -> Self {
        Self { core: Core::new() }
    }
}

impl KeyFunction for 优化游戏简易 {
    fn execute(&self, stop_requested: Arc<AtomicBool>) {
        self.core.execute(OptimizeMode::Minimal, stop_requested);
    }
}

// ── Helpers ────────────────────────────────────────────

/// 找游戏窗口 — 三级序（2026-08-22 定）：
/// 1. **已登记名单优先**：进程名 → pid → 该进程的可见主窗口
///    （覆盖窗口类不明的游戏，如 Endfield）；
/// 2. 窗口类兜底：UnityWndClass / UnrealWindow（未登记游戏）；
/// 3. 均未命中 → NULL（调用方报错）。
fn find_game_window() -> HWND {
    // 1. 名单序（同开多游戏时按名单顺序取胜）
    if let Some((pid, name)) =
        utils::affinity::find_pid_by_names(crate::functions::GAME_PROCESS_NAMES)
    {
        let hwnd = find_window_by_pid(pid);
        if is_valid_window(hwnd) {
            info!("优化游戏: 名单命中 {name} (PID {pid})");
            return hwnd;
        }
    }
    // 2. 窗口类兜底（未登记的 Unity / Unreal 游戏）
    find_class_game_window()
}

/// 窗口类查找（三级序第 2 级 — 独立导出，「线程采样」的兜底同款）：
/// UnityWndClass（原神/崩铁/绝区零等）→ UnrealWindow（鸣潮/黑猴等）。
/// 未命中返回 NULL。
pub(crate) fn find_class_game_window() -> HWND {
    unsafe {
        // 原神、崩铁、绝区零 (Unity)
        if let Ok(hwnd) = FindWindowW(windows::core::w!("UnityWndClass"), None) {
            if !hwnd.is_invalid() {
                return hwnd;
            }
        }
        // 鸣潮、黑猴 (Unreal Engine)
        if let Ok(hwnd) = FindWindowW(windows::core::w!("UnrealWindow"), None) {
            if !hwnd.is_invalid() {
                return hwnd;
            }
        }
    }
    HWND::default() // NULL handle
}

/// 检查窗口句柄是否有效。
fn is_valid_window(hwnd: HWND) -> bool {
    !hwnd.is_invalid() && unsafe { IsWindow(Some(hwnd)) }.as_bool()
}

/// 枚举顶层窗口，找 pid 的可见主窗口（有标题者优先 — 启动器/覆盖层等
/// 无标题或辅助窗口退居其次）。未找到返回 NULL。
fn find_window_by_pid(pid: u32) -> HWND {
    struct Ctx {
        pid: u32,
        titled: HWND,
        any: HWND,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        let ctx = unsafe { &mut *(lparam.0 as *mut Ctx) };
        let mut wpid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut wpid)) };
        if wpid != ctx.pid {
            return windows::core::BOOL::from(true); // 继续枚举
        }
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return windows::core::BOOL::from(true);
        }
        let has_title = unsafe { GetWindowTextLengthW(hwnd) } > 0;
        if has_title && ctx.titled.is_invalid() {
            ctx.titled = hwnd;
        }
        if ctx.any.is_invalid() {
            ctx.any = hwnd;
        }
        windows::core::BOOL::from(true)
    }

    let mut ctx = Ctx {
        pid,
        titled: HWND::default(),
        any: HWND::default(),
    };
    // EnumWindows 同步回调；失败（如无权限）按未找到处理
    let _ = unsafe { EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut Ctx as isize)) };
    if is_valid_window(ctx.titled) {
        ctx.titled
    } else {
        ctx.any
    }
}

/// 获取窗口标题。
fn get_window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}
