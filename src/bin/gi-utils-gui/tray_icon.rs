//! 托盘图标原料与共享句柄 — Tray icon assets & shared handle.
//!
//! 纯数据 / 纯 GDI 部分，无窗口依赖：像素生成、RGBA→HICON、.ico 预加载、
//! 进程级共享句柄 `SharedIcon`。全部从原 tray.rs 原样迁移（零逻辑改动），
//! 仅按契约调整可见性与文档。
//!
//! 所有权契约（L4）：主线程创建与销毁共享图标，托盘线程与各轮 app 只读使用。

// edition 2024 lint 豁免（模块级）：本模块的 unsafe fn 是纯 Win32/GDI
// 区段（LoadImageW/CreateBitmap/CreateIconIndirect），unsafe 边界即函数
// 签名本身 — 内部逐调用嵌套 unsafe {} 只增噪音。
#![allow(unsafe_op_in_unsafe_fn)]

use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

// ═══════════════════════════════════════════════════════════════════
// 像素生成（纯数据）
// ═══════════════════════════════════════════════════════════════════

/// 生成 32x32 RGBA 像素（蓝色圆形 + 白色 "G"）— 纯数据函数，无 Win32 依赖。
/// 返回 (pixels, width, height)，width == height == 32。
pub fn create_tray_icon_pixels() -> (Vec<u8>, u32, u32) {
    let size = 32u32;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    let blue = [0x1Au8, 0x73, 0xE8, 0xFF];
    let white = [0xFFu8, 0xFF, 0xFF, 0xFF];
    let transparent = [0x00u8, 0x00, 0x00, 0x00];
    let g_shape: &[(u32, u32)] = &[
        (10, 6),
        (11, 6),
        (12, 6),
        (13, 6),
        (14, 6),
        (15, 6),
        (16, 6),
        (17, 6),
        (18, 6),
        (19, 6),
        (20, 6),
        (21, 6),
        (9, 7),
        (9, 8),
        (9, 9),
        (9, 10),
        (9, 11),
        (9, 12),
        (9, 13),
        (9, 14),
        (9, 15),
        (9, 16),
        (9, 17),
        (9, 18),
        (9, 19),
        (9, 20),
        (9, 21),
        (9, 22),
        (9, 23),
        (10, 24),
        (11, 24),
        (12, 24),
        (13, 24),
        (14, 24),
        (15, 24),
        (16, 24),
        (17, 24),
        (18, 24),
        (19, 24),
        (20, 24),
        (21, 24),
        (22, 19),
        (22, 20),
        (22, 21),
        (22, 22),
        (22, 23),
        (16, 15),
        (17, 15),
        (18, 15),
        (19, 15),
        (20, 15),
        (21, 15),
        (22, 15),
    ];
    for y in 0..size {
        for x in 0..size {
            let is_g = g_shape.contains(&(x, y));
            let dx = x as f32 - 15.5f32;
            let dy = y as f32 - 15.5f32;
            let in_circle = (dx * dx + dy * dy).sqrt() < 14.5f32;
            if is_g {
                pixels.extend_from_slice(&white);
            } else if in_circle {
                pixels.extend_from_slice(&blue);
            } else {
                pixels.extend_from_slice(&transparent);
            }
        }
    }
    (pixels, size, size)
}

// ═══════════════════════════════════════════════════════════════════
// HICON 创建（纯 GDI）
// ═══════════════════════════════════════════════════════════════════

/// RGBA → HICON（纯 GDI）。RGBA→BGRA 翻转 → CreateBitmap×2（AND mask 全 0：
/// 32bpp BGRA alpha 承载透明度，全 0xFF 按掩蔽语义渲染成不透明方块）→
/// CreateIconIndirect → 临时位图立即 DeleteObject。任何失败返回 None 不 panic。
pub(crate) unsafe fn create_hicon_from_rgba(rgba: &[u8], w: u32, h: u32) -> Option<HICON> {
    use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HGDIOBJ};
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};

    // RGBA → BGRA + 翻转
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    for row in (0..h).rev() {
        let start = (row * w * 4) as usize;
        let end = start + (w * 4) as usize;
        for px in rgba[start..end].chunks(4) {
            bgra.push(px[2]); // B
            bgra.push(px[1]); // G
            bgra.push(px[0]); // R
            bgra.push(px[3]); // A
        }
    }
    // AND mask 全 0（1bpp）：hbmColor 是 32bpp BGRA，alpha 通道已承载
    // 透明度。全 0xFF 会按"掩蔽"语义渲染 — 图标呈不透明方形。
    // All-zero AND mask: the 32bpp color bitmap carries alpha; an
    // all-0xFF mask would mask out every pixel and show a square icon.
    let mask_bits: Vec<u8> = vec![0; ((w * h) as usize + 7) / 8];
    let hbm_mask = CreateBitmap(
        w as i32,
        h as i32,
        1,
        1,
        Some(mask_bits.as_ptr() as *const std::ffi::c_void),
    );
    let hbm_color = CreateBitmap(
        w as i32,
        h as i32,
        1,
        32,
        Some(bgra.as_ptr() as *const std::ffi::c_void),
    );

    // CreateBitmap 失败或资源不足 → 不进入 panic 路径，返回 None 优雅降级
    if hbm_mask.is_invalid() || hbm_color.is_invalid() {
        let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
        let _ = DeleteObject(HGDIOBJ(hbm_color.0));
        return None;
    }
    let icon_info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: hbm_mask,
        hbmColor: hbm_color,
    };
    let icon = match CreateIconIndirect(&icon_info) {
        Ok(icon) if !icon.is_invalid() => Some(icon),
        _ => None,
    };
    // CreateIconIndirect 复制了位图内容 — 释放临时位图（避免泄漏）
    let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
    let _ = DeleteObject(HGDIOBJ(hbm_color.0));
    icon
}

