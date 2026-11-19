//! `WH_MOUSE_LL` low-level global mouse hook on a dedicated Win32 message-pump thread.
//!
//! Confined `unsafe` boundary (ADR 0012). The hook procedure never blocks or allocates heavily:
//! it reads `MSLLHOOKSTRUCT`, stamps `nss_core::host_now()`, forwards to the installed callback,
//! and immediately calls `CallNextHookEx`.

#![allow(unsafe_code)]

use std::sync::{mpsc, OnceLock};
use std::thread::{self, JoinHandle};

use nss_core::{host_now, NssError, Point2};
use parking_lot::Mutex;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK, MSG, MSLLHOOKSTRUCT,
    PM_NOREMOVE, WH_MOUSE_LL, WM_QUIT,
};

use crate::hook::{map_win32_mouse_message, ClickCallback, RawMouseClick};

static ACTIVE_CALLBACK: OnceLock<Mutex<Option<ClickCallback>>> = OnceLock::new();

fn callback_slot() -> &'static Mutex<Option<ClickCallback>> {
    ACTIVE_CALLBACK.get_or_init(|| Mutex::new(None))
}

unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && lparam.0 != 0 {
        if let Some((button, kind)) = map_win32_mouse_message(wparam.0 as u32) {
            let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            let click = RawMouseClick {
                point: Point2::new(f64::from(info.pt.x), f64::from(info.pt.y)),
                host_time: host_now(),
                button,
                kind,
            };
            if let Some(cb) = callback_slot().try_lock().and_then(|g| g.clone()) {
                cb(click);
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

pub struct WinMouseHook {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl WinMouseHook {
    pub fn install(callback: ClickCallback) -> Result<Self, NssError> {
        *callback_slot().lock() = Some(callback);
        let (tx, rx) = mpsc::sync_channel::<Result<u32, NssError>>(1);

        let join = thread::Builder::new()
            .name("nss-mouse-hook".into())
            .spawn(move || {
                // SAFETY: `SetWindowsHookExW(WH_MOUSE_LL)` runs on this dedicated thread with a
                // standard `GetMessageW` pump and unhooks cleanly before exiting.
                unsafe {
                    let tid = GetCurrentThreadId();
                    let module = GetModuleHandleW(None).ok().map(Into::into);
                    let hook: HHOOK =
                        match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), module, 0) {
                            Ok(h) => h,
                            Err(e) => {
                                let _ = tx.send(Err(NssError::Io(format!(
                                    "Failed to install low-level mouse hook: {e}"
                                ))));
                                return;
                            }
                        };
                    // Force Win32 to create this thread's message queue before publishing `tid`,
                    // otherwise an immediate `PostThreadMessageW(WM_QUIT)` could be lost.
                    let mut msg = MSG::default();
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                    let _ = tx.send(Ok(tid));

                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = UnhookWindowsHookEx(hook);
                }
            })
            .map_err(|e| NssError::Io(format!("Failed to spawn mouse hook thread: {e}")))?;

        match rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                join: Some(join),
            }),
            Ok(Err(e)) => {
                *callback_slot().lock() = None;
                let _ = join.join();
                Err(e)
            }
            Err(_) => {
                *callback_slot().lock() = None;
                let _ = join.join();
                Err(NssError::Io(
                    "Mouse hook thread terminated before reporting status.".into(),
                ))
            }
        }
    }

    pub fn uninstall(&mut self) {
        *callback_slot().lock() = None;
        if let Some(join) = self.join.take() {
            // SAFETY: `thread_id` belongs to our message-pump thread; `WM_QUIT` breaks `GetMessageW`.
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            let _ = join.join();
        }
    }
}

impl Drop for WinMouseHook {
    fn drop(&mut self) {
        self.uninstall();
    }
}
