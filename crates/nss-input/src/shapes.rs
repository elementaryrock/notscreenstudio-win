//! Cursor shape fingerprinting, classification and change recording.
//!
//! Port of upstream `NSSInput/CursorShapes.swift`, implementing the Windows fallback ladder
//! decided in ADR 0008 and PLAN.md §4.5:
//! 1. Shared `HCURSOR` handle match against `LoadCursorW(IDC_*)`.
//! 2. Size-independent 16x16 bitmap fingerprint (`CursorFingerprint`) + SHA-256 bitmap hash lookup.
//! 3. `CursorShape::Unknown` (which the renderer draws as the arrow).

use std::collections::HashMap;
use std::sync::Arc;

use nss_core::{CursorShape, CursorShapeChange};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

/// A size-independent 16x16 alpha/luminance description of a cursor image.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorFingerprint {
    /// Image width / height.
    pub aspect: f64,
    /// Hotspot X as a fraction of width (0..1, origin top-left).
    pub hot_spot_x: f64,
    /// Hotspot Y as a fraction of height (0..1, origin top-left).
    pub hot_spot_y: f64,
    /// `SIDE * SIDE` pairs of `(alpha, luminance)`, each in `0.0..=1.0`.
    pub pixels: Vec<f32>,
}

impl CursorFingerprint {
    pub const SIDE: usize = 16;

    #[must_use]
    pub fn new(aspect: f64, hot_spot_x: f64, hot_spot_y: f64, pixels: Vec<f32>) -> Self {
        Self {
            aspect,
            hot_spot_x,
            hot_spot_y,
            pixels,
        }
    }

    /// Builds a fingerprint from tightly-packed RGBA8 pixels and a pixel-space hotspot.
    #[must_use]
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8], hot_x: f64, hot_y: f64) -> Option<Self> {
        if width == 0 || height == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
            return None;
        }
        let side = Self::SIDE;
        let w = width as usize;
        let h = height as usize;
        let mut pixels = Vec::with_capacity(side * side * 2);

        // Area-box downsample into 16x16 so rescaled cursors (e.g. 24px vs 48px) fingerprint identically.
        for gy in 0..side {
            let y0 = (gy * h) as f64 / side as f64;
            let y1 = ((gy + 1) * h) as f64 / side as f64;
            let iy0 = (y0.floor() as usize).min(h - 1);
            let iy1 = (y1.ceil() as usize).clamp(iy0 + 1, h);

            for gx in 0..side {
                let x0 = (gx * w) as f64 / side as f64;
                let x1 = ((gx + 1) * w) as f64 / side as f64;
                let ix0 = (x0.floor() as usize).min(w - 1);
                let ix1 = (x1.ceil() as usize).clamp(ix0 + 1, w);

                let mut a_sum = 0.0f32;
                let mut l_sum = 0.0f32;
                let mut n = 0.0f32;
                for y in iy0..iy1 {
                    for x in ix0..ix1 {
                        let off = (y * w + x) * 4;
                        let r = f32::from(rgba[off]) / 255.0;
                        let g = f32::from(rgba[off + 1]) / 255.0;
                        let b = f32::from(rgba[off + 2]) / 255.0;
                        let a = f32::from(rgba[off + 3]) / 255.0;
                        a_sum += a;
                        l_sum += (r + g + b) / 3.0;
                        n += 1.0;
                    }
                }
                let inv = if n > 0.0 { 1.0 / n } else { 0.0 };
                pixels.push(a_sum * inv);
                pixels.push(l_sum * inv);
            }
        }

        Some(Self {
            aspect: f64::from(width) / f64::from(height),
            hot_spot_x: hot_x / f64::from(width),
            hot_spot_y: hot_y / f64::from(height),
            pixels,
        })
    }

    /// Mean absolute pixel difference (`0.0` identical .. `1.0`), or `f32::INFINITY` on mismatch.
    #[must_use]
    pub fn pixel_distance(&self, other: &Self) -> f32 {
        if self.pixels.len() != other.pixels.len() || self.pixels.is_empty() {
            return f32::INFINITY;
        }
        let sum: f32 = self
            .pixels
            .iter()
            .zip(&other.pixels)
            .map(|(a, b)| (a - b).abs())
            .sum();
        sum / self.pixels.len() as f32
    }

    /// True when aspect ratio and normalized hotspot match within `tolerance`.
    #[must_use]
    pub fn same_geometry(&self, other: &Self, tolerance: f64) -> bool {
        (self.aspect - other.aspect).abs() <= tolerance
            && (self.hot_spot_x - other.hot_spot_x).abs() <= tolerance
            && (self.hot_spot_y - other.hot_spot_y).abs() <= tolerance
    }
}

