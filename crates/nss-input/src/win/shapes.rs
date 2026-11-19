//! Windows cursor shape inspection via `GetCursorInfo`, `LoadCursorW(IDC_*)` and `GetIconInfoExW`.
//!
//! Confined `unsafe` boundary (ADR 0012). Implements ADR 0008's 3-step ladder:
//! 1. Shared system handle table (`LoadCursorW(None, IDC_*)`) built once per recording.
//! 2. SHA-256 hash + 16x16 fingerprint of the cursor bitmap extracted via `GetIconInfoExW` /
//!    `GetDIBits`.
//! 3. `CursorShape::Unknown` (which the renderer draws as the arrow).

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use nss_core::CursorShape;
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorInfo, GetIconInfoExW, LoadCursorW, CURSORINFO, CURSOR_SHOWING, HCURSOR, ICONINFOEXW,
    IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_NO, IDC_SIZEALL, IDC_SIZENS, IDC_SIZEWE,
};

use crate::shapes::{
    bitmap_hash, CursorFingerprint, CursorShapeIdentifier, CursorShapeSampler, ShapeReference,
};

const STANDARD_IDC: &[(CursorShape, PCWSTR)] = &[
    (CursorShape::Arrow, IDC_ARROW),
    (CursorShape::IBeam, IDC_IBEAM),
    (CursorShape::PointingHand, IDC_HAND),
    (CursorShape::Crosshair, IDC_CROSS),
    (CursorShape::ResizeLeftRight, IDC_SIZEWE),
    (CursorShape::ResizeUpDown, IDC_SIZENS),
    (CursorShape::OpenHand, IDC_SIZEALL),
    (CursorShape::NotAllowed, IDC_NO),
];

/// Builds a [`CursorShapeSampler`] backed by `GetCursorInfo` and the standard Win32 cursor table.
#[must_use]
pub fn make_system_sampler() -> Option<CursorShapeSampler> {
    let identifier = Arc::new(build_standard_identifier());
    Some(Arc::new(move || sample_live_cursor(&identifier)))
}

/// Builds the handle map, hash map, and 16x16 fingerprint table for all standard Win32 cursors.
#[must_use]
pub fn build_standard_identifier() -> CursorShapeIdentifier {
    let mut handle_map: HashMap<usize, CursorShape> = HashMap::new();
    let mut hash_map: HashMap<[u8; 32], CursorShape> = HashMap::new();
    let mut references: Vec<ShapeReference> = Vec::new();

    for &(shape, idc) in STANDARD_IDC {
        // SAFETY: `LoadCursorW(None, IDC_*)` returns shared system cursor handles that must NOT
        // be destroyed.
        let Ok(hcursor) = (unsafe { LoadCursorW(None, idc) }) else {
            continue;
        };
        let raw = hcursor.0 as usize;
        handle_map.entry(raw).or_insert(shape);
        if let Some((hash, fp)) = extract_cursor_fingerprint(hcursor) {
            hash_map.entry(hash).or_insert(shape);
            references.push(ShapeReference {
                shape,
                fingerprint: fp,
            });
        }
    }

    CursorShapeIdentifier::new(references)
        .with_handle_map(handle_map)
        .with_hash_map(hash_map)
}

