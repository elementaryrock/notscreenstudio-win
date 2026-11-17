// Separable Gaussian blur of an image texture, in linear light.
// Translation of upstream `ImageBlur.source` (MSL compute) into two fullscreen render passes: one
// call per direction, with the working size and the source mip level passed in.

struct BlurParams {
    // dir.x, dir.y, sigma (working texels), radius (taps each side)
    params: vec4<f32>,
    // source mip level, working width, working height, unused
    geometry: vec4<u32>,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: BlurParams;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;

@vertex
fn nss_blur_vertex(@builtin(vertex_index) vid: u32) -> VOut {
    let q = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var o: VOut;
    o.pos = vec4<f32>(q * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

@fragment
fn nss_blur_fragment(in: VOut) -> @location(0) vec4<f32> {
    // Always the render target's size (the working size), never derived from the source texture: the
    // second pass reads a single-mip intermediate, so its level is 0 and its size is the working size
    // too.
    let size = vec2<f32>(f32(p.geometry.y), f32(p.geometry.z));
    let level = f32(p.geometry.x);
    let dir = p.params.xy;
    let sigma = max(p.params.z, 1e-4);
    let r = i32(p.params.w);
    let k = -0.5 / (sigma * sigma);

    let step = dir / size;
    let uv = in.pos.xy / size;

    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var i: i32 = -r; i <= r; i = i + 1) {
        let q = clamp(uv + step * f32(i), vec2<f32>(0.0), vec2<f32>(1.0));
        let wt = exp(f32(i * i) * k);
        acc = acc + textureSampleLevel(src, samp, q, level) * wt;
        wsum = wsum + wt;
    }
    return acc / max(wsum, 1e-6);
}
