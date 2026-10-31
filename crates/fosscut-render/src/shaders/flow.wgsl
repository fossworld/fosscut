// Optical flow between two source frames and the passes that use it (ADR-0035).
//
// Flow is estimated coarse to fine over the frames' own mip levels with dense iterative
// Lucas-Kanade (B. D. Lucas and T. Kanade, "An Iterative Image Registration Technique with
// an Application to Stereo Vision", 1981, in the pyramidal form of J.-Y. Bouguet,
// "Pyramidal Implementation of the Lucas Kanade Feature Tracker", 2001): at each level the
// flow of the level above is refined by a few Gauss-Newton steps over a Gaussian window,
// with errors down-weighted robustly (a Cauchy weight) so occluded pixels pull less, then
// smoothed by a joint bilateral filter guided by the picture. Both directions are refined
// together: a flow texture holds the first frame's flow to the second in rg and the second
// frame's flow to the first in ba, both in texture coordinates, so it is valid at every
// level and resolution.
//
// In-between frames find, for each output pixel, the point of each frame that moves onto it
// (a fixed-point search from several candidate motions) and mix the two by how consistent
// their flows are, so what one frame hides comes from the other (forward-backward
// consistency, as in S. Baker et al., "A Database and Evaluation Methodology for Optical
// Flow", 2011). Motion blur gathers along the fastest motion near each pixel, weighting each
// sample by whether its own motion reaches the pixel, after M. McGuire et al., "A
// Reconstruction Filter for Plausible Motion Blur", 2012, without depth. Temporal noise
// reduction averages the frame with its neighbours warped along the flow, leaving out
// pixels whose flow is inconsistent or whose colour differs.

struct FlowParams {
    p: array<vec4f, 4>,
}

@group(0) @binding(0) var tex_a: texture_2d<f32>;
@group(0) @binding(1) var tex_b: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;
@group(0) @binding(3) var flow_a: texture_2d<f32>;
@group(0) @binding(4) var flow_b: texture_2d<f32>;
@group(0) @binding(5) var<uniform> params: FlowParams;
@group(0) @binding(6) var tex_c: texture_2d<f32>;
@group(0) @binding(7) var flow_c: texture_2d<f32>;

const FLOW_LUMA = vec3f(0.2126, 0.7152, 0.0722);

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    return full_screen_position(index);
}

