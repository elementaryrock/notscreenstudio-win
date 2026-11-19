//! Sequential FFmpeg-backed [`FrameSource`] with exact **at-or-before** frame selection.
//!
//! Port of upstream `NSSExport/AssetFrameSource.swift`. Both preview and export use this same
//! frame-selection rule (PLAN.md Appendix B):
//!
//! - For any requested time `t`, return the decoded frame with the largest PTS `<= t`.
//! - For `t` before the first frame of the track, hold the first frame.
//! - For `t` after the last frame of the track, hold the last frame.
//! - Forward sequential access reads one frame at a time (bounded memory: holds at most 2 frames);
//!   backward seeks or large forward jumps restart the decoder at the nearest keyframe at or
//!   before the target frame.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

use nss_core::{MediaTime, NssError};
use nss_render::{FrameError, FrameSource, Image};
use parking_lot::Mutex;

use crate::ffmpeg::{FfmpegPaths, PacketInfo};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Optional error-reporting side channel matching upstream `DecodeErrorReporting`.
pub trait DecodeErrorReporting {
    fn screen_error(&self) -> Option<NssError>;
    fn camera_error(&self) -> Option<NssError>;
}

/// Decodes screen and optional camera tracks from a `.nssproj` bundle using the exact
/// at-or-before frame selection rule.
pub struct AssetFrameSource {
    screen: SequentialVideoReader,
    camera: Option<SequentialVideoReader>,
}

impl AssetFrameSource {
    /// Opens `screen_path` and optional `camera_path`. Returns `NssError::InvalidProject` if a
    /// file does not exist or has no video track.
    pub fn open(screen_path: &Path, camera_path: Option<&Path>) -> Result<Self, NssError> {
        let paths = FfmpegPaths::discover()?;
        let screen = SequentialVideoReader::open(paths.clone(), screen_path)?;
        let camera = match camera_path {
            Some(p) => Some(SequentialVideoReader::open(paths, p)?),
            None => None,
        };
        Ok(Self { screen, camera })
    }

    #[must_use]
    pub fn screen_duration(&self) -> f64 {
        self.screen.duration
    }
}

impl FrameSource for AssetFrameSource {
    fn screen_frame(&mut self, t: MediaTime) -> Result<Option<Image>, FrameError> {
        Ok(self.screen.frame_at(t))
    }

    fn camera_frame(&mut self, t: MediaTime) -> Result<Option<Image>, FrameError> {
        Ok(self.camera.as_mut().and_then(|c| c.frame_at(t)))
    }
}

impl DecodeErrorReporting for AssetFrameSource {
    fn screen_error(&self) -> Option<NssError> {
        self.screen.last_error()
    }

    fn camera_error(&self) -> Option<NssError> {
        self.camera
            .as_ref()
            .and_then(SequentialVideoReader::last_error)
    }
}

struct DecodedFrame {
    index: usize,
    image: Image,
}

struct ActivePipe {
    child: Child,
    stdout: ChildStdout,
    next_index: usize,
}

impl Drop for ActivePipe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct SequentialVideoReader {
    paths: FfmpegPaths,
    path: PathBuf,
    width: u32,
    height: u32,
    duration: f64,
    packets: Vec<PacketInfo>,
    pipe: Option<ActivePipe>,
    current: Option<DecodedFrame>,
    next: Option<DecodedFrame>,
    exhausted: bool,
    failed: bool,
    last_error: Mutex<Option<NssError>>,
}

impl SequentialVideoReader {
    /// Jump threshold in frames before restarting at a nearer keyframe instead of decoding through.
    const JUMP_FRAMES: usize = 90;

    fn open(paths: FfmpegPaths, path: &Path) -> Result<Self, NssError> {
        let probe = paths.probe(path)?;
        if !probe.has_video || probe.width == 0 || probe.height == 0 {
            return Err(NssError::InvalidProject(format!(
                "No video track in {}",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
            )));
        }
        let mut packets = paths.probe_video_packets(path)?;
        if packets.is_empty() && probe.nominal_fps > 0.0 && probe.duration > 0.0 {
            let count = ((probe.duration * probe.nominal_fps).round() as usize).max(1);
            packets = (0..count)
                .map(|i| PacketInfo {
                    pts: (i as f64) / probe.nominal_fps,
                    is_keyframe: i == 0,
                })
                .collect();
        }
        if packets.is_empty() {
            return Err(NssError::InvalidProject(format!(
                "No video frames in {}",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
            )));
        }

        Ok(Self {
            paths,
            path: path.to_path_buf(),
            width: probe.width,
            height: probe.height,
            duration: probe.duration,
            packets,
            pipe: None,
            current: None,
            next: None,
            exhausted: false,
            failed: false,
            last_error: Mutex::new(None),
        })
    }

    fn last_error(&self) -> Option<NssError> {
        self.last_error.lock().clone()
    }

    /// Pure at-or-before index lookup into `self.packets`.
    ///
    /// Small tolerance (`1e-6`) avoids IEEE-754 rounding missing an exact frame boundary like
    /// `6 / 30.0` vs `0.2`.
    fn target_index(&self, t: f64) -> usize {
        if self.packets.is_empty() {
            return 0;
        }
        let pos = self.packets.partition_point(|p| p.pts <= t + 1e-6);
        pos.saturating_sub(1)
    }

