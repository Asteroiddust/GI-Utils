//! Interception 用户层 API 的原生 Rust 移植（重写深化版）
//!
//! 协议契约移植自 oblitum/Interception `library/interception.c`（LGPL 3.0，
//! 许可文本见 `Interception/licenses/`）：内核驱动侧保持不变，本模块实现
//! 用户态 DeviceIoControl 协议端。
//!
//! # 相对 C 原版的现代化
//!
//! 原库为 2012-2015 时代写法（ANSI API、每调用 HeapAlloc、错误全忽略、
//! 手工清理）。本移植除忠实协议外全部现代化：
//!
//! - **类型化 send/receive**：直接收发 [`InterceptionKeyStroke`] /
//!   [`InterceptionMouseStroke`] 切片，栈上分批转换 — 消灭 C 版每次
//!   调用的 HeapAlloc 与字节缓冲往返
//! - **设备新类型**：[`KeyboardDevice`] / [`MouseDevice`] / [`Device`] —
//!   设备类型混淆在编译期不存在（send_keyboard 只接受 KeyboardDevice）；
//!   索引越界在构造处 panic（绝不静默截断/错位 — review）
//! - **批量接收**：一次 IOCTL_READ 可读 [`MAX_STROKES_PER_IOCTL`] 条
//!   （协议本身支持 nstroke，引擎以 32 条批缓冲调用）
//! - **宽字符 API**：CreateFileW（CreateFileA 是 Win9x 遗留惯例）
//! - **错误传播**：CreateFile / CreateEvent / DeviceIoControl 失败返回
//!   `windows::core::Result`（C 版全部忽略）；create 中途失败由 Drop 自动清理
//! - **set_filter 接受 Rust 闭包**（C 版需 extern "C" 函数指针）
//! - **Context RAII**：句柄由结构所有权 + Drop 管理，无手工释放路径

#![allow(dead_code)] // 协议完整性保留（precedence/hardware_id 等当前未调用）

use std::ffi::c_void;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_NO_MORE_ITEMS, HANDLE, INVALID_HANDLE_VALUE, WAIT_EVENT,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_CREATION_DISPOSITION, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ,
    FILE_SHARE_MODE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects};
use windows::core::{HRESULT, PCWSTR, Result};

use tracing::warn;

// ═══════════════════════════════════════════════════════════════════
// 设备编号 — 新类型（编译期防呆，替代 C 版的裸 int + 运行时判定）
// ═══════════════════════════════════════════════════════════════════

/// 最大键盘设备数。
pub const MAX_KEYBOARD: usize = 10;

/// 最大鼠标设备数。
pub const MAX_MOUSE: usize = 10;

/// 最大总设备数（键盘 + 鼠标）。
pub const MAX_DEVICE: usize = MAX_KEYBOARD + MAX_MOUSE;

/// 键盘设备（索引 0..10）。只能传给键盘专用方法 — 类型层面不可混淆。
///
/// 字段私有：唯一构造途径是 [`keyboard`]，越界索引在构造处立即 panic —
/// 绝不静默截断或路由错位（review：此前 `pub u8` + `as u8` 截断，
/// keyboard(256) 会静默命中设备 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardDevice(u8);

/// 鼠标设备（索引 0..10）。只能传给鼠标专用方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseDevice(u8);

/// 接收路径的设备标识（wait 返回值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Keyboard(KeyboardDevice),
    Mouse(MouseDevice),
}

impl Device {
    /// 驱动协议设备号（键盘 1..=10，鼠标 11..=20）。
    #[inline]
    pub fn number(self) -> usize {
        match self {
            Device::Keyboard(d) => d.0 as usize + 1,
            Device::Mouse(d) => MAX_KEYBOARD + d.0 as usize + 1,
        }
    }

    /// 是否为键盘设备。
    #[inline]
    pub const fn is_keyboard(self) -> bool {
        matches!(self, Device::Keyboard(_))
    }
}

/// 键盘设备（索引 0..10）。越界 panic（常量调用在编译期报错；
/// const fn 无法带格式化消息，运行时 panic 位置即此处断言）。
#[inline]
pub const fn keyboard(index: usize) -> KeyboardDevice {
    assert!(index < MAX_KEYBOARD);
    KeyboardDevice(index as u8)
}

/// 鼠标设备（索引 0..10）。越界 panic（常量调用在编译期报错）。
#[inline]
pub const fn mouse(index: usize) -> MouseDevice {
    assert!(index < MAX_MOUSE);
    MouseDevice(index as u8)
}

