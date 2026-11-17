//! Project, ProjectReference, export presets and the project library.
//!
//! Port of upstream `NSSCore/Project.swift`. This is the schema of `project.json`, which is
//! the one file format the Mac and Windows builds deliberately share (PLAN.md §7.3), so the
//! key names and defaults here are a compatibility contract, not an implementation detail.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::basics::{MediaTime, Timestamp};
use crate::recording::RecordingAssets;
use crate::style::Style;
use crate::timeline::Timeline;

/// A `.nssproj` bundle's `project.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub created_at: Timestamp,
    pub modified_at: Timestamp,
    pub assets: RecordingAssets,
    pub timeline: Timeline,
    pub style: Style,
    /// v2 (Round 3): `Mesh` and `SystemWallpaper` backgrounds and `Style.backgroundBlur`;
    /// v1 `Wallpaper{name}` backgrounds migrate to `Mesh` (see `nss-project`'s migrator).
    pub schema_version: i32,
}

impl Project {
    /// Current schema version.
    pub const CURRENT_SCHEMA_VERSION: i32 = 2;

    pub fn new(name: impl Into<String>, assets: RecordingAssets) -> Self {
        let now = Timestamp::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            created_at: now,
            modified_at: now,
            assets,
            timeline: Timeline::default(),
            style: Style::default(),
            schema_version: Self::CURRENT_SCHEMA_VERSION,
        }
    }

    /// Marks the project as edited now.
    pub fn touch(&mut self) {
        self.modified_at = Timestamp::now();
    }
}

impl Serialize for Project {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("Project", 8)?;
        st.serialize_field("id", &ProjectUuid(self.id))?;
        st.serialize_field("name", &self.name)?;
        st.serialize_field("createdAt", &self.created_at)?;
        st.serialize_field("modifiedAt", &self.modified_at)?;
        st.serialize_field("assets", &self.assets)?;
        st.serialize_field("timeline", &self.timeline)?;
        st.serialize_field("style", &self.style)?;
        st.serialize_field("schemaVersion", &self.schema_version)?;
        st.end()
    }
}

struct ProjectUuid(Uuid);

impl Serialize for ProjectUuid {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string().to_uppercase())
    }
}

impl<'de> Deserialize<'de> for Project {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Repr {
            #[serde(default, with = "crate::serde_util::uppercase_uuid::option")]
            id: Option<Uuid>,
            name: String,
            created_at: Option<Timestamp>,
            modified_at: Option<Timestamp>,
            assets: RecordingAssets,
            timeline: Option<Timeline>,
            style: Option<Style>,
            schema_version: Option<i32>,
        }
        let r = Repr::deserialize(d)?;
        let created_at = r.created_at.unwrap_or_else(Timestamp::now);
        Ok(Project {
            id: r.id.unwrap_or_else(Uuid::new_v4),
            name: r.name,
            created_at,
            // Upstream: `modifiedAt = decodeIfPresent ?? createdAt`, not "now".
            modified_at: r.modified_at.unwrap_or(created_at),
            assets: r.assets,
            timeline: r.timeline.unwrap_or_default(),
            style: r.style.unwrap_or_default(),
            schema_version: r.schema_version.unwrap_or(Project::CURRENT_SCHEMA_VERSION),
        })
    }
}

/// Reference to a project bundle on disk (a `.nssproj` directory).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectReference {
    pub url: PathBuf,
}

impl ProjectReference {
    pub const FILE_EXTENSION: &'static str = "nssproj";
    pub const PROJECT_FILE_NAME: &'static str = "project.json";
    pub const CURSOR_FILE_NAME: &'static str = "cursor.json";

    pub fn new(url: impl Into<PathBuf>) -> Self {
        Self { url: url.into() }
    }

    /// The bundle's `project.json`.
    #[must_use]
    pub fn project_file(&self) -> PathBuf {
        self.url.join(Self::PROJECT_FILE_NAME)
    }

    /// The bundle's `project.json.bak` (the last known-good copy).
    #[must_use]
    pub fn backup_file(&self) -> PathBuf {
        self.url.join(format!("{}.bak", Self::PROJECT_FILE_NAME))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VideoCodec {
    #[default]
    #[serde(rename = "h264")]
    H264,
    #[serde(rename = "hevc")]
    Hevc,
}

impl VideoCodec {
    /// The FFmpeg encoder name for this codec (ADR 0005).
    #[must_use]
    pub fn ffmpeg_encoder(self) -> &'static str {
        match self {
            Self::H264 => "libx264",
            Self::Hevc => "libx265",
        }
    }

    pub const ALL: [VideoCodec; 2] = [Self::H264, Self::Hevc];
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ExportPreset {
    pub resolution: crate::style::ExportResolution,
    pub fps: i32,
    pub codec: VideoCodec,
}

impl Default for ExportPreset {
    fn default() -> Self {
        Self {
            resolution: crate::style::ExportResolution::P1080,
            fps: 60,
            codec: VideoCodec::H264,
        }
    }
}

impl ExportPreset {
    #[must_use]
    pub fn output_size(&self, aspect: crate::style::AspectRatio) -> crate::basics::PixelSize {
        aspect.output_size(self.resolution)
    }

    /// Total frames for a clip of `duration` seconds, at this preset's fps.
    #[must_use]
    pub fn frame_count(&self, duration: MediaTime) -> u64 {
        if self.fps <= 0 || !duration.is_finite() || duration <= 0.0 {
            return 0;
        }
        (duration * f64::from(self.fps)).round() as u64
    }
}

/// Progress of an export, reported to the UI.
///
/// `Finished(PathBuf)` is a newtype case, so it encodes as `{"finished":"…"}` like Swift's
/// `.finished(URL)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ExportProgress {
    Idle {},
    Running {
        fraction: f64,
        frames_written: u64,
        total_frames: u64,
    },
    Finished(PathBuf),
    Failed {
        message: String,
    },
    Cancelled {},
}

impl Default for ExportProgress {
    fn default() -> Self {
        Self::Idle {}
    }
}

/// One row on the home screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: Uuid,
    pub name: String,
    #[serde(rename = "createdAt")]
    pub created_at: Timestamp,
    pub duration: MediaTime,
    pub bundle: ProjectReference,
    #[serde(default, rename = "thumbnailURL")]
    pub thumbnail_url: Option<PathBuf>,
}

