//! FFmpeg / FFprobe discovery, media probing and encoder capability detection (ADR 0002 & ADR 0005).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nss_core::{NssError, VideoCodec};
use serde::Deserialize;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Tested FFmpeg release reported in missing-binary user dialogs (ADR 0002).
pub const TESTED_FFMPEG_VERSION: &str = "8.1.1";
pub const FFMPEG_DOWNLOAD_URL: &str = "https://www.gyan.dev/ffmpeg/builds/";

/// Located paths to `ffmpeg` and `ffprobe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

impl FfmpegPaths {
    /// Finds `ffmpeg` and `ffprobe` next to the running executable first, then on `PATH`.
    pub fn discover() -> Result<Self, NssError> {
        let ffmpeg = find_binary("ffmpeg")?;
        let ffprobe = find_binary("ffprobe").unwrap_or_else(|_| {
            // If `ffmpeg` came from a directory next to the exe, try sibling `ffprobe`.
            ffmpeg
                .parent()
                .map(|d| d.join(exe_name("ffprobe")))
                .filter(|p| p.is_file())
                .unwrap_or_else(|| PathBuf::from(exe_name("ffprobe")))
        });
        Ok(Self { ffmpeg, ffprobe })
    }

    /// Queries FFmpeg for available H.264 / HEVC software and hardware encoders.
    #[must_use]
    pub fn capabilities(&self) -> FfmpegCapabilities {
        let mut cmd = Command::new(&self.ffmpeg);
        cmd.args(["-hide_banner", "-encoders"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let Ok(out) = cmd.output() else {
            return FfmpegCapabilities::default();
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let has = |name: &str| text.contains(name);
        FfmpegCapabilities {
            libx264: has("libx264"),
            libx265: has("libx265"),
            h264_nvenc: has("h264_nvenc"),
            h264_qsv: has("h264_qsv"),
            h264_amf: has("h264_amf"),
            hevc_nvenc: has("hevc_nvenc"),
            hevc_qsv: has("hevc_qsv"),
            hevc_amf: has("hevc_amf"),
        }
    }

    /// Probes stream metadata of a media file using `ffprobe`.
    pub fn probe(&self, path: &Path) -> Result<MediaProbe, NssError> {
        if !path.is_file() {
            return Err(NssError::InvalidProject(format!(
                "Missing media file {}",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
            )));
        }
        let mut cmd = Command::new(&self.ffprobe);
        cmd.args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let out = cmd.output().map_err(|e| {
            NssError::Io(format!("Failed to run ffprobe on {}: {e}", path.display()))
        })?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(NssError::Io(format!(
                "Cannot inspect {}: {}",
                path.display(),
                err.trim()
            )));
        }
        let raw: RawProbe = serde_json::from_slice(&out.stdout)
            .map_err(|e| NssError::Io(format!("Invalid ffprobe JSON: {e}")))?;
        Ok(MediaProbe::from_raw(raw))
    }

    /// Reads the sorted presentation timestamps (and keyframe flags) of all packets in video
    /// stream 0 without decoding any pixels.
    pub fn probe_video_packets(&self, path: &Path) -> Result<Vec<PacketInfo>, NssError> {
        let mut cmd = Command::new(&self.ffprobe);
        cmd.args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,dts_time,flags",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let out = cmd.output().map_err(|e| {
            NssError::Io(format!(
                "Failed to probe packet timestamps for {}: {e}",
                path.display()
            ))
        })?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(NssError::Io(format!(
                "Cannot read video packets of {}: {}",
                path.display(),
                err.trim()
            )));
        }

        let text = String::from_utf8_lossy(&out.stdout);
        let mut packets = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split(',').collect();
            let pts_str = parts.first().copied().unwrap_or("");
            let dts_str = parts.get(1).copied().unwrap_or("");
            let flags = parts.get(2).copied().unwrap_or("");
            let pts = pts_str
                .parse::<f64>()
                .ok()
                .or_else(|| dts_str.parse::<f64>().ok());
            if let Some(pts) = pts {
                if pts.is_finite() {
                    packets.push(PacketInfo {
                        pts,
                        is_keyframe: flags.contains('K'),
                    });
                }
            }
        }
        packets.sort_by(|a, b| a.pts.total_cmp(&b.pts));
        Ok(packets)
    }
}

/// One video packet's presentation timestamp and keyframe flag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PacketInfo {
    pub pts: f64,
    pub is_keyframe: bool,
}

/// Available encoders detected from the installed FFmpeg build.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FfmpegCapabilities {
    pub libx264: bool,
    pub libx265: bool,
    pub h264_nvenc: bool,
    pub h264_qsv: bool,
    pub h264_amf: bool,
    pub hevc_nvenc: bool,
    pub hevc_qsv: bool,
    pub hevc_amf: bool,
}

