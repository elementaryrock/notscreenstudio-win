//! The uniform-slot table and the WGSL sources.
//!
//! Port of upstream `NSSRender/Shaders.swift`. The uniform block is a flat array of `vec4<f32>`
//! slots; the shader gets `const U_<NAME>: u32 = <index>;` for every slot, generated from the same
//! enum the CPU writer indexes with, so the two layouts cannot drift apart.
//!
//! MSL's `in.pos.xy` and WGSL's `@builtin(position).xy` are both framebuffer coordinates with the
//! origin at the top-left and pixel centres at `+0.5`, so the geometry translates unchanged.

/// Up to 8 gradient stops / mesh colour points.
pub const MAX_STOPS: usize = 8;
/// Up to 8 simultaneous click ripples.
pub const MAX_RIPPLES: usize = 8;

/// One `vec4<f32>` of uniform data. The order is load-bearing: it is the shader's index space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Slot {
    // W, H, ditherEnabled, 0
    Out = 0,
    // x, y, w, h (output px)
    Content,
    // cornerRadius px, camera scale, camera centerX, camera centerY
    Content2,
    // duv.x, duv.y per output px, taps per axis, 0
    ScreenSs,
    // sigma px, opacity, offsetX px, offsetY px
    Shadow,
    // kind (0 solid, 1 linear gradient, 2 image, 3 mesh), count, 0, 0
    Bg,
    // dir.x, dir.y, 1 / length, 0
    BgGrad,
    // uv scale x, uv scale y, lod, 0
    BgImage,
    // linear rgb, alpha
    Solid,
    Stop0,
    Stop1,
    Stop2,
    Stop3,
    Stop4,
    Stop5,
    Stop6,
    Stop7,
    // location (gradient) or position xy (mesh)
    Loc0,
    Loc1,
    Loc2,
    Loc3,
    Loc4,
    Loc5,
    Loc6,
    Loc7,
    // hotspot x, hotspot y (px), px per cursor point (incl. press squish), alpha
    Cursor,
    // blur x, blur y (px), samples, atlas mip level
    CursorBlur,
    // minX, minY, maxX, maxY (px)
    CursorBox,
    // atlas uv of the cell's image area: x, y, w, h
    CursorCell,
    // hotspot x, y (pt from the image's top-left), image w, h (pt)
    CursorCell2,
    // count, thickness px, 0, 0
    RippleParams,
    Ripple0,
    Ripple1,
    Ripple2,
    Ripple3,
    Ripple4,
    Ripple5,
    Ripple6,
    Ripple7,
    // centre x, centre y, half w, half h (px)
    Bubble,
    // 0, corner radius px, border px, visible
    Bubble2,
    // uv scale x, uv scale y, mirror, taps per axis
    BubbleUv,
    // duv.x, duv.y per output px, lod, 0
    BubbleSs,
    // linear rgb, alpha
    BorderColor,
    // sigma px, opacity, offsetX px, offsetY px
    BubbleShadow,
    // mesh warp, 0, 0, 0
    Wallpaper,
}

impl Slot {
    /// Every slot, in index order.
    pub const ALL: [Slot; 46] = [
        Slot::Out,
        Slot::Content,
        Slot::Content2,
        Slot::ScreenSs,
        Slot::Shadow,
        Slot::Bg,
        Slot::BgGrad,
        Slot::BgImage,
        Slot::Solid,
        Slot::Stop0,
        Slot::Stop1,
        Slot::Stop2,
        Slot::Stop3,
        Slot::Stop4,
        Slot::Stop5,
        Slot::Stop6,
        Slot::Stop7,
        Slot::Loc0,
        Slot::Loc1,
        Slot::Loc2,
        Slot::Loc3,
        Slot::Loc4,
        Slot::Loc5,
        Slot::Loc6,
        Slot::Loc7,
        Slot::Cursor,
        Slot::CursorBlur,
        Slot::CursorBox,
        Slot::CursorCell,
        Slot::CursorCell2,
        Slot::RippleParams,
        Slot::Ripple0,
        Slot::Ripple1,
        Slot::Ripple2,
        Slot::Ripple3,
        Slot::Ripple4,
        Slot::Ripple5,
        Slot::Ripple6,
        Slot::Ripple7,
        Slot::Bubble,
        Slot::Bubble2,
        Slot::BubbleUv,
        Slot::BubbleSs,
        Slot::BorderColor,
        Slot::BubbleShadow,
        Slot::Wallpaper,
    ];

