// The single-pass compositor. Translation of upstream `NSSRender/Shaders.swift` (MSL) to WGSL.
//
// The slot defines (`U_OUT`, `U_CONTENT`, ...) are prepended by `shaders.rs`; the indices come from
// the same enum the CPU uniform writer uses, so the two layouts cannot drift.
//
// Two deliberate translation choices, both documented in docs/ADR/0011-renderer-parity.md:
//   * every texture read is `textureSampleLevel`. `textureSample` needs uniform control flow, and
//     every sampling site here is inside a loop or a branch. Explicit LOD also removes the last
//     implementation-defined part of the pass, which is what makes preview == export byte-for-byte.
//   * the background image's mip level is computed on the CPU and passed in `U_BGIMAGE.z`, for the
//     same reason (it replaces the implicit derivative LOD of the MSL build).

struct Uniforms {
    slots: array<vec4<f32>, SLOT_COUNT>,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var atlasSamp: sampler;
@group(0) @binding(3) var screenTex: texture_2d<f32>;
@group(0) @binding(4) var cameraTex: texture_2d<f32>;
@group(0) @binding(5) var imageTex: texture_2d<f32>;
@group(0) @binding(6) var atlasTex: texture_2d<f32>;

// ---------- colour ----------

fn linearToSrgb(c: vec3<f32>) -> vec3<f32> {
    let v = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(v, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), v * 12.92, v <= vec3<f32>(0.0031308));
}

fn oklabToLinear(lab: vec3<f32>) -> vec3<f32> {
    let l_ = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
    let m_ = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
    let s_ = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    return vec3<f32>(
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    );
}

fn hash2(p: vec2<u32>) -> f32 {
    var v: u32 = p.x * 1664525u + p.y * 1013904223u + 0x9E3779B9u;
    v = v ^ (v >> 16u);
    v = v * 0x7feb352du;
    v = v ^ (v >> 15u);
    v = v * 0x846ca68bu;
    v = v ^ (v >> 16u);
    return f32(v) * (1.0 / 4294967295.0);
}

// ---------- shapes ----------

