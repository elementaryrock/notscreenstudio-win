import Foundation

/// Uniform layout: a flat array of `float4` slots. The shader gets `#define U_<NAME> <index>` for each
/// case, so the Swift writer and the MSL reader can never disagree about offsets or padding.
enum Slot: Int, CaseIterable {
    case out            // W, H, ditherEnabled, 0
    case content        // x, y, w, h (output px)
    case content2       // cornerRadius px, camera scale, camera centerX, camera centerY
    case screenSS       // duv.x, duv.y per output px, taps per axis, 0
    case shadow         // sigma px, opacity, offsetX px, offsetY px
    case bg             // kind (0 solid, 1 linear gradient, 2 image, 3 mesh), count, 0, 0
    case bgGrad         // dir.x, dir.y, 1 / length, 0
    case bgImage        // uv scale x, uv scale y, 0, 0
    case solid          // linear rgb, alpha
    case stop0, stop1, stop2, stop3, stop4, stop5, stop6, stop7   // OKLab + alpha
    case loc0, loc1, loc2, loc3, loc4, loc5, loc6, loc7           // location (gradient) or position xy (wallpaper)
    case cursor         // hotspot x, hotspot y (px), px per cursor point (incl. press squish), alpha
    case cursorBlur     // blur x, blur y (px), samples, atlas mip level
    case cursorBox      // minX, minY, maxX, maxY (px)
    case cursorCell     // atlas uv of the cell's image area: x, y, w, h
    case cursorCell2    // hotspot x, y (pt from the image's top-left), image w, h (pt)
    case rippleParams   // count, thickness px, 0, 0
    case ripple0, ripple1, ripple2, ripple3, ripple4, ripple5, ripple6, ripple7  // x, y, radius, alpha
    case bubble         // centre x, centre y, half w, half h (px)
    case bubble2        // 0, corner radius px, border px, visible
    case bubbleUV       // uv scale x, uv scale y, mirror, taps per axis
    case bubbleSS       // duv.x, duv.y per output px, 0, 0
    case borderColor    // linear rgb, alpha
    case bubbleShadow   // sigma px, opacity, offsetX px, offsetY px
    case wallpaper      // mesh warp, 0, 0, 0

    static let maxStops = 8
    static let maxRipples = 8
    static var byteCount: Int { allCases.count * MemoryLayout<SIMD4<Float>>.stride }
}

enum ShaderSource {
    static var defines: String {
        Slot.allCases.map { "#define U_\(String(describing: $0).uppercased()) \($0.rawValue)" }
            .joined(separator: "\n")
    }

