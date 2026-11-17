//! sRGB / OKLab helpers used to prepare uniforms on the CPU.
//!
//! Port of upstream `NSSRender/Wallpapers.swift`'s `ColorMath`. These must stay bit-for-bit the same
//! expressions as the WGSL in `shader.wgsl`: the CPU rasterizer is the reference the GPU output is
//! checked against (see `tests/gpu.rs`), and the gradient/mesh thumbnails come out of the CPU path.

use nss_core::RGBAColor;

/// A linear-light RGB triple.
pub type LinearRgb = [f64; 3];

#[must_use]
pub fn srgb_to_linear(c: f64) -> f64 {
    let v = c.clamp(0.0, 1.0);
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

#[must_use]
pub fn linear_to_srgb(c: f64) -> f64 {
    let v = c.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB colour to linear-light RGB.
#[must_use]
pub fn linear(c: RGBAColor) -> LinearRgb {
    [
        srgb_to_linear(c.r),
        srgb_to_linear(c.g),
        srgb_to_linear(c.b),
    ]
}

/// sRGB 8-bit channel to linear light.
#[must_use]
pub fn byte_to_linear(b: u8) -> f64 {
    srgb_to_linear(f64::from(b) / 255.0)
}

/// Linear light to an sRGB 8-bit channel (rounded, like upstream's `byte()`).
#[must_use]
pub fn linear_to_byte(c: f64) -> u8 {
    let v = (linear_to_srgb(c) * 255.0).round();
    if v.is_nan() {
        0
    } else {
        v.clamp(0.0, 255.0) as u8
    }
}

/// OKLab from linear-light RGB.
#[must_use]
pub fn oklab_from_linear(lin: LinearRgb) -> LinearRgb {
    let l = 0.412_221_470_8 * lin[0] + 0.536_332_536_3 * lin[1] + 0.051_445_992_9 * lin[2];
    let m = 0.211_903_498_2 * lin[0] + 0.680_699_545_1 * lin[1] + 0.107_396_956_6 * lin[2];
    let s = 0.088_302_461_9 * lin[0] + 0.281_718_837_6 * lin[1] + 0.629_978_700_5 * lin[2];
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    [
        0.210_454_255_3 * l_ + 0.793_617_785_0 * m_ - 0.004_072_046_8 * s_,
        1.977_998_495_1 * l_ - 2.428_592_205_0 * m_ + 0.450_593_709_9 * s_,
        0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766_0 * s_,
    ]
}

/// OKLab from an sRGB colour.
#[must_use]
pub fn oklab(c: RGBAColor) -> LinearRgb {
    oklab_from_linear(linear(c))
}

/// Linear-light RGB from OKLab (the shader's `oklabToLinear`).
#[must_use]
pub fn oklab_to_linear(lab: LinearRgb) -> LinearRgb {
    let l_ = lab[0] + 0.396_337_777_4 * lab[1] + 0.215_803_757_3 * lab[2];
    let m_ = lab[0] - 0.105_561_345_8 * lab[1] - 0.063_854_172_8 * lab[2];
    let s_ = lab[0] - 0.089_484_177_5 * lab[1] - 1.291_485_548_0 * lab[2];
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_and_linear_round_trip() {
        for i in 0..=255_u8 {
            let v = f64::from(i) / 255.0;
            let back = linear_to_srgb(srgb_to_linear(v));
            assert!((back - v).abs() < 2e-6, "{i}: {v} -> {back}");
        }
    }

    #[test]
    fn linear_endpoints_and_midpoint() {
        assert!(srgb_to_linear(0.0).abs() < 1e-12);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-12);
        // 50% sRGB is about 21% linear light, which is why every blend here happens in linear.
        assert!((srgb_to_linear(0.5) - 0.214_04).abs() < 1e-4);
        assert_eq!(linear_to_byte(0.0), 0);
        assert_eq!(linear_to_byte(1.0), 255);
    }

    #[test]
    fn oklab_round_trips_through_linear() {
        for c in [
            RGBAColor::WHITE,
            RGBAColor::BLACK,
            RGBAColor::new(0.2, 0.7, 0.4, 1.0),
        ] {
            let back = oklab_to_linear(oklab(c));
            let want = linear(c);
            for i in 0..3 {
                // Cube roots and the inverse matrices cost about 1e-7 in absolute terms; the
                // shader has the same wobble and it is far below one 8-bit step.
                assert!(
                    (back[i] - want[i]).abs() < 1e-6,
                    "{c:?} channel {i}: {back:?} vs {want:?}"
                );
            }
        }
    }

    #[test]
    fn white_maps_to_l_of_one_half_turn() {
        let lab = oklab(RGBAColor::WHITE);
        assert!((lab[0] - 1.0).abs() < 1e-4, "{lab:?}");
        assert!(lab[1].abs() < 1e-4 && lab[2].abs() < 1e-4, "{lab:?}");
    }
}
