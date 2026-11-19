//! Contracts shared by every crate, plus the error type.
//!
//! Port of upstream `NSSCore/Protocols.swift`. Two deliberate differences from the Swift
//! originals, both recorded in ADR 0009:
//!
//! 1. **Traits are synchronous.** Upstream marks most of these `async` because Swift
//!    actors and `await` are the idiomatic way to hop queues there. In Rust, capture on
//!    Windows needs an STA thread with a WinRT message loop and encoding needs a worker
//!    process anyway, so the async work lives *inside* the implementations (own thread +
//!    channel) rather than in the trait signature. That keeps `nss-core` free of any
//!    runtime dependency and keeps the motion path a plain pure function.
//! 2. **Frame-level traits live in `nss-render`.** `FrameSource` and `FrameRenderer` in
//!    upstream traffic in `CVPixelBuffer`, which has no platform-neutral analogue. They
//!    are defined against `nss-render`'s `Frame` type in Stage 4 so that `nss-core` stays
//!    buildable on any host (PLAN.md §3 hard rule).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use thiserror::Error;
use uuid::Uuid;

use crate::basics::{MediaTime, Timestamp};
use crate::cursor::CursorTrack;
use crate::frame_state::FrameState;
use crate::project::{Project, ProjectReference, ProjectSummary};
use crate::recording::{RecordingAssets, RecordingOptions, RecordingState};

pub type Result<T, E = NssError> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NssError {
    /// A code path that is declared but not built yet (upstream `notImplemented`).
    #[error("not implemented: {0}")]
    NotImplemented(String),

    /// The platform or a device cannot do this (no D3D12 adapter, unsupported format).
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// Camera / microphone / capture consent was refused.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// The project bundle is missing, corrupt or fails migration.
    #[error("invalid project: {0}")]
    InvalidProject(String),

    /// The compositor or encoder failed.
    #[error("render failed: {0}")]
    RenderFailed(String),

    /// Disk, media or encoder I/O failure.
    #[error("io error: {0}")]
    Io(String),
}

impl NssError {
    /// Message shown to the user. Upstream keeps these short and actionable.
    #[must_use]
    pub fn user_message(&self) -> &str {
        match self {
            Self::NotImplemented(m)
            | Self::Unsupported(m)
            | Self::PermissionDenied(m)
            | Self::InvalidProject(m)
            | Self::RenderFailed(m)
            | Self::Io(m) => m,
        }
    }
}

impl From<std::io::Error> for NssError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

// MARK: - Motion

/// Pure, deterministic map from time to per-frame state. Built from a [`CursorTrack`],
/// [`crate::timeline::Timeline`] and [`crate::style::CursorStyle`].
///
/// MUST NOT depend on call order or wall-clock: calling [`MotionModel::frame_state`] with
/// the same `t` always returns the same value, in any order, from any thread. Preview and
/// export share one implementation so they match.
pub trait MotionModel: Send {
    fn frame_state(&self, t: MediaTime) -> FrameState;

    /// Duration the model was built for.
    fn duration(&self) -> MediaTime;
}

// MARK: - Project persistence

/// Create/load/save/autosave `.nssproj` bundles.
pub trait ProjectStore {
    /// Default library folder, e.g. `%USERPROFILE%\Videos\notscreenstudio`.
    fn library_url(&self) -> &Path;

    /// Create an empty bundle directory for a new recording and return its reference.
    fn create_project(&self, name: &str) -> Result<ProjectReference>;

    fn load(&self, reference: &ProjectReference) -> Result<Project>;

    fn save(&self, project: &Project, reference: &ProjectReference) -> Result<()>;

    fn load_cursor_track(
        &self,
        reference: &ProjectReference,
        assets: &RecordingAssets,
    ) -> Result<CursorTrack>;

    fn save_cursor_track(
        &self,
        track: &CursorTrack,
        reference: &ProjectReference,
        assets: &RecordingAssets,
    ) -> Result<()>;

    /// Schedule a debounced autosave; later calls replace earlier pending ones.
    fn schedule_autosave(&self, project: &Project, reference: &ProjectReference);

    fn list_projects(&self) -> Result<Vec<ProjectReference>>;
}

// MARK: - Recording

/// Records screen (+ optional mic/camera) into a project bundle directory.
///
/// `recording_start_host_time` in the returned [`RecordingAssets`] is the clock time of the
/// first screen frame, which is the t = 0 origin for every other timestamp.
pub trait RecordingSession {
    /// Checks (and asks for) the permissions the options need, before any recording UI shows.
    fn preflight(&self, options: &RecordingOptions) -> Result<()>;

    /// Start capturing into `bundle`. Writes the screen/camera/mic files there.
    fn start(&mut self, options: RecordingOptions, bundle: &Path) -> Result<()>;