impl ProjectSummary {
    pub fn new(name: impl Into<String>, duration: MediaTime, bundle: ProjectReference) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            created_at: Timestamp::now(),
            duration,
            bundle,
            thumbnail_url: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::basics::{NormalizedPoint, PixelSize};
    use crate::style::CameraShape;
    use crate::timeline::{ZoomBlock, ZoomFocus};

    #[test]
    fn project_round_trips_through_json() {
        // Mirrors upstream NSSCoreTests.projectRoundTripsThroughJSON.
        let assets = RecordingAssets {
            camera_file: Some("camera.mov".into()),
            ..RecordingAssets::new(PixelSize::new(2560, 1440), 60.0, 12.5)
        };
        let mut project = Project::new("Demo", assets);
        project.timeline.zoom_blocks = vec![ZoomBlock {
            focus: ZoomFocus::Manual(NormalizedPoint::new(0.3, 0.7)),
            ..ZoomBlock::new(1.0, 3.0)
        }];
        project.style.camera.shape = CameraShape::Tall;

        let text = serde_json::to_string(&project).unwrap();
        let back: Project = serde_json::from_str(&text).unwrap();
        assert_eq!(back, project);
        assert_eq!(back.schema_version, Project::CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn older_project_json_without_newer_fields_still_decodes() {
        // Verbatim from upstream NSSCoreTests.olderProjectJSONWithoutNewerFieldsStillDecodes.
        let json = r#"
        {"name":"Old","assets":{"sourcePixelSize":{"width":100,"height":50},"frameRate":60,"duration":2},
         "style":{"padding":0.1,"cursor":{"size":2}},"timeline":{}}
        "#;
        let p: Project = serde_json::from_str(json).unwrap();
        assert_eq!(p.name, "Old");
        assert_eq!(p.assets.start_host_time, 0.0);
        assert_eq!(p.assets.screen_file, RecordingAssets::DEFAULT_SCREEN_FILE);
        assert_eq!(p.style.padding, 0.1);
        assert_eq!(p.style.cursor.size, 2.0);
        assert_eq!(
            p.style.cursor.smoothing,
            crate::style::CursorStyle::default().smoothing
        );
        assert_eq!(p.style.camera, crate::style::CameraBubble::default());
        assert!(p.timeline.auto_zoom_enabled);
        assert_eq!(
            p.modified_at, p.created_at,
            "modifiedAt falls back to createdAt"
        );
    }

    #[test]
    fn camera_without_border_round_trips() {
        // Mirrors upstream NSSCoreTests.cameraWithoutBorderRoundTrips.
        let mut p = Project::new("n", RecordingAssets::new(PixelSize::new(1, 1), 60.0, 1.0));
        p.style.camera.border = None;
        let text = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&text).unwrap();
        assert!(back.style.camera.border.is_none());
        assert_eq!(back, p);
    }

    #[test]
    fn export_preset_defaults() {
        // Mirrors upstream NSSCoreTests.exportPresetDefaults.
        let p = ExportPreset::default();
        assert_eq!(p.fps, 60);
        assert_eq!(
            p.output_size(crate::style::AspectRatio::R9x16),
            PixelSize::new(1080, 1920)
        );
        assert_eq!(p.frame_count(2.0), 120);
        assert_eq!(p.frame_count(-1.0), 0);
    }

    #[test]
    fn export_progress_encodings_match_swift() {
        assert_eq!(
            serde_json::to_string(&ExportProgress::Idle {}).unwrap(),
            r#"{"idle":{}}"#
        );
        let running = ExportProgress::Running {
            fraction: 0.5,
            frames_written: 30,
            total_frames: 60,
        };
        assert_eq!(
            serde_json::to_string(&running).unwrap(),
            r#"{"running":{"fraction":0.5,"framesWritten":30,"totalFrames":60}}"#
        );
        let finished = ExportProgress::Finished(PathBuf::from("C:/out.mp4"));
        assert_eq!(
            serde_json::to_string(&finished).unwrap(),
            r#"{"finished":"C:/out.mp4"}"#
        );
    }

    #[test]
    fn project_reference_paths() {
        let r = ProjectReference::new("C:/Movies/notscreenstudio/a.nssproj");
        assert!(r.project_file().ends_with("project.json"));
        assert!(r.backup_file().ends_with("project.json.bak"));
    }
}
