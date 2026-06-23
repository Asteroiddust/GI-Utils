//! Screen utilities: pixel color capture, cursor position.
//!
//! Uses GDI `GetPixel` for single-pixel reads (~0.01ms per call).
//! For bulk capture, consider DXGI Desktop Duplication.

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC, HDC};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

// ── RAII screen DC ────────────────────────────────────────────

/// RAII wrapper for a screen device context.
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
        unsafe { ReleaseDC(None, self.hdc); }
    }
}

// GDI device contexts are process-wide resources — safe to share across threads.
unsafe impl Send for ScreenDC {}
unsafe impl Sync for ScreenDC {}

// ── Pixel color ───────────────────────────────────────────────

/// RGBA color value from a screen pixel.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PixelColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl PixelColor {
    /// Create from a raw Windows COLORREF value (0x00BBGGRR).
    pub fn from_colorref(cr: u32) -> Self {
        Self {
            r: (cr & 0xFF) as u8,
            g: ((cr >> 8) & 0xFF) as u8,
            b: ((cr >> 16) & 0xFF) as u8,
        }
    }

    /// Cosine similarity between two colors in RGB space.
    ///
    /// Returns a value in `[−1, 1]`; 1.0 means identical colors.
    /// For game UI pixel detection, a threshold of `> 0.98` is typical.
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

/// Capture the color of a pixel at the given screen coordinates.
///
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

/// A cached screen DC for repeated pixel reads.
///
/// Unlike [`get_pixel_color`], which creates/destroys a DC on every call,
/// `PixelReader` holds the DC open for the lifetime of the reader.
/// Use this when reading pixels in a tight loop (e.g. continuous monitoring).
pub struct PixelReader {
    dc: ScreenDC,
}

impl PixelReader {
    /// Acquire a screen DC for repeated reads.
    /// Returns `None` if the DC cannot be acquired.
    pub fn new() -> Option<Self> {
        Some(Self { dc: ScreenDC::new()? })
    }

    /// Read the color of a pixel at the given screen coordinates.
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

/// Get the current cursor position in screen coordinates.
///
/// Returns `None` if the call fails (extremely rare).
pub fn get_cursor_pos() -> Option<POINT> {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) }.ok()?;
    Some(pt)
}