// ═══════════════════════════════════════════════════════════════════
// .ico 文件加载（LoadImageW — 仅启动健康态）
// ═══════════════════════════════════════════════════════════════════

/// LoadImageW + LR_LOADFROMFILE，32×32；失败重试 5 次 × 200ms 后放弃。
/// 仅由 preload_tray_icon 调用（启动健康态）— 恢复轮绝不重试 LoadImageW
/// （L4 WIC 污染是进程级永久态，重试无效）。
unsafe fn load_ico_from_file(path: &str) -> Option<HICON> {
    use windows::Win32::UI::WindowsAndMessaging::{IMAGE_ICON, LR_LOADFROMFILE, LoadImageW};
    use windows::core::PCWSTR;

    let mut wide: Vec<u16> = path.encode_utf16().collect();
    // LoadImageW 扫描至 0x0000 — 显式 NUL 终止，防越界读（review 发现，
    // 与 tray.rs szTip 同类的边界纪律）
    wide.push(0);
    // 睡眠唤醒/显示重置后 WIC 成像组件可能瞬时不可用（崩溃恢复轮首次尝试
    // 失败、数秒后自愈）— 重试 5 次 × 200ms 再放弃，覆盖恢复窗口期。
    for _ in 0..5 {
        match LoadImageW(
            None,
            PCWSTR(wide.as_ptr()),
            IMAGE_ICON,
            32,
            32,
            LR_LOADFROMFILE,
        ) {
            Ok(handle) if !handle.is_invalid() => return Some(HICON(handle.0)),
            _ => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
    None
}

/// 组合入口：非空 path 且加载成功用 .ico，否则回退 create_hicon_from_rgba。绝不失败。
unsafe fn load_icon(icon_path: &str, pixels: &[u8], w: u32, h: u32) -> Option<HICON> {
    if !icon_path.is_empty() {
        if let Some(icon) = load_ico_from_file(icon_path) {
            return Some(icon);
        }
        tracing::warn!(
            "tray icon '{}' could not be loaded, using generated icon",
            icon_path
        );
    }
    create_hicon_from_rgba(pixels, w, h)
}

// ═══════════════════════════════════════════════════════════════════
// 进程级共享句柄
// ═══════════════════════════════════════════════════════════════════

/// 进程级共享托盘图标。契约（L4）：主线程创建与销毁，托盘线程与各轮 app 只读使用。
/// HICON 是进程级 GDI 对象；windows-rs 因含裸指针未标 Send，此处显式声明 —
/// 由"单写者销毁 + 其余只读"契约支撑，契约不扩展。
#[derive(Clone)]
pub struct SharedIcon(HICON);
unsafe impl Send for SharedIcon {}

impl SharedIcon {
    /// 原始 HICON — 供 WM_SETICON 只读使用（app 帧循环 / 托盘线程）。
    pub(crate) fn raw(&self) -> HICON {
        self.0
    }

    /// 仅 Runtime::shutdown 最后一步调用（恰一次，所有引用方已退出）。
    /// 内部 DestroyIcon(self.0)。
    pub(crate) fn destroy(&self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

/// 启动预加载入口 — 仅 main 在健康 GDI/WIC 态调用一次；恢复轮绝不调用（L4 WIC 污染）。
/// icon_path 为空或 LoadImageW 失败 → 回退纯 GDI 生成；两者皆败 → None（托盘线程自建兜底）。
pub fn preload_tray_icon(icon_path: &str, pixels: &[u8], w: u32, h: u32) -> Option<SharedIcon> {
    unsafe { load_icon(icon_path, pixels, w, h) }.map(SharedIcon)
}
