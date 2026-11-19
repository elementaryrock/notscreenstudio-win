//! The Windows facts the rest of the crate reasons about, without any of the API calls.
//!
//! `EnumDisplayMonitors` and `EnumWindows` return raw Win32 structs; this module is the shape those
//! structs are copied into, plus the arithmetic that turns them into the picker's data and into a
//! [`ContentSnapshot`]. Keeping it here means the picker, the filter and the recorders all agree on
//! what a monitor is, and it can be tested without a desktop session.

use nss_core::{PixelSize, Point2, Rect, Size2};

use crate::capture_filter::{App, ContentSnapshot, Window};
use crate::capture_math::{is_pickable_window, pixel_size};

#[cfg(windows)]
pub mod devices;
#[cfg(windows)]
pub mod enumerate;
#[cfg(windows)]
pub mod exclusion;
#[cfg(windows)]
pub mod permissions;
#[cfg(windows)]
pub mod wgc;

/// One monitor, in virtual-desktop pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// `HMONITOR` as a plain integer.
    pub id: u32,
    /// Bounds in the virtual desktop, which is why the origin can be negative on a multi-monitor setup.
    pub rect: Rect,
    /// The part of `rect` not covered by the taskbar.
    pub work_rect: Rect,
    /// Dots per inch, 96 = 100 %.
    pub dpi: u32,
    pub is_primary: bool,
    /// The device name, e.g. `\\.\DISPLAY1`.
    pub device_name: String,
}

impl Monitor {
    /// Pixels per point at this monitor's DPI.
    #[must_use]
    pub fn scale(&self) -> f64 {
        let dpi = if self.dpi == 0 { 96 } else { self.dpi };
        f64::from(dpi) / 96.0
    }

    /// Size in physical pixels.
    #[must_use]
    pub fn pixel_size(&self) -> PixelSize {
        pixel_size(
            Size2::new(self.rect.width(), self.rect.height()),
            self.scale(),
        )
    }

    /// The label the picker shows, matching upstream's "which display is this?" wording.
    #[must_use]
    pub fn display_name(&self, index: usize) -> String {
        let p = self.pixel_size();
        let scale = self.scale();
        let scale = if (scale - 1.0).abs() < 0.01 {
            String::new()
        } else {
            format!(", {scale:.1}x")
        };
        let primary = if self.is_primary { ", primary" } else { "" };
        format!(
            "Display {} ({}x{}{scale}{primary})",
            index + 1,
            p.width,
            p.height
        )
    }
}

/// One top-level window, in virtual-desktop pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowInfo {
    /// `HWND` as a plain integer.
    pub handle: u64,
    pub process_id: u32,
    /// Executable file name, the Windows equivalent of upstream's bundle ID.
    pub app_name: Option<String>,
    pub title: String,
    /// DWM extended frame bounds: the visible frame, without the invisible resize border.
    pub is_on_screen: bool,
    /// True when DWM reports the window cloaked (another virtual desktop, a suspended UWP app).
    pub is_cloaked: bool,
    /// `WS_EX_TOOLWINDOW`: tool palettes and other never-in-the-picker windows.
    pub is_tool: bool,
    pub width: f64,
    pub height: f64,
}

impl WindowInfo {
    /// Whether the picker offers this window: visible, not cloaked, not a tool window, owned by a
    /// process we could resolve, and big enough to be a real window rather than a 1x1 helper.
    #[must_use]
    pub fn is_pickable(&self) -> bool {
        is_pickable_window(
            self.is_on_screen,
            self.is_cloaked,
            self.is_tool,
            self.process_id != 0,
            self.width,
            self.height,
        )
    }

    /// The label the picker shows: the window title, or the app name for an untitled window.
    #[must_use]
    pub fn picker_name(&self) -> String {
        let title = self.title.trim();
        if !title.is_empty() {
            return title.to_owned();
        }
        self.app_name
            .clone()
            .unwrap_or_else(|| "Untitled window".into())
    }
}

/// Builds the hardware-free snapshot the filter decides on.
#[must_use]
pub fn snapshot(monitors: &[Monitor], windows: &[WindowInfo]) -> ContentSnapshot {
    let mut apps: Vec<App> = Vec::new();
    for w in windows {
        if apps.iter().any(|a| a.process_id == w.process_id) {
            continue;
        }
        apps.push(App {
            process_id: w.process_id,
            app_name: w.app_name.clone(),
        });
    }
    ContentSnapshot {
        display_ids: monitors.iter().map(|m| m.id).collect(),
        apps,
        windows: windows
            .iter()
            .filter(|w| w.is_pickable())
            .map(|w| Window {
                handle: w.handle,
                owner_process_id: (w.process_id != 0).then_some(w.process_id),
                owner_app: w.app_name.clone(),
                title: w.title.clone(),
                is_on_screen: w.is_on_screen,
                is_cloaked: w.is_cloaked,
                is_tool: w.is_tool,
            })
            .collect(),
    }
}

/// The display a virtual-desktop point falls on, which is how an area capture finds its monitor.
#[must_use]
pub fn monitor_at(monitors: &[Monitor], point: Point2) -> Option<&Monitor> {
    monitors.iter().find(|m| m.rect.contains(point.x, point.y))
}