// ═══════════════════════════════════════════════════════════════════
// IOCTL 码 — 与内核驱动的协议契约，移植自 interception.c
// CTL_CODE(FILE_DEVICE_UNKNOWN=0x22, fn, METHOD_BUFFERED=0, FILE_ANY_ACCESS=0)
// ═══════════════════════════════════════════════════════════════════

const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

const IOCTL_SET_PRECEDENCE: u32 = ctl_code(0x22, 0x801, 0, 0);
const IOCTL_GET_PRECEDENCE: u32 = ctl_code(0x22, 0x802, 0, 0);
const IOCTL_SET_FILTER: u32 = ctl_code(0x22, 0x804, 0, 0);
const IOCTL_GET_FILTER: u32 = ctl_code(0x22, 0x808, 0, 0);
const IOCTL_SET_EVENT: u32 = ctl_code(0x22, 0x810, 0, 0);
const IOCTL_WRITE: u32 = ctl_code(0x22, 0x820, 0, 0);
const IOCTL_READ: u32 = ctl_code(0x22, 0x840, 0, 0);
const IOCTL_GET_HARDWARE_ID: u32 = ctl_code(0x22, 0x880, 0, 0);

// ═══════════════════════════════════════════════════════════════════
// 按键状态标志 — Key state flags（interception.h 同名常量）
// ═══════════════════════════════════════════════════════════════════

/// 按键按下。
pub const INTERCEPTION_KEY_DOWN: u16 = 0x00;
/// 按键松开。
pub const INTERCEPTION_KEY_UP: u16 = 0x01;
/// E0 扩展标志 — 表示该扫描码属于扩展键 (如方向键、RAlt)。
pub const INTERCEPTION_KEY_E0: u16 = 0x02;
/// E1 扩展标志 — 用于暂停键等极少数键。
pub const INTERCEPTION_KEY_E1: u16 = 0x04;
pub const INTERCEPTION_KEY_TERMSRV_SET_LED: u16 = 0x08;
pub const INTERCEPTION_KEY_TERMSRV_SHADOW: u16 = 0x10;
pub const INTERCEPTION_KEY_TERMSRV_VKPACKET: u16 = 0x20;

// ── 键盘过滤器标志 — Filter key states ─────────────────────────

/// 不过滤任何键盘事件。
pub const INTERCEPTION_FILTER_KEY_NONE: u16 = 0x0000;
/// 过滤所有键盘事件。
pub const INTERCEPTION_FILTER_KEY_ALL: u16 = 0xFFFF;
/// 过滤按键按下事件。
pub const INTERCEPTION_FILTER_KEY_DOWN: u16 = INTERCEPTION_KEY_UP;
/// 过滤按键松开事件。
pub const INTERCEPTION_FILTER_KEY_UP: u16 = INTERCEPTION_KEY_UP << 1;
/// 过滤带 E0 标志的键盘事件。
pub const INTERCEPTION_FILTER_KEY_E0: u16 = INTERCEPTION_KEY_E0 << 1;
/// 过滤带 E1 标志的键盘事件。
pub const INTERCEPTION_FILTER_KEY_E1: u16 = INTERCEPTION_KEY_E1 << 1;
pub const INTERCEPTION_FILTER_KEY_TERMSRV_SET_LED: u16 = INTERCEPTION_KEY_TERMSRV_SET_LED << 1;
pub const INTERCEPTION_FILTER_KEY_TERMSRV_SHADOW: u16 = INTERCEPTION_KEY_TERMSRV_SHADOW << 1;
pub const INTERCEPTION_FILTER_KEY_TERMSRV_VKPACKET: u16 = INTERCEPTION_KEY_TERMSRV_VKPACKET << 1;

// ── 鼠标状态标志 — Mouse state flags ────────────────────────────

/// 鼠标左键按下。
pub const INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN: u16 = 0x001;
/// 鼠标左键松开。
pub const INTERCEPTION_MOUSE_LEFT_BUTTON_UP: u16 = 0x002;
/// 鼠标右键按下。
pub const INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN: u16 = 0x004;
/// 鼠标右键松开。
pub const INTERCEPTION_MOUSE_RIGHT_BUTTON_UP: u16 = 0x008;
/// 鼠标中键按下。
pub const INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN: u16 = 0x010;
/// 鼠标中键松开。
pub const INTERCEPTION_MOUSE_MIDDLE_BUTTON_UP: u16 = 0x020;
/// 鼠标第四键按下。
pub const INTERCEPTION_MOUSE_BUTTON_4_DOWN: u16 = 0x040;
/// 鼠标第四键松开。
pub const INTERCEPTION_MOUSE_BUTTON_4_UP: u16 = 0x080;
/// 鼠标第五键按下。
pub const INTERCEPTION_MOUSE_BUTTON_5_DOWN: u16 = 0x100;
/// 鼠标第五键松开。
pub const INTERCEPTION_MOUSE_BUTTON_5_UP: u16 = 0x200;
/// 鼠标垂直滚轮滚动。
pub const INTERCEPTION_MOUSE_WHEEL: u16 = 0x400;
/// 鼠标水平滚轮滚动。
pub const INTERCEPTION_MOUSE_HWHEEL: u16 = 0x800;

