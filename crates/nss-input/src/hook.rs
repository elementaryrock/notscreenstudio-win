//! Global low-level mouse click hook (`WH_MOUSE_LL`).
//!
//! Translates Win32 mouse button messages into `(MouseButton, ClickKind)` and runs the hook on a
//! dedicated message-loop thread so it never blocks the UI or the 120 Hz position sampler.

use std::sync::Arc;

use nss_core::{ClickKind, MouseButton, NssError, Point2};

/// Raw click event delivered from the OS mouse hook on the shared monotonic host clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawMouseClick {
    pub point: Point2,
    pub host_time: f64,
    pub button: MouseButton,
    pub kind: ClickKind,
}

/// Callback invoked for every global mouse down/up event. Must return quickly (no blocking I/O).
pub type ClickCallback = Arc<dyn Fn(RawMouseClick) + Send + Sync>;

/// Maps a Win32 `WM_*BUTTON*` message ID to `(MouseButton, ClickKind)`.
#[must_use]
pub const fn map_win32_mouse_message(msg: u32) -> Option<(MouseButton, ClickKind)> {
    const WM_LBUTTONDOWN: u32 = 0x0201;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_RBUTTONDOWN: u32 = 0x0204;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_MBUTTONDOWN: u32 = 0x0207;
    const WM_MBUTTONUP: u32 = 0x0208;
    const WM_XBUTTONDOWN: u32 = 0x020B;
    const WM_XBUTTONUP: u32 = 0x020C;

    match msg {
        WM_LBUTTONDOWN => Some((MouseButton::Left, ClickKind::Down)),
        WM_LBUTTONUP => Some((MouseButton::Left, ClickKind::Up)),
        WM_RBUTTONDOWN => Some((MouseButton::Right, ClickKind::Down)),
        WM_RBUTTONUP => Some((MouseButton::Right, ClickKind::Up)),
        WM_MBUTTONDOWN | WM_XBUTTONDOWN => Some((MouseButton::Other, ClickKind::Down)),
        WM_MBUTTONUP | WM_XBUTTONUP => Some((MouseButton::Other, ClickKind::Up)),
        _ => None,
    }
}

/// RAII handle for an active global mouse hook. Dropping removes the hook and joins its thread.
pub struct MouseHookHandle {
    #[cfg(windows)]
    inner: Option<crate::win::hook::WinMouseHook>,
}

impl MouseHookHandle {
    pub fn install(callback: ClickCallback) -> Result<Self, NssError> {
        #[cfg(windows)]
        {
            let inner = crate::win::hook::WinMouseHook::install(callback)?;
            Ok(Self { inner: Some(inner) })
        }
        #[cfg(not(windows))]
        {
            let _ = callback;
            Ok(Self {})
        }
    }

    pub fn uninstall(&mut self) {
        #[cfg(windows)]
        if let Some(mut h) = self.inner.take() {
            h.uninstall();
        }
    }
}

impl Drop for MouseHookHandle {
    fn drop(&mut self) {
        self.uninstall();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_left_right_middle_and_x_button_messages() {
        assert_eq!(
            map_win32_mouse_message(0x0201),
            Some((MouseButton::Left, ClickKind::Down))
        );
        assert_eq!(
            map_win32_mouse_message(0x0202),
            Some((MouseButton::Left, ClickKind::Up))
        );
        assert_eq!(
            map_win32_mouse_message(0x0204),
            Some((MouseButton::Right, ClickKind::Down))
        );
        assert_eq!(
            map_win32_mouse_message(0x0205),
            Some((MouseButton::Right, ClickKind::Up))
        );
        assert_eq!(
            map_win32_mouse_message(0x0207),
            Some((MouseButton::Other, ClickKind::Down))
        );
        assert_eq!(
            map_win32_mouse_message(0x020C),
            Some((MouseButton::Other, ClickKind::Up))
        );
        assert_eq!(map_win32_mouse_message(0x0200), None); // WM_MOUSEMOVE ignored
    }
}