    /// Stop and finalise files; returns the assets (without cursor data, see [`InputLogger`]).
    fn stop(&mut self) -> Result<RecordingAssets>;

    fn cancel(&mut self);

    fn state(&self) -> RecordingState;
}

/// Logs mouse movement (~120 Hz) and clicks, in normalized source space.
pub trait InputLogger {
    /// `source` determines the normalization rect. The logger stamps every event with the
    /// clock time while running; it does not guess the recording origin.
    fn start(&mut self, source: crate::recording::CaptureSource) -> Result<()>;

    /// Stops and returns the track rebased by the logger: each `t` is
    /// `(event clock time - recording_start_host_time)`, so t = 0 is the first screen frame
    /// (`RecordingAssets::start_host_time`), the same clock. Events before the origin are
    /// dropped or clamped to 0.
    fn stop(&mut self, recording_start_host_time: f64) -> CursorTrack;
}

// MARK: - Project library (home screen)

pub trait ProjectLibrary {
    /// All projects, newest first.
    fn summaries(&self) -> Result<Vec<ProjectSummary>>;

    /// Thumbnail for a project, generating it if missing. `None` if it cannot be made.
    fn thumbnail_url(&self, summary: &ProjectSummary) -> Option<PathBuf>;

    /// Moves the project bundle to the Recycle Bin.
    fn delete(&self, summary: &ProjectSummary) -> Result<()>;

    fn rename(&self, summary: &ProjectSummary, to: &str) -> Result<()>;
}

/// In-memory [`ProjectLibrary`] for tests and previews.
#[derive(Debug, Default)]
pub struct InMemoryProjectLibrary {
    items: Mutex<Vec<ProjectSummary>>,
}

impl InMemoryProjectLibrary {
    pub fn new(items: Vec<ProjectSummary>) -> Self {
        Self {
            items: Mutex::new(items),
        }
    }
}

impl ProjectLibrary for InMemoryProjectLibrary {
    fn summaries(&self) -> Result<Vec<ProjectSummary>> {
        let items = self
            .items
            .lock()
            .map_err(|_| NssError::Io("library lock poisoned".into()))?;
        let mut v = items.clone();
        v.sort_by(|a, b| {
            b.created_at
                .partial_cmp(&a.created_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(v)
    }

    fn thumbnail_url(&self, summary: &ProjectSummary) -> Option<PathBuf> {
        self.items
            .lock()
            .ok()?
            .iter()
            .find(|s| s.id == summary.id)
            .and_then(|s| s.thumbnail_url.clone())
    }

    fn delete(&self, summary: &ProjectSummary) -> Result<()> {
        let mut items = self
            .items
            .lock()
            .map_err(|_| NssError::Io("library lock poisoned".into()))?;
        items.retain(|s| s.id != summary.id);
        Ok(())
    }

    fn rename(&self, summary: &ProjectSummary, to: &str) -> Result<()> {
        let mut items = self
            .items
            .lock()
            .map_err(|_| NssError::Io("library lock poisoned".into()))?;
        if let Some(item) = items.iter_mut().find(|s| s.id == summary.id) {
            item.name = to.to_owned();
        }
        Ok(())
    }
}

/// A project row plus its identity, for callers that prefer not to hold a whole summary.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryEntry {
    pub id: Uuid,
    pub name: String,
    pub created_at: Timestamp,
    pub bundle: ProjectReference,
}

impl From<&ProjectSummary> for LibraryEntry {
    fn from(s: &ProjectSummary) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
            created_at: s.created_at,
            bundle: s.bundle.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_map_to_the_io_variant() {
        let e: NssError = std::io::Error::new(std::io::ErrorKind::NotFound, "gone").into();
        assert!(matches!(e, NssError::Io(_)));
        assert_eq!(e.user_message(), "gone");
    }

    #[test]
    fn messages_are_displayed_verbatim() {
        let e = NssError::Unsupported("no D3D12 adapter".into());
        assert_eq!(e.to_string(), "unsupported: no D3D12 adapter");
    }

    #[test]
    fn in_memory_library_sorts_renames_and_deletes() {
        // Mirrors upstream Round2ContractTests.inMemoryLibrary.
        let r = ProjectReference::new("/tmp/a.nssproj");
        let old = ProjectSummary {
            created_at: Timestamp::from_unix_secs(1.0),
            ..ProjectSummary::new("old", 1.0, r.clone())
        };
        let new = ProjectSummary {
            created_at: Timestamp::from_unix_secs(9.0),
            ..ProjectSummary::new("new", 2.0, r)
        };
        let lib = InMemoryProjectLibrary::new(vec![old.clone(), new.clone()]);

        let names: Vec<String> = lib
            .summaries()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["new", "old"]);

        lib.rename(&old, "renamed").unwrap();
        lib.delete(&new).unwrap();

        let names: Vec<String> = lib
            .summaries()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["renamed"]);
    }
}