// 按钮别名
/// 鼠标第一键按下（左键别名）。
pub const INTERCEPTION_MOUSE_BUTTON_1_DOWN: u16 = INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN;
/// 鼠标第一键松开（左键别名）。
pub const INTERCEPTION_MOUSE_BUTTON_1_UP: u16 = INTERCEPTION_MOUSE_LEFT_BUTTON_UP;
/// 鼠标第二键按下（右键别名）。
pub const INTERCEPTION_MOUSE_BUTTON_2_DOWN: u16 = INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN;
/// 鼠标第二键松开（右键别名）。
pub const INTERCEPTION_MOUSE_BUTTON_2_UP: u16 = INTERCEPTION_MOUSE_RIGHT_BUTTON_UP;
/// 鼠标第三键按下（中键别名）。
pub const INTERCEPTION_MOUSE_BUTTON_3_DOWN: u16 = INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN;
/// 鼠标第三键松开（中键别名）。
pub const INTERCEPTION_MOUSE_BUTTON_3_UP: u16 = INTERCEPTION_MOUSE_MIDDLE_BUTTON_UP;

// ── 鼠标过滤器标志 — Filter mouse states ────────────────────────

/// 不过滤任何鼠标事件。
pub const INTERCEPTION_FILTER_MOUSE_NONE: u16 = 0x0000;
/// 过滤所有鼠标事件。
pub const INTERCEPTION_FILTER_MOUSE_ALL: u16 = 0xFFFF;

pub const INTERCEPTION_FILTER_MOUSE_LEFT_BUTTON_DOWN: u16 = INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_LEFT_BUTTON_UP: u16 = INTERCEPTION_MOUSE_LEFT_BUTTON_UP;
pub const INTERCEPTION_FILTER_MOUSE_RIGHT_BUTTON_DOWN: u16 = INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_RIGHT_BUTTON_UP: u16 = INTERCEPTION_MOUSE_RIGHT_BUTTON_UP;
pub const INTERCEPTION_FILTER_MOUSE_MIDDLE_BUTTON_DOWN: u16 = INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_MIDDLE_BUTTON_UP: u16 = INTERCEPTION_MOUSE_MIDDLE_BUTTON_UP;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_1_DOWN: u16 = INTERCEPTION_MOUSE_BUTTON_1_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_1_UP: u16 = INTERCEPTION_MOUSE_BUTTON_1_UP;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_2_DOWN: u16 = INTERCEPTION_MOUSE_BUTTON_2_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_2_UP: u16 = INTERCEPTION_MOUSE_BUTTON_2_UP;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_3_DOWN: u16 = INTERCEPTION_MOUSE_BUTTON_3_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_3_UP: u16 = INTERCEPTION_MOUSE_BUTTON_3_UP;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_4_DOWN: u16 = INTERCEPTION_MOUSE_BUTTON_4_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_4_UP: u16 = INTERCEPTION_MOUSE_BUTTON_4_UP;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_5_DOWN: u16 = INTERCEPTION_MOUSE_BUTTON_5_DOWN;
pub const INTERCEPTION_FILTER_MOUSE_BUTTON_5_UP: u16 = INTERCEPTION_MOUSE_BUTTON_5_UP;
/// 过滤鼠标垂直滚轮事件。
pub const INTERCEPTION_FILTER_MOUSE_WHEEL: u16 = INTERCEPTION_MOUSE_WHEEL;
/// 过滤鼠标水平滚轮事件。
pub const INTERCEPTION_FILTER_MOUSE_HWHEEL: u16 = INTERCEPTION_MOUSE_HWHEEL;
/// 过滤鼠标移动事件。
pub const INTERCEPTION_FILTER_MOUSE_MOVE: u16 = 0x1000;

// ── 鼠标移动标志 — Mouse move flags ─────────────────────────────