/// Stable SHA-256 digest of a cursor's raw RGBA pixels + dimensions + hotspot (ADR 0008).
#[must_use]
pub fn bitmap_hash(width: u32, height: u32, hot_x: u32, hot_y: u32, rgba: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(width.to_le_bytes());
    hasher.update(height.to_le_bytes());
    hasher.update(hot_x.to_le_bytes());
    hasher.update(hot_y.to_le_bytes());
    hasher.update(rgba);
    hasher.finalize().into()
}

/// Reference entry inside [`CursorShapeIdentifier`].
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeReference {
    pub shape: CursorShape,
    pub fingerprint: CursorFingerprint,
}

/// Maps cursor handles, hashes and fingerprints to [`CursorShape`].
#[derive(Debug, Clone)]
pub struct CursorShapeIdentifier {
    pub references: Vec<ShapeReference>,
    pub handle_map: HashMap<usize, CursorShape>,
    pub hash_map: HashMap<[u8; 32], CursorShape>,
    pub geometry_tolerance: f64,
    pub pixel_tolerance: f32,
}

impl CursorShapeIdentifier {
    #[must_use]
    pub fn new(references: Vec<ShapeReference>) -> Self {
        Self {
            references,
            handle_map: HashMap::new(),
            hash_map: HashMap::new(),
            geometry_tolerance: 0.03,
            pixel_tolerance: 0.01,
        }
    }

    #[must_use]
    pub fn with_handle_map(mut self, handle_map: HashMap<usize, CursorShape>) -> Self {
        self.handle_map = handle_map;
        self
    }

    #[must_use]
    pub fn with_hash_map(mut self, hash_map: HashMap<[u8; 32], CursorShape>) -> Self {
        self.hash_map = hash_map;
        self
    }

    /// Identifies a cursor using the 3-step ADR 0008 ladder:
    /// 1. Direct shared `HCURSOR` handle match.
    /// 2. Exact bitmap SHA-256 hash match.
    /// 3. Nearest 16x16 fingerprint within tolerances, else `CursorShape::Unknown`.
    #[must_use]
    pub fn identify_full(
        &self,
        handle: usize,
        hash: Option<&[u8; 32]>,
        fingerprint: Option<&CursorFingerprint>,
    ) -> CursorShape {
        if let Some(&shape) = self.handle_map.get(&handle) {
            return shape;
        }
        if let Some(h) = hash {
            if let Some(&shape) = self.hash_map.get(h) {
                return shape;
            }
        }
        self.identify(fingerprint)
    }

    /// Best fingerprint match, or `CursorShape::Unknown` when nothing is close enough.
    #[must_use]
    pub fn identify(&self, fingerprint: Option<&CursorFingerprint>) -> CursorShape {
        let Some(fp) = fingerprint else {
            return CursorShape::Unknown;
        };
        let mut best = CursorShape::Unknown;
        let mut best_dist = f32::INFINITY;
        for r in &self.references {
            if !r.fingerprint.same_geometry(fp, self.geometry_tolerance) {
                continue;
            }
            let d = r.fingerprint.pixel_distance(fp);
            if d <= self.pixel_tolerance && d < best_dist {
                best = r.shape;
                best_dist = d;
            }
        }
        best
    }
}

/// Callback returning the live cursor shape right now, or `None` when unreadable.
pub type CursorShapeSampler = Arc<dyn Fn() -> Option<CursorShape> + Send + Sync>;

/// Records cursor shape transitions. Free of timers and Win32 so unit tests drive it directly.
pub struct CursorShapeRecorder {
    sampler: CursorShapeSampler,
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    changes: Mutex<Vec<CursorShapeChange>>,
}

impl CursorShapeRecorder {
    pub fn new(sampler: CursorShapeSampler, clock: Arc<dyn Fn() -> f64 + Send + Sync>) -> Self {
        Self {
            sampler,
            clock,
            changes: Mutex::new(Vec::new()),
        }
    }

    /// Samples once and appends an event only when the shape changed from the last recorded one.
    pub fn tick(&self) {
        let Some(shape) = (self.sampler)() else {
            return;
        };
        let now = (self.clock)();
        let mut changes = self.changes.lock();
        if changes.last().map(|c| c.shape) != Some(shape) {
            changes.push(CursorShapeChange::new(now, shape));
        }
    }

    #[must_use]
    pub fn change_count(&self) -> usize {
        self.changes.lock().len()
    }