// A flow texture (unfilterable 32-bit float) sampled bilinearly at `uv`, clamped to its edges.
fn flow_sample(t: texture_2d<f32>, uv: vec2f) -> vec4f {
    let dims = vec2i(textureDimensions(t));
    let x = uv * vec2f(dims) - vec2f(0.5);
    let base = floor(x);
    let f = x - base;
    let i = vec2i(base);
    let top = dims - vec2i(1);
    let a = textureLoad(t, clamp(i, vec2i(0), top), 0);
    let b = textureLoad(t, clamp(i + vec2i(1, 0), vec2i(0), top), 0);
    let c = textureLoad(t, clamp(i + vec2i(0, 1), vec2i(0), top), 0);
    let d = textureLoad(t, clamp(i + vec2i(1, 1), vec2i(0), top), 0);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn flow_load(t: texture_2d<f32>, uv: vec2f) -> vec4f {
    let dims = vec2i(textureDimensions(t));
    let i = clamp(vec2i(floor(uv * vec2f(dims))), vec2i(0), dims - vec2i(1));
    return textureLoad(t, i, 0);
}

// ---- Preparing a level: smoothed luma and its gradient ----

// Perceptual luma (sRGB-encoded luminance) of a premultiplied linear texel.
fn luma_of(c: vec4f) -> f32 {
    return encode_transfer(vec3f(dot(c.rgb, FLOW_LUMA)), 1u).x;
}

fn luma_at(t: texture_2d<f32>, q: vec2i, lod: i32) -> f32 {
    let dims = vec2i(textureDimensions(t, lod));
    return luma_of(textureLoad(t, clamp(q, vec2i(0), dims - vec2i(1)), lod));
}

// A chroma signal of a premultiplied linear texel: sRGB-encoded red minus blue, which tells
// apart colours of the same luma.
fn chroma_of(c: vec4f) -> f32 {
    let e = encode_transfer(c.rgb, 1u);
    return e.r - e.b;
}

// A 3x3 binomial blur of the luma, its Sobel gradient (per pixel), and the chroma, at mip
// level `lod`.
fn prepared(t: texture_2d<f32>, p: vec2i, lod: i32) -> vec4f {
    let l00 = luma_at(t, p + vec2i(-1, -1), lod);
    let l10 = luma_at(t, p + vec2i(0, -1), lod);
    let l20 = luma_at(t, p + vec2i(1, -1), lod);
    let l01 = luma_at(t, p + vec2i(-1, 0), lod);
    let l11 = luma_at(t, p, lod);
    let l21 = luma_at(t, p + vec2i(1, 0), lod);
    let l02 = luma_at(t, p + vec2i(-1, 1), lod);
    let l12 = luma_at(t, p + vec2i(0, 1), lod);
    let l22 = luma_at(t, p + vec2i(1, 1), lod);
    let s = (l00 + 2.0 * l10 + l20 + 2.0 * l01 + 4.0 * l11 + 2.0 * l21 + l02 + 2.0 * l12 + l22) / 16.0;
    let gx = ((l20 + 2.0 * l21 + l22) - (l00 + 2.0 * l01 + l02)) / 8.0;
    let gy = ((l02 + 2.0 * l12 + l22) - (l00 + 2.0 * l10 + l20)) / 8.0;
    let dims = vec2i(textureDimensions(t, lod));
    let chroma = chroma_of(textureLoad(t, clamp(p, vec2i(0), dims - vec2i(1)), lod));
    return vec4f(s, gx, gy, chroma);
}

// How much a window pixel `q` (prepared) belongs with the centre `c`: pixels of another
// colour likely move differently, so they count less (adaptive support weights, as in
// J. Xiao et al., "Bilateral Filtering-Based Optical Flow Estimation with Occlusion
// Detection", 2006).
fn support(q: vec4f, c: vec4f, sigma: f32) -> f32 {
    let dl = q.x - c.x;
    let dc = q.w - c.w;
    return exp(-(dl * dl + dc * dc) / (2.0 * sigma * sigma));
}

struct Prepared {
    @location(0) a: vec4f,
    @location(1) b: vec4f,
}

// Both frames at mip level p[0].x.
@fragment
fn fs_prep(@builtin(position) position: vec4f) -> Prepared {
    let lod = i32(params.p[0].x + 0.5);
    let p = vec2i(position.xy);
    var out: Prepared;
    out.a = prepared(tex_a, p, lod);
    out.b = prepared(tex_b, p, lod);
    return out;
}

// ---- Refining the flow ----

// The coarser level's flow (flow_a) at this level, or no motion when p[0].x is 0.
@fragment
fn fs_upsample(@builtin(position) position: vec4f) -> @location(0) vec4f {
    if params.p[0].x < 0.5 {
        return vec4f(0.0);
    }
    let size = vec2f(textureDimensions(tex_a));
    return flow_sample(flow_a, position.xy / size);
}

// Solves (G + lambda I) x = b for a symmetric G = [g.x g.y; g.y g.z].
fn solve(g: vec3f, b: vec2f, lambda: f32) -> vec2f {
    let a = g.x + lambda;
    let c = g.z + lambda;
    let det = max(a * c - g.y * g.y, 1e-12);
    return vec2f(c * b.x - g.y * b.y, a * b.y - g.y * b.x) / det;
}

fn limit(step: vec2f, most: f32) -> vec2f {
    let l = length(step);
    return select(step, step * (most / l), l > most);
}

// One Gauss-Newton step of both flows (flow_a) over prepared frames tex_a and tex_b of this
// level. p[0]: window radius, window sigma, damping, robust error scale; p[1].x: colour
// sigma of the window's support weights.
@fragment
fn fs_lk(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = vec2f(textureDimensions(tex_a));
    let top = vec2i(size) - vec2i(1);
    let p = vec2i(position.xy);
    let f = textureLoad(flow_a, p, 0);
    let u = f.xy * size;
    let v = f.zw * size;
    let radius = i32(params.p[0].x + 0.5);
    let sigma = params.p[0].y;
    let lambda = params.p[0].z;
    let scale = params.p[0].w;
    let colour = params.p[1].x;
    let centre_a = textureLoad(tex_a, p, 0);
    let centre_b = textureLoad(tex_b, p, 0);
    var ga = vec3f(0.0);
    var ba = vec2f(0.0);
    var gb = vec3f(0.0);
    var bb = vec2f(0.0);
    var total = 0.0;
    for (var dy = -radius; dy <= radius; dy = dy + 1) {
        for (var dx = -radius; dx <= radius; dx = dx + 1) {
            let q = clamp(p + vec2i(dx, dy), vec2i(0), top);
            let w = exp(-f32(dx * dx + dy * dy) / (2.0 * sigma * sigma));
            total = total + w;
            let a = textureLoad(tex_a, q, 0);
            let b = textureLoad(tex_b, q, 0);
            let centre = vec2f(q) + vec2f(0.5);
            // The first frame's pixel against where its flow puts it in the second, and
            // the other way round.
            let in_b = textureSampleLevel(tex_b, smp, (centre + u) / size, 0.0).x;
            let in_a = textureSampleLevel(tex_a, smp, (centre + v) / size, 0.0).x;
            let ea = in_b - a.x;
            let eb = in_a - b.x;
            let ra = w * support(a, centre_a, colour) / (1.0 + (ea / scale) * (ea / scale));
            let rb = w * support(b, centre_b, colour) / (1.0 + (eb / scale) * (eb / scale));
            ga = ga + ra * vec3f(a.y * a.y, a.y * a.z, a.z * a.z);
            ba = ba + ra * ea * a.yz;
            gb = gb + rb * vec3f(b.y * b.y, b.y * b.z, b.z * b.z);
            bb = bb + rb * eb * b.yz;
        }
    }
    let norm = 1.0 / max(total, 1e-6);
    let du = limit(-solve(ga * norm, ba * norm, lambda), 2.0);
    let dv = limit(-solve(gb * norm, bb * norm, lambda), 2.0);
    return vec4f((u + du) / size, (v + dv) / size);
}

// Smooths both flows (flow_a) with a joint bilateral filter guided by each frame's luma and
// chroma (tex_a, tex_b), so motion edges follow picture edges. p[0]: radius, spatial sigma,
// colour sigma.
@fragment
fn fs_smooth(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = vec2i(textureDimensions(tex_a));
    let top = size - vec2i(1);
    let p = vec2i(position.xy);
    let radius = i32(params.p[0].x + 0.5);
    let sigma = params.p[0].y;
    let range = params.p[0].z;
    let la = textureLoad(tex_a, p, 0);
    let lb = textureLoad(tex_b, p, 0);
    var fa = vec2f(0.0);
    var fb = vec2f(0.0);
    var wa = 0.0;
    var wb = 0.0;
    for (var dy = -radius; dy <= radius; dy = dy + 1) {
        for (var dx = -radius; dx <= radius; dx = dx + 1) {
            let q = clamp(p + vec2i(dx, dy), vec2i(0), top);
            let s = exp(-f32(dx * dx + dy * dy) / (2.0 * sigma * sigma));
            let f = textureLoad(flow_a, q, 0);
            let xa = s * support(textureLoad(tex_a, q, 0), la, range);
            let xb = s * support(textureLoad(tex_b, q, 0), lb, range);
            fa = fa + xa * f.xy;
            fb = fb + xb * f.zw;
            wa = wa + xa;
            wb = wb + xb;
        }
    }
    return vec4f(fa / max(wa, 1e-6), fb / max(wb, 1e-6));
}

// ---- In-between frames ----

struct Correspondence {
    @location(0) offsets: vec4f,
    @location(1) weights: vec4f,
}

fn inside_uv(uv: vec2f) -> bool {
    return all(uv >= vec2f(0.0)) && all(uv <= vec2f(1.0));
}

// How much to trust a frame's point for a pixel: one that lands on the pixel counts; one
// whose flow is inconsistent shows content the other frame does not have (it is hidden
// there), so it counts much less, but still wins over a frame with no point landing there.
fn trust(miss: f32, odd: f32, tolerance: f32) -> f32 {
    let landing = exp(-(miss / tolerance) * (miss / tolerance));
    return landing * max(exp(-(odd / tolerance) * (odd / tolerance)), 0.05);
}

// For each pixel at fraction t (p[0].x) of the way from the first frame to the second, where
// in each frame the content shown there comes from (as offsets in texture coordinates) and
// how much to trust each (flow_a: rg first to second, ba second to first). Candidate
// motions come from the flows at the pixel and around it; for each frame the candidate whose
// point lands back on the pixel best, with the most consistent flow, wins. p[0].y: the
// error in flow pixels at which a frame's weight falls to a third; p[0].z and p[0].w: the
// near and far distances, in flow pixels, at which candidates are looked for.
@fragment
fn fs_correspond(@builtin(position) position: vec4f) -> Correspondence {
    let size = vec2f(textureDimensions(flow_a));
    let uv = position.xy / size;
    let t = clamp(params.p[0].x, 0.0, 1.0);
    let tolerance = max(params.p[0].y, 0.1);
    var best0 = 1e9;
    var best1 = 1e9;
    var d0 = vec2f(0.0);
    var d1 = vec2f(0.0);
    // How far each chosen point lands from the pixel, and how inconsistent its flow is.
    var miss_a = 1e3;
    var miss_b = 1e3;
    var odd_a = 1e3;
    var odd_b = 1e3;
    for (var i = 0; i < 9; i = i + 1) {
        // The pixel itself, then four directions at each of two distances.
        var o = vec2f(0.0);
        if i > 0 {
            let k = i - 1;
            let reach = select(params.p[0].w, params.p[0].z, k < 4);
            let axis = k % 4;
            o = select(select(select(vec2f(0.0, -1.0), vec2f(0.0, 1.0), axis == 1), vec2f(-1.0, 0.0), axis == 2), vec2f(1.0, 0.0), axis == 3) * reach;
        }
        let around = flow_sample(flow_a, uv + o / size);
        for (var j = 0; j < 2; j = j + 1) {
            // A motion from the first frame to the second: its own flow, or the reverse of
            // the second's.
            let motion = select(-around.zw, around.xy, j == 0);
            // The first frame: the point x0 with x0 + t * flow(x0) on the pixel.
            var x0 = uv - t * motion;
            var m0 = flow_sample(flow_a, x0).xy;
            x0 = uv - t * m0;
            m0 = flow_sample(flow_a, x0).xy;
            let miss0 = length((x0 + t * m0 - uv) * size);
            let back0 = flow_sample(flow_a, x0 + m0).zw;
            let odd0 = length((m0 + back0) * size);
            var e0 = miss0 + 0.5 * odd0;
            if !inside_uv(x0) {
                e0 = e0 + 1e3;
            }
            if e0 < best0 {
                best0 = e0;
                d0 = x0 - uv;
                miss_a = select(1e3, miss0, inside_uv(x0));
                odd_a = odd0;
            }
            // The second frame: the point x1 with x1 + (1 - t) * flow back(x1) on the pixel.
            var x1 = uv + (1.0 - t) * motion;
            var m1 = flow_sample(flow_a, x1).zw;
            x1 = uv - (1.0 - t) * m1;
            m1 = flow_sample(flow_a, x1).zw;
            let miss1 = length((x1 + (1.0 - t) * m1 - uv) * size);
            let back1 = flow_sample(flow_a, x1 + m1).xy;
            let odd1 = length((m1 + back1) * size);
            var e1 = miss1 + 0.5 * odd1;
            if !inside_uv(x1) {
                e1 = e1 + 1e3;
            }
            if e1 < best1 {
                best1 = e1;
                d1 = x1 - uv;
                miss_b = select(1e3, miss1, inside_uv(x1));
                odd_b = odd1;
            }
        }
    }
    let w0 = (1.0 - t) * trust(miss_a, odd_a, tolerance);
    let w1 = t * trust(miss_b, odd_b, tolerance);
    var out: Correspondence;
    out.offsets = vec4f(d0, d1);
    out.weights = vec4f(w0, w1, 0.0, 1.0);
    return out;
}

// The in-between frame at fraction t (p[0].x): the first frame (tex_a) and the second
// (tex_b) sampled where the correspondence (flow_a, offsets) says, mixed by its weights
// (tex_c). Where neither frame is trusted, both still mix by t.
@fragment
fn fs_interpolate(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = vec2f(textureDimensions(tex_a));
    let uv = position.xy / size;
    let t = clamp(params.p[0].x, 0.0, 1.0);
    let d = flow_sample(flow_a, uv);
    let w = textureSampleLevel(tex_c, smp, uv, 0.0);
    let a = textureSampleLevel(tex_a, smp, uv + d.xy, 0.0);
    let b = textureSampleLevel(tex_b, smp, uv + d.zw, 0.0);
    let w0 = max(w.x, 0.0) + (1.0 - t) * 1e-3;
    let w1 = max(w.y, 0.0) + t * 1e-3;
    return (w0 * a + w1 * b) / (w0 + w1);
}

// ---- Motion blur ----

// The motion of the picture at flow pixel q, as half the distance it travels while the
// shutter is open, in picture pixels: from the flow to the next frame (flow_a) and to the
// previous one (flow_b). p[0]: shutter half-length in source frames (per-frame motion times
// half the shutter), whether there is a next and a previous frame, and p[1].xy whether each
// flow is stored the other way round; p[1].zw: the picture's size in pixels.
fn velocity(q: vec2i) -> vec2f {
    let a = textureLoad(flow_a, q, 0);
    let b = textureLoad(flow_b, q, 0);
    let next = select(a.xy, a.zw, params.p[1].x > 0.5);
    let previous = select(b.xy, b.zw, params.p[1].y > 0.5);
    let has_next = params.p[0].y > 0.5;
    let has_previous = params.p[0].z > 0.5;
    var motion = vec2f(0.0);
    if has_next && has_previous {
        motion = 0.5 * (next - previous);
    } else if has_next {
        motion = next;
    } else if has_previous {
        motion = -previous;
    }
    return motion * params.p[0].x * params.p[1].zw;
}

fn velocity_at(uv: vec2f) -> vec2f {
    let dims = vec2i(textureDimensions(flow_a));
    return velocity(clamp(vec2i(floor(uv * vec2f(dims))), vec2i(0), dims - vec2i(1)));
}

// The fastest motion in each tile of p[2].x by p[2].x flow pixels, and its length in rg, b.
@fragment
fn fs_tiles(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let tile = i32(params.p[2].x + 0.5);
    let dims = vec2i(textureDimensions(flow_a));
    let origin = vec2i(position.xy) * tile;
    var fastest = vec2f(0.0);
    for (var y = 0; y < tile; y = y + 1) {
        for (var x = 0; x < tile; x = x + 1) {
            let q = min(origin + vec2i(x, y), dims - vec2i(1));
            let v = velocity(q);
            if dot(v, v) > dot(fastest, fastest) {
                fastest = v;
            }
        }
    }
    return vec4f(fastest, length(fastest), 1.0);
}

// An integer hash (PCG, as in effects.wgsl) of a pixel, 0 to 1: the same on every GPU.
fn pixel_hash(p: vec2i) -> f32 {
    var v = vec3u(bitcast<u32>(p.x), bitcast<u32>(p.y), 7919u) * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3u(16u);
    v.x += v.y * v.z;
    return f32(v.x >> 8u) / 16777216.0;
}

// Motion blur of the picture (tex_a): samples along the fastest motion around the pixel
// (the tiles in flow_c), each counted for the share of the shutter its own motion spends on
// this pixel; what moving content does not cover shows the pixel itself, or what its motion
// uncovers. p[2].y: samples; p[2].z: the most a pixel moves, in pixels.
@fragment
fn fs_motion_blur(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = vec2f(textureDimensions(tex_a));
    let pixel = position.xy;
    let uv = pixel / size;
    let centre = textureLoad(tex_a, vec2i(pixel), 0);
    let tiles = vec2i(textureDimensions(flow_c));
    let tile = clamp(vec2i(floor(uv * vec2f(tiles))), vec2i(0), tiles - vec2i(1));
    var fastest = vec2f(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let v = textureLoad(flow_c, clamp(tile + vec2i(x, y), vec2i(0), tiles - vec2i(1)), 0).xy;
            if dot(v, v) > dot(fastest, fastest) {
                fastest = v;
            }
        }
    }
    let most = params.p[2].z;
    fastest = limit(fastest, most);
    let reach = length(fastest);
    if reach < 0.5 {
        return centre;
    }
    let own = limit(velocity_at(uv), most);
    let own_reach = length(own);
    let count = max(i32(params.p[2].y + 0.5), 2);
    let spacing = 2.0 * reach / f32(count);
    let tolerance = max(1.0, spacing);
    let jitter = pixel_hash(vec2i(pixel)) - 0.5;
    var acc = vec4f(0.0);
    var cover = 0.0;
    var still = vec4f(0.0);
    var still_count = 0.0;
    for (var i = 0; i < count; i = i + 1) {
        let s = -1.0 + (2.0 * f32(i) + 1.0 + jitter) / f32(count);
        let q = pixel + s * fastest;
        let quv = q / size;
        let colour = textureSampleLevel(tex_a, smp, quv, 0.0);
        let vq = limit(velocity_at(quv), most);
        let lq = length(vq);
        if lq >= 0.5 {
            // Does q's own streak, centred on it, pass over this pixel?
            let r = pixel - q;
            let along = dot(r, vq) / (lq * lq);
            let off = length(r - along * vq);
            if abs(along) <= 1.0 && off <= tolerance {
                let w = spacing / (2.0 * lq);
                acc = acc + w * colour;
                cover = cover + w;
            }
        } else if own_reach >= 0.5 && abs(s) * reach <= own_reach {
            // Still content this pixel's own motion uncovers.
            still = still + colour;
            still_count = still_count + 1.0;
        }
    }
    if cover >= 1.0 {
        return acc / cover;
    }
    var rest = centre;
    if still_count > 0.0 {
        let uncovered = min(still_count * spacing / (2.0 * own_reach), 1.0);
        rest = mix(centre, still / still_count, uncovered);
    }
    return acc + (1.0 - cover) * rest;
}