/// 鼠标移动模式：相对移动。
pub const INTERCEPTION_MOUSE_MOVE_RELATIVE: u16 = 0x000;
/// 鼠标移动模式：绝对移动。
pub const INTERCEPTION_MOUSE_MOVE_ABSOLUTE: u16 = 0x001;
/// 使用虚拟桌面坐标（绝对模式时）。
pub const INTERCEPTION_MOUSE_VIRTUAL_DESKTOP: u16 = 0x002;
/// 鼠标属性变更标志。
pub const INTERCEPTION_MOUSE_ATTRIBUTES_CHANGED: u16 = 0x004;
/// 禁止鼠标移动合并 (coalescing)。
pub const INTERCEPTION_MOUSE_MOVE_NOCOALESCE: u16 = 0x008;
/// 终端服务远程桌面源标志。
pub const INTERCEPTION_MOUSE_TERMSRV_SRC_SHADOW: u16 = 0x100;

// ═══════════════════════════════════════════════════════════════════
// Stroke 结构（公开 API 类型，与 interception.h 布局一致）
// ═══════════════════════════════════════════════════════════════════

/// Interception 键盘输入数据包。
///
/// 大小: 8 字节, 对齐: 4。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterceptionKeyStroke {
    pub code: u16,
    pub state: u16,
    pub information: u32,
}

/// Interception 鼠标输入数据包。
///
/// 大小: 20 字节, 对齐: 4。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterceptionMouseStroke {
    pub state: u16,
    pub flags: u16,
    pub rolling: i16,
    pub x: i32,
    pub y: i32,
    pub information: u32,
}

// ═══════════════════════════════════════════════════════════════════
// 驱动线上格式 — hidclass 输入报告（IOCTL 读写缓冲，WDK 文档结构）
// ═══════════════════════════════════════════════════════════════════

/// KEYBOARD_INPUT_DATA — 12 字节，对齐 4。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyboardInputData {
    unit_id: u16,
    make_code: u16,
    flags: u16,
    reserved: u16,
    extra_information: u32,
}

/// MOUSE_INPUT_DATA — 24 字节，对齐 4。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MouseInputData {
    unit_id: u16,
    flags: u16,
    button_flags: u16,
    button_data: u16,
    raw_buttons: u32,
    last_x: i32,
    last_y: i32,
    extra_information: u32,
}

const _: () = assert!(std::mem::size_of::<KeyboardInputData>() == 12);
const _: () = assert!(std::mem::size_of::<MouseInputData>() == 24);
const _: () = assert!(std::mem::align_of::<KeyboardInputData>() == 4);
const _: () = assert!(std::mem::align_of::<MouseInputData>() == 4);

// ── 双向字段映射（协议保真的纯函数 — 可无驱动单测）────────────

impl From<InterceptionKeyStroke> for KeyboardInputData {
    fn from(ks: InterceptionKeyStroke) -> Self {
        Self {
            unit_id: 0,
            make_code: ks.code,
            flags: ks.state,
            reserved: 0,
            extra_information: ks.information,
        }
    }
}

impl From<KeyboardInputData> for InterceptionKeyStroke {
    fn from(raw: KeyboardInputData) -> Self {
        Self {
            code: raw.make_code,
            state: raw.flags,
            information: raw.extra_information,
        }
    }
}

impl From<InterceptionMouseStroke> for MouseInputData {
    fn from(ms: InterceptionMouseStroke) -> Self {
        Self {
            unit_id: 0,
            flags: ms.flags,
            button_flags: ms.state,
            button_data: ms.rolling as u16,
            raw_buttons: 0,
            last_x: ms.x,
            last_y: ms.y,
            extra_information: ms.information,
        }
    }
}

impl From<MouseInputData> for InterceptionMouseStroke {
    fn from(raw: MouseInputData) -> Self {
        Self {
            state: raw.button_flags,
            flags: raw.flags,
            rolling: raw.button_data as i16,
            x: raw.last_x,
            y: raw.last_y,
            information: raw.extra_information,
        }
    }
}

/// 单次 IOCTL 的栈缓冲 stroke 上限：一次读/写最多 32 条（C 版 HeapAlloc
/// 任意大小，栈分批零分配；批量接收的引擎侧缓冲大小同此）。
pub const MAX_STROKES_PER_IOCTL: usize = 32;

// ═══════════════════════════════════════════════════════════════════
// Context — 20 个设备（10 键盘 + 10 鼠标）的句柄集合，RAII
// ═══════════════════════════════════════════════════════════════════