    /// Number of slots (the uniform block is `COUNT * 16` bytes).
    pub const COUNT: usize = Slot::ALL.len();

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The `#define`-style name the shader sees (`U_CONTENT2`, `U_BGGRAD`, ...).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Slot::Out => "OUT",
            Slot::Content => "CONTENT",
            Slot::Content2 => "CONTENT2",
            Slot::ScreenSs => "SCREENSS",
            Slot::Shadow => "SHADOW",
            Slot::Bg => "BG",
            Slot::BgGrad => "BGGRAD",
            Slot::BgImage => "BGIMAGE",
            Slot::Solid => "SOLID",
            Slot::Stop0 => "STOP0",
            Slot::Stop1 => "STOP1",
            Slot::Stop2 => "STOP2",
            Slot::Stop3 => "STOP3",
            Slot::Stop4 => "STOP4",
            Slot::Stop5 => "STOP5",
            Slot::Stop6 => "STOP6",
            Slot::Stop7 => "STOP7",
            Slot::Loc0 => "LOC0",
            Slot::Loc1 => "LOC1",
            Slot::Loc2 => "LOC2",
            Slot::Loc3 => "LOC3",
            Slot::Loc4 => "LOC4",
            Slot::Loc5 => "LOC5",
            Slot::Loc6 => "LOC6",
            Slot::Loc7 => "LOC7",
            Slot::Cursor => "CURSOR",
            Slot::CursorBlur => "CURSORBLUR",
            Slot::CursorBox => "CURSORBOX",
            Slot::CursorCell => "CURSORCELL",
            Slot::CursorCell2 => "CURSORCELL2",
            Slot::RippleParams => "RIPPLEPARAMS",
            Slot::Ripple0 => "RIPPLE0",
            Slot::Ripple1 => "RIPPLE1",
            Slot::Ripple2 => "RIPPLE2",
            Slot::Ripple3 => "RIPPLE3",
            Slot::Ripple4 => "RIPPLE4",
            Slot::Ripple5 => "RIPPLE5",
            Slot::Ripple6 => "RIPPLE6",
            Slot::Ripple7 => "RIPPLE7",
            Slot::Bubble => "BUBBLE",
            Slot::Bubble2 => "BUBBLE2",
            Slot::BubbleUv => "BUBBLEUV",
            Slot::BubbleSs => "BUBBLESS",
            Slot::BorderColor => "BORDERCOLOR",
            Slot::BubbleShadow => "BUBBLESHADOW",
            Slot::Wallpaper => "WALLPAPER",
        }
    }
}

/// The constants prepended to every WGSL source.
#[must_use]
pub fn defines() -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(2 * 1024);
    for slot in Slot::ALL {
        let _ = writeln!(s, "const U_{}: u32 = {}u;", slot.name(), slot.index());
    }
    let _ = writeln!(
        s,
        "const SLOT_COUNT: u32 = {SLOT_COUNT}u;",
        SLOT_COUNT = Slot::COUNT
    );
    let _ = writeln!(s, "const MAX_STOPS: u32 = {MAX_STOPS}u;");
    let _ = writeln!(s, "const MAX_RIPPLES: u32 = {MAX_RIPPLES}u;");
    s
}

/// The full WGSL source of the compositor pass, with the slot defines prepended.
#[must_use]
pub fn shader_source() -> String {
    let mut s = defines();
    s.push_str(include_str!("shader.wgsl"));
    s
}

/// The WGSL source of the separable background-blur kernel.
#[must_use]
pub fn blur_source() -> String {
    let mut s = defines();
    s.push_str(include_str!("blur.wgsl"));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_sequential_and_named_uniquely() {
        for (i, slot) in Slot::ALL.iter().enumerate() {
            assert_eq!(slot.index(), i, "{slot:?} is out of order");
        }
        let mut names: Vec<&str> = Slot::ALL.iter().map(|s| s.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate slot name");
    }

    #[test]
    fn the_shader_can_see_every_slot() {
        let src = shader_source();
        for slot in Slot::ALL {
            assert!(
                src.contains(&format!(
                    "const U_{}: u32 = {}u;",
                    slot.name(),
                    slot.index()
                )),
                "missing define for {slot:?}"
            );
        }
        assert!(src.contains(&format!("SLOT_COUNT: u32 = {}", Slot::COUNT)));
    }

    #[test]
    fn the_uniform_block_is_a_multiple_of_sixteen_bytes() {
        assert_eq!(
            Slot::COUNT * 16,
            std::mem::size_of::<[f32; 4]>() * Slot::COUNT
        );
        assert_eq!(
            Slot::COUNT % 4,
            2,
            "46 slots = 736 bytes, a valid uniform block size"
        );
    }

    #[test]
    fn the_shader_never_uses_implicit_derivative_sampling() {
        // `textureSample` is only legal in uniform control flow, and every sampling site here sits
        // inside a loop or a branch. Using `textureSampleLevel` everywhere also makes the preview
        // and the export agree bit for bit, which is the whole point of the port.
        let src = shader_source();
        assert!(
            !src.contains("textureSample("),
            "found an implicit-derivative sample"
        );
        assert!(src.contains("textureSampleLevel("));
    }
}