    static var source: String {
        """
        #include <metal_stdlib>
        using namespace metal;

        \(defines)
        #define MAX_STOPS \(Slot.maxStops)
        #define MAX_RIPPLES \(Slot.maxRipples)

        struct VOut { float4 pos [[position]]; };

        vertex VOut nss_vertex(uint vid [[vertex_id]]) {
            float2 p = float2(float((vid << 1) & 2), float(vid & 2));
            VOut o;
            o.pos = float4(p * 2.0 - 1.0, 0.0, 1.0);
            return o;
        }

        // ---------- colour ----------
        static float3 linearToSrgb(float3 c) {
            c = clamp(c, 0.0, 1.0);
            return select(1.055 * pow(c, 1.0 / 2.4) - 0.055, c * 12.92, c <= 0.0031308);
        }

        static float3 oklabToLinear(float3 lab) {
            float l_ = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
            float m_ = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
            float s_ = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
            float l = l_ * l_ * l_, m = m_ * m_ * m_, s = s_ * s_ * s_;
            return float3( 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
                          -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
                          -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s);
        }

        static float hash(uint2 p) {
            uint v = p.x * 1664525u + p.y * 1013904223u + 0x9E3779B9u;
            v ^= v >> 16; v *= 0x7feb352du; v ^= v >> 15; v *= 0x846ca68bu; v ^= v >> 16;
            return float(v) * (1.0 / 4294967295.0);
        }

        // ---------- shapes ----------
        static float sdRoundBox(float2 p, float2 b, float r) {
            float2 q = abs(p) - b + r;
            return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
        }

        static float coverage(float d) { return clamp(0.5 - d, 0.0, 1.0); }

        static float erfApprox(float x) {
            float s = sign(x), a = abs(x);
            float t = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
            t *= t;
            return s - s / (t * t);
        }

        // Gaussian-blurred coverage of a shape whose signed distance is `d` (analytic, single pass).
        static float softShadow(float d, float sigma) {
            if (sigma < 0.5) { return coverage(d); }
            return 0.5 - 0.5 * erfApprox(d / (sigma * 1.41421356));
        }

        // Footprint-aware fetch: `taps` x `taps` bilinear samples spread over the output pixel's
        // footprint (`duv` in uv units). taps == 1 is a plain bilinear fetch. Every tap is clamped to
        // the source (0...1), so nothing outside the texture is ever read.
        static float4 sampleFootprint(texture2d<float> t, sampler s, float2 uv, float2 duv, int taps) {
            if (taps <= 1) { return t.sample(s, clamp(uv, 0.0, 1.0)); }
            float4 acc = 0.0;
            float inv = 1.0 / float(taps);
            for (int y = 0; y < taps; y++) {
                for (int x = 0; x < taps; x++) {
                    float2 o = (float2(x, y) + 0.5) * inv - 0.5;
                    acc += t.sample(s, clamp(uv + o * duv, 0.0, 1.0));
                }
            }
            return acc * (inv * inv);
        }

        // ---------- background ----------
        static float3 background(float2 p, constant float4* u, texture2d<float> image) {
            float2 size = u[U_OUT].xy;
            int kind = int(u[U_BG].x);
            int count = int(u[U_BG].y);
            if (kind == 1 && count > 0) {
                float t = clamp(dot(p - 0.5 * size, u[U_BGGRAD].xy) * u[U_BGGRAD].z + 0.5, 0.0, 1.0);
                float4 c = u[U_STOP0];
                for (int i = 1; i < count; i++) {
                    float l0 = u[U_LOC0 + i - 1].x, l1 = u[U_LOC0 + i].x;
                    float f = t >= l0 ? clamp((t - l0) / max(l1 - l0, 1e-5), 0.0, 1.0) : 0.0;
                    c = mix(c, u[U_STOP0 + i], f);
                }
                return oklabToLinear(c.xyz) * c.w;
            }
            if (kind == 2) {
                constexpr sampler mip(filter::linear, mip_filter::linear, address::clamp_to_edge);
                float2 uv = (p / size - 0.5) * u[U_BGIMAGE].xy + 0.5;
                return image.sample(mip, uv).rgb;
            }
            if (kind == 3 && count > 0) {
                float2 uv = p / size;
                float warp = u[U_WALLPAPER].x;
                uv += warp * float2(sin(uv.y * 5.1 + 1.3), sin(uv.x * 4.3 + 0.7));
                float2 aspect = size / max(size.x, size.y);
                float3 acc = 0.0;
                float wsum = 0.0;
                for (int i = 0; i < count; i++) {
                    float2 d = (uv - u[U_LOC0 + i].xy) * aspect;
                    float w = 1.0 / pow(dot(d, d) + 0.015, 1.6);
                    acc += u[U_STOP0 + i].xyz * w;
                    wsum += w;
                }
                return oklabToLinear(acc / wsum);
            }
            return u[U_SOLID].rgb * u[U_SOLID].a;
        }

        // ---------- main pass ----------
        fragment float4 nss_fragment(VOut in [[stage_in]],
                                     constant float4* u [[buffer(0)]],
                                     texture2d<float> screen [[texture(0)]],
                                     texture2d<float> camera [[texture(1)]],
                                     texture2d<float> image [[texture(2)]],
                                     texture2d<float> cursorAtlas [[texture(3)]]) {
            constexpr sampler lin(filter::linear, address::clamp_to_edge);
            float2 p = in.pos.xy;
            float3 c = background(p, u, image);
            float bgWeight = 1.0;

            // Screen drop shadow (analytic blur of the rounded-rect SDF).
            float4 rect = u[U_CONTENT];
            float2 hs = 0.5 * rect.zw;
            float2 centre = rect.xy + hs;
            float radius = u[U_CONTENT2].x;
            float4 sh = u[U_SHADOW];
            if (sh.y > 0.0) {
                float ds = sdRoundBox(p - centre - sh.zw, hs, radius);
                c *= 1.0 - sh.y * softShadow(ds, sh.x);
            }

            // Screen content, zoomed by the camera, clipped by the anti-aliased rounded rect.
            float dc = sdRoundBox(p - centre, hs, radius);
            float cov = coverage(dc);
            if (cov > 0.0) {
                float scale = u[U_CONTENT2].y;
                float2 camCentre = u[U_CONTENT2].zw;
                float2 local = (p - rect.xy) / rect.zw;
                float2 uv = camCentre + (local - 0.5) / scale;
                float3 s = sampleFootprint(screen, lin, uv, u[U_SCREENSS].xy, int(u[U_SCREENSS].z)).rgb;

                // Click ripples: anti-aliased ring with a soft dark halo so it reads on light and dark UIs.
                int rc = int(u[U_RIPPLEPARAMS].x);
                float th = u[U_RIPPLEPARAMS].y;
                for (int i = 0; i < rc; i++) {
                    float4 r = u[U_RIPPLE0 + i];
                    float dist = length(p - r.xy);
                    float ring = abs(dist - r.z) - 0.5 * th;
                    float halo = (1.0 - smoothstep(0.0, 1.5 * th, ring)) * 0.35 * r.w;
                    s *= 1.0 - halo;
                    float fill = coverage(dist - r.z) * 0.12 * r.w;
                    float a = coverage(ring) * 0.9 * r.w;
                    s = mix(s, float3(1.0), fill);
                    s = mix(s, float3(1.0), a);
                }

                // Cursor: the system cursor image from the atlas (linear, premultiplied, shadow baked in),
                // hotspot on the cursor position. Motion blur averages N atlas samples along the blur vector.
                float4 cur = u[U_CURSOR];
                float4 box = u[U_CURSORBOX];
                if (cur.w > 0.0 && p.x >= box.x && p.y >= box.y && p.x <= box.z && p.y <= box.w) {
                    constexpr sampler atlasSampler(filter::linear, mip_filter::linear, address::clamp_to_zero);
                    float4 blur = u[U_CURSORBLUR];
                    float4 cell = u[U_CURSORCELL];
                    float4 img = u[U_CURSORCELL2];
                    int n = max(int(blur.z), 1);
                    float4 acc = 0.0;
                    for (int i = 0; i < n; i++) {
                        float t = (float(i) + 0.5) / float(n) - 0.5;
                        float2 q = (p - cur.xy - blur.xy * t) / cur.z + img.xy;  // points in the cursor image
                        // Outside the image (plus a margin inside the cell padding) is transparent;
                        // this also keeps long blur vectors from reaching neighbouring cells.
                        if (all(q > -2.0) && all(q < img.zw + 2.0)) {
                            acc += cursorAtlas.sample(atlasSampler, cell.xy + q / img.zw * cell.zw, level(blur.w));
                        }
                    }
                    acc *= cur.w / float(n);
                    s = acc.rgb + s * (1.0 - acc.a);
                }

                c = mix(c, s, cov);
                bgWeight = 1.0 - cov;
            }

            // Camera bubble with its own soft shadow, SDF mask, aspect-fill crop, mirror and border.
            float4 b2 = u[U_BUBBLE2];
            if (b2.w > 0.0) {
                float4 bb = u[U_BUBBLE];
                float2 q = p - bb.xy;
                float4 bs = u[U_BUBBLESHADOW];
                if (bs.y > 0.0) {
                    float dsh = sdRoundBox(q - bs.zw, bb.zw, b2.y);
                    c *= 1.0 - bs.y * softShadow(dsh, bs.x);
                }
                float d = sdRoundBox(q, bb.zw, b2.y);
                float bc = coverage(d);
                if (bc > 0.0) {
                    float4 buv = u[U_BUBBLEUV];
                    float2 local = q / (2.0 * bb.zw) + 0.5;
                    if (buv.z > 0.5) { local.x = 1.0 - local.x; }
                    float2 uv = (local - 0.5) * buv.xy + 0.5;
                    float3 cam = sampleFootprint(camera, lin, uv, u[U_BUBBLESS].xy, int(buv.w)).rgb;
                    float bw = b2.z;
                    if (bw > 0.0) {
                        float4 bcol = u[U_BORDERCOLOR];
                        float band = 1.0 - coverage(d + bw);
                        cam = mix(cam, bcol.rgb, band * bcol.a);
                    }
                    c = mix(c, cam, bc);
                    bgWeight *= 1.0 - bc;
                }
            }

            float3 outc = linearToSrgb(c);
            // Triangular dither (+-1 LSB) on background pixels only, to break up gradient banding.
            if (u[U_OUT].z > 0.0) {
                uint2 ip = uint2(p);
                float n = hash(ip) - hash(ip + uint2(7919u, 104729u));
                outc += n * (1.0 / 255.0) * bgWeight;
            }
            return float4(outc, 1.0);
        }
        """
    }
}
