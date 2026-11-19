//! Deleting a bundle the way Explorer does: to the Recycle Bin, not `remove_dir_all`.
//!
//! Upstream injects the trash call (`FileProjectLibrary(trash:)`) so tests never touch the real
//! Trash. The same seam exists here via [`crate::library::Trash`]; this module is the Windows
//! implementation.
//!
//! This is the only `unsafe` code in the project layer: `SHFileOperationW` is the only way to
//! reach the Recycle Bin without pulling in COM. Everything it touches is a plain `#[repr(C)]`
//! struct we zero-initialise and fill, and one wide (UTF-16) string that outlives the call.

use std::path::Path;

use crate::errors::ProjectError;
use crate::library::Trash;

/// Moves bundles to the Recycle Bin with `SHFileOperationW(FO_DELETE)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RecycleBinTrash;

impl Trash for RecycleBinTrash {
    fn move_to_trash(&self, path: &Path) -> Result<(), ProjectError> {
        platform::move_to_recycle_bin(path)
    }
}

#[cfg(windows)]
mod platform {
    use super::{Path, ProjectError};
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{RPC_E_CHANGED_MODE, S_FALSE, S_OK};
    use windows_sys::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows_sys::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
        SHFILEOPSTRUCTW,
    };

    /// Normalizes `path` for `SHFileOperationW`: absolute, no verbatim `\\?\` prefix, backslashes
    /// only, no embedded NULs, and never a bare drive root.
    fn wide_list(path: &Path) -> Result<Vec<u16>, ProjectError> {
        let abs: PathBuf = if path.is_relative() {
            std::env::current_dir()
                .map_err(|e| ProjectError::io(path, e))?
                .join(path)
        } else {
            path.to_path_buf()
        };
        let raw = abs.to_string_lossy().replace('/', "\\");
        if raw.contains('\0') {
            return Err(ProjectError::Delete(
                "Refusing to delete a path containing a NUL character".to_owned(),
            ));
        }
        // `SHFileOperationW` rejects `\\?\` extended-length prefixes.
        let cleaned = if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}")
        } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
            rest.to_owned()
        } else {
            raw
        };
        // A trailing separator makes the shell report the folder itself inconsistently; the
        // operation wants the item path.
        let trimmed = cleaned.trim_end_matches('\\');
        // Never allow deleting an empty string, a bare drive root (`C:`), or a UNC root.
        let bytes = trimmed.as_bytes();
        if trimmed.is_empty()
            || (bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
            || trimmed.starts_with(r"\\") && trimmed[2..].matches('\\').count() < 2
        {
            return Err(ProjectError::Delete(format!(
                "Refusing to move root path {} to the Recycle Bin",
                path.display()
            )));
        }
        let mut wide: Vec<u16> = std::ffi::OsStr::new(trimmed).encode_wide().collect();
        wide.push(0);
        wide.push(0);
        Ok(wide)
    }

    #[allow(unsafe_code)]
    fn run_sh_file_op(mut from: Vec<u16>, display: &str) -> Result<(), ProjectError> {
        let mut operation = SHFILEOPSTRUCTW {
            hwnd: std::ptr::null_mut(),
            wFunc: FO_DELETE,
            // SAFETY: `from` is a live, double-NUL-terminated wide buffer that outlives the call.
            pFrom: from.as_mut_ptr(),
            pTo: std::ptr::null(),
            fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16,
            fAnyOperationsAborted: 0,
            hNameMappings: std::ptr::null_mut(),
            lpszProgressTitle: std::ptr::null(),
        };
        // SAFETY: `operation` is a fully initialised SHFILEOPSTRUCTW; `pFrom` points at `from`,
        // which is not moved or dropped until the call returns.
        let code: i32 = unsafe { SHFileOperationW(&mut operation) };
        if code != 0 {
            return Err(ProjectError::Delete(format!(
                "Couldn't move {display} to the Recycle Bin (0x{code:04X})"
            )));
        }
        if operation.fAnyOperationsAborted != 0 {
            return Err(ProjectError::Delete(format!(
                "Moving {display} to the Recycle Bin was cancelled"
            )));
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    pub fn move_to_recycle_bin(path: &Path) -> Result<(), ProjectError> {
        if !path.exists() {
            return Err(ProjectError::Delete(format!(
                "{} is already gone",
                path.display()
            )));
        }
        let from = wide_list(path)?;
        let display = path.display().to_string();

        // `SHFileOperationW` requires STA COM initialisation. If this thread is already in MTA
        // (e.g. a background worker pool), hop to a short-lived STA helper thread.
        // SAFETY: `CoInitializeEx` with null reserved pointer and standard STA flags is valid.
        let hr = unsafe {
            CoInitializeEx(
                std::ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        if hr == RPC_E_CHANGED_MODE {
            return std::thread::spawn(move || {
                // SAFETY: fresh thread, pair CoInitializeEx with CoUninitialize on S_OK/S_FALSE.
                let inner_hr = unsafe {
                    CoInitializeEx(
                        std::ptr::null(),
                        (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
                    )
                };
                let res = run_sh_file_op(from, &display);
                if inner_hr == S_OK || inner_hr == S_FALSE {
                    // SAFETY: balances the matching CoInitializeEx on this thread.
                    unsafe { CoUninitialize() };
                }
                res
            })
            .join()
            .unwrap_or_else(|_| {
                Err(ProjectError::Delete(
                    "Recycle Bin helper panicked".to_owned(),
                ))
            });
        }
        let res = run_sh_file_op(from, &display);
        if hr == S_OK || hr == S_FALSE {
            // SAFETY: balances the matching CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
        res
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Path, ProjectError};

    pub fn move_to_recycle_bin(path: &Path) -> Result<(), ProjectError> {
        Err(ProjectError::Delete(format!(
            "No Recycle Bin on this platform; refusing to delete {}",
            path.display()
        )))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;

    /// A real delete to the real Recycle Bin. Kept in the normal suite because it is the only
    /// test that proves the FFI call is wired correctly; it touches nothing but its own temp dir.
    #[test]
    fn a_bundle_goes_to_the_recycle_bin() {
        let dir = std::env::temp_dir().join(format!("nss-trash-{}", uuid::Uuid::new_v4()));
        let bundle = dir.join("Demo.nssproj");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join("project.json"), b"{}").unwrap();
        RecycleBinTrash.move_to_trash(&bundle).expect("recycle");
        assert!(!bundle.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_bundle_is_an_error_not_a_panic() {
        let dir = std::env::temp_dir().join(format!("nss-trash-missing-{}", uuid::Uuid::new_v4()));
        let err = RecycleBinTrash
            .move_to_trash(&dir.join("nope.nssproj"))
            .unwrap_err();
        assert!(matches!(err, ProjectError::Delete(_)));
    }

    #[test]
    fn a_trailing_separator_is_tolerated() {
        let dir = std::env::temp_dir().join(format!("nss-trash-slash-{}", uuid::Uuid::new_v4()));
        let bundle = dir.join("Slash.nssproj");
        fs::create_dir_all(&bundle).unwrap();
        let with_separator = std::path::PathBuf::from(format!("{}\\", bundle.display()));
        RecycleBinTrash
            .move_to_trash(&with_separator)
            .expect("recycle with a trailing separator");
        assert!(!bundle.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