// ---- Temporal noise reduction ----

fn display_of(c: vec4f) -> vec3f {
    return clamp(encode_transfer(c.rgb / max(c.a, 1e-6), 1u), vec3f(0.0), vec3f(1.0));
}

// The picture (tex_a) averaged with the previous frame (tex_b, flow flow_a) and the next one
// (tex_c, flow flow_b). p[0]: colour sigma, whether there is a previous and a next frame,
// flow tolerance in flow pixels; p[1].xy: whether each flow is stored the other way round.
@fragment
fn fs_denoise_temporal(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = vec2f(textureDimensions(tex_a));
    let uv = position.xy / size;
    let centre = textureLoad(tex_a, vec2i(position.xy), 0);
    let here = display_of(centre);
    let sigma = max(params.p[0].x, 1e-3);
    let tolerance = max(params.p[0].w, 0.1);
    var sum = centre;
    var total = 1.0;
    if params.p[0].y > 0.5 {
        let w = neighbour_weight(tex_b, flow_a, params.p[1].x > 0.5, uv, here, sigma, tolerance);
        sum = sum + w.x * neighbour_colour(tex_b, flow_a, params.p[1].x > 0.5, uv);
        total = total + w.x;
    }
    if params.p[0].z > 0.5 {
        let w = neighbour_weight(tex_c, flow_b, params.p[1].y > 0.5, uv, here, sigma, tolerance);
        sum = sum + w.x * neighbour_colour(tex_c, flow_b, params.p[1].y > 0.5, uv);
        total = total + w.x;
    }
    return sum / total;
}

