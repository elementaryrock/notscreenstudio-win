//! Separable Gaussian blur for image backgrounds, in linear light.
//!
//! Port of upstream `NSSRender/ImageBlur.swift`. The radius is relative to the image
//! (`BackgroundBlur::sigma`), not the output, so the same blurred texture serves every output size
//! and the preview and the export match. To stay cheap at large radii the blur reads a smaller mip
//! level of the source (each level halves sigma in texels), keeping the kernel at most ~19 taps per
//! pass.
//!
//! Upstream does this in a Metal compute kernel. wgpu gets two fullscreen render passes instead
//! (`blur.wgsl`): same maths, same two-pass separable structure, and it avoids the storage-texture
//! format question entirely — an `Rgba16Float` render target is core WebGPU, a `rgba16float`
//! storage texture is not equally available on every backend.

use nss_core::BackgroundBlur;

/// The kernel works at a mip level where sigma is at most this many texels.
pub const MAX_WORKING_SIGMA: f64 = 6.0;
/// Below this sigma (in full-resolution texels) blurring is skipped.
pub const MIN_SIGMA: f64 = 0.3;

/// Mip level to blur at, and the sigma in that level's texels, for a full-resolution `sigma`.
#[must_use]
pub fn plan(sigma: f64, mip_levels: u32) -> (u32, f64) {
    if !sigma.is_finite() || sigma <= MAX_WORKING_SIGMA {
        return (0, sigma.max(0.0));
    }
    let level = (sigma / MAX_WORKING_SIGMA).log2().ceil().max(0.0) as u32;
    let level = level.min(mip_levels.saturating_sub(1)).min(31);
    (level, sigma / f64::from(1u32 << level))
}

/// Taps each side for a sigma in working-level texels (upstream: `ceil(3 * sigma)`, at least 1).
#[must_use]
pub fn radius_for(sigma: f64) -> u32 {
    if !sigma.is_finite() || sigma <= 0.0 {
        return 1;
    }
    (3.0 * sigma).ceil().clamp(1.0, 256.0) as u32
}

/// The amount to blur for a style, in image pixels of an image whose shorter side is `short_side`.
///
/// Returns `None` when no blur is needed, which is what makes "blur below 1/100 draws the sharp
/// image" true — the quantisation happens before this check, not after.
#[must_use]
pub fn sigma_for(amount: f64, short_side: u32) -> Option<f64> {
    let quantized = BackgroundBlur::quantized(amount);
    if quantized <= 0.0 {
        return None;
    }
    let sigma = BackgroundBlur::sigma(quantized, short_side.min(i32::MAX as u32) as i32);
    if sigma >= MIN_SIGMA {
        Some(sigma)
    } else {
        None
    }
}

