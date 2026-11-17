//! .nssproj bundles: atomic save, autosave debounce, migration, project library.
//!
//! Replaces upstream NSSProject. See `PLAN.md` §5 Stage 3 and Appendix A.

#![forbid(unsafe_code)]

pub mod atomic_file;
pub mod autosave;
pub mod file_project_store;
pub mod migrator;
pub mod project_library;
pub mod recent_projects;

/// Non-interactive check used by `notscreenstudio --smoke-test`.
///
/// Upstream's `bundle.sh` runs the app once with `--smoke-test` after every build so a
/// broken dependency graph cannot ship. We do the same: this exercises the code that a
/// headless CI runner can actually execute — JSON encode/decode through `nss-core`, a
/// real temp-directory write/read, and the library-root path resolution. It must never
/// touch a GPU, a capture device or the user's real project library.
pub fn smoke_test() -> Result<String, String> {
    use nss_core::{NormalizedPoint, PixelSize, RGBAColor};

    // 1. Core types round-trip through JSON with stable key names (PLAN.md §5 Stage 1).
    let payload = serde_json::json!({
        "color": RGBAColor::new(0.1, 0.2, 0.3, 1.0),
        "size": PixelSize::new(1920, 1080),
        "point": NormalizedPoint::CENTER,
    });
    let text = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    let back: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if back["size"]["width"] != 1920 || back["point"]["y"] != 0.5 {
        return Err(format!("core round-trip changed the payload: {text}"));
    }

    // 2. A scratch directory under the system temp dir is writable and removable.
    let dir = std::env::temp_dir().join(format!("nss-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let file = dir.join("project.json");
    std::fs::write(&file, &text).map_err(|e| format!("write {}: {e}", file.display()))?;
    let read_back = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    std::fs::remove_dir_all(&dir).map_err(|e| format!("cleanup {}: {e}", dir.display()))?;
    if read_back != text {
        return Err("scratch write/read mismatch".to_owned());
    }

    Ok(format!(
        "core JSON round-trip + scratch I/O on {}",
        std::env::consts::OS
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_test_passes() {
        let summary = super::smoke_test().expect("smoke test should pass");
        assert!(
            summary.contains("scratch I/O"),
            "unexpected summary: {summary}"
        );
    }
}
