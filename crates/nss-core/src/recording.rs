//! RecordingOptions, RecordingState, CaptureSource, RecordingAssets.
//!
//! Port of upstream `NSSCore/Recording.swift`. `RecordingAssets` is what a project bundle
//! stores about its media, including `startHostTime` — the t = 0 origin that every cursor
//! and click timestamp is rebased onto.

use serde::{Deserialize, Serialize};

use crate::basics::{MediaTime, PixelSize, Rect, Timestamp};

/// What to record.
///
/// Serialised exactly as Swift's `Codable`: `{"display":{"id":1}}`,
/// `{"window":{"id":2}}`, `{"area":{"displayID":1,"rect":{…}}}`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CaptureSource {
    /// A whole display.
    Display { id: u32 },
    /// A single window.
    Window { id: u32 },
    /// A rectangle on a display. `rect` is in that display's points, origin top-left.
    Area {
        #[serde(rename = "displayID")]
        display_id: u32,
        rect: Rect,
    },
}

impl CaptureSource {
    /// True for [`CaptureSource::Area`], which the compositor crops out of a display capture.
    #[must_use]
    pub fn is_area(self) -> bool {
        matches!(self, Self::Area { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingOptions {
    pub source: CaptureSource,
    /// Device id of the microphone; `None` = no mic.
    #[serde(default, rename = "micDeviceID")]
    pub mic_device_id: Option<String>,
    /// Device id of the camera; `None` = no camera.
    #[serde(default, rename = "cameraDeviceID")]
    pub camera_device_id: Option<String>,
    #[serde(default, rename = "captureSystemAudio")]
    pub capture_system_audio: bool,
    /// Seconds counted down before recording starts; 0 = off.
    #[serde(
        default = "RecordingOptions::default_countdown_seconds",
        rename = "countdownSeconds"
    )]
    pub countdown_seconds: i32,
}

impl RecordingOptions {
    /// The countdown default for an older project that has no `countdownSeconds` key.
    pub const DEFAULT_COUNTDOWN_SECONDS: i32 = 3;

    const fn default_countdown_seconds() -> i32 {
        Self::DEFAULT_COUNTDOWN_SECONDS
    }

    pub fn new(source: CaptureSource) -> Self {
        Self {
            source,
            mic_device_id: None,
            camera_device_id: None,
            capture_system_audio: false,
            countdown_seconds: Self::DEFAULT_COUNTDOWN_SECONDS,
        }
    }
}

/// Files inside a `.nssproj` bundle. Names are relative to the bundle root.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingAssets {
    #[serde(
        default = "RecordingAssets::default_screen_file",
        rename = "screenFile"
    )]
    pub screen_file: String,
    #[serde(default, rename = "cameraFile")]
    pub camera_file: Option<String>,
    #[serde(default, rename = "micFile")]
    pub mic_file: Option<String>,
    #[serde(
        default = "RecordingAssets::default_cursor_file",
        rename = "cursorFile"
    )]
    pub cursor_file: String,
    /// Pixel size of the recorded screen video.
    #[serde(rename = "sourcePixelSize")]
    pub source_pixel_size: PixelSize,
    /// Frame rate of the screen video.
    #[serde(rename = "frameRate")]
    pub frame_rate: f64,
    /// Duration in seconds of the recording.
    pub duration: MediaTime,
    /// Shared time origin (t = 0) of the recording: the clock time, in seconds, of the
    /// presentation timestamp of the FIRST screen frame written to `screen_file`.
    ///
    /// Cursor/click timestamps are rebased to this origin by the input logger's `stop`.
    #[serde(default, rename = "startHostTime")]
    pub start_host_time: f64,
    /// Project thumbnail (relative to the bundle root); `None` until generated.
    #[serde(default, rename = "thumbnailFile")]
    pub thumbnail_file: Option<String>,
}

impl RecordingAssets {
    pub const DEFAULT_SCREEN_FILE: &'static str = "screen.mov";
    pub const DEFAULT_CAMERA_FILE: &'static str = "camera.mov";
    pub const DEFAULT_MIC_FILE: &'static str = "mic.m4a";
    pub const DEFAULT_CURSOR_FILE: &'static str = "cursor.json";

    fn default_screen_file() -> String {
        Self::DEFAULT_SCREEN_FILE.to_owned()
    }

    fn default_cursor_file() -> String {
        Self::DEFAULT_CURSOR_FILE.to_owned()
    }

    pub fn new(source_pixel_size: PixelSize, frame_rate: f64, duration: MediaTime) -> Self {
        Self {
            screen_file: Self::default_screen_file(),
            camera_file: None,
            mic_file: None,
            cursor_file: Self::default_cursor_file(),
            source_pixel_size,
            frame_rate,
            duration,
            start_host_time: 0.0,
            thumbnail_file: None,
        }
    }

    /// True when `name` is a plain file name that stays inside the project bundle: non-empty,
    /// no path separators, no Windows drive prefix or NTFS stream (`:`), no control characters,
    /// not `.` / `..`, not ending with space/dot, and not a Windows reserved device name (`CON`,
    /// `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`).
    #[must_use]
    pub fn is_safe_file_name(name: &str) -> bool {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains(['/', '\\', '\0', ':'])
            || name.bytes().any(|b| b < 0x20)
            || name.ends_with(['.', ' '])
        {
            return false;
        }
        let stem = name
            .split('.')
            .next()
            .unwrap_or(name)
            .trim_end()
            .to_ascii_lowercase();
        const RESERVED: [&str; 22] = [
            "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
            "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
        ];
        if RESERVED.contains(&stem.as_str()) {
            return false;
        }
        let mut comps = std::path::Path::new(name).components();
        matches!(
            (comps.next(), comps.next()),
            (Some(std::path::Component::Normal(_)), None)
        )
    }

