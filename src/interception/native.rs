//! Interception 用户层 API 的原生 Rust 移植
//!
//! 移植自 oblitum/Interception `library/interception.c`（LGPL 3.0，许可文本见
//! `Interception/licenses/`）。替代原预编译 interception.lib：内核驱动侧
//! 保持不变，本模块只实现用户态 DeviceIoControl 协议端。
//!
//! # 相对 C 原版的改进
//!
//! - **类型化 send/receive**：直接收发 [`InterceptionKeyStroke`] /
//!   [`InterceptionMouseStroke`] 切片 — 消除 C 版每次调用的 HeapAlloc
//!   （栈上分批转换）；原 `strokes.rs` 的非对齐字节缓冲往返整体删除
//! - **错误传播**：CreateFile / CreateEvent / DeviceIoControl 失败返回
//!   `windows::core::Result`（C 版全部忽略返回值）；create 中途失败时
//!   已打开的资源由 `Drop` 自动清理（C 版依赖手工 destroy 路径）
//! - **set_filter 接受 Rust 闭包**（C 版需 extern "C" 函数指针）
//! - **设备判定为 `const` 内联纯函数**，零 FFI 调用开销
//! - **Context RAII**：句柄由结构所有权 + Drop 管理，无手工释放路径

#![allow(dead_code)] // 协议完整性保留（precedence/hardware_id 等当前未调用）

use std::ffi::c_void;