/// 单个设备的句柄对（文件句柄 + "有数据"事件句柄）。
///
/// Drop 对齐 C 版 `interception_destroy_context`：CloseHandle 失败忽略。
struct DeviceSlot {
    handle: HANDLE,
    unempty: HANDLE,
}

impl Default for DeviceSlot {
    fn default() -> Self {
        Self {
            handle: INVALID_HANDLE_VALUE,
            unempty: HANDLE::default(),
        }
    }
}

impl Drop for DeviceSlot {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
        if !self.unempty.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.unempty);
            }
        }
    }
}

/// Interception 用户层上下文：持有全部 20 个设备的句柄。
///
/// 对应 C 版 `InterceptionContext`（`InterceptionDeviceArray`）。
/// 创建失败（驱动未安装/权限不足）返回 `Err`，已打开的资源由 Drop 清理。
pub struct Context {
    devices: [DeviceSlot; MAX_DEVICE],
}

// HANDLE 为进程内可共享的内核对象引用；设备句柄/事件句柄可跨线程移动。
// 仅 Send 不标 Sync：receive_* 为 pub &self 方法，标 Sync 会让多线程
// 并发 IOCTL_READ 瓜分同一驱动队列（review）— 单线程接收契约由上层
// 类型执行（InterceptionContext !Sync；SendContext 仅暴露发送且驱动
// 支持并发 send，其 Sync 在 context.rs 单独声明）。
unsafe impl Send for Context {}

/// 打开单个设备：CreateFileW → CreateEventW → IOCTL_SET_EVENT 注册事件
/// （对齐 C 版 `interception_create_context` 循环体，宽字符 API 化）。
///
/// 句柄**即拿即填**进局部槽：后续任何一步失败 `?` 返回时，已打开的
/// 句柄随 DeviceSlot::drop 关闭（对齐 C 版每条失败路径显式 destroy —
/// review：此前 handle/unempty 是局部变量，CreateEventW/IOCTL 失败即
/// 泄漏，模块文档"Drop 自动清理"的承诺对在飞局部变量不成立）。
fn open_device(index: usize) -> Result<DeviceSlot> {
    let name: Vec<u16> = format!("\\\\.\\interception{index:02}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut slot = DeviceSlot::default();
    // C: CreateFileA(name, GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL)
    slot.handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_MODE(0),
            None,
            FILE_CREATION_DISPOSITION(OPEN_EXISTING.0),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }?;
    // C: CreateEventA(NULL, TRUE, FALSE, NULL)
    slot.unempty = unsafe { CreateEventW(None, true, false, None) }?;
    // C: 2 元素句柄数组 {event, NULL} 作输入缓冲注册事件
    let handles = [slot.unempty, HANDLE::default()];
    unsafe {
        DeviceIoControl(
            slot.handle,
            IOCTL_SET_EVENT,
            Some(handles.as_ptr() as *const c_void),
            std::mem::size_of_val(&handles) as u32,
            None,
            0,
            None,
            None,
        )?;
    }
    Ok(slot)
}

// ═══════════════════════════════════════════════════════════════════
// IOCTL 原语与分批转换骨架 — 四个收发函数共用的泛化层
// ═══════════════════════════════════════════════════════════════════

/// bytes_returned → 记录条数，钳制到缓冲容量（驱动异常谎报超缓冲
/// 字节数时不死循环、不越界 — review）。
#[inline]
fn records_from_bytes(bytes_returned: u32, record_size: usize, capacity: usize) -> usize {
    (bytes_returned as usize / record_size).min(capacity)
}

/// IOCTL_READ 原语：读入 `out` 字节缓冲，返回实际读取字节数。
///
/// ERROR_NO_MORE_ITEMS（0x80070103）是**队列已空的正常信号** — 驱动对
/// 空队列读以此状态完成（排空循环每次都会遇到一次）。C 版不区分错误与
/// 空读（统一按 bytes_returned=0 处理）；此处显式归一为 `Ok(0)`，只有
/// 真错误才向上传播告警（实测：否则每次按键都刷一条 warn）。
fn ioctl_read(handle: HANDLE, out: &mut [u8]) -> Result<u32> {
    let mut bytes_returned = 0u32;
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_READ,
            None,
            0,
            Some(out.as_mut_ptr() as *mut c_void),
            out.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    }
    .or_else(|e| {
        if e.code() == HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0) {
            Ok(())
        } else {
            Err(e)
        }
    })?;
    Ok(bytes_returned)
}

