//! Crash-safe file replacement.
//!
//! Port of upstream `NSSProject/AtomicFile.swift`, adapted to Windows.
//!
//! The contract is unchanged: the data goes to a temp file in the same directory, is flushed to
//! disk, then swapped over the destination. A crash leaves either the old file or the new one,
//! never half of one.
//!
//! What changed on Windows (see `docs/ADR/0010-windows-atomic-write.md`):
//!
//! * `std::fs::rename` already maps to `MoveFileExW(.., MOVEFILE_REPLACE_EXISTING)`, so it
//!   replaces an existing file; no separate code path is needed for "destination exists".
//! * The swap can fail with a *sharing violation* (error 32/33, or `PermissionDenied`) when
//!   Explorer, an antivirus scanner or our own thumbnail generator has the destination open.
//!   That is transient, so the swap is retried with a short backoff before giving up.
//! * A directory cannot be fsynced without an unsafe handle, so durability stops at the file:
//!   `sync_all()` on the temp file, then the rename. The `.bak` copy plus the fallback in
//!   `FileProjectStore::load` covers the window a POSIX build would close with a directory fsync.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::errors::ProjectError;

/// Marker inside every temp file name, so a crash's leftovers can be recognised and removed.
pub const TEMP_MARKER: &str = ".nss-tmp-";

/// How long the swap keeps retrying a transient sharing violation.
const REPLACE_RETRY_BUDGET: Duration = Duration::from_millis(1_500);

/// Writes `data` to `to`, replacing whatever is there, atomically.
///
/// `before_replace` runs after the temp file is fully written and synced and before the swap.
/// It exists for tests (upstream has the same hook) and for callers that want to validate the
/// bytes on disk before they become live.
pub fn write(
    data: &[u8],
    to: &Path,
    before_replace: Option<&dyn Fn() -> Result<(), ProjectError>>,
) -> Result<(), ProjectError> {
    let dir = to.parent().unwrap_or_else(|| Path::new("."));
    let leaf = to
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let temp = dir.join(format!(".{leaf}{TEMP_MARKER}{}", uuid::Uuid::new_v4()));

    let result = (|| -> Result<(), ProjectError> {
        // `create_new` so a temp name collision can never truncate someone else's file.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| ProjectError::io(&temp, e))?;
        file.write_all(data)
            .map_err(|e| ProjectError::io(&temp, e))?;
        file.sync_all().map_err(|e| ProjectError::io(&temp, e))?;
        drop(file);

        if let Some(hook) = before_replace {
            hook()?;
        }
        replace(&temp, to)
    })();

    if result.is_err() {
        // Never leave a temp file behind on a failed write.
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Writes `data`, treating any error from the hook as a reason to keep the old file.
pub fn write_checked(
    data: &[u8],
    to: &Path,
    before_replace: impl Fn() -> Result<(), ProjectError>,
) -> Result<(), ProjectError> {
    write(data, to, Some(&before_replace))
}

/// Convenience wrapper with no hook.
pub fn write_atomic(data: &[u8], to: &Path) -> Result<(), ProjectError> {
    write(data, to, None)
}

/// Removes temp files a crash left behind in `directory`. Returns how many were removed.
pub fn remove_stale_temps(directory: &Path) -> usize {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.')
            && name.contains(TEMP_MARKER)
            && fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// The temp file names that [`write`] would consider its own.
#[must_use]
pub fn is_temp_name(name: &str) -> bool {
    name.starts_with('.') && name.contains(TEMP_MARKER)
}

/// `rename(2)`, retried while Windows reports the destination as busy.
fn replace(from: &Path, to: &Path) -> Result<(), ProjectError> {
    let started = std::time::Instant::now();
    let mut backoff = Duration::from_millis(2);
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let transient = e.kind() == std::io::ErrorKind::PermissionDenied
                    || e.raw_os_error() == Some(32) // ERROR_SHARING_VIOLATION
                    || e.raw_os_error() == Some(33); // ERROR_LOCK_VIOLATION
                if !transient || started.elapsed() + backoff > REPLACE_RETRY_BUDGET {
                    return Err(ProjectError::io(to, e));
                }
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_millis(100));
            }
        }
    }
}

/// A path plus its temp-file name, for callers that want to pre-create the directory.
#[must_use]
pub fn temp_path_for(to: &Path) -> PathBuf {
    let leaf = to
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    to.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{leaf}{TEMP_MARKER}{}", uuid::Uuid::new_v4()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nss-atomic-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn write_replaces_and_leaves_no_temps() {
        let dir = scratch("replace");
        let file = dir.join("a.json");
        write_atomic(b"one", &file).unwrap();
        write_atomic(b"two", &file).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "two");
        assert_eq!(names(&dir), vec!["a.json".to_owned()]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_hook_keeps_the_old_file_and_cleans_up() {
        let dir = scratch("hook");
        let file = dir.join("a.json");
        write_atomic(b"good", &file).unwrap();
        let err = write_checked(b"bad", &file, || Err(ProjectError::Corrupt("boom".into())));
        assert!(err.is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "good");
        assert_eq!(names(&dir), vec!["a.json".to_owned()]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_directory_fails_without_leaving_anything() {
        let dir = scratch("missing");
        let file = dir.join("nope").join("a.json");
        let err = write_atomic(b"x", &file).expect_err("no such directory");
        assert!(matches!(err, ProjectError::Io { .. }));
        assert!(names(&dir).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stale_temps_from_a_crash_are_removed() {
        let dir = scratch("stale");
        fs::write(dir.join(format!(".project.json{TEMP_MARKER}ABC")), b"junk").unwrap();
        fs::write(dir.join("project.json"), b"keep").unwrap();
        fs::write(dir.join("cursor.json"), b"keep").unwrap();
        assert_eq!(remove_stale_temps(&dir), 1);
        assert_eq!(
            names(&dir),
            vec!["cursor.json".to_owned(), "project.json".to_owned()]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn temp_names_are_recognisable() {
        assert!(is_temp_name(".project.json.nss-tmp-1234"));
        assert!(!is_temp_name("project.json"));
        assert!(!is_temp_name("nss-tmp-1234"));
        let t = temp_path_for(Path::new("C:/x/project.json"));
        assert!(is_temp_name(&t.file_name().unwrap().to_string_lossy()));
        assert_eq!(t.parent().unwrap(), Path::new("C:/x"));
    }

    #[test]
    fn a_large_payload_survives_the_round_trip() {
        // cursor.json for a one-hour recording is tens of megabytes; make sure the write is not
        // accidentally chunked incorrectly.
        let dir = scratch("large");
        let file = dir.join("cursor.json");
        let data: Vec<u8> = (0..2_000_000u32).map(|i| (i % 251) as u8).collect();
        write_atomic(&data, &file).unwrap();
        assert_eq!(fs::read(&file).unwrap(), data);
        fs::remove_dir_all(&dir).unwrap();
    }
}
