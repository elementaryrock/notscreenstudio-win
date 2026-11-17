//! Error type and result alias shared by every crate.
//!
//! Port of upstream `NSSError`. The variants intentionally mirror the Swift ones so
//! user-facing messages stay equivalent.

use thiserror::Error;

pub type Result<T, E = NssError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
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
}