    /// The first stored file name that is not a plain name inside the bundle, if any.
    ///
    /// Guards the bundle against a hand-edited or hostile `project.json` that points outside
    /// the project directory (upstream `Tests/NSSIntegrationTests/AssetNameSafetyTests`).
    #[must_use]
    pub fn unsafe_file_name(&self) -> Option<&str> {
        let names = [
            Some(&self.screen_file),
            Some(&self.cursor_file),
            self.camera_file.as_ref(),
            self.mic_file.as_ref(),
            self.thumbnail_file.as_ref(),
        ];
        names
            .into_iter()
            .flatten()
            .map(String::as_str)
            .find(|n| !Self::is_safe_file_name(n))
    }

    /// Resolves a bundle-relative asset name, rejecting unsafe ones.
    #[must_use]
    pub fn resolve(&self, bundle: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
        Self::is_safe_file_name(name).then(|| bundle.join(name))
    }

    /// The recorded source aspect ratio (width / height).
    #[must_use]
    pub fn source_aspect(&self) -> f64 {
        self.source_pixel_size.aspect()
    }
}

/// Lifecycle of a recording, mirrored into the tray icon and the recording bar.
///
/// Swift encodes the payload-less cases as `{"idle":{}}`; the Rust variants below do the
/// same, which is why they are declared as empty struct variants rather than unit variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RecordingState {
    Idle {},
    Preparing {},
    Recording { started_at: Timestamp },
    Stopping {},
    Finished {},
    Failed { message: String },
}

impl Default for RecordingState {
    fn default() -> Self {
        Self::Idle {}
    }
}

impl RecordingState {
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Preparing {} | Self::Recording { .. } | Self::Stopping {}
        )
    }

    #[must_use]
    pub fn is_paused_or_idle(&self) -> bool {
        !self.is_active()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_additions() {
        // Mirrors upstream Round2ContractTests.recordingAdditions.
        assert_eq!(
            RecordingOptions::new(CaptureSource::Display { id: 1 }).countdown_seconds,
            3
        );

        let old: RecordingOptions =
            serde_json::from_str(r#"{"source":{"display":{"id":1}},"captureSystemAudio":false}"#)
                .unwrap();
        assert_eq!(old.countdown_seconds, 3);

        let a: RecordingAssets = serde_json::from_str(
            r#"{"sourcePixelSize":{"width":10,"height":10},"frameRate":60,"duration":1}"#,
        )
        .unwrap();
        assert_eq!(a.thumbnail_file, None);
        assert_eq!(a.screen_file, RecordingAssets::DEFAULT_SCREEN_FILE);
        assert_eq!(a.cursor_file, RecordingAssets::DEFAULT_CURSOR_FILE);
        assert_eq!(a.start_host_time, 0.0);
    }

    #[test]
    fn capture_source_encodings_match_swift() {
        let d = serde_json::to_string(&CaptureSource::Display { id: 1 }).unwrap();
        assert_eq!(d, r#"{"display":{"id":1}}"#);
        let w = serde_json::to_string(&CaptureSource::Window { id: 7 }).unwrap();
        assert_eq!(w, r#"{"window":{"id":7}}"#);
        let a = serde_json::to_string(&CaptureSource::Area {
            display_id: 2,
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        })
        .unwrap();
        assert!(
            a.starts_with(r#"{"area":{"displayID":2,"rect":{"origin":{"x":0.0"#),
            "{a}"
        );
    }

    #[test]
    fn start_host_time_round_trips() {
        // Mirrors upstream NSSCoreTests.startHostTimeRoundTrips.
        let a = RecordingAssets {
            start_host_time: 12345.678,
            ..RecordingAssets::new(PixelSize::new(1, 1), 60.0, 1.0)
        };
        let text = serde_json::to_string(&a).unwrap();
        let back: RecordingAssets = serde_json::from_str(&text).unwrap();
        assert_eq!(back.start_host_time, 12345.678);
    }

    #[test]
    fn unsafe_file_names_are_detected() {
        let mut a = RecordingAssets::new(PixelSize::new(1, 1), 60.0, 1.0);
        assert_eq!(a.unsafe_file_name(), None);
        for bad in ["../evil", "a/b", "a\\b", "..", ".", "", "x\0y"] {
            a.screen_file = bad.to_owned();
            assert!(a.unsafe_file_name().is_some(), "{bad:?} should be rejected");
        }
        a.screen_file = "screen.mov".to_owned();
        a.camera_file = Some("C:/abs/camera.mov".to_owned());
        assert!(a.unsafe_file_name().is_some());
    }

    #[test]
    fn recording_state_encodings_match_swift() {
        assert_eq!(
            serde_json::to_string(&RecordingState::Idle {}).unwrap(),
            r#"{"idle":{}}"#
        );
        assert_eq!(
            serde_json::to_string(&RecordingState::Failed {
                message: "disk full".into()
            })
            .unwrap(),
            r#"{"failed":{"message":"disk full"}}"#
        );
        let rec = RecordingState::Recording {
            started_at: Timestamp(10.0),
        };
        assert_eq!(
            serde_json::to_string(&rec).unwrap(),
            r#"{"recording":{"startedAt":10.0}}"#
        );
        assert!(rec.is_active());
        assert!(!RecordingState::Idle {}.is_active());
    }
}