/// IOCTL_WRITE 原语：写入 `input` 字节缓冲，返回实际写入字节数。
fn ioctl_write(handle: HANDLE, input: &[u8]) -> Result<u32> {
    let mut bytes_returned = 0u32;
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_WRITE,
            Some(input.as_ptr() as *const c_void),
            input.len() as u32,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        )?;
    }
    Ok(bytes_returned)
}

/// 分批写入骨架：栈上构造线上格式（`fill`）→ ioctl_write → 字节换算。
/// 部分写/失败即停并告警（C 版失败后返回垃圾计数 — 此处确定语义）。
fn write_chunks<Raw, Out>(handle: HANDLE, input: &[Out], fill: impl Fn(&Out) -> Raw) -> usize
where
    Raw: Copy + Default,
{
    let record = std::mem::size_of::<Raw>();
    let mut written = 0usize;
    for chunk in input.chunks(MAX_STROKES_PER_IOCTL) {
        let mut raw = [Raw::default(); MAX_STROKES_PER_IOCTL];
        for (i, item) in chunk.iter().enumerate() {
            raw[i] = fill(item);
        }
        // 仅本块实际占用的前缀字节入缓冲（数组尾部零填充不参与）
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(raw.as_ptr() as *const u8, chunk.len() * record) };
        match ioctl_write(handle, bytes) {
            Ok(n) => {
                let done = records_from_bytes(n, record, chunk.len());
                written += done;
                if done < chunk.len() {
                    warn!(
                        "interception write: partial ({done}/{}) — 尾部丢弃",
                        chunk.len()
                    );
                    break;
                }
            }
            Err(e) => {
                warn!("interception write failed: {e}");
                break;
            }
        }
    }
    written
}

/// 分批读取骨架：ioctl_read → 钳制换算 → `map` 转换回公开 stroke。
/// 返回实际读到的条数（调用方据此切片 — 前缀契约）。
fn read_chunks<Raw, Out>(handle: HANDLE, out: &mut [Out], map: impl Fn(&Raw) -> Out) -> usize
where
    Raw: Copy + Default,
{
    let record = std::mem::size_of::<Raw>();
    let mut total = 0usize;
    for chunk in out.chunks_mut(MAX_STROKES_PER_IOCTL) {
        let mut raw = [Raw::default(); MAX_STROKES_PER_IOCTL];
        let bytes: &mut [u8] = unsafe {
            std::slice::from_raw_parts_mut(raw.as_mut_ptr() as *mut u8, chunk.len() * record)
        };
        match ioctl_read(handle, bytes) {
            Ok(n) => {
                let done = records_from_bytes(n, record, chunk.len());
                for (dst, src) in chunk.iter_mut().zip(raw.iter()).take(done) {
                    *dst = map(src);
                }
                total += done;
                if done < chunk.len() {
                    // 读不满即队列已排空 — 本驱动实测语义（C 版调用方
                    // 自行循环直到返回 0，同此约定；非"对齐 interception_receive"
                    // 的文档式承诺，review 核实 C 库不钳制 nstroke）
                    break;
                }
            }
            Err(e) => {
                warn!("interception read failed: {e}");
                break;
            }
        }
    }
    total
}

impl Context {
    /// 打开全部 20 个设备并注册事件（对齐 C 版 `interception_create_context`）。
    pub fn create() -> Result<Self> {
        let mut devices = std::array::from_fn(|_| DeviceSlot::default());
        for (i, slot) in devices.iter_mut().enumerate() {
            *slot = open_device(i)?; // 失败时已填充的槽随数组 Drop 关闭句柄
        }
        Ok(Self { devices })
    }

    fn device_slot(&self, number: usize) -> &DeviceSlot {
        // number 由 Device 新类型产生，恒为 1..=20
        &self.devices[number - 1]
    }

    // ── 等待 ───────────────────────────────────────────────

    /// 阻塞等待任意设备有数据；等待失败返回 `None`（对齐
    /// `interception_wait` 失败返回 0 的哨兵语义，Rust 化 —
    /// review：此前把 WAIT_FAILED 伪装成"键盘 0 有数据"，
    /// 故障被永久遮蔽且配合粘滞事件形成无超时忙等）。
    pub fn wait(&self) -> Option<Device> {
        self.wait_with_timeout(u32::MAX)
    }

