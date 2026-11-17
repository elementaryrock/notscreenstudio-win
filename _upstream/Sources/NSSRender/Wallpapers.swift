import Foundation
import NSSCore

/// Legacy (schema v1) procedural wallpapers for `Background.wallpaper(name:)`. The colour points live in
/// `BackgroundPresets.legacyWallpaper(named:)` (NSSCore) and are drawn by the same shader path as `.mesh`.
public enum Wallpapers {
    /// Names accepted by `Background.wallpaper(name:)`. Unknown names fall back to the first one.
    public static let names: [String] = BackgroundPresets.legacyWallpaperNames
}

/// sRGB / OKLab helpers used to prepare uniforms on the CPU.
enum ColorMath {
    static func srgbToLinear(_ c: Double) -> Double {
        let v = max(0, min(1, c))
        return v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)
    }

    static func linear(_ c: RGBAColor) -> SIMD3<Double> {
        SIMD3(srgbToLinear(c.r), srgbToLinear(c.g), srgbToLinear(c.b))
    }

    static func linearToSrgb(_ c: Double) -> Double {
        let v = max(0, min(1, c))
        return v <= 0.0031308 ? v * 12.92 : 1.055 * pow(v, 1 / 2.4) - 0.055
    }

    static func oklabToLinear(_ lab: SIMD3<Double>) -> SIMD3<Double> {
        let l_ = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z
        let m_ = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z
        let s_ = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z
        let l = l_ * l_ * l_, m = m_ * m_ * m_, s = s_ * s_ * s_
        return SIMD3(
            4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
        )
    }

    static func oklab(_ c: RGBAColor) -> SIMD3<Double> {
        let lin = linear(c)
        let l = 0.4122214708 * lin.x + 0.5363325363 * lin.y + 0.0514459929 * lin.z
        let m = 0.2119034982 * lin.x + 0.6806995451 * lin.y + 0.1073969566 * lin.z
        let s = 0.0883024619 * lin.x + 0.2817188376 * lin.y + 0.6299787005 * lin.z
        let l_ = cbrt(l), m_ = cbrt(m), s_ = cbrt(s)
        return SIMD3(
            0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
            1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
            0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_
        )
    }
}
