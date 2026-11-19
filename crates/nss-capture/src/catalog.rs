//! Source picker catalog: enumerates displays and pickable windows.
//!
//! Port of upstream `NSSCapture/CaptureSourceCatalog.swift`.

use nss_core::{CaptureSource, PixelSize, Rect, Size2};
use serde::{Deserialize, Serialize};

use crate::capture_math::is_own_app;
use crate::win::{Monitor, WindowInfo};

/// One display available to record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDisplayInfo {
    pub id: u32,
    pub name: String,
    /// Virtual-desktop bounds in pixels.
    pub frame: Rect,
    pub pixel_size: PixelSize,
    pub scale: f64,
    pub is_main: bool,
}

impl CaptureDisplayInfo {
    #[must_use]
    pub const fn source(&self) -> CaptureSource {
        CaptureSource::Display { id: self.id }
    }
}

/// One top-level window available to record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureWindowInfo {
    /// Lower 32 bits of the HWND (matches `CaptureSource::Window { id }`).
    pub id: u32,
    /// Full 64-bit Win32 `HWND`.
    pub hwnd: u64,
    pub title: String,
    pub app_name: String,
    pub process_id: u32,
    pub frame: Rect,
    /// The display holding most of this window, if known.
    pub display_id: Option<u32>,
}

impl CaptureWindowInfo {
    #[must_use]
    pub const fn source(&self) -> CaptureSource {
        CaptureSource::Window { id: self.id }
    }
}

/// Complete picker snapshot: primary display first, windows sorted by `(app_name, title)`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSourceList {
    pub displays: Vec<CaptureDisplayInfo>,
    pub windows: Vec<CaptureWindowInfo>,
}

/// Lists displays and pickable windows for the setup bar / source picker.
pub struct CaptureSourceCatalog;

impl CaptureSourceCatalog {
    /// Enumerates live displays and windows on Windows, excluding our own process.
    #[must_use]
    pub fn load() -> CaptureSourceList {
        #[cfg(windows)]
        {
            let monitors: Vec<Monitor> = crate::win::enumerate::enumerate_monitors()
                .into_iter()
                .map(|m| m.monitor)
                .collect();
            let windows = crate::win::enumerate::enumerate_windows();
            let bounds: Vec<Rect> = windows
                .iter()
                .map(|w| {
                    crate::win::enumerate::window_bounds(w.handle)
                        .unwrap_or_else(|| Rect::new(0.0, 0.0, w.width, w.height))
                })
                .collect();
            let own_pid = std::process::id();
            let own_app = crate::win::enumerate::current_app_name();
            Self::from_snapshot(&monitors, &windows, &bounds, own_pid, own_app.as_deref())
        }
        #[cfg(not(windows))]
        {
            CaptureSourceList::default()
        }
    }

    /// Pure, hardware-free catalog builder used by both [`Self::load`] and unit tests.
    #[must_use]
    pub fn from_snapshot(
        monitors: &[Monitor],
        windows: &[WindowInfo],
        window_frames: &[Rect],
        own_pid: u32,
        own_app: Option<&str>,
    ) -> CaptureSourceList {
        let mut sorted_monitors: Vec<&Monitor> = monitors.iter().collect();
        sorted_monitors.sort_by_key(|m| (!m.is_primary, m.id));

        let displays: Vec<CaptureDisplayInfo> = sorted_monitors
            .iter()
            .enumerate()
            .map(|(i, m)| CaptureDisplayInfo {
                id: m.id,
                name: m.display_name(i),
                frame: m.rect,
                pixel_size: m.pixel_size(),
                scale: m.scale(),
                is_main: m.is_primary,
            })
            .collect();

        let mut pickable: Vec<CaptureWindowInfo> = Vec::new();
        for (idx, w) in windows.iter().enumerate() {
            if !w.is_pickable() {
                continue;
            }
            if is_own_app(w.process_id, w.app_name.as_deref(), own_pid, own_app) {
                continue;
            }
            let frame = window_frames
                .get(idx)
                .copied()
                .unwrap_or_else(|| Rect::new(0.0, 0.0, w.width, w.height));
            let display_id = dominant_display(&displays, frame);
            let app_name = w
                .app_name
                .clone()
                .unwrap_or_else(|| "Application".to_owned());
            pickable.push(CaptureWindowInfo {
                id: w.handle as u32,
                hwnd: w.handle,
                title: w.picker_name(),
                app_name,
                process_id: w.process_id,
                frame,
                display_id,
            });
        }
        pickable.sort_by(|a, b| {
            (a.app_name.to_lowercase(), a.title.to_lowercase())
                .cmp(&(b.app_name.to_lowercase(), b.title.to_lowercase()))
        });

        CaptureSourceList {
            displays,
            windows: pickable,
        }
    }