    /// 带超时等待。返回有待处理数据的 [`Device`]；超时/失败返回 `None`
    /// （对齐 `interception_wait_with_timeout` 返回 0 的语义，Rust 化）。
    pub fn wait_with_timeout(&self, milliseconds: u32) -> Option<Device> {
        let handles: [HANDLE; MAX_DEVICE] = self.devices.each_ref().map(|d| d.unempty);
        let result: WAIT_EVENT = unsafe { WaitForMultipleObjects(&handles, false, milliseconds) };
        // WAIT_OBJECT_0 + n (n < 20)；WAIT_TIMEOUT(0x102) / WAIT_FAILED(0xFFFF_FFFF)
        // 均落在区间外
        let index = result.0 as usize;
        if index >= MAX_DEVICE {
            return None;
        }
        Some(if index < MAX_KEYBOARD {
            Device::Keyboard(KeyboardDevice(index as u8))
        } else {
            Device::Mouse(MouseDevice((index - MAX_KEYBOARD) as u8))
        })
    }

    // ── 过滤 / 优先级 / 硬件 ID ─────────────────────────────

    /// 为满足谓词的设备设置过滤器（对齐 `interception_set_filter`）。
    pub fn set_filter(&self, predicate: impl Fn(Device) -> bool, filter: u16) {
        for (i, slot) in self.devices.iter().enumerate() {
            let device = if i < MAX_KEYBOARD {
                Device::Keyboard(KeyboardDevice(i as u8))
            } else {
                Device::Mouse(MouseDevice((i - MAX_KEYBOARD) as u8))
            };
            if predicate(device) {
                unsafe {
                    let _ = DeviceIoControl(
                        slot.handle,
                        IOCTL_SET_FILTER,
                        Some(&filter as *const u16 as *const c_void),
                        std::mem::size_of::<u16>() as u32,
                        None,
                        0,
                        None,
                        None,
                    );
                }
            }
        }
    }

    /// 读取指定设备的当前过滤器；失败返回 0（对齐 C 版失败语义）。
    pub fn get_filter(&self, device: Device) -> u16 {
        let mut filter = 0u16;
        unsafe {
            let _ = DeviceIoControl(
                self.device_slot(device.number()).handle,
                IOCTL_GET_FILTER,
                None,
                0,
                Some(&mut filter as *mut u16 as *mut c_void),
                std::mem::size_of::<u16>() as u32,
                None,
                None,
            );
        }
        filter
    }

    /// 设置设备优先级（对齐 `interception_set_precedence`）。
    pub fn set_precedence(&self, device: Device, precedence: i32) {
        unsafe {
            let _ = DeviceIoControl(
                self.device_slot(device.number()).handle,
                IOCTL_SET_PRECEDENCE,
                Some(&precedence as *const i32 as *const c_void),
                std::mem::size_of::<i32>() as u32,
                None,
                0,
                None,
                None,
            );
        }
    }

    /// 读取设备优先级；失败返回 0。
    pub fn get_precedence(&self, device: Device) -> i32 {
        let mut precedence = 0i32;
        unsafe {
            let _ = DeviceIoControl(
                self.device_slot(device.number()).handle,
                IOCTL_GET_PRECEDENCE,
                None,
                0,
                Some(&mut precedence as *mut i32 as *mut c_void),
                std::mem::size_of::<i32>() as u32,
                None,
                None,
            );
        }
        precedence
    }

    /// 读取设备硬件 ID 字符串，返回写入的字节数。
    pub fn get_hardware_id(&self, device: Device, buffer: &mut [u8]) -> u32 {
        let mut bytes_returned = 0u32;
        unsafe {
            let _ = DeviceIoControl(
                self.device_slot(device.number()).handle,
                IOCTL_GET_HARDWARE_ID,
                None,
                0,
                Some(buffer.as_mut_ptr() as *mut c_void),
                buffer.len() as u32,
                Some(&mut bytes_returned),
                None,
            );
        }
        bytes_returned
    }

    // ── 发送（类型化，栈上分批转换）──────────────────────────

    /// 发送键盘 stroke 序列，返回实际写入数（对齐 `interception_send`）。
    ///
    /// DeviceIoControl 失败即停：返回值是确定语义（C 版失败后读取未定义的
    /// bytes_returned，返回垃圾计数 — 文档化改进）。
    pub fn send_keyboard(
        &self,
        device: KeyboardDevice,
        strokes: &[InterceptionKeyStroke],
    ) -> usize {
        write_chunks(
            self.device_slot(device.0 as usize + 1).handle,
            strokes,
            |ks| KeyboardInputData::from(*ks),
        )
    }

    /// 发送鼠标 stroke 序列，返回实际写入数。
    pub fn send_mouse(&self, device: MouseDevice, strokes: &[InterceptionMouseStroke]) -> usize {
        write_chunks(
            self.device_slot(MAX_KEYBOARD + device.0 as usize + 1)
                .handle,
            strokes,
            |ms| MouseInputData::from(*ms),
        )
    }