fn sdRoundBox(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + r;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn coverage(d: f32) -> f32 {
    return clamp(0.5 - d, 0.0, 1.0);
}

fn erfApprox(x: f32) -> f32 {
    let s = sign(x);
    let a = min(abs(x), 10.0);
    var t = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
    t = t * t;
    return s - s / (t * t);
}

// Gaussian-blurred coverage of a shape whose signed distance is `d` (analytic, single pass).
fn softShadow(d: f32, sigma: f32) -> f32 {
    if (sigma < 0.5) {
        return coverage(d);
    }
    return 0.5 - 0.5 * erfApprox(d / (sigma * 1.41421356));
}

// Footprint-aware fetch: `taps` x `taps` bilinear samples spread over the output pixel's footprint
// (`duv` in uv units). taps == 1 is a plain bilinear fetch. Every tap is clamped to the source
// (0...1), so nothing outside the texture is ever read.
fn sampleFootprint(t: texture_2d<f32>, s: sampler, uv: vec2<f32>, duv: vec2<f32>, taps: i32) -> vec4<f32> {
    if (taps <= 1) {
        return textureSampleLevel(t, s, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    }
    var acc = vec4<f32>(0.0);
    let n = i32(taps);
    let inv = 1.0 / f32(n);
    for (var y: i32 = 0; y < n; y = y + 1) {
        for (var x: i32 = 0; x < n; x = x + 1) {
            let o = (vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) * inv - vec2<f32>(0.5);
            acc = acc + textureSampleLevel(t, s, clamp(uv + o * duv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
        }
    }
    return acc * (inv * inv);
}

// ---------- background ----------

fn background(p: vec2<f32>) -> vec3<f32> {
    let size = u.slots[U_OUT].xy;
    let kind = i32(u.slots[U_BG].x);
    let count = u32(u.slots[U_BG].y);

    if (kind == 1 && count > 0u) {
        let t = clamp(dot(p - 0.5 * size, u.slots[U_BGGRAD].xy) * u.slots[U_BGGRAD].z + 0.5, 0.0, 1.0);
        var c = u.slots[U_STOP0];
        for (var i: u32 = 1u; i < count; i = i + 1u) {
            let l0 = u.slots[U_LOC0 + i - 1u].x;
            let l1 = u.slots[U_LOC0 + i].x;
            var f = 0.0;
            if (t >= l0) {
                f = clamp((t - l0) / max(l1 - l0, 1e-5), 0.0, 1.0);
            }
            c = mix(c, u.slots[U_STOP0 + i], f);
        }
        return oklabToLinear(c.xyz) * c.w;
    }

    if (kind == 2) {
        let uv = (p / size - vec2<f32>(0.5)) * u.slots[U_BGIMAGE].xy + vec2<f32>(0.5);
        return textureSampleLevel(imageTex, samp, uv, u.slots[U_BGIMAGE].z).rgb;
    }

    if (kind == 3 && count > 0u) {
        var uv = p / size;
        let warp = u.slots[U_WALLPAPER].x;
        uv = uv + warp * vec2<f32>(sin(uv.y * 5.1 + 1.3), sin(uv.x * 4.3 + 0.7));
        let aspect = size / max(size.x, size.y);
        var acc = vec3<f32>(0.0);
        var wsum = 0.0;
        for (var i: u32 = 0u; i < count; i = i + 1u) {
            let d = (uv - u.slots[U_LOC0 + i].xy) * aspect;
            let w = 1.0 / pow(dot(d, d) + 0.015, 1.6);
            acc = acc + u.slots[U_STOP0 + i].xyz * w;
            wsum = wsum + w;
        }
        return oklabToLinear(acc / wsum);
    }

    return u.slots[U_SOLID].rgb * u.slots[U_SOLID].a;
}

// ---------- main pass ----------

@vertex
fn nss_vertex(@builtin(vertex_index) vid: u32) -> VOut {
    let p = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var o: VOut;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

@fragment
fn nss_fragment(in: VOut) -> @location(0) vec4<f32> {
    let p = in.pos.xy;
    var c = background(p);
    var bgWeight = 1.0;

    // Screen drop shadow (analytic blur of the rounded-rect SDF).
    let rect = u.slots[U_CONTENT];
    let hs = 0.5 * rect.zw;
    let centre = rect.xy + hs;
    let radius = u.slots[U_CONTENT2].x;
    let sh = u.slots[U_SHADOW];
    if (sh.y > 0.0) {
        let ds = sdRoundBox(p - centre - sh.zw, hs, radius);
        c = c * (1.0 - sh.y * softShadow(ds, sh.x));
    }

    // Screen content, zoomed by the camera, clipped by the anti-aliased rounded rect.
    let dc = sdRoundBox(p - centre, hs, radius);
    let cov = coverage(dc);
    if (cov > 0.0) {
        let scale = u.slots[U_CONTENT2].y;
        let camCentre = u.slots[U_CONTENT2].zw;
        let local = (p - rect.xy) / rect.zw;
        let uv = camCentre + (local - vec2<f32>(0.5)) / scale;
        var s = sampleFootprint(screenTex, samp, uv, u.slots[U_SCREENSS].xy, i32(u.slots[U_SCREENSS].z)).rgb;

        // Click ripples: anti-aliased ring with a soft dark halo so it reads on light and dark UIs.
        let rc = i32(u.slots[U_RIPPLEPARAMS].x);
        let th = u.slots[U_RIPPLEPARAMS].y;
        for (var i: i32 = 0; i < rc; i = i + 1) {
            let r = u.slots[U_RIPPLE0 + u32(i)];
            let dist = length(p - r.xy);
            let ring = abs(dist - r.z) - 0.5 * th;
            let halo = (1.0 - smoothstep(0.0, 1.5 * th, ring)) * 0.35 * r.w;
            s = s * (1.0 - halo);
            let fill = coverage(dist - r.z) * 0.12 * r.w;
            let a = coverage(ring) * 0.9 * r.w;
            s = mix(s, vec3<f32>(1.0), vec3<f32>(fill));
            s = mix(s, vec3<f32>(1.0), vec3<f32>(a));
        }

        // Cursor: the system cursor image from the atlas (linear, premultiplied, shadow baked in),
        // hotspot on the cursor position. Motion blur averages N atlas samples along the blur vector.
        let cur = u.slots[U_CURSOR];
        let box = u.slots[U_CURSORBOX];
        if (cur.w > 0.0 && p.x >= box.x && p.y >= box.y && p.x <= box.z && p.y <= box.w) {
            let blur = u.slots[U_CURSORBLUR];
            let cell = u.slots[U_CURSORCELL];
            let img = u.slots[U_CURSORCELL2];
            let n = max(i32(blur.z), 1);
            var acc = vec4<f32>(0.0);
            for (var i: i32 = 0; i < n; i = i + 1) {
                let t = (f32(i) + 0.5) / f32(n) - 0.5;
                let q = (p - cur.xy - blur.xy * t) / cur.z + img.xy; // points in the cursor image
                // Outside the image (plus a margin inside the cell padding) is transparent; this
                // also keeps long blur vectors from reaching neighbouring cells.
                if (all(q > vec2<f32>(-2.0)) && all(q < img.zw + vec2<f32>(2.0))) {
                    acc = acc + textureSampleLevel(atlasTex, atlasSamp, cell.xy + q / img.zw * cell.zw, blur.w);
                }
            }
            acc = acc * (cur.w / f32(n));
            s = acc.rgb + s * (1.0 - acc.a);
        }

        c = mix(c, s, vec3<f32>(cov));
        bgWeight = 1.0 - cov;
    }

    // Camera bubble with its own soft shadow, SDF mask, aspect-fill crop, mirror and border.
    let b2 = u.slots[U_BUBBLE2];
    if (b2.w > 0.0) {
        let bb = u.slots[U_BUBBLE];
        let q = p - bb.xy;
        let bs = u.slots[U_BUBBLESHADOW];
        if (bs.y > 0.0) {
            let dsh = sdRoundBox(q - bs.zw, bb.zw, b2.y);
            c = c * (1.0 - bs.y * softShadow(dsh, bs.x));
        }
        let d = sdRoundBox(q, bb.zw, b2.y);
        let bc = coverage(d);
        if (bc > 0.0) {
            let buv = u.slots[U_BUBBLEUV];
            var local = q / (2.0 * bb.zw) + vec2<f32>(0.5);
            if (buv.z > 0.5) {
                local.x = 1.0 - local.x;
            }
            let uv = (local - vec2<f32>(0.5)) * buv.xy + vec2<f32>(0.5);
            var cam = sampleFootprint(cameraTex, samp, uv, u.slots[U_BUBBLESS].xy, i32(buv.w)).rgb;
            let bw = b2.z;
            if (bw > 0.0) {
                let bcol = u.slots[U_BORDERCOLOR];
                let band = 1.0 - coverage(d + bw);
                cam = mix(cam, bcol.rgb, vec3<f32>(band * bcol.a));
            }
            c = mix(c, cam, vec3<f32>(bc));
            bgWeight = bgWeight * (1.0 - bc);
        }
    }

    let outc = linearToSrgb(c);
    // Triangular dither (+-1 LSB) on background pixels only, to break up gradient banding.
    if (u.slots[U_OUT].z > 0.0) {
        let ip = vec2<u32>(p);
        let n = hash2(ip) - hash2(ip + vec2<u32>(7919u, 104729u));
        return vec4<f32>(outc + n * (1.0 / 255.0) * bgWeight, 1.0);
    }
    return vec4<f32>(outc, 1.0);
}