    /// Rebases to `recording_start_host_time`, keeping the shape active at `t = 0` and rounding
    /// timestamps to `1e-4` s (exact port of upstream `CursorShapeRecorder.finish`).
    #[must_use]
    pub fn finish(&self, recording_start_host_time: f64) -> Vec<CursorShapeChange> {
        let changes = self.changes.lock().clone();
        let mut out = Vec::with_capacity(changes.len());
        let mut carry: Option<CursorShape> = None;

        for c in changes {
            let t = c.t - recording_start_host_time;
            if t <= 0.0 {
                carry = Some(c.shape);
                continue;
            }
            if let Some(shape) = carry.take() {
                out.push(CursorShapeChange::new(0.0, shape));
            }
            let r = (t * 1e4).round() / 1e4;
            if out.last().map(|last: &CursorShapeChange| last.shape) != Some(c.shape) {
                out.push(CursorShapeChange::new(r, c.shape));
            }
        }
        if out.is_empty() {
            if let Some(shape) = carry {
                out.push(CursorShapeChange::new(0.0, shape));
            }
        }
        out
    }
}

/// Builds the live Windows cursor-shape sampler (handle table + fingerprint table built once).
pub struct SystemCursorShapeSampler;

impl SystemCursorShapeSampler {
    #[must_use]
    pub fn make() -> Option<CursorShapeSampler> {
        #[cfg(windows)]
        {
            crate::win::shapes::make_system_sampler()
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_rgba(w: u32, h: u32, mut f: impl FnMut(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                out.extend_from_slice(&f(x, y));
            }
        }
        out
    }

    fn triangle(size: u32) -> Vec<u8> {
        make_rgba(size, size, |x, y| {
            let fx = f64::from(x) / f64::from(size);
            let fy = f64::from(y) / f64::from(size);
            if fy <= 0.8 && fx <= fy * 0.7 {
                [0, 0, 0, 255]
            } else {
                [0, 0, 0, 0]
            }
        })
    }

    fn bar(w: u32, h: u32) -> Vec<u8> {
        make_rgba(w, h, |x, _| {
            let fx = f64::from(x) / f64::from(w);
            if (0.4..=0.6).contains(&fx) {
                [0, 0, 0, 255]
            } else {
                [0, 0, 0, 0]
            }
        })
    }

    fn disc(size: u32, gray: u8) -> Vec<u8> {
        let c = f64::from(size) / 2.0;
        let r = c - 2.0;
        make_rgba(size, size, |x, y| {
            let dx = f64::from(x) + 0.5 - c;
            let dy = f64::from(y) + 0.5 - c;
            if dx * dx + dy * dy <= r * r {
                [gray, gray, gray, 255]
            } else {
                [0, 0, 0, 0]
            }
        })
    }

    #[test]
    fn emits_only_on_change_and_skips_unreadable_samples() {
        let script = [
            Some(CursorShape::Arrow),
            Some(CursorShape::Arrow),
            Some(CursorShape::IBeam),
            None,
            Some(CursorShape::IBeam),
            Some(CursorShape::PointingHand),
            Some(CursorShape::Unknown),
            Some(CursorShape::Unknown),
            Some(CursorShape::Arrow),
        ];
        let idx = Arc::new(AtomicUsize::new(0));
        let idx_c = Arc::clone(&idx);
        let time = Arc::new(Mutex::new(10.0));
        let time_c = Arc::clone(&time);

        let rec = CursorShapeRecorder::new(
            Arc::new(move || {
                let i = idx_c.fetch_add(1, Ordering::Relaxed);
                script[i.min(script.len() - 1)]
            }),
            Arc::new(move || *time_c.lock()),
        );
        for _ in 0..9 {
            rec.tick();
            *time.lock() += 1.0 / 30.0;
        }
        let shapes: Vec<CursorShape> = rec.finish(9.0).into_iter().map(|c| c.shape).collect();
        assert_eq!(
            shapes,
            [
                CursorShape::Arrow,
                CursorShape::IBeam,
                CursorShape::PointingHand,
                CursorShape::Unknown,
                CursorShape::Arrow,
            ]
        );
    }

    #[test]
    fn finish_rebases_and_keeps_shape_in_effect_at_origin() {
        let script = [
            CursorShape::Arrow,
            CursorShape::IBeam,
            CursorShape::PointingHand,
            CursorShape::Arrow,
        ];
        let times = Arc::new(Mutex::new(vec![101.0, 100.2345678, 99.5, 99.0]));
        let idx = Arc::new(AtomicUsize::new(0));
        let idx_c = Arc::clone(&idx);
        let times_c = Arc::clone(&times);

        let rec = CursorShapeRecorder::new(
            Arc::new(move || Some(script[idx_c.fetch_add(1, Ordering::Relaxed)])),
            Arc::new(move || times_c.lock().pop().unwrap()),
        );
        for _ in 0..4 {
            rec.tick();
        }
        let out = rec.finish(100.0);
        assert_eq!(
            out,
            [
                CursorShapeChange::new(0.0, CursorShape::IBeam),
                CursorShapeChange::new(0.2346, CursorShape::PointingHand),
                CursorShapeChange::new(1.0, CursorShape::Arrow),
            ]
        );
    }

    #[test]
    fn all_changes_before_origin_leave_one_at_zero() {
        let script = [CursorShape::Arrow, CursorShape::Crosshair];
        let times = Arc::new(Mutex::new(vec![2.0, 1.0]));
        let idx = Arc::new(AtomicUsize::new(0));
        let idx_c = Arc::clone(&idx);
        let times_c = Arc::clone(&times);

        let rec = CursorShapeRecorder::new(
            Arc::new(move || Some(script[idx_c.fetch_add(1, Ordering::Relaxed)])),
            Arc::new(move || times_c.lock().pop().unwrap()),
        );
        rec.tick();
        rec.tick();
        assert_eq!(
            rec.finish(5.0),
            [CursorShapeChange::new(0.0, CursorShape::Crosshair)]
        );
        assert!(
            CursorShapeRecorder::new(Arc::new(|| None), Arc::new(|| 0.0))
                .finish(0.0)
                .is_empty()
        );
    }

    #[test]
    fn identifies_exact_rescaled_and_disambiguates_hotspot_and_pixels() {
        let id = CursorShapeIdentifier::new(vec![
            ShapeReference {
                shape: CursorShape::Arrow,
                fingerprint: CursorFingerprint::from_rgba(32, 32, &triangle(32), 4.0, 4.0).unwrap(),
            },
            ShapeReference {
                shape: CursorShape::IBeam,
                fingerprint: CursorFingerprint::from_rgba(16, 32, &bar(16, 32), 8.0, 16.0).unwrap(),
            },
            ShapeReference {
                shape: CursorShape::OpenHand,
                fingerprint: CursorFingerprint::from_rgba(32, 32, &disc(32, 0), 16.0, 16.0)
                    .unwrap(),
            },
            ShapeReference {
                shape: CursorShape::ClosedHand,
                fingerprint: CursorFingerprint::from_rgba(32, 32, &disc(32, 255), 16.0, 16.0)
                    .unwrap(),
            },
            ShapeReference {
                shape: CursorShape::PointingHand,
                fingerprint: CursorFingerprint::from_rgba(32, 32, &disc(32, 0), 13.0, 8.0).unwrap(),
            },
        ]);

        let tri32 = CursorFingerprint::from_rgba(32, 32, &triangle(32), 4.0, 4.0).unwrap();
        let tri64 = CursorFingerprint::from_rgba(64, 64, &triangle(64), 8.0, 8.0).unwrap();
        let bar16 = CursorFingerprint::from_rgba(16, 32, &bar(16, 32), 8.0, 16.0).unwrap();

        assert_eq!(id.identify(Some(&tri32)), CursorShape::Arrow);
        assert_eq!(id.identify(Some(&tri64)), CursorShape::Arrow);
        assert_eq!(id.identify(Some(&bar16)), CursorShape::IBeam);

        let open = CursorFingerprint::from_rgba(32, 32, &disc(32, 0), 16.0, 16.0).unwrap();
        let point = CursorFingerprint::from_rgba(32, 32, &disc(32, 0), 13.0, 8.0).unwrap();
        let closed = CursorFingerprint::from_rgba(32, 32, &disc(32, 255), 16.0, 16.0).unwrap();
        let unknown_gray =
            CursorFingerprint::from_rgba(32, 32, &disc(32, 128), 16.0, 16.0).unwrap();

        assert_eq!(id.identify(Some(&open)), CursorShape::OpenHand);
        assert_eq!(id.identify(Some(&point)), CursorShape::PointingHand);
        assert_eq!(id.identify(Some(&closed)), CursorShape::ClosedHand);
        assert_eq!(id.identify(Some(&unknown_gray)), CursorShape::Unknown);
        assert_eq!(id.identify(None), CursorShape::Unknown);
    }

    #[test]
    fn handle_match_takes_precedence_over_fingerprint() {
        let mut handles = HashMap::new();
        handles.insert(0xDEAD, CursorShape::IBeam);
        let id = CursorShapeIdentifier::new(Vec::new()).with_handle_map(handles);
        assert_eq!(id.identify_full(0xDEAD, None, None), CursorShape::IBeam);
        assert_eq!(id.identify_full(0xBEEF, None, None), CursorShape::Unknown);
    }
}
