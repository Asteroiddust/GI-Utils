//! 屏幕工具：像素取色与光标位置 — Screen utilities: pixel color capture, cursor position.
//!
//! 使用 GDI `GetPixel` 进行单像素读取（每次调用约 0.01ms）。
//! Uses GDI `GetPixel` for single-pixel reads (~0.01ms per call).
//! 如需批量截取，考虑使用 DXGI Desktop Duplication。
//! For bulk capture, consider DXGI Desktop Duplication.

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC, HDC};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

// ── RAII screen DC ────────────────────────────────────────────

/// RAII 封装的屏幕设备上下文 — RAII wrapper for a screen device context.
/// Automatically calls `ReleaseDC` on drop.
struct ScreenDC {
    hdc: HDC,
}

impl ScreenDC {
    fn new() -> Option<Self> {
        let hdc = unsafe { GetDC(None) };
        if hdc.is_invalid() {
            None
        } else {
            Some(Self { hdc })
        }
    }

    fn raw(&self) -> HDC {
        self.hdc
    }
}

impl Drop for ScreenDC {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(None, self.hdc);
        }
    }
}

// GDI 设备上下文是进程级资源，绑定到桌面。可在线程间安全共享，因为：
//   - `GetDC(NULL)` 返回在整个会话中有效的桌面 DC
//   - `GetPixel` 是只读操作——并发读取不产生竞争
//   - DC 句柄在 `ReleaseDC`（仅发生在 Drop 时）之前始终有效
//   - `&self` 方法不暴露可变状态
// GDI device contexts (DC) are process-wide resources tied to a desktop.
// Safe to share across threads because:
//   - `GetDC(NULL)` returns a desktop DC valid for the entire session
//   - `GetPixel` is a read-only operation — concurrent reads don't race
//   - The DC handle is valid until `ReleaseDC`, which only happens on Drop
//   - No mutable state is exposed through `&self` methods
unsafe impl Send for ScreenDC {}
unsafe impl Sync for ScreenDC {}

// ── Pixel color ───────────────────────────────────────────────

/// RGBA 像素颜色 — RGBA color value from a screen pixel.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PixelColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl PixelColor {
    /// 从原始 COLORREF 值创建 — Create from a raw Windows COLORREF value (0x00BBGGRR).
    pub fn from_colorref(cr: u32) -> Self {
        Self {
            r: (cr & 0xFF) as u8,
            g: ((cr >> 8) & 0xFF) as u8,
            b: ((cr >> 16) & 0xFF) as u8,
        }
    }

    /// 余弦相似度 — Cosine similarity between two colors in RGB space.
    ///
    /// 返回值在 `[−1, 1]` 范围；1.0 表示颜色完全相同。
    /// Returns a value in `[−1, 1]`; 1.0 means identical colors.
    /// 游戏 UI 像素检测通常使用 `> 0.98` 阈值。
    /// For game UI pixel detection, a threshold of `> 0.98` is typical.
    /// 建议先做 R/G/B 通道预过滤，跳过昂贵的 `sqrt` 运算。
    /// Pre-filter with an R/G/B channel check to skip expensive `sqrt`.
    pub fn cosine_similarity(&self, other: &PixelColor) -> f64 {
        let (r1, g1, b1) = (self.r as f64, self.g as f64, self.b as f64);
        let (r2, g2, b2) = (other.r as f64, other.g as f64, other.b as f64);

        let dot = r1 * r2 + g1 * g2 + b1 * b2;
        let mag1 = (r1 * r1 + g1 * g1 + b1 * b1).sqrt();
        let mag2 = (r2 * r2 + g2 * g2 + b2 * b2).sqrt();

        if mag1 == 0.0 || mag2 == 0.0 {
            return 0.0;
        }
        dot / (mag1 * mag2)
    }
}

/// 取屏幕像素颜色 — Capture the color of a pixel at the given screen coordinates.
///
/// 若无法获取 DC 或像素在可见区域外（`CLR_INVALID`）则返回 `None`。
/// Returns `None` if the DC cannot be acquired, or if the pixel is
/// outside the visible region (`CLR_INVALID`).
pub fn get_pixel_color(x: i32, y: i32) -> Option<PixelColor> {
    let dc = ScreenDC::new()?;

    // Only the GetPixel FFI call is unsafe
    let cr = unsafe { GetPixel(dc.raw(), x, y) }.0;

    // CLR_INVALID (0xFFFFFFFF) means the pixel is off-screen or clipped
    if cr == 0xFFFF_FFFF {
        return None;
    }

    Some(PixelColor::from_colorref(cr))
}

// ── PixelReader (cached DC) ────────────────────────────────────

/// 缓存 DC 的像素读取器 — A cached screen DC for repeated pixel reads.
///
/// 与 [`get_pixel_color`] 不同（每次调用创建/销毁 DC），`PixelReader` 在读取器
/// 存活期间保持 DC 开启。适用于紧循环中的连续像素读取（如持续监控）。
/// Unlike [`get_pixel_color`], which creates/destroys a DC on every call,
/// `PixelReader` holds the DC open for the lifetime of the reader.
/// Use this when reading pixels in a tight loop (e.g. continuous monitoring).
///
/// **DC 故意不释放（泄漏策略）**：PixelReader 实例在注册线程（GUI 主线程/
/// 工厂）创建、在功能线程使用，最后一次 Arc 可能在功能线程 drop —
/// MSDN 契约要求 ReleaseDC 与 GetDC 同线程，跨线程释放无合法路径；
/// 泄漏单个桌面 DC 到进程退出由 OS 回收（与托盘自建图标同策略，
/// review 3.1）。
pub struct PixelReader {
    dc: std::mem::ManuallyDrop<ScreenDC>,
}

impl PixelReader {
    /// 获取屏幕 DC — Acquire a screen DC for repeated reads.
    /// 获取失败则返回 `None`。
    /// Returns `None` if the DC cannot be acquired.
    pub fn new() -> Option<Self> {
        Some(Self {
            dc: std::mem::ManuallyDrop::new(ScreenDC::new()?),
        })
    }

    /// 原始 DC 句柄 — Raw DC handle for direct GDI calls.
    pub fn raw_dc(&self) -> HDC {
        self.dc.raw()
    }

    /// 读取像素颜色 — Read the color of a pixel at the given screen coordinates.
    /// 像素在可见区域外则返回 `None`。
    /// Returns `None` if the pixel is outside the visible region.
    pub fn read(&self, x: i32, y: i32) -> Option<PixelColor> {
        let cr = unsafe { GetPixel(self.dc.raw(), x, y) }.0;
        if cr == 0xFFFF_FFFF {
            return None;
        }
        Some(PixelColor::from_colorref(cr))
    }
}

// ── Cursor position ───────────────────────────────────────────

/// 获取光标位置 — Get the current cursor position in screen coordinates.
///
/// 调用失败则返回 `None`（极为罕见）。
/// Returns `None` if the call fails (extremely rare).
pub fn get_cursor_pos() -> Option<POINT> {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) }.ok()?;
    Some(pt)
}