/// A CPU separable Gaussian blur of linear-light RGBA f32 pixels, used as the reference in tests and
/// by the thumbnail path.
///
/// `sigma` is in the pixels of the buffer passed in; `radius` taps each side. Edges clamp, so the
/// borders keep their colour instead of darkening.
#[must_use]
pub fn blur_linear_rgba(
    pixels: &[f32],
    width: u32,
    height: u32,
    sigma: f64,
    radius: u32,
) -> Vec<f32> {
    if width == 0 || height == 0 || !sigma.is_finite() || sigma <= 0.0 {
        return pixels.to_vec();
    }
    let Some(expected) = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
    else {
        return pixels.to_vec();
    };
    if pixels.len() < expected {
        return pixels.to_vec();
    }
    let k = -0.5 / (sigma * sigma);
    let r = radius.min(256) as i64;
    let w_usize = width as usize;
    let tap = |i: i64| (((i * i) as f64) * k).exp();
    let mut tmp = vec![0.0_f32; pixels.len()];
    // Horizontal.
    for y in 0..height as i64 {
        let row_base = (y as usize) * w_usize;
        for x in 0..width as i64 {
            let mut acc = [0.0_f64; 4];
            let mut wsum = 0.0;
            for d in -r..=r {
                let sx = (x + d).clamp(0, width as i64 - 1) as usize;
                let w = tap(d);
                let i = (row_base + sx) * 4;
                for c in 0..4 {
                    acc[c] += f64::from(pixels[i + c]) * w;
                }
                wsum += w;
            }
            let o = (row_base + x as usize) * 4;
            for c in 0..4 {
                tmp[o + c] = (acc[c] / wsum) as f32;
            }
        }
    }
    // Vertical.
    let mut out = vec![0.0_f32; pixels.len()];
    for y in 0..height as i64 {
        let row_base = (y as usize) * w_usize;
        for x in 0..width as i64 {
            let mut acc = [0.0_f64; 4];
            let mut wsum = 0.0;
            for d in -r..=r {
                let sy = (y + d).clamp(0, height as i64 - 1) as usize;
                let w = tap(d);
                let i = (sy * w_usize + x as usize) * 4;
                for c in 0..4 {
                    acc[c] += f64::from(tmp[i + c]) * w;
                }
                wsum += w;
            }
            let o = (row_base + x as usize) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / wsum) as f32;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_keeps_the_kernel_small() {
        assert_eq!(plan(3.0, 10), (0, 3.0));
        assert_eq!(plan(12.0, 10), (1, 6.0));
        let (level, sigma) = plan(100.0, 10);
        assert!(sigma <= MAX_WORKING_SIGMA && sigma > MAX_WORKING_SIGMA / 2.0);
        assert!((sigma * f64::from(1u32 << level) - 100.0).abs() < 1e-9);
        assert_eq!(plan(100.0, 2).0, 1, "capped at the last mip level");
        assert_eq!(plan(f64::NAN, 4), (0, 0.0));
    }

    #[test]
    fn a_blur_below_the_threshold_means_no_blur() {
        assert_eq!(sigma_for(0.004, 512), None);
        assert_eq!(sigma_for(0.0, 512), None);
        assert_eq!(sigma_for(f64::NAN, 512), None);
        // 0.6 of a 512 px shorter side is 10.75 px of sigma.
        let sigma = sigma_for(0.6, 512).unwrap();
        assert!((sigma - 0.6 * 0.035 * 512.0).abs() < 1e-9);
        // Quantised to 1/100, so 0.604 behaves like 0.6.
        assert_eq!(sigma_for(0.604, 512), sigma_for(0.6, 512));
    }

    #[test]
    fn the_radius_is_three_sigma_and_never_zero() {
        assert_eq!(radius_for(0.3), 1);
        assert_eq!(radius_for(1.0), 3);
        assert_eq!(radius_for(6.0), 18);
    }

    #[test]
    fn the_cpu_blur_averages_neighbours_and_clamps_at_the_edges() {
        // A single bright column in the middle of a black strip.
        let (w, h) = (9u32, 1u32);
        let mut px = vec![0.0_f32; (w * h * 4) as usize];
        for c in 0..4 {
            px[(4 * 4 + c) as usize] = 1.0;
        }
        let out = blur_linear_rgba(&px, w, h, 1.0, 3);
        let at = |x: usize| out[x * 4];
        // One lonely pixel can only keep about 40% of its energy (the kernel is normalised), but it
        // stays the brightest pixel and the light spreads symmetrically.
        assert!(at(4) > 0.35 && at(4) < 0.45, "centre {}", at(4));
        assert!(at(3) > 0.2 && at(3) < at(4));
        assert!((at(3) - at(5)).abs() < 1e-6, "symmetric");
        assert!(at(0) < 0.01, "far pixels stay black");
        let total: f32 = (0..9).map(at).sum();
        assert!((total - 1.0).abs() < 1e-3, "energy is conserved: {total}");
        // Edge pixels are not dark: a flat image stays flat.
        let flat = vec![0.5_f32; (16 * 4 * 4) as usize];
        let blurred = blur_linear_rgba(&flat, 16, 4, 2.0, 6);
        assert!(blurred.iter().all(|v| (*v - 0.5).abs() < 1e-6));
    }
}
