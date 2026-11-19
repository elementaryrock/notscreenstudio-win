//! Errors for the project layer, and their mapping to [`nss_core::NssError`].
//!
//! Upstream splits these into `NSSError` (shared) plus a separate `ProjectMigrationError` so
//! callers never match on message text. Same here, with one Windows addition: the atomic-write
//! path can fail on a transient sharing violation, which the UI should retry rather than report.

use nss_core::NssError;
use std::path::PathBuf;

/// Why a project could not be migrated. Typed so callers never match on message text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectMigrationError {
    #[error("Project was saved by a newer version (schema {found}, this app reads {supported})")]
    NewerSchema { found: i32, supported: i32 },

    #[error("No migration from schema {from}")]
    NoMigration { from: i32 },
}

/// Failures from the project layer.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    /// The bundle directory is missing.
    #[error("Project bundle not found: {0}")]
    NotFound(String),

    /// `project.json` (or a fallback) is not valid JSON or not a valid project.
    #[error("{0}")]
    Corrupt(String),

    /// The file is a newer schema than this build understands.
    #[error("{0}")]
    Migration(#[from] ProjectMigrationError),

    /// A name for a bundle, project or asset is not usable.
    #[error("{0}")]
    InvalidName(String),

    /// A file could not be read or written.
    #[error("{path}: {source}", path = .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Deleting from the library failed (for example the Recycle Bin refused).
    #[error("{0}")]
    Delete(String),
}

impl ProjectError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// True when retrying the same operation shortly may succeed.
    ///
    /// Windows hands out sharing violations when an antivirus scanner, the thumbnail generator or
    /// the search indexer has the file open for a few milliseconds. `atomic_file` already retries
    /// internally; this is for callers that want another go after a longer pause.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Io { source, .. } => {
                matches!(
                    source.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Interrupted
                ) || matches!(source.raw_os_error(), Some(32 | 33))
            }
            _ => false,
        }
    }
}

impl From<ProjectError> for NssError {
    fn from(e: ProjectError) -> Self {
        match e {
            ProjectError::Io { path, source } => {
                NssError::Io(format!("{}: {source}", path.display()))
            }
            ProjectError::Delete(m) => NssError::Io(m),
            other => NssError::InvalidProject(other.to_string()),
        }
    }
}

impl From<std::io::Error> for ProjectError {
    fn from(e: std::io::Error) -> Self {
        Self::Io {
            path: PathBuf::new(),
            source: e,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_become_nss_io_and_everything_else_is_an_invalid_project() {
        let io: NssError =
            ProjectError::io("C:/p/project.json", std::io::Error::other("disk")).into();
        assert!(matches!(io, NssError::Io(_)));
        assert!(io.to_string().contains("project.json"));

        let corrupt: NssError = ProjectError::Corrupt("nope".into()).into();
        assert!(matches!(corrupt, NssError::InvalidProject(_)));

        let migration: NssError = ProjectError::Migration(ProjectMigrationError::NewerSchema {
            found: 3,
            supported: 2,
        })
        .into();
        assert!(migration.to_string().contains("newer version"));
    }

    #[test]
    fn only_sharing_and_interrupt_errors_are_transient() {
        let denied = ProjectError::io(
            "x",
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        );
        assert!(denied.is_transient());
        let missing = ProjectError::io("x", std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(!missing.is_transient());
        assert!(!ProjectError::Corrupt("x".into()).is_transient());
    }
}
