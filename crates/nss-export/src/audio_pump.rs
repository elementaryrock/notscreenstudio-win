//! Audio trim, 48 kHz stereo resampling, drift compensation and AAC 192 kbps mux planning.
//!
//! Port of upstream `NSSExport/AudioPump.swift` (PLAN.md §7.5).

use std::path::{Path, PathBuf};

use crate::ffmpeg::FfmpegPaths;

/// Standard output audio parameters (PLAN.md §7.5).
pub const OUTPUT_SAMPLE_RATE: u32 = 48_000;
pub const OUTPUT_CHANNELS: u32 = 2;
pub const OUTPUT_AAC_BITRATE: u32 = 192_000;

/// Describes which audio sources in the bundle overlap the trimmed export range and the exact
/// FFmpeg arguments to trim, resample/drift-compensate, mix and encode them into the MP4.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioMixPlan {
    pub inputs: Vec<PathBuf>,
    pub trim_start: f64,
    pub trim_length: f64,
}

impl AudioMixPlan {
    /// Inspects candidate media files (`mic.m4a`, `screen.mov`) and returns a mix plan only if at
    /// least one contains an audio stream overlapping `[start, start + length]`.
    #[must_use]
    pub fn discover(
        paths: &FfmpegPaths,
        candidates: &[&Path],
        start: f64,
        length: f64,
    ) -> Option<Self> {
        if length <= 0.0 {
            return None;
        }
        let mut inputs = Vec::new();
        for &path in candidates {
            if !path.is_file() {
                continue;
            }
            if let Ok(probe) = paths.probe(path) {
                if probe.audio_tracks > 0 && probe.audio_duration > start {
                    inputs.push(path.to_path_buf());
                }
            }
        }
        if inputs.is_empty() {
            None
        } else {
            Some(Self {
                inputs,
                trim_start: start.max(0.0),
                trim_length: length,
            })
        }
    }

    /// Builds the FFmpeg input, filtergraph, map and encoder arguments for this audio plan,
    /// assuming rawvideo occupies input index `0`.
    #[must_use]
    pub fn ffmpeg_args(&self) -> (Vec<String>, Vec<String>) {
        let mut input_args = Vec::new();
        for path in &self.inputs {
            input_args.push("-ss".into());
            input_args.push(format!("{:.6}", self.trim_start));
            input_args.push("-t".into());
            input_args.push(format!("{:.6}", self.trim_length));
            input_args.push("-i".into());
            input_args.push(path.to_string_lossy().into_owned());
        }

        let mut output_args = Vec::new();
        if self.inputs.len() == 1 {
            output_args.extend([
                "-filter_complex".into(),
                format!(
                    "[1:a:0]aresample={OUTPUT_SAMPLE_RATE}:async=1,aformat=sample_fmts=fltp:channel_layouts=stereo,atrim=0:{:.6},asetpts=PTS-STARTPTS[aout]",
                    self.trim_length
                ),
                "-map".into(),
                "[aout]".into(),
            ]);
        } else {
            let mut chains = Vec::new();
            let mut labels = Vec::new();
            for i in 0..self.inputs.len() {
                let idx = i + 1;
                let lbl = format!("[a{i}]");
                chains.push(format!(
                    "[{idx}:a:0]aresample={OUTPUT_SAMPLE_RATE}:async=1,aformat=sample_fmts=fltp:channel_layouts=stereo,atrim=0:{:.6},asetpts=PTS-STARTPTS{lbl}",
                    self.trim_length
                ));
                labels.push(lbl);
            }
            let filter = format!(
                "{};{}amix=inputs={}:duration=longest:dropout_transition=0[aout]",
                chains.join(";"),
                labels.join(""),
                self.inputs.len()
            );
            output_args.extend([
                "-filter_complex".into(),
                filter,
                "-map".into(),
                "[aout]".into(),
            ]);
        }

        output_args.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            OUTPUT_AAC_BITRATE.to_string(),
            "-ar".into(),
            OUTPUT_SAMPLE_RATE.to_string(),
            "-ac".into(),
            OUTPUT_CHANNELS.to_string(),
        ]);

        (input_args, output_args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_audio_plan_resamples_to_48k_stereo_aac_192k() {
        let plan = AudioMixPlan {
            inputs: vec![PathBuf::from("mic.m4a")],
            trim_start: 0.1,
            trim_length: 0.3,
        };
        let (in_args, out_args) = plan.ffmpeg_args();
        assert!(in_args.contains(&"mic.m4a".to_string()));
        assert!(out_args.contains(&"aac".to_string()));
        assert!(out_args.contains(&"192000".to_string()));
        assert!(out_args.contains(&"48000".to_string()));
        assert!(out_args.contains(&"2".to_string()));
        assert!(out_args
            .iter()
            .any(|a| a.contains("aresample=48000:async=1")));
    }

    #[test]
    fn dual_audio_plan_uses_amix_filter() {
        let plan = AudioMixPlan {
            inputs: vec![PathBuf::from("mic.m4a"), PathBuf::from("screen.mov")],
            trim_start: 0.0,
            trim_length: 2.0,
        };
        let (_, out_args) = plan.ffmpeg_args();
        assert!(out_args.iter().any(|a| a.contains("amix=inputs=2")));
    }
}
