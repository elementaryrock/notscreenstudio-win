//! Persisted user preferences (`ShellSettings`) and atomic JSON store.
//!
//! Port of upstream `NSSUI/Shell/ShellSettings.swift`. Every field decodes with a default so
//! older settings files remain valid across upgrades.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nss_core::{AspectRatio, CameraBubble, CameraShape, CaptureSource, NormalizedPoint};
use serde::{Deserialize, Serialize};

pub const COUNTDOWN_CHOICES: [i32; 3] = [0, 3, 5];
pub const DEFAULT_COUNTDOWN_SECONDS: i32 = 3;
pub const DEFAULT_BUBBLE_SIZE: f64 = 0.18;
pub const DEFAULT_BUBBLE_CENTER: NormalizedPoint = NormalizedPoint { x: 0.88, y: 0.80 };
pub const DEFAULT_RECORD_HOTKEY: &str = "Ctrl+Shift+R";
pub const DEFAULT_PAUSE_HOTKEY: &str = "Ctrl+Shift+P";
pub const MAX_RECENT_IMAGES: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellSettings {
    #[serde(default)]
    pub source: Option<CaptureSource>,
    #[serde(default)]
    pub mic_enabled: bool,
    #[serde(default)]
    pub mic_device_id: Option<String>,
    #[serde(default)]
    pub capture_system_audio: bool,
    #[serde(default)]
    pub camera_enabled: bool,
    #[serde(default)]
    pub camera_device_id: Option<String>,
    #[serde(default = "default_camera_shape")]
    pub camera_shape: CameraShape,
    #[serde(default = "default_bubble_size")]
    pub bubble_size: f64,
    #[serde(default = "default_bubble_center")]
    pub bubble_center: NormalizedPoint,
    #[serde(default = "default_bubble_roundness")]
    pub bubble_roundness: f64,
    #[serde(default = "default_countdown")]
    pub countdown_seconds: i32,
    #[serde(default = "default_record_hotkey")]
    pub record_hotkey: String,
    #[serde(default = "default_pause_hotkey")]
    pub pause_hotkey: String,
    #[serde(default)]
    pub default_aspect: AspectRatio,
    #[serde(default)]
    pub default_export_dir: Option<PathBuf>,
    #[serde(default)]
    pub recent_background_images: Vec<String>,
    #[serde(default)]
    pub inspector_sections: HashMap<String, bool>,
}

fn default_camera_shape() -> CameraShape {
    CameraShape::Circle
}
fn default_bubble_size() -> f64 {
    DEFAULT_BUBBLE_SIZE
}
fn default_bubble_center() -> NormalizedPoint {
    DEFAULT_BUBBLE_CENTER
}
fn default_bubble_roundness() -> f64 {
    CameraBubble::default().roundness
}
fn default_countdown() -> i32 {
    DEFAULT_COUNTDOWN_SECONDS
}
fn default_record_hotkey() -> String {
    DEFAULT_RECORD_HOTKEY.to_owned()
}
fn default_pause_hotkey() -> String {
    DEFAULT_PAUSE_HOTKEY.to_owned()
}

impl Default for ShellSettings {
    fn default() -> Self {
        Self {
            source: None,
            mic_enabled: false,
            mic_device_id: None,
            capture_system_audio: false,
            camera_enabled: false,
            camera_device_id: None,
            camera_shape: CameraShape::Circle,
            bubble_size: DEFAULT_BUBBLE_SIZE,
            bubble_center: DEFAULT_BUBBLE_CENTER,
            bubble_roundness: CameraBubble::default().roundness,
            countdown_seconds: DEFAULT_COUNTDOWN_SECONDS,
            record_hotkey: DEFAULT_RECORD_HOTKEY.to_owned(),
            pause_hotkey: DEFAULT_PAUSE_HOTKEY.to_owned(),
            default_aspect: AspectRatio::R16x9,
            default_export_dir: None,
            recent_background_images: Vec::new(),
            inspector_sections: HashMap::new(),
        }
    }
}

impl ShellSettings {
    /// Clamps persisted fields to legal ranges (matches upstream `ShellSettings.init(from:)`).
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.bubble_roundness = if self.bubble_roundness.is_finite() {
            self.bubble_roundness.clamp(0.0, 1.0)
        } else {
            default_bubble_roundness()
        };
        self.bubble_size = if self.bubble_size.is_finite() {
            self.bubble_size.clamp(0.08, 0.50)
        } else {
            DEFAULT_BUBBLE_SIZE
        };
        self.bubble_center = if self.bubble_center.x.is_finite() && self.bubble_center.y.is_finite()
        {
            self.bubble_center.clamped()
        } else {
            DEFAULT_BUBBLE_CENTER
        };
        if !COUNTDOWN_CHOICES.contains(&self.countdown_seconds) {
            self.countdown_seconds = DEFAULT_COUNTDOWN_SECONDS;
        }
        if self.recent_background_images.len() > MAX_RECENT_IMAGES {
            self.recent_background_images.truncate(MAX_RECENT_IMAGES);
        }
        self
    }

    /// Adds a background image path to the front of the 8-item MRU list.
    pub fn record_recent_image(&mut self, path: String) {
        self.recent_background_images
            .retain(|p| !p.eq_ignore_ascii_case(&path));
        self.recent_background_images.insert(0, path);
        self.recent_background_images.truncate(MAX_RECENT_IMAGES);
    }
}

/// JSON file store for [`ShellSettings`].
#[derive(Debug, Clone)]
pub struct ShellSettingsStore {
    path: PathBuf,
}

impl ShellSettingsStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn default_location() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .unwrap_or_else(std::env::temp_dir);
        Self::new(base.join("notscreenstudio").join("settings.json"))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn load(&self) -> ShellSettings {
        let Ok(bytes) = std::fs::read(&self.path) else {
            return ShellSettings::default();
        };
        serde_json::from_slice::<ShellSettings>(&bytes)
            .map(ShellSettings::sanitized)
            .unwrap_or_default()
    }

    pub fn save(&self, settings: &ShellSettings) -> Result<(), String> {
        let clean = settings.clone().sanitized();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create settings folder: {e}"))?;
        }
        let json = serde_json::to_vec_pretty(&clean)
            .map_err(|e| format!("Could not serialize settings: {e}"))?;
        nss_project::atomic_file::write_atomic(&json, &self.path)
            .map_err(|e| format!("Could not save settings file: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_json_decodes_with_all_defaults() {
        let s: ShellSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.sanitized(), ShellSettings::default());
    }

    #[test]
    fn sanitized_clamps_roundness_and_invalid_countdown() {
        let s = ShellSettings {
            bubble_roundness: 2.5,
            countdown_seconds: 99,
            ..ShellSettings::default()
        }
        .sanitized();
        assert_eq!(s.bubble_roundness, 1.0);
        assert_eq!(s.countdown_seconds, DEFAULT_COUNTDOWN_SECONDS);
    }

    #[test]
    fn store_round_trips_and_caps_recent_images_at_eight() {
        let dir = tempfile::tempdir().unwrap();
        let store = ShellSettingsStore::new(dir.path().join("settings.json"));
        let mut s = ShellSettings::default();
        for i in 0..12 {
            s.record_recent_image(format!("C:\\img\\bg_{i}.png"));
        }
        assert_eq!(s.recent_background_images.len(), MAX_RECENT_IMAGES);
        assert_eq!(s.recent_background_images[0], "C:\\img\\bg_11.png");
        s.mic_enabled = true;
        s.countdown_seconds = 5;
        store.save(&s).unwrap();
        let loaded = store.load();
        assert_eq!(loaded, s);
    }
}