    /// Fits `aspect` inside `max_size` while keeping the aspect ratio (matches upstream
    /// `CaptureSourceCatalog.fitted`).
    #[must_use]
    pub fn fitted(aspect: Size2, max_size: Size2) -> Size2 {
        if aspect.width <= 0.0 || aspect.height <= 0.0 {
            return max_size;
        }
        let s = (max_size.width / aspect.width).min(max_size.height / aspect.height);
        Size2::new(
            (aspect.width * s).round().max(1.0),
            (aspect.height * s).round().max(1.0),
        )
    }
}

fn dominant_display(displays: &[CaptureDisplayInfo], frame: Rect) -> Option<u32> {
    displays
        .iter()
        .map(|d| (d.id, intersection_area(d.frame, frame)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .and_then(|(id, area)| (area > 0.0).then_some(id))
}

fn intersection_area(a: Rect, b: Rect) -> f64 {
    let x0 = a.min_x().max(b.min_x());
    let y0 = a.min_y().max(b.min_y());
    let x1 = a.max_x().min(b.max_x());
    let y1 = a.max_y().min(b.max_y());
    if x1 <= x0 || y1 <= y0 {
        0.0
    } else {
        (x1 - x0) * (y1 - y0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_fit_keeps_aspect() {
        let wide =
            CaptureSourceCatalog::fitted(Size2::new(1920.0, 1080.0), Size2::new(320.0, 200.0));
        assert_eq!(wide, Size2::new(320.0, 180.0));

        let tall =
            CaptureSourceCatalog::fitted(Size2::new(500.0, 1000.0), Size2::new(320.0, 200.0));
        assert_eq!(tall, Size2::new(100.0, 200.0));
    }

    #[test]
    fn catalog_orders_primary_display_first_and_excludes_own_app() {
        let monitors = vec![
            Monitor {
                id: 2,
                rect: Rect::new(1920.0, 0.0, 1280.0, 720.0),
                work_rect: Rect::new(1920.0, 0.0, 1280.0, 680.0),
                dpi: 96,
                is_primary: false,
                device_name: r"\\.\DISPLAY2".into(),
            },
            Monitor {
                id: 1,
                rect: Rect::new(0.0, 0.0, 1920.0, 1080.0),
                work_rect: Rect::new(0.0, 0.0, 1920.0, 1040.0),
                dpi: 96,
                is_primary: true,
                device_name: r"\\.\DISPLAY1".into(),
            },
        ];
        let windows = vec![
            WindowInfo {
                handle: 10,
                process_id: 100,
                app_name: Some("notscreenstudio.exe".into()),
                title: "HUD".into(),
                is_on_screen: true,
                is_cloaked: false,
                is_tool: false,
                width: 400.0,
                height: 100.0,
            },
            WindowInfo {
                handle: 20,
                process_id: 200,
                app_name: Some("Code.exe".into()),
                title: "main.rs".into(),
                is_on_screen: true,
                is_cloaked: false,
                is_tool: false,
                width: 1200.0,
                height: 800.0,
            },
            WindowInfo {
                handle: 30,
                process_id: 300,
                app_name: Some("Arc.exe".into()),
                title: "Docs".into(),
                is_on_screen: true,
                is_cloaked: false,
                is_tool: false,
                width: 900.0,
                height: 700.0,
            },
        ];
        let frames = vec![
            Rect::new(0.0, 0.0, 400.0, 100.0),
            Rect::new(2000.0, 10.0, 1200.0, 800.0),
            Rect::new(100.0, 100.0, 900.0, 700.0),
        ];

        let list = CaptureSourceCatalog::from_snapshot(
            &monitors,
            &windows,
            &frames,
            100,
            Some("notscreenstudio.exe"),
        );
        assert_eq!(
            list.displays.iter().map(|d| d.id).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(
            list.windows
                .iter()
                .map(|w| w.app_name.as_str())
                .collect::<Vec<_>>(),
            ["Arc.exe", "Code.exe"]
        );
        assert_eq!(list.windows[0].display_id, Some(1));
        assert_eq!(list.windows[1].display_id, Some(2));
    }
}