    // ── 接收（类型化，栈上分批转换）──────────────────────────

    /// 接收键盘 stroke 序列，返回**实际读到的前缀切片**（对齐
    /// `interception_receive`）。
    ///
    /// 一次 IOCTL_READ 最多读 `out` 长度条（上限 [`MAX_STROKES_PER_IOCTL`]
    /// 分批）— 引擎侧以批缓冲调用，突发输入一个系统调用取回。
    ///
    /// 返回切片而非条数是**防误用 API 设计**：调用方遍历返回值天然只
    /// 覆盖真实条目 — 若返回 usize 而调用方误遍历整个缓冲，缓冲尾部的
    /// 陈旧/零值条目会被当作真实输入转发（实测：每条真实事件后跟 31 条
    /// 幻影 code=0 按键，取消 modifier 松开触发动作 — Win 开始菜单/
    /// Shift 输入法切换/Win+Tab 全部失效）。
    pub fn receive_keyboard<'a>(
        &self,
        device: KeyboardDevice,
        out: &'a mut [InterceptionKeyStroke],
    ) -> &'a mut [InterceptionKeyStroke] {
        let total = read_chunks(
            self.device_slot(device.0 as usize + 1).handle,
            out,
            |raw: &KeyboardInputData| InterceptionKeyStroke::from(*raw),
        );
        &mut out[..total]
    }

    /// 接收鼠标 stroke 序列，返回实际读到的前缀切片。
    pub fn receive_mouse<'a>(
        &self,
        device: MouseDevice,
        out: &'a mut [InterceptionMouseStroke],
    ) -> &'a mut [InterceptionMouseStroke] {
        let total = read_chunks(
            self.device_slot(MAX_KEYBOARD + device.0 as usize + 1)
                .handle,
            out,
            |raw: &MouseInputData| InterceptionMouseStroke::from(*raw),
        );
        &mut out[..total]
    }
}

// ═══════════════════════════════════════════════════════════════════
// 单测 — 无驱动可测的纯逻辑：协议字段映射往返、设备编号、钳制算术
// （幻影按键回归网：记录数钳制 + 前缀切片契约的算术底座）
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_numbering_matches_c_protocol() {
        assert_eq!(Device::Keyboard(keyboard(0)).number(), 1);
        assert_eq!(Device::Keyboard(keyboard(9)).number(), 10);
        assert_eq!(Device::Mouse(mouse(0)).number(), 11);
        assert_eq!(Device::Mouse(mouse(9)).number(), 20);
        assert!(Device::Keyboard(keyboard(0)).is_keyboard());
        assert!(!Device::Mouse(mouse(0)).is_keyboard());
    }

    #[test]
    #[should_panic]
    fn keyboard_constructor_rejects_out_of_range() {
        let _ = keyboard(10);
    }

    #[test]
    #[should_panic]
    fn mouse_constructor_rejects_out_of_range() {
        let _ = mouse(256);
    }

    #[test]
    fn keyboard_roundtrip_preserves_all_fields() {
        let ks = InterceptionKeyStroke {
            code: 0x64,
            state: 0x03, // E0 | KEY_UP
            information: 0xDEAD_BEEF,
        };
        let raw: KeyboardInputData = ks.into();
        // 线上格式 unit_id/reserved 恒 0（对齐 C 版 send 转换）
        assert_eq!((raw.unit_id, raw.reserved), (0, 0));
        let back: InterceptionKeyStroke = raw.into();
        assert_eq!(back, ks);
    }

    #[test]
    fn mouse_roundtrip_preserves_all_fields() {
        let ms = InterceptionMouseStroke {
            state: 0x0402,
            flags: 1,
            rolling: -120,
            x: 65535,
            y: -1,
            information: 7,
        };
        let raw: MouseInputData = ms.into();
        let back: InterceptionMouseStroke = raw.into();
        assert_eq!(back, ms);
    }

    #[test]
    fn records_from_bytes_clamps_to_capacity() {
        // 驱动异常谎报超缓冲字节数时不越界（review V7）
        assert_eq!(records_from_bytes(12 * 40, 12, 32), 32);
        assert_eq!(records_from_bytes(12 * 5, 12, 32), 5);
        assert_eq!(records_from_bytes(0, 12, 32), 0);
        assert_eq!(records_from_bytes(11, 12, 32), 0); // 非整条字节忽略
    }
}