/// An area's bounds relative to the display it was drawn on.
///
/// Upstream stored the area in display points with the origin at the display's top-left; the drag
/// selector works in virtual-desktop pixels, so this converts.
#[must_use]
pub fn area_in_display(monitor: &Monitor, area: Rect) -> Rect {
    let scale = monitor.scale();
    Rect::new(
        (area.min_x() - monitor.rect.min_x()) / scale,
        (area.min_y() - monitor.rect.min_y()) / scale,
        area.width() / scale,
        area.height() / scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(id: u32, x: f64, y: f64, w: f64, h: f64, dpi: u32) -> Monitor {
        Monitor {
            id,
            rect: Rect::new(x, y, w, h),
            work_rect: Rect::new(x, y, w, h - 40.0),
            dpi,
            is_primary: id == 1,
            device_name: format!("\\\\.\\DISPLAY{id}"),
        }
    }

    fn window(handle: u64, pid: u32, title: &str, w: f64, h: f64) -> WindowInfo {
        WindowInfo {
            handle,
            process_id: pid,
            app_name: Some("Code.exe".into()),
            title: title.into(),
            is_on_screen: true,
            is_cloaked: false,
            is_tool: false,
            width: w,
            height: h,
        }
    }

    #[test]
    fn scale_follows_the_dpi_and_a_zero_dpi_is_100_percent() {
        assert_eq!(monitor(1, 0.0, 0.0, 1920.0, 1080.0, 96).scale(), 1.0);
        assert_eq!(monitor(1, 0.0, 0.0, 1920.0, 1080.0, 144).scale(), 1.5);
        assert_eq!(monitor(1, 0.0, 0.0, 1920.0, 1080.0, 0).scale(), 1.0);
    }

    #[test]
    fn pixel_size_is_the_logical_size_times_the_scale() {
        let m = monitor(1, 0.0, 0.0, 1280.0, 720.0, 144);
        assert_eq!((m.pixel_size().width, m.pixel_size().height), (1920, 1080));
    }

    #[test]
    fn display_names_read_like_the_picker() {
        assert_eq!(
            monitor(1, 0.0, 0.0, 1920.0, 1080.0, 96).display_name(0),
            "Display 1 (1920x1080, primary)"
        );
        assert_eq!(
            monitor(2, 1920.0, 0.0, 1280.0, 720.0, 144).display_name(1),
            "Display 2 (1920x1080, 1.5x)"
        );
    }

    #[test]
    fn only_pickable_windows_reach_the_snapshot() {
        let mut cloaked = window(2, 20, "Hidden", 800.0, 600.0);
        cloaked.is_cloaked = true;
        let mut tool = window(3, 30, "Palette", 800.0, 600.0);
        tool.is_tool = true;
        let tiny = window(4, 40, "Tiny", 10.0, 10.0);
        let windows = vec![window(1, 10, "Editor", 800.0, 600.0), cloaked, tool, tiny];
        let snap = snapshot(&[monitor(1, 0.0, 0.0, 1920.0, 1080.0, 96)], &windows);
        assert_eq!(snap.display_ids, [1]);
        assert_eq!(
            snap.windows.iter().map(|w| w.handle).collect::<Vec<_>>(),
            [1]
        );
        // The app list keeps every window's process, pickable or not: a cloaked window still belongs
        // to a process, and that process must be excluded.
        assert_eq!(snap.apps.len(), 4);
    }

    #[test]
    fn a_window_name_falls_back_to_the_app_then_to_a_default() {
        assert_eq!(
            window(1, 10, "  Editor ", 800.0, 600.0).picker_name(),
            "Editor"
        );
        let mut untitled = window(1, 10, "", 800.0, 600.0);
        assert_eq!(untitled.picker_name(), "Code.exe");
        untitled.app_name = None;
        assert_eq!(untitled.picker_name(), "Untitled window");
    }

    #[test]
    fn an_area_finds_the_monitor_it_was_drawn_on() {
        let monitors = vec![
            monitor(1, 0.0, 0.0, 1920.0, 1080.0, 96),
            monitor(2, 1920.0, 0.0, 1280.0, 720.0, 96),
        ];
        assert_eq!(
            monitor_at(&monitors, Point2::new(2500.0, 100.0)).map(|m| m.id),
            Some(2)
        );
        assert_eq!(
            monitor_at(&monitors, Point2::new(500.0, 100.0)).map(|m| m.id),
            Some(1)
        );
        assert!(monitor_at(&monitors, Point2::new(-5.0, 100.0)).is_none());
    }

    #[test]
    fn an_area_on_a_scaled_second_monitor_is_relative_to_that_display() {
        let m = monitor(2, 1920.0, 0.0, 1280.0, 720.0, 144);
        let area = area_in_display(&m, Rect::new(1920.0, 100.0, 300.0, 150.0));
        assert_eq!((area.min_x(), area.min_y()), (0.0, 100.0 / 1.5));
        assert_eq!((area.width(), area.height()), (200.0, 100.0));
    }
}