    fn frame_at(&mut self, t: f64) -> Option<Image> {
        if self.failed || self.packets.is_empty() {
            return None;
        }
        let target = self.target_index(t);

        if let Some(ref cur) = self.current {
            if cur.index == target {
                return Some(cur.image.clone());
            }
        }

        let need_restart = match (&self.pipe, &self.current) {
            (None, _) => true,
            (Some(_), Some(cur)) if target < cur.index => true,
            (Some(_), Some(cur)) if target > cur.index + Self::JUMP_FRAMES => {
                // Only restart if there is a keyframe strictly after `cur.index` and at or before `target`.
                self.keyframe_before(target) > cur.index
            }
            _ => false,
        };

        if need_restart {
            let start_idx = self.keyframe_before(target);
            if let Err(e) = self.start_pipe(start_idx) {
                self.fail(e);
                return None;
            }
        }

        self.advance_to(target);
        if self.failed {
            return None;
        }

        self.current.as_ref().map(|f| f.image.clone())
    }

    fn keyframe_before(&self, target: usize) -> usize {
        for i in (0..=target.min(self.packets.len().saturating_sub(1))).rev() {
            if self.packets[i].is_keyframe {
                return i;
            }
        }
        0
    }

    fn start_pipe(&mut self, start_idx: usize) -> Result<(), NssError> {
        self.pipe = None;
        self.current = None;
        self.next = None;
        self.exhausted = false;

        let mut cmd = Command::new(&self.paths.ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error"]);
        if start_idx > 0 {
            let seek_pts = self.packets[start_idx].pts.max(0.0);
            cmd.args(["-noaccurate_seek", "-ss", &format!("{seek_pts:.6}")]);
        }
        cmd.arg("-i").arg(&self.path).args([
            "-map",
            "0:v:0",
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ]);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(crate::ffmpeg::CREATE_NO_WINDOW);

        let mut child = cmd.spawn().map_err(|e| {
            NssError::Io(format!(
                "Could not start FFmpeg decoder for {}: {e}",
                self.path.display()
            ))
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| NssError::Io("FFmpeg decoder stdout was not piped.".into()))?;

        self.pipe = Some(ActivePipe {
            child,
            stdout,
            next_index: start_idx,
        });
        Ok(())
    }

    fn advance_to(&mut self, target: usize) {
        while !self.failed {
            if self.next.is_none() && !self.exhausted {
                self.next = self.read_one_frame();
            }
            match self.next.as_ref() {
                Some(n) if n.index <= target => {
                    self.current = self.next.take();
                }
                Some(_) => {
                    if self.current.is_none() {
                        self.current = self.next.take();
                    }
                    break;
                }
                None => break,
            }
        }
    }

    fn read_one_frame(&mut self) -> Option<DecodedFrame> {
        let Some(ref mut pipe) = self.pipe else {
            self.exhausted = true;
            return None;
        };
        let frame_bytes = (self.width as usize) * (self.height as usize) * 4;
        let mut buf = vec![0u8; frame_bytes];
        let mut filled = 0usize;
        while filled < frame_bytes {
            match pipe.stdout.read(&mut buf[filled..]) {
                Ok(0) => {
                    self.exhausted = true;
                    return None;
                }
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    self.fail(NssError::Io(format!(
                        "Decode read failed for {}: {e}",
                        self.path.display()
                    )));
                    return None;
                }
            }
        }
        let idx = pipe.next_index;
        pipe.next_index += 1;
        let image = Image::new(self.width, self.height, buf).ok()?;
        Some(DecodedFrame { index: idx, image })
    }

    fn fail(&mut self, err: NssError) {
        self.failed = true;
        *self.last_error.lock() = Some(err);
    }
}

/// Pure helper selecting the at-or-before frame index from a sorted list of PTS values.
#[must_use]
pub fn select_at_or_before(pts: &[f64], t: f64) -> Option<usize> {
    if pts.is_empty() {
        return None;
    }
    let pos = pts.partition_point(|&p| p <= t + 1e-6);
    Some(pos.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_or_before_selection_handles_exact_between_before_and_after() {
        let pts = [0.0, 0.5, 3.0];
        assert_eq!(select_at_or_before(&pts, -1.0), Some(0));
        assert_eq!(select_at_or_before(&pts, 0.0), Some(0));
        assert_eq!(select_at_or_before(&pts, 0.49), Some(0));
        assert_eq!(select_at_or_before(&pts, 0.5), Some(1));
        assert_eq!(select_at_or_before(&pts, 2.5), Some(1));
        assert_eq!(select_at_or_before(&pts, 2.99), Some(1));
        assert_eq!(select_at_or_before(&pts, 3.0), Some(2));
        assert_eq!(select_at_or_before(&pts, 99.0), Some(2));
        assert_eq!(select_at_or_before(&[], 0.0), None);
    }

    #[test]
    fn holds_first_frame_when_track_starts_late() {
        let pts = [1.0, 1.5];
        assert_eq!(select_at_or_before(&pts, 0.2), Some(0));
        assert_eq!(select_at_or_before(&pts, 0.3), Some(0));
        assert_eq!(select_at_or_before(&pts, 1.2), Some(0));
        assert_eq!(select_at_or_before(&pts, 1.6), Some(1));
    }
}
