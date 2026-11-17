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
    use windows_sys::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
        SHFILEOPSTRUCTW,
    };

    /// A NUL-separated, double-NUL-terminated wide string, as `SHFileOperationW` requires.
    fn wide_list(path: &Path) -> Vec<u16> {
        // A trailing separator makes the shell report the folder itself inconsistently; the
        // operation wants the item path.
        let text = path.to_string_lossy();
        let trimmed = text.trim_end_matches(['\\', '/']).to_owned();
        let item = if trimmed.is_empty() {
            text.as_ref()
        } else {
            trimmed.as_str()
        };
        let mut wide: Vec<u16> = std::ffi::OsStr::new(item).encode_wide().collect();
        wide.push(0);
        wide.push(0);
        wide
    }

    #[allow(unsafe_code)]
    pub fn move_to_recycle_bin(path: &Path) -> Result<(), ProjectError> {
        if !path.exists() {
            return Err(ProjectError::Delete(format!(
                "{} is already gone",
                path.display()
            )));
        }
        let mut from = wide_list(path);
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
                "Couldn't move {} to the Recycle Bin (0x{code:04X})",
                path.display()
            )));
        }
        if operation.fAnyOperationsAborted != 0 {
            return Err(ProjectError::Delete(format!(
                "Moving {} to the Recycle Bin was cancelled",
                path.display()
            )));
        }
        Ok(())
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