fn neighbour_to(flow: texture_2d<f32>, swapped: bool, uv: vec2f) -> vec2f {
    let f = flow_sample(flow, uv);
    return select(f.xy, f.zw, swapped);
}

fn neighbour_colour(picture: texture_2d<f32>, flow: texture_2d<f32>, swapped: bool, uv: vec2f) -> vec4f {
    return textureSampleLevel(picture, smp, uv + neighbour_to(flow, swapped, uv), 0.0);
}

// The weight of a neighbour's warped pixel: its flow's forward-backward consistency and how
// close its colour is to this pixel's, 0 outside the picture.
fn neighbour_weight(
    picture: texture_2d<f32>,
    flow: texture_2d<f32>,
    swapped: bool,
    uv: vec2f,
    here: vec3f,
    sigma: f32,
    tolerance: f32,
) -> vec2f {
    let dims = vec2f(textureDimensions(flow));
    let to = neighbour_to(flow, swapped, uv);
    let q = uv + to;
    let g = flow_sample(flow, q);
    let back = select(g.zw, g.xy, swapped);
    let error = length((to + back) * dims) / tolerance;
    let colour = textureSampleLevel(picture, smp, q, 0.0);
    let d = length(display_of(colour) - here) / sigma;
    return vec2f(select(0.0, exp(-error * error - d * d), inside_uv(q)), 0.0);
}