fn sample_live_cursor(identifier: &CursorShapeIdentifier) -> Option<CursorShape> {
    let mut info = CURSORINFO {
        cbSize: std::mem::size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info.cbSize` is initialized properly.
    unsafe { GetCursorInfo(&mut info).ok()? };
    if (info.flags.0 & CURSOR_SHOWING.0) == 0 || info.hCursor.is_invalid() {
        return None;
    }

    let raw = info.hCursor.0 as usize;
    // Fast path: O(1) handle match for all standard IDC_* cursors (no GDI bitmap copies).
    if let Some(&shape) = identifier.handle_map.get(&raw) {
        return Some(shape);
    }

    // Fallback path: hash + 16x16 fingerprint from `GetIconInfoExW`.
    if let Some((hash, fp)) = extract_cursor_fingerprint(info.hCursor) {
        return Some(identifier.identify_full(raw, Some(&hash), Some(&fp)));
    }
    Some(CursorShape::Unknown)
}

fn extract_cursor_fingerprint(hcursor: HCURSOR) -> Option<([u8; 32], CursorFingerprint)> {
    let mut icon = ICONINFOEXW {
        cbSize: std::mem::size_of::<ICONINFOEXW>() as u32,
        ..Default::default()
    };
    // SAFETY: `GetIconInfoExW` populates new GDI bitmaps (`hbmColor`, `hbmMask`) that we delete
    // before returning.
    let ok = unsafe { GetIconInfoExW(hcursor.into(), &mut icon).as_bool() };
    if !ok {
        return None;
    }

    let result = unsafe { read_icon_rgba(&icon) }.and_then(|(w, h, rgba)| {
        let hash = bitmap_hash(w, h, icon.xHotspot, icon.yHotspot, &rgba);
        let fp = CursorFingerprint::from_rgba(
            w,
            h,
            &rgba,
            f64::from(icon.xHotspot),
            f64::from(icon.yHotspot),
        )?;
        Some((hash, fp))
    });

    // SAFETY: release bitmaps allocated by `GetIconInfoExW`.
    unsafe {
        if !icon.hbmColor.is_invalid() {
            let _ = DeleteObject(icon.hbmColor.into());
        }
        if !icon.hbmMask.is_invalid() {
            let _ = DeleteObject(icon.hbmMask.into());
        }
    }

    result
}

unsafe fn read_icon_rgba(icon: &ICONINFOEXW) -> Option<(u32, u32, Vec<u8>)> {
    let has_color = !icon.hbmColor.is_invalid();
    let src_bmp: HBITMAP = if has_color {
        icon.hbmColor
    } else if !icon.hbmMask.is_invalid() {
        icon.hbmMask
    } else {
        return None;
    };

    let mut bm = BITMAP::default();
    let got = GetObjectW(
        src_bmp.into(),
        std::mem::size_of::<BITMAP>() as i32,
        Some((&raw mut bm).cast()),
    );
    if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
        return None;
    }

    let width = bm.bmWidth as u32;
    let height = if has_color {
        bm.bmHeight as u32
    } else {
        // Monochrome cursors pack AND + XOR masks vertically (2x height).
        (bm.bmHeight as u32 / 2).max(1)
    };

    let hdc = CreateCompatibleDC(None);
    if hdc.is_invalid() {
        return None;
    }

    let total_h = if has_color {
        height
    } else {
        bm.bmHeight as u32
    };
    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // Negative height requests top-down DIB scanlines.
            biHeight: -(total_h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bgra = vec![0u8; (width as usize) * (total_h as usize) * 4];
    let lines = GetDIBits(
        hdc,
        src_bmp,
        0,
        total_h,
        Some(bgra.as_mut_ptr().cast()),
        &mut bi,
        DIB_RGB_COLORS,
    );
    let _ = DeleteDC(hdc);
    if lines == 0 {
        return None;
    }

    let pixel_count = (width as usize) * (height as usize);
    let mut rgba = vec![0u8; pixel_count * 4];
    if has_color {
        for i in 0..pixel_count {
            let b = bgra[i * 4];
            let g = bgra[i * 4 + 1];
            let r = bgra[i * 4 + 2];
            let a = bgra[i * 4 + 3];
            rgba[i * 4] = r;
            rgba[i * 4 + 1] = g;
            rgba[i * 4 + 2] = b;
            rgba[i * 4 + 3] = a;
        }
    } else {
        // Monochrome cursor: top half is AND mask (0 = opaque, 255 = transparent), bottom half is XOR mask.
        for i in 0..pixel_count {
            let and_px = bgra[i * 4];
            let xor_off = (pixel_count + i) * 4;
            let xor_px = bgra.get(xor_off).copied().unwrap_or(0);
            let opaque = and_px == 0;
            let white = xor_px != 0;
            if opaque {
                let lum = if white { 255 } else { 0 };
                rgba[i * 4] = lum;
                rgba[i * 4 + 1] = lum;
                rgba[i * 4 + 2] = lum;
                rgba[i * 4 + 3] = 255;
            } else if white {
                // Inverted pixel (e.g. classic I-beam): mark visible mid-grey for fingerprinting.
                rgba[i * 4] = 128;
                rgba[i * 4 + 1] = 128;
                rgba[i * 4 + 2] = 128;
                rgba[i * 4 + 3] = 255;
            }
        }
    }

    Some((width, height, rgba))
}
