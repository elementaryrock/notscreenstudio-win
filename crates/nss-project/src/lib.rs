//! `.nssproj` bundles: atomic save, autosave debounce, migration, project library.
//!
//! Replaces upstream NSSProject. See `PLAN.md` §5 Stage 3 and Appendix A.
//!
//! Layout of a bundle (a directory, not an archive, so media can be replaced in place):
//!
//! ```text
//! Demo.nssproj/
//!   project.json          pretty, sorted keys, schema 2
//!   project.json.bak      the last known-good project.json
//!   cursor.json           compact; the largest file in the bundle
//!   screen.mov            the recording
//!   camera.mov, mic.m4a   optional
//!   thumbnail.jpg         generated on demand
//! ```
//!
//! `project.json` is the one format shared byte-for-byte with the Mac build (PLAN.md §7.3), so key
//! names come from `nss-core` and this crate never renames a field.
//!
//! `nss-project` is *not* platform-neutral (it owns the Recycle Bin call), but it is deliberately
//! sync: the Tauri layer owns the async boundary (ADR 0009).

#![deny(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod atomic_file;
pub mod debounce;
pub mod errors;
pub mod library;
pub mod migrator;
pub mod recent_projects;
pub mod store;
pub mod windows_trash;

pub use debounce::{AutosaveDebouncer, DebounceClock, ManualClock, SystemClock};
pub use errors::{ProjectError, ProjectMigrationError};
pub use library::{
    FfmpegThumbnailer, FileProjectLibrary, LibraryOptions, NoopThumbnailer, Thumbnailer, Trash,
};
pub use migrator::ProjectMigrator;
pub use recent_projects::{RecentProject, RecentProjects};
pub use store::{default_library_dir, sanitize_name, FileProjectStore, StoreOptions};
pub use windows_trash::RecycleBinTrash;

/// Non-interactive check used by `notscreenstudio --smoke-test`.
///
/// Upstream's `bundle.sh` runs the app once with `--smoke-test` after every build so a broken
/// dependency graph cannot ship. We do the same: this exercises the code a headless CI runner can
/// actually execute — JSON encode/decode through `nss-core`, a real bundle round-trip through the
/// store, and the library listing. It must never touch a GPU, a capture device or the user's real
/// project library, so everything happens in a scratch directory that is removed afterwards.
pub fn smoke_test() -> Result<String, String> {
    use nss_core::{CursorTrack, NormalizedPoint, PixelSize, RGBAColor, RecordingAssets};

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

    // 2. A real bundle: create, save, load, list, cursor track, then clean up.
    let root = std::env::temp_dir().join(format!("nss-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let result = (|| -> Result<String, String> {
        let store = FileProjectStore::at(&root);
        let reference = store.create_project("smoke").map_err(|e| e.to_string())?;
        let mut project = nss_core::Project::new(
            "smoke",
            RecordingAssets::new(PixelSize::new(1920, 1080), 60.0, 2.0),
        );
        project.touch();
        store
            .save(&project, &reference)
            .map_err(|e| e.to_string())?;
        let loaded = store.load(&reference).map_err(|e| e.to_string())?;
        if loaded != project {
            return Err("project save/load round-trip changed the project".to_owned());
        }

        let track = CursorTrack {
            samples: vec![nss_core::CursorSample::new(0.0, 0.25, 0.75)],
            ..CursorTrack::default()
        };
        store
            .save_cursor_track(&track, &reference, &loaded.assets)
            .map_err(|e| e.to_string())?;
        if store
            .load_cursor_track(&reference, &loaded.assets)
            .map_err(|e| e.to_string())?
            != track
        {
            return Err("cursor track round-trip changed the track".to_owned());
        }

        let library = FileProjectLibrary::at(&root);
        let summaries = library.summaries();
        if summaries.len() != 1 || summaries[0].name != "smoke" {
            return Err(format!("library listing looks wrong: {summaries:?}"));
        }
        Ok(format!(
            "core JSON round-trip + bundle I/O + library listing on {}",
            std::env::consts::OS
        ))
    })();
    let _ = std::fs::remove_dir_all(&root);
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_test_passes() {
        let summary = super::smoke_test().expect("smoke test should pass");
        assert!(
            summary.contains("bundle I/O"),
            "unexpected summary: {summary}"
        );
    }
}
