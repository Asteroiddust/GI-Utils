//! Interception 驱动库的原始 FFI 绑定
//!
//! 这些声明精确对应 `interception.h`。本模块的所有函数均为 unsafe 直接调用，
//! 请使用 `super::context` 和 `super::strokes` 中的安全封装。
//! Raw FFI bindings to the Interception driver library — mirrors `interception.h`.
//! Use the safe wrappers in `super::context` and `super::strokes` instead.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::c_void;

// ── Type aliases ─────────────────────────────────────────────

/// Interception 上下文的不透明句柄。通过 `interception_create_context` 创建。
pub type InterceptionContext = *mut c_void;

/// 设备索引：键盘 1-10，鼠标 11-20。
pub type InterceptionDevice = i32;

/// 设备优先级值。
pub type InterceptionPrecedence = i32;

/// 过滤器位掩码 (bitmask)，指定要拦截的事件类型。
pub type InterceptionFilter = u16;

/// 谓词函数指针类型，传递给 `interception_set_filter` 用於筛选设备。
pub type InterceptionPredicate = unsafe extern "C" fn(InterceptionDevice) -> i32;

// ── Device index macros (re-exported as const fns) ───────────

/// 最大键盘设备数。
pub const INTERCEPTION_MAX_KEYBOARD: i32 = 10;

/// 最大鼠标设备数。
pub const INTERCEPTION_MAX_MOUSE: i32 = 10;

/// 最大总设备数（键盘 + 鼠标）。
pub const INTERCEPTION_MAX_DEVICE: i32 = INTERCEPTION_MAX_KEYBOARD + INTERCEPTION_MAX_MOUSE;

/// 根据索引返回键盘设备 ID（从 1 开始）。
pub const fn interception_keyboard(index: i32) -> InterceptionDevice {
    index + 1
}

/// 根据索引返回鼠标设备 ID（从键盘最大数 + 1 开始）。
pub const fn interception_mouse(index: i32) -> InterceptionDevice {
    INTERCEPTION_MAX_KEYBOARD + index + 1
}

// ── Key state flags ──────────────────────────────────────────

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

// ── Filter key states ────────────────────────────────────────

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

// ── Mouse state flags ────────────────────────────────────────

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

// Aliases
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

// ── Filter mouse states ──────────────────────────────────────

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

// ── Mouse flags ──────────────────────────────────────────────

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

// ── Stroke structs (repr(C) to match C layout) ───────────────

/// Interception 键盘输入数据包。
///
/// 大小: 8 字节, 对齐: 4。通过 Interception 发送或接收。
/// Size: 8 bytes, alignment: 4.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InterceptionKeyStroke {
    pub code: u16,
    pub state: u16,
    pub information: u32,
}

/// Interception 鼠标输入数据包。
///
/// 大小: 20 字节, 对齐: 4。通过 Interception 发送或接收。
/// Size: 20 bytes, alignment: 4.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InterceptionMouseStroke {
    pub state: u16,
    pub flags: u16,
    pub rolling: i16,
    pub x: i32,
    pub y: i32,
    pub information: u32,
}

/// 原始输入缓冲区大小（取鼠标数据包大小）。
///
/// 键盘数据包可安全放入此缓冲区，尾部填充为零。
/// Matches `sizeof(InterceptionMouseStroke)` — keyboard strokes fit
/// inside with overflow zeroed.
pub const STROKE_SIZE: usize = std::mem::size_of::<InterceptionMouseStroke>();

/// 不透明输入缓冲区，始终通过指针传递给 Interception 函数。
///
/// 用于读写键盘/鼠标输入的原始字节。
/// Opaque stroke buffer — always passed by pointer to Interception functions.
pub type InterceptionStroke = [u8; STROKE_SIZE];

// ── FFI function declarations ────────────────────────────────

extern "C" {
    /// 创建 Interception 上下文，分配驱动资源。
    pub fn interception_create_context() -> InterceptionContext;
    /// 销毁 Interception 上下文，释放驱动资源。
    pub fn interception_destroy_context(context: InterceptionContext);

    /// 为指定设备设置过滤器，决定拦截哪些输入事件。
    pub fn interception_set_filter(
        context: InterceptionContext,
        predicate: InterceptionPredicate,
        filter: InterceptionFilter,
    );

    /// 等待输入事件到达，阻塞直到有事件。
    pub fn interception_wait(context: InterceptionContext) -> InterceptionDevice;

    /// 带超时的等待，超时返回 0。
    pub fn interception_wait_with_timeout(
        context: InterceptionContext,
        milliseconds: u32,
    ) -> InterceptionDevice;

    /// 向指定设备发送输入数据包。
    pub fn interception_send(
        context: InterceptionContext,
        device: InterceptionDevice,
        stroke: *const InterceptionStroke,
        nstroke: u32,
    ) -> i32;

    /// 从指定设备接收输入数据包。
    pub fn interception_receive(
        context: InterceptionContext,
        device: InterceptionDevice,
        stroke: *mut InterceptionStroke,
        nstroke: u32,
    ) -> i32;

    /// 检查设备是否为键盘。
    pub fn interception_is_keyboard(device: InterceptionDevice) -> i32;
    /// 检查设备是否为鼠标。
    pub fn interception_is_mouse(device: InterceptionDevice) -> i32;
    /// 检查设备是否无效。
    pub fn interception_is_invalid(device: InterceptionDevice) -> i32;

    /// 获取设备的硬件 ID 字符串。
    pub fn interception_get_hardware_id(
        context: InterceptionContext,
        device: InterceptionDevice,
        hardware_id_buffer: *mut c_void,
        buffer_size: u32,
    ) -> u32;

    /// 获取设备的当前优先级。
    pub fn interception_get_precedence(
        context: InterceptionContext,
        device: InterceptionDevice,
    ) -> InterceptionPrecedence;

    /// 设置设备的优先级。
    pub fn interception_set_precedence(
        context: InterceptionContext,
        device: InterceptionDevice,
        precedence: InterceptionPrecedence,
    );

    /// 获取设备的当前过滤器。
    pub fn interception_get_filter(
        context: InterceptionContext,
        device: InterceptionDevice,
    ) -> InterceptionFilter;
}