use windows::core::{Result, PCSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_EVENT};
use windows::Win32::Storage::FileSystem::{
    CreateFileA, FILE_CREATION_DISPOSITION, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ,
    FILE_SHARE_MODE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Threading::{CreateEventA, WaitForMultipleObjects};

// ═══════════════════════════════════════════════════════════════════
// 设备编号 — Device numbering（与 interception.h 宏一一对应）
// ═══════════════════════════════════════════════════════════════════

/// 最大键盘设备数。
pub const MAX_KEYBOARD: usize = 10;

/// 最大鼠标设备数。
pub const MAX_MOUSE: usize = 10;

/// 最大总设备数（键盘 + 鼠标）。
pub const MAX_DEVICE: usize = MAX_KEYBOARD + MAX_MOUSE;

/// 键盘设备号（1..=10）。
#[inline]
pub const fn keyboard(index: usize) -> usize {
    index + 1
}

/// 鼠标设备号（11..=20）。
#[inline]
pub const fn mouse(index: usize) -> usize {
    MAX_KEYBOARD + index + 1
}

/// 设备号是否为键盘（C 版 `interception_is_keyboard` 的纯函数化；
/// 区间比较手写 — `RangeInclusive::contains` 尚非 const fn）。
#[inline]
pub const fn is_keyboard(device: usize) -> bool {
    device >= 1 && device <= MAX_KEYBOARD
}

/// 设备号是否为鼠标。
#[inline]
pub const fn is_mouse(device: usize) -> bool {
    device > MAX_KEYBOARD && device <= MAX_DEVICE
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
pub const INTERCEPTION_FILTER_KEY_TERMSRV_VKPACKET: u16 =
    INTERCEPTION_KEY_TERMSRV_VKPACKET << 1;

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

/// 单次 IOCTL 的栈缓冲 stroke 上限，超出分批（C 版 HeapAlloc 任意大小，
/// 栈分批零分配且足够本项目 1-stroke 调用）。
const MAX_STROKES_PER_IOCTL: usize = 32;

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

// HANDLE 为进程内可共享的内核对象引用；设备句柄/事件句柄跨线程使用安全
// （并发 send 由驱动串行化，事件仅被等待线程消费）。
unsafe impl Send for Context {}
unsafe impl Sync for Context {}

/// 打开单个设备：CreateFile → CreateEvent → IOCTL_SET_EVENT 注册事件
/// （对齐 C 版 `interception_create_context` 循环体）。
fn open_device(index: usize) -> Result<DeviceSlot> {
    let name = format!("\\\\.\\interception{index:02}");
    // C: CreateFileA(name, GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL)
    let handle = unsafe {
        CreateFileA(
            PCSTR(name.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_MODE(0),
            None,
            FILE_CREATION_DISPOSITION(OPEN_EXISTING.0),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }?;
    // C: CreateEventA(NULL, TRUE, FALSE, NULL)
    let unempty = unsafe { CreateEventA(None, true, false, None) }?;
    // C: 2 元素句柄数组 {event, NULL} 作输入缓冲注册事件
    let handles = [unempty, HANDLE::default()];
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_SET_EVENT,
            Some(handles.as_ptr() as *const c_void),
            std::mem::size_of_val(&handles) as u32,
            None,
            0,
            None,
            None,
        )?;
    }
    Ok(DeviceSlot { handle, unempty })
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

    fn device_slot(&self, device: usize) -> &DeviceSlot {
        // 调用方保证 device 为有效设备号（1..=20）
        &self.devices[device - 1]
    }

    // ── 等待 ───────────────────────────────────────────────

    /// 阻塞等待任意设备有数据（对齐 `interception_wait`）。
    pub fn wait(&self) -> usize {
        // 任一事件被置位即返回；等待失败按超时处理（返回 0）
        self.wait_with_timeout(u32::MAX).unwrap_or(0)
    }

    /// 带超时等待。返回 1..=20 的设备号；超时/失败返回 `None`
    /// （对齐 `interception_wait_with_timeout` 返回 0 的语义，Rust 化）。
    pub fn wait_with_timeout(&self, milliseconds: u32) -> Option<usize> {
        let handles: [HANDLE; MAX_DEVICE] = self.devices.each_ref().map(|d| d.unempty);
        let result: WAIT_EVENT = unsafe { WaitForMultipleObjects(&handles, false, milliseconds) };
        // WAIT_OBJECT_0 + n (n < 20)；WAIT_TIMEOUT(0x102) / WAIT_FAILED(0xFFFF_FFFF)
        // 均落在区间外
        let index = result.0;
        (index < MAX_DEVICE as u32).then_some(index as usize + 1)
    }

    // ── 过滤 / 优先级 / 硬件 ID ─────────────────────────────

    /// 为满足谓词的设备设置过滤器（对齐 `interception_set_filter`）。
    pub fn set_filter(&self, predicate: impl Fn(usize) -> bool, filter: u16) {
        for (i, slot) in self.devices.iter().enumerate() {
            if predicate(i + 1) {
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
    pub fn get_filter(&self, device: usize) -> u16 {
        let mut filter = 0u16;
        unsafe {
            let _ = DeviceIoControl(
                self.device_slot(device).handle,
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
    pub fn set_precedence(&self, device: usize, precedence: i32) {
        unsafe {
            let _ = DeviceIoControl(
                self.device_slot(device).handle,
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
    pub fn get_precedence(&self, device: usize) -> i32 {
        let mut precedence = 0i32;
        unsafe {
            let _ = DeviceIoControl(
                self.device_slot(device).handle,
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
    pub fn get_hardware_id(&self, device: usize, buffer: &mut [u8]) -> u32 {
        let mut bytes_returned = 0u32;
        unsafe {
            let _ = DeviceIoControl(
                self.device_slot(device).handle,
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
    pub fn send_keyboard(&self, device: usize, strokes: &[InterceptionKeyStroke]) -> usize {
        let slot = self.device_slot(device);
        let mut written = 0usize;
        for chunk in strokes.chunks(MAX_STROKES_PER_IOCTL) {
            let mut raw = [KeyboardInputData::default(); MAX_STROKES_PER_IOCTL];
            for (i, ks) in chunk.iter().enumerate() {
                raw[i] = KeyboardInputData {
                    unit_id: 0,
                    make_code: ks.code,
                    flags: ks.state,
                    reserved: 0,
                    extra_information: ks.information,
                };
            }
            let mut bytes_returned = 0u32;
            if unsafe {
                DeviceIoControl(
                    slot.handle,
                    IOCTL_WRITE,
                    Some(raw.as_ptr() as *const c_void),
                    (chunk.len() * std::mem::size_of::<KeyboardInputData>()) as u32,
                    None,
                    0,
                    Some(&mut bytes_returned),
                    None,
                )
            }
            .is_err()
            {
                break;
            }
            written += bytes_returned as usize / std::mem::size_of::<KeyboardInputData>();
        }
        written
    }

    /// 发送鼠标 stroke 序列，返回实际写入数。
    pub fn send_mouse(&self, device: usize, strokes: &[InterceptionMouseStroke]) -> usize {
        let slot = self.device_slot(device);
        let mut written = 0usize;
        for chunk in strokes.chunks(MAX_STROKES_PER_IOCTL) {
            let mut raw = [MouseInputData::default(); MAX_STROKES_PER_IOCTL];
            for (i, ms) in chunk.iter().enumerate() {
                raw[i] = MouseInputData {
                    unit_id: 0,
                    flags: ms.flags,
                    button_flags: ms.state,
                    button_data: ms.rolling as u16,
                    raw_buttons: 0,
                    last_x: ms.x,
                    last_y: ms.y,
                    extra_information: ms.information,
                };
            }
            let mut bytes_returned = 0u32;
            if unsafe {
                DeviceIoControl(
                    slot.handle,
                    IOCTL_WRITE,
                    Some(raw.as_ptr() as *const c_void),
                    (chunk.len() * std::mem::size_of::<MouseInputData>()) as u32,
                    None,
                    0,
                    Some(&mut bytes_returned),
                    None,
                )
            }
            .is_err()
            {
                break;
            }
            written += bytes_returned as usize / std::mem::size_of::<MouseInputData>();
        }
        written
    }

    // ── 接收（类型化，栈上分批转换）──────────────────────────

    /// 接收键盘 stroke 序列，返回实际读取数（对齐 `interception_receive`）。
    pub fn receive_keyboard(&self, device: usize, out: &mut [InterceptionKeyStroke]) -> usize {
        let slot = self.device_slot(device);
        let mut total = 0usize;
        for chunk in out.chunks_mut(MAX_STROKES_PER_IOCTL) {
            let mut raw = [KeyboardInputData::default(); MAX_STROKES_PER_IOCTL];
            let mut bytes_returned = 0u32;
            if unsafe {
                DeviceIoControl(
                    slot.handle,
                    IOCTL_READ,
                    None,
                    0,
                    Some(raw.as_mut_ptr() as *mut c_void),
                    (chunk.len() * std::mem::size_of::<KeyboardInputData>()) as u32,
                    Some(&mut bytes_returned),
                    None,
                )
            }
            .is_err()
            {
                break;
            }
            let n = bytes_returned as usize / std::mem::size_of::<KeyboardInputData>();
            for (i, ks) in chunk.iter_mut().enumerate().take(n) {
                *ks = InterceptionKeyStroke {
                    code: raw[i].make_code,
                    state: raw[i].flags,
                    information: raw[i].extra_information,
                };
            }
            total += n;
            if n < chunk.len() {
                break; // 读不满即无更多数据（对齐 C 版单次调用语义）
            }
        }
        total
    }

    /// 接收鼠标 stroke 序列，返回实际读取数。
    pub fn receive_mouse(&self, device: usize, out: &mut [InterceptionMouseStroke]) -> usize {
        let slot = self.device_slot(device);
        let mut total = 0usize;
        for chunk in out.chunks_mut(MAX_STROKES_PER_IOCTL) {
            let mut raw = [MouseInputData::default(); MAX_STROKES_PER_IOCTL];
            let mut bytes_returned = 0u32;
            if unsafe {
                DeviceIoControl(
                    slot.handle,
                    IOCTL_READ,
                    None,
                    0,
                    Some(raw.as_mut_ptr() as *mut c_void),
                    (chunk.len() * std::mem::size_of::<MouseInputData>()) as u32,
                    Some(&mut bytes_returned),
                    None,
                )
            }
            .is_err()
            {
                break;
            }
            let n = bytes_returned as usize / std::mem::size_of::<MouseInputData>();
            for (i, ms) in chunk.iter_mut().enumerate().take(n) {
                *ms = InterceptionMouseStroke {
                    state: raw[i].button_flags,
                    flags: raw[i].flags,
                    rolling: raw[i].button_data as i16,
                    x: raw[i].last_x,
                    y: raw[i].last_y,
                    information: raw[i].extra_information,
                };
            }
            total += n;
            if n < chunk.len() {
                break;
            }
        }
        total
    }
}