impl FfmpegCapabilities {
    /// Selects the reliable software encoder for `codec` (ADR 0005: software is always available
    /// and deterministic across machines; hardware encoders are opt-in/fallback-safe).
    #[must_use]
    pub fn encoder_for(&self, codec: VideoCodec) -> Option<&'static str> {
        match codec {
            VideoCodec::H264 => {
                if self.libx264 {
                    Some("libx264")
                } else if self.h264_nvenc {
                    Some("h264_nvenc")
                } else if self.h264_qsv {
                    Some("h264_qsv")
                } else if self.h264_amf {
                    Some("h264_amf")
                } else {
                    None
                }
            }
            VideoCodec::Hevc => {
                if self.libx265 {
                    Some("libx265")
                } else if self.hevc_nvenc {
                    Some("hevc_nvenc")
                } else if self.hevc_qsv {
                    Some("hevc_qsv")
                } else if self.hevc_amf {
                    Some("hevc_amf")
                } else {
                    None
                }
            }
        }
    }
}

/// Summary of a media file's streams and timing from `ffprobe`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaProbe {
    pub has_video: bool,
    pub width: u32,
    pub height: u32,
    pub video_codec: Option<String>,
    pub nominal_fps: f64,
    pub video_frames: u64,
    pub first_video_pts: f64,
    pub duration: f64,
    pub audio_tracks: usize,
    pub audio_sample_rate: u32,
    pub audio_channels: u32,
    pub audio_duration: f64,
}

#[derive(Debug, Deserialize)]
struct RawProbe {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Debug, Deserialize)]
struct RawFormat {
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    nb_frames: Option<String>,
    start_time: Option<String>,
    duration: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
}

impl MediaProbe {
    fn from_raw(raw: RawProbe) -> Self {
        let mut out = Self::default();
        if let Some(fmt) = raw.format {
            if let Some(d) = fmt.duration.and_then(|s| s.parse::<f64>().ok()) {
                if d.is_finite() && d > 0.0 {
                    out.duration = d;
                }
            }
        }
        for s in raw.streams {
            match s.codec_type.as_deref() {
                Some("video") if !out.has_video => {
                    out.has_video = true;
                    out.width = s.width.unwrap_or(0);
                    out.height = s.height.unwrap_or(0);
                    out.video_codec = s.codec_name;
                    out.nominal_fps = s
                        .avg_frame_rate
                        .as_deref()
                        .and_then(parse_rational)
                        .filter(|&f| f > 0.0)
                        .or_else(|| s.r_frame_rate.as_deref().and_then(parse_rational))
                        .unwrap_or(0.0);
                    out.video_frames = s
                        .nb_frames
                        .as_deref()
                        .and_then(|n| n.parse::<u64>().ok())
                        .unwrap_or(0);
                    out.first_video_pts = s
                        .start_time
                        .as_deref()
                        .and_then(|t| t.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    if let Some(d) = s.duration.and_then(|v| v.parse::<f64>().ok()) {
                        if d.is_finite() && d > 0.0 && out.duration <= 0.0 {
                            out.duration = d;
                        }
                    }
                }
                Some("audio") => {
                    out.audio_tracks += 1;
                    if out.audio_tracks == 1 {
                        out.audio_sample_rate = s
                            .sample_rate
                            .as_deref()
                            .and_then(|r| r.parse::<u32>().ok())
                            .unwrap_or(0);
                        out.audio_channels = s.channels.unwrap_or(0);
                        out.audio_duration = s
                            .duration
                            .as_deref()
                            .and_then(|d| d.parse::<f64>().ok())
                            .unwrap_or(out.duration);
                    }
                }
                _ => {}
            }
        }
        out
    }
}

fn parse_rational(s: &str) -> Option<f64> {
    if let Some((a, b)) = s.split_once('/') {
        let num = a.parse::<f64>().ok()?;
        let den = b.parse::<f64>().ok()?;
        if den > 0.0 {
            Some(num / den)
        } else {
            None
        }
    } else {
        s.parse::<f64>().ok()
    }
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

fn find_binary(stem: &str) -> Result<PathBuf, NssError> {
    let name = exe_name(stem);
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(&name);
            if sibling.is_file() {
                return Ok(sibling);
            }
        }
    }
    let mut cmd = Command::new(&name);
    cmd.arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    if let Ok(st) = cmd.status() {
        if st.success() {
            return Ok(PathBuf::from(name));
        }
    }
    #[cfg(windows)]
    if let Some(found) = fallback_windows_binary(&name) {
        return Ok(found);
    }
    Err(NssError::Unsupported(format!(
        "FFmpeg is required for export and playback (tested with FFmpeg {TESTED_FFMPEG_VERSION}). \
         Place ffmpeg.exe and ffprobe.exe next to the app or install FFmpeg on PATH ({FFMPEG_DOWNLOAD_URL})."
    )))
}

#[cfg(windows)]
fn fallback_windows_binary(exe_name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        candidates.push(local.join("notscreenstudio").join("ffmpeg").join(exe_name));
        candidates.push(
            local
                .join("Microsoft")
                .join("WinGet")
                .join("Links")
                .join(exe_name),
        );
    }
    if let Some(prog_data) = std::env::var_os("ProgramData") {
        candidates.push(
            PathBuf::from(prog_data)
                .join("chocolatey")
                .join("bin")
                .join(exe_name),
        );
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        candidates.push(
            PathBuf::from(profile)
                .join("scoop")
                .join("shims")
                .join(exe_name),
        );
    }
    candidates.into_iter().find(|p| p.is_file())
}
