// Effect passes (ADR-0011, items 1 and 4): each entry point applies one effect to a
// premultiplied linear-light texture and writes the result at the same size, except the
// two halves of the separable blur. Per-pixel effects work on the unpremultiplied color;
// tone and color operations use sRGB-encoded values, as people grade, while exposure,
// white balance and vignetting multiply linear light. Formulas are FossCut's own,
// documented in ADR-0011's implementation notes.

struct EffectParams {
    p: array<vec4f, 12>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
// The curves table (256 x 1: red, green, blue, master) or a second picture (sharpen, mix).
@group(0) @binding(2) var aux_texture: texture_2d<f32>;
@group(0) @binding(3) var lut_texture: texture_3d<f32>;
@group(0) @binding(4) var<uniform> effect: EffectParams;
// Coverage of an adjustment layer's masks over the canvas (fs_mix, when p[0].y is 1).
@group(0) @binding(5) var mask_texture: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    return full_screen_position(index);
}

const LUMA = vec3f(0.2126, 0.7152, 0.0722);

fn load(position: vec4f) -> vec4f {
    return textureLoad(input_texture, vec2i(position.xy), 0);
}

fn unpremultiply(texel: vec4f) -> vec3f {
    return texel.rgb / max(texel.a, 1e-6);
}

fn to_display(linear: vec3f) -> vec3f {
    return clamp(encode_transfer(linear, 1u), vec3f(0.0), vec3f(1.0));
}

fn from_display(display: vec3f) -> vec3f {
    return decode_transfer(clamp(display, vec3f(0.0), vec3f(1.0)), 1u);
}

fn finish(linear: vec3f, alpha: f32) -> vec4f {
    return vec4f(max(linear, vec3f(0.0)) * alpha, alpha);
}

// ---- Copy and mix ----

@fragment
fn fs_copy(@builtin(position) position: vec4f) -> @location(0) vec4f {
    return load(position);
}

// Mixes the original (aux) with the effected input by p[0].x, an adjustment layer's
// opacity, times its masks' coverage when p[0].y is 1.
@fragment
fn fs_mix(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let original = textureLoad(aux_texture, vec2i(position.xy), 0);
    var amount = clamp(effect.p[0].x, 0.0, 1.0);
    if effect.p[0].y > 0.5 {
        let uv = position.xy / vec2f(textureDimensions(input_texture));
        amount = amount * textureSampleLevel(mask_texture, linear_sampler, uv, 0.0).r;
    }
    return mix(original, load(position), amount);
}

// ---- color.adjust ----

@fragment
fn fs_adjust(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let p0 = effect.p[0];
    let p1 = effect.p[1];
    let p2 = effect.p[2];
    // Exposure and white balance multiply linear light.
    var c = unpremultiply(texel) * exp2(p0.x);
    let temperature = p0.y;
    let tint = p0.z;
    c = c * vec3f(
        exp2(0.3 * temperature + 0.15 * tint),
        exp2(-0.3 * tint),
        exp2(-0.3 * temperature + 0.15 * tint),
    );
    var v = to_display(c);
    // Whites and blacks move the white and black points.
    let white = 1.0 - 0.2 * p1.w;
    let black = -0.2 * p2.x;
    v = (v - vec3f(black)) / max(white - black, 1e-3);
    v = clamp(v, vec3f(0.0), vec3f(1.0));
    // Brightness bends the midtones; contrast pivots around mid gray.
    v = pow(v, vec3f(exp2(-p1.x)));
    v = vec3f(0.5) + (v - vec3f(0.5)) * (1.0 + p0.w);
    v = clamp(v, vec3f(0.0), vec3f(1.0));
    // Shadows and highlights act on the dark and bright parts, toward black or white.
    let l = dot(v, LUMA);
    let in_shadows = 1.0 - smoothstep(0.0, 0.6, l);
    let in_highlights = smoothstep(0.4, 1.0, l);
    let shadows = p1.z * in_shadows * 0.6;
    let highlights = p1.y * in_highlights * 0.6;
    v = v + select(v * shadows, (vec3f(1.0) - v) * shadows, shadows > 0.0);
    v = v + select(v * highlights, (vec3f(1.0) - v) * highlights, highlights > 0.0);
    // Saturation everywhere, vibrance mostly on muted colors.
    let luma = dot(v, LUMA);
    let chroma = max(v.r, max(v.g, v.b)) - min(v.r, min(v.g, v.b));
    let factor = 1.0 + p2.y + p2.z * (1.0 - clamp(chroma, 0.0, 1.0));
    v = vec3f(luma) + (v - vec3f(luma)) * max(factor, 0.0);
    return finish(from_display(v), texel.a);
}

// ---- color.curves ----

fn curve(value: f32, channel: u32) -> f32 {
    let x = clamp(value, 0.0, 1.0) * (255.0 / 256.0) + 0.5 / 256.0;
    let entry = textureSampleLevel(aux_texture, linear_sampler, vec2f(x, 0.5), 0.0);
    return entry[channel];
}

@fragment
fn fs_curves(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    var v = to_display(unpremultiply(texel));
    v = vec3f(curve(v.r, 3u), curve(v.g, 3u), curve(v.b, 3u));
    v = vec3f(curve(v.r, 0u), curve(v.g, 1u), curve(v.b, 2u));
    return finish(from_display(v), texel.a);
}

// ---- Hue, saturation and lightness ----

fn rgb_to_hsl(c: vec3f) -> vec3f {
    let high = max(c.r, max(c.g, c.b));
    let low = min(c.r, min(c.g, c.b));
    let l = (high + low) / 2.0;
    let d = high - low;
    if d < 1e-6 {
        return vec3f(0.0, 0.0, l);
    }
    let s = d / (1.0 - abs(2.0 * l - 1.0) + 1e-6);
    var h: f32;
    if high == c.r {
        h = (c.g - c.b) / d;
        if h < 0.0 {
            h = h + 6.0;
        }
    } else if high == c.g {
        h = (c.b - c.r) / d + 2.0;
    } else {
        h = (c.r - c.g) / d + 4.0;
    }
    return vec3f(h * 60.0, clamp(s, 0.0, 1.0), l);
}

fn hsl_to_rgb(hsl: vec3f) -> vec3f {
    let h = fract(hsl.x / 360.0) * 6.0;
    let c = (1.0 - abs(2.0 * hsl.z - 1.0)) * hsl.y;
    let x = c * (1.0 - abs(h % 2.0 - 1.0));
    var rgb: vec3f;
    if h < 1.0 {
        rgb = vec3f(c, x, 0.0);
    } else if h < 2.0 {
        rgb = vec3f(x, c, 0.0);
    } else if h < 3.0 {
        rgb = vec3f(0.0, c, x);
    } else if h < 4.0 {
        rgb = vec3f(0.0, x, c);
    } else if h < 5.0 {
        rgb = vec3f(x, 0.0, c);
    } else {
        rgb = vec3f(c, 0.0, x);
    }
    return rgb + vec3f(hsl.z - c / 2.0);
}


@fragment
fn fs_hsl(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    var hsl = rgb_to_hsl(to_display(unpremultiply(texel)));
    // Band centres in degrees: red, orange, yellow, green, aqua, blue, purple, magenta.
    var bands = array<f32, 9>(0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 300.0, 360.0);
    // Each hue lies between two band centres; their settings blend by distance, so the
    // weights always add up to one.
    var lower = 0u;
    for (var i = 0u; i < 8u; i = i + 1u) {
        if hsl.x >= bands[i] {
            lower = i;
        }
    }
    let t = (hsl.x - bands[lower]) / (bands[lower + 1u] - bands[lower]);
    let upper = (lower + 1u) % 8u;
    let settings = mix(effect.p[lower].xyz, effect.p[upper].xyz, t);
    // Grays carry no hue: the more colorful a pixel, the more the band applies.
    let weight = smoothstep(0.0, 0.15, hsl.y);
    hsl.x = hsl.x + settings.x * 30.0 * weight;
    hsl.y = clamp(hsl.y * (1.0 + settings.y * weight), 0.0, 1.0);
    let light = settings.z * weight * 0.5;
    hsl.z = clamp(hsl.z + select(hsl.z * light, (1.0 - hsl.z) * light, light > 0.0), 0.0, 1.0);
    return finish(from_display(hsl_to_rgb(hsl)), texel.a);
}

// ---- color.wheels ----

// A wheel position as a color push with no brightness of its own: the direction is the
// hue (0 is red, counterclockwise), the length the strength.
fn push(wheel: vec2f) -> vec3f {
    let strength = clamp(length(wheel), 0.0, 1.0);
    if strength < 1e-6 {
        return vec3f(0.0);
    }
    let hue = atan2(wheel.y, wheel.x) * 57.29578;
    let rgb = hsl_to_rgb(vec3f(hue, 1.0, 0.5));
    return (rgb - vec3f(dot(rgb, LUMA))) * strength;
}

@fragment
fn fs_wheels(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let lift = effect.p[0];
    let gamma = effect.p[1];
    let gain = effect.p[2];
    let offset = effect.p[3];
    var v = to_display(unpremultiply(texel));
    v = v + vec3f(offset.z * 0.25) + push(offset.xy) * 0.25;
    let gains = vec3f(1.0 + gain.z * 0.5) + push(gain.xy) * 0.5;
    let lifts = vec3f(lift.z * 0.25) + push(lift.xy) * 0.25;
    v = v * gains + lifts * (vec3f(1.0) - v);
    let exponent = exp2(-(vec3f(gamma.z * 0.7) + push(gamma.xy)));
    v = pow(clamp(v, vec3f(0.0), vec3f(1.0)), exponent);
    return finish(from_display(v), texel.a);
}

// ---- color.lut ----

@fragment
fn fs_lut(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let entries = effect.p[0].x;
    let intensity = clamp(effect.p[0].y, 0.0, 1.0);
    let protect = clamp(effect.p[0].z, 0.0, 1.0);
    let color = unpremultiply(texel);
    // LUTs expect the display-referred signal, as the composite's LUT does.
    let signal = clamp(encode_transfer(color, 0u), vec3f(0.0), vec3f(1.0));
    let coord = signal * ((entries - 1.0) / entries) + vec3f(0.5 / entries);
    let graded = decode_transfer(textureSampleLevel(lut_texture, linear_sampler, coord, 0.0).rgb, 0u);
    // Skin tones: orange hues of moderate saturation keep more of the original.
    let hsl = rgb_to_hsl(to_display(color));
    let skin = smoothstep(5.0, 18.0, hsl.x) * (1.0 - smoothstep(40.0, 55.0, hsl.x))
        * smoothstep(0.1, 0.25, hsl.y) * (1.0 - smoothstep(0.65, 0.85, hsl.y));
    let amount = intensity * (1.0 - protect * skin);
    return finish(mix(color, graded, amount), texel.a);
}

// ---- color.vignette ----

@fragment
fn fs_vignette(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let p = effect.p[0];
    let size = effect.p[1].xy;
    let amount = p.x;
    let roundness = p.z;
    var d = (position.xy / size - vec2f(0.5)) * 2.0;
    // 0 follows the picture's shape; 1 is a circle; negative values grow squarer.
    let aspect = size.x / max(size.y, 1.0);
    d.x = d.x * mix(1.0, aspect, clamp(roundness, 0.0, 1.0));
    let power = mix(2.0, 8.0, clamp(-roundness, 0.0, 1.0));
    let distance = pow(pow(abs(d.x), power) + pow(abs(d.y), power), 1.0 / power) / sqrt(2.0);
    let edge = smoothstep(p.y, p.y + p.w * 0.8 + 0.02, distance) * abs(amount);
    var c = unpremultiply(texel);
    if amount >= 0.0 {
        c = c * (1.0 - edge);
    } else {
        c = c + (vec3f(1.0) - c) * edge;
    }
    return finish(c, texel.a);
}

// ---- stylize.grain ----

fn hash(cell: vec2f, seed: f32) -> f32 {
    let p = fract(vec3f(cell, seed) * vec3f(0.1031, 0.1030, 0.0973));
    let q = p + dot(p, p.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

@fragment
fn fs_grain(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let p = effect.p[0];
    let cell = floor(position.xy / max(p.y * effect.p[1].x, 0.5));
    // Two uniform samples make a triangular distribution: softer, film-like grain.
    let noise = hash(cell, p.z) + hash(cell + vec2f(17.0, 31.0), p.z) - 1.0;
    var v = to_display(unpremultiply(texel));
    let midtones = 1.0 - pow(abs(2.0 * dot(v, LUMA) - 1.0), 2.0);
    v = v + vec3f(noise * p.x * 0.12 * (0.35 + 0.65 * midtones));
    return finish(from_display(v), texel.a);
}

// ---- key.chroma ----

fn cbcr(c: vec3f) -> vec2f {
    let y = dot(c, LUMA);
    return vec2f((c.b - y) / 1.8556, (c.r - y) / 1.5748);
}

// Removes the key color (p[1].rgb, sRGB-encoded): pixels whose chroma is within the
// similarity of the key's become transparent, softly over the smoothness, and the key's
// chroma is taken out of what stays by the spill amount. p[0]: similarity, smoothness,
// spill.
@fragment
fn fs_chroma(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let p = effect.p[0];
    let v = to_display(unpremultiply(texel));
    let key = cbcr(effect.p[1].rgb);
    var chroma = cbcr(v);
    let threshold = p.x * 0.5;
    let distance = length(chroma - key);
    let keep = smoothstep(threshold, threshold + p.y * 0.5 + 0.001, distance);
    let direction = key / max(length(key), 1e-6);
    let along = max(dot(chroma, direction), 0.0);
    chroma = chroma - direction * along * clamp(p.z, 0.0, 1.0);
    let y = dot(v, LUMA);
    let r = y + 1.5748 * chroma.y;
    let b = y + 1.8556 * chroma.x;
    let g = (y - 0.2126 * r - 0.0722 * b) / 0.7152;
    return finish(from_display(vec3f(r, g, b)), texel.a * keep);
}

// ---- Stylize and motion effects ----

fn input_size() -> vec2f {
    return vec2f(textureDimensions(input_texture));
}

fn sample_at(uv: vec2f) -> vec4f {
    return textureSampleLevel(input_texture, linear_sampler, uv, 0.0);
}

// An integer hash (PCG3D, Jarzynski and Olano, "Hash Functions for GPU Rendering", 2020).
// Integer arithmetic gives the same bits on every GPU, which sine-based hashes do not.
fn pcg3d(input: vec3u) -> vec3u {
    var v = input * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3u(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

// A value from 0 to 1 for three whole numbers.
fn noise3(a: f32, b: f32, c: f32) -> f32 {
    let v = vec3u(bitcast<u32>(i32(floor(a))), bitcast<u32>(i32(floor(b))), bitcast<u32>(i32(floor(c))));
    return f32(pcg3d(v).x >> 8u) / 16777216.0;
}

// Bands jump sideways and colors split; p[0]: amount, band height, frame seed.
@fragment
fn fs_glitch(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = effect.p[0];
    let size = input_size();
    let uv = position.xy / size;
    let step = floor(p.z / 3.0);
    let band = floor(uv.y / max(p.y, 0.001));
    let chance = noise3(band, step, 1.0);
    var shifted = uv;
    if chance > 1.0 - p.x * 0.4 {
        shifted.x = shifted.x + (noise3(band, step, 2.0) - 0.5) * p.x * 0.2;
    }
    let split = p.x * 0.012 * (noise3(0.0, step, 3.0) - 0.5);
    let r = sample_at(shifted + vec2f(split, 0.0));
    let g = sample_at(shifted);
    let b = sample_at(shifted - vec2f(split, 0.0));
    return vec4f(r.r, g.g, b.b, g.a);
}

// Red one way, blue the other; p[0]: distance as a fraction of the width, angle.
@fragment
fn fs_rgb_split(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = effect.p[0];
    let uv = position.xy / input_size();
    let offset = vec2f(cos(p.y), sin(p.y)) * p.x;
    let r = sample_at(uv + offset);
    let g = sample_at(uv);
    let b = sample_at(uv - offset);
    return vec4f(r.r, g.g, b.b, max(g.a, max(r.a, b.a)));
}

// Square blocks, each the average of its area (a mip level); p[0].x: block size in pixels.
@fragment
fn fs_pixelate(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let cell = max(effect.p[0].x, 1.0);
    let size = input_size();
    let centre = (floor(position.xy / cell) + 0.5) * cell;
    return textureSampleLevel(input_texture, linear_sampler, centre / size, log2(cell));
}

// Worn tape; p[0]: amount, frame seed.
@fragment
fn fs_vhs(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = effect.p[0];
    let size = input_size();
    let uv = position.xy / size;
    let line = floor(position.y);
    let wobble = (noise3(line, p.y, 4.0) - 0.5) * 0.004 * p.x;
    let bleed = 0.006 * p.x;
    let base = sample_at(uv + vec2f(wobble, 0.0));
    let r = sample_at(uv + vec2f(wobble + bleed, 0.0)).r;
    let b = sample_at(uv + vec2f(wobble - bleed, 0.0)).b;
    var v = to_display(unpremultiply(vec4f(r, base.g, b, base.a)));
    let scan = 1.0 - 0.15 * p.x * (0.5 + 0.5 * sin(position.y * 3.14159));
    let grain = (noise3(position.x, line, p.y + 5.0) - 0.5) * 0.1 * p.x;
    v = v * scan + vec3f(grain);
    return finish(from_display(v), base.a);
}

// Halves reflected; p[0].x: 0 horizontal, 1 vertical, 2 quad.
@fragment
fn fs_mirror(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let mode = effect.p[0].x;
    var uv = position.xy / input_size();
    if mode < 0.5 || mode > 1.5 {
        uv.x = 0.5 - abs(uv.x - 0.5);
    }
    if mode > 0.5 {
        uv.y = 0.5 - abs(uv.y - 0.5);
    }
    return sample_at(uv);
}

@fragment
fn fs_posterize(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let levels = max(effect.p[0].x, 2.0);
    let v = floor(to_display(unpremultiply(texel)) * levels) / (levels - 1.0);
    return finish(from_display(v), texel.a);
}

@fragment
fn fs_invert(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let v = to_display(unpremultiply(texel));
    return finish(from_display(mix(v, vec3f(1.0) - v, clamp(effect.p[0].x, 0.0, 1.0))), texel.a);
}

// A smooth random walk; p[0]: amount, speed, seconds.
@fragment
fn fs_shake(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = effect.p[0];
    let t = p.z * p.y;
    let k = floor(t);
    let f = smoothstep(0.0, 1.0, fract(t));
    let x = mix(noise3(k, 0.0, 6.0), noise3(k + 1.0, 0.0, 6.0), f) - 0.5;
    let y = mix(noise3(k, 0.0, 7.0), noise3(k + 1.0, 0.0, 7.0), f) - 0.5;
    let uv = position.xy / input_size() + vec2f(x, y) * 2.0 * p.x;
    return sample_at(uv);
}

// Streaks towards the middle; p[0].x: amount.
@fragment
fn fs_zoom_blur(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let amount = effect.p[0].x;
    let uv = position.xy / input_size();
    var sum = vec4f(0.0);
    for (var i = 0; i < 16; i = i + 1) {
        let scale = 1.0 - amount * 0.3 * f32(i) / 15.0;
        sum = sum + sample_at(vec2f(0.5) + (uv - vec2f(0.5)) * scale);
    }
    return sum / 16.0;
}

// White flashes on a beat; p[0]: amount, rate, seconds.
@fragment
fn fs_flash(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let p = effect.p[0];
    let beat = pow(max(cos(6.28318 * p.y * p.z), 0.0), 8.0) * p.x;
    let v = to_display(unpremultiply(texel));
    return finish(from_display(v + (vec3f(1.0) - v) * beat), texel.a);
}

// Hues cycle; p[0]: amount, cycles per second, seconds.
@fragment
fn fs_rainbow(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let p = effect.p[0];
    var hsl = rgb_to_hsl(to_display(unpremultiply(texel)));
    hsl.x = hsl.x + 360.0 * fract(p.y * p.z) * p.x + 60.0 * p.x * position.x / input_size().x;
    hsl.y = clamp(hsl.y + 0.3 * p.x, 0.0, 1.0);
    return finish(from_display(hsl_to_rgb(hsl)), texel.a);
}

// Zooms in and out on a beat; p[0]: amount, rate, seconds.
@fragment
fn fs_zoom_pulse(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = effect.p[0];
    let scale = 1.0 + p.x * (0.5 + 0.5 * sin(6.28318 * p.y * p.z));
    let uv = vec2f(0.5) + (position.xy / input_size() - vec2f(0.5)) / scale;
    return sample_at(uv);
}

// ---- Transitions: the outgoing clip (input) into the incoming clip (aux) ----

// Premultiplied `top` over `bottom`.
fn over(top: vec4f, bottom: vec4f) -> vec4f {
    return top + bottom * (1.0 - top.a);
}

fn inside(uv: vec2f) -> bool {
    return all(uv >= vec2f(0.0)) && all(uv <= vec2f(1.0));
}

fn outgoing_at(uv: vec2f, lod: f32) -> vec4f {
    if !inside(uv) {
        return vec4f(0.0);
    }
    return textureSampleLevel(input_texture, linear_sampler, uv, lod);
}

fn incoming_at(uv: vec2f, lod: f32) -> vec4f {
    if !inside(uv) {
        return vec4f(0.0);
    }
    return textureSampleLevel(aux_texture, linear_sampler, uv, lod);
}

fn turn(v: vec2f, degrees: f32, aspect: f32) -> vec2f {
    let a = radians(degrees);
    let d = (v - vec2f(0.5)) * vec2f(aspect, 1.0);
    let r = vec2f(d.x * cos(a) + d.y * sin(a), -d.x * sin(a) + d.y * cos(a));
    return r / vec2f(aspect, 1.0) + vec2f(0.5);
}

// p[0]: case (0 dissolve, 1 dip, 2 wipe, 3 slide, 4 push, 5 zoom, 6 blur, 7 iris, 8 clock,
// 9 spin), progress, softness, direction (0 left, 1 right, 2 up, 3 down); p[1]: dip color.
@fragment
fn fs_transition(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = effect.p[0];
    let kind = u32(p.x + 0.5);
    let t = clamp(p.y, 0.0, 1.0);
    let soft = max(p.z, 0.001);
    let size = input_size();
    let aspect = size.x / max(size.y, 1.0);
    let uv = position.xy / size;
    var d = vec2f(-1.0, 0.0);
    let way = u32(p.w + 0.5);
    if way == 1u {
        d = vec2f(1.0, 0.0);
    } else if way == 2u {
        d = vec2f(0.0, -1.0);
    } else if way == 3u {
        d = vec2f(0.0, 1.0);
    }
    let a = outgoing_at(uv, 0.0);
    let b = incoming_at(uv, 0.0);
    switch kind {
        case 1u: {
            let color = vec4f(decode_transfer(effect.p[1].rgb, 1u), 1.0);
            if t < 0.5 {
                return mix(a, color, t * 2.0);
            }
            return mix(color, b, t * 2.0 - 1.0);
        }
        case 2u: {
            // The edge travels along d; behind it the incoming clip shows.
            let q = 0.5 - dot(uv - vec2f(0.5), d);
            let edge = t * (1.0 + soft) - soft;
            return mix(a, b, 1.0 - smoothstep(edge, edge + soft, 1.0 - q));
        }
        case 3u: {
            return over(incoming_at(uv + (1.0 - t) * d, 0.0), a);
        }
        case 4u: {
            return over(incoming_at(uv + (1.0 - t) * d, 0.0), outgoing_at(uv - t * d, 0.0));
        }
        case 5u: {
            let zoomed = outgoing_at(vec2f(0.5) + (uv - vec2f(0.5)) / (1.0 + 2.0 * t), 0.0);
            return mix(zoomed, b, smoothstep(0.0, 1.0, t));
        }
        case 6u: {
            let lod = 6.0 * (1.0 - abs(2.0 * t - 1.0));
            return mix(outgoing_at(uv, lod), incoming_at(uv, lod), smoothstep(0.3, 0.7, t));
        }
        case 7u: {
            let distance = length((uv - vec2f(0.5)) * vec2f(aspect, 1.0)) / length(vec2f(aspect, 1.0) * 0.5);
            let edge = t * (1.0 + soft);
            return mix(a, b, 1.0 - smoothstep(edge - soft, edge, distance));
        }
        case 8u: {
            let v = uv - vec2f(0.5);
            let angle = fract(atan2(v.x * aspect, -v.y) / 6.28318 + 1.0);
            let edge = t * (1.0 + soft);
            return mix(a, b, 1.0 - smoothstep(edge - soft, edge, angle));
        }
        case 9u: {
            let out = outgoing_at(vec2f(0.5) + (turn(uv, 90.0 * t, aspect) - vec2f(0.5)) / max(1.0 - t, 0.01), 0.0);
            let into = incoming_at(vec2f(0.5) + (turn(uv, -90.0 * (1.0 - t), aspect) - vec2f(0.5)) / max(t, 0.01), 0.0);
            return mix(out, into, t);
        }
        default: {
            return mix(a, b, t);
        }
    }
}

// The input (premultiplied) over the canvas in aux, in linear light.
@fragment
fn fs_over(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = vec2i(position.xy);
    return over(textureLoad(input_texture, p, 0), textureLoad(aux_texture, p, 0));
}

// ---- blur.gaussian and the blur half of detail.sharpen ----

// One direction of a separable Gaussian blur on premultiplied color. p[0]: direction,
// sigma in sampled texels, level of detail; p[1].x: taps on each side; p[1].yz: output
// size; p[2].xy: one sampled texel in texture coordinates.
@fragment
fn fs_blur(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = effect.p[0];
    let taps = i32(effect.p[1].x);
    let uv = position.xy / effect.p[1].yz;
    let step = p.xy * effect.p[2].xy;
    let sigma = max(p.z, 1e-3);
    var sum = vec4f(0.0);
    var total = 0.0;
    for (var i = -taps; i <= taps; i = i + 1) {
        let w = exp(-0.5 * f32(i * i) / (sigma * sigma));
        sum = sum + w * textureSampleLevel(input_texture, linear_sampler, uv + step * f32(i), p.w);
        total = total + w;
    }
    return sum / total;
}

// ---- detail.sharpen: the picture plus the difference from its blur (aux) ----

@fragment
fn fs_sharpen(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    let blurred = textureLoad(aux_texture, vec2i(position.xy), 0);
    let amount = effect.p[0].x;
    let sharpened = texel + (texel - blurred) * amount;
    let alpha = clamp(sharpened.a, 0.0, 1.0);
    return vec4f(clamp(sharpened.rgb, vec3f(0.0), vec3f(alpha * 64.0)), alpha);
}

// ---- Subject effects (ADR-0033): key.subject and the body effects ----
//
// mask_texture holds the field prepared on the CPU at the matte's size (subject.rs): red the
// subject's or person's coverage, green the signed distance from the person's edge in picture
// heights (negative inside), blue a blurred coverage. It covers the whole picture and is
// sampled bilinearly, so a small matte gives smooth edges at any size. p[1].rgb is the
// effect's colour in linear light.

fn field_at(uv: vec2f) -> vec4f {
    return textureSampleLevel(mask_texture, linear_sampler, uv, 0.0);
}

// How much of the subject is at a pixel whose matte is `m`, for threshold `t`: the edge falls
// at the threshold; values around it keep the matte's own softness, and the band narrows
// towards either end so 0 keeps everything and 1 only solid subject.
fn subject_share(m: f32, threshold: f32) -> f32 {
    let t = clamp(threshold, 0.0, 1.0);
    let band = max(0.5 * min(t, 1.0 - t), 0.002);
    return smoothstep(t - band, t + band, m);
}

// key.subject: keeps the subject where its matte (feathered on the CPU) passes the threshold,
// or the background with invert, and puts nothing, a colour or the background blurred (aux)
// where the picture goes. p[0]: threshold, invert, background (0 none, 1 colour, 2 blur).
@fragment
fn fs_subject_key(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    let p = effect.p[0];
    var keep = subject_share(field_at(position.xy / input_size()).r, p.x);
    if p.y > 0.5 {
        keep = 1.0 - keep;
    }
    var behind = vec4f(0.0);
    if p.z > 1.5 {
        // The blurred picture without its subject, divided by its blurred alpha: the
        // background's own colours, fading out where no background is near.
        let blurred = textureLoad(aux_texture, vec2i(position.xy), 0);
        let colour = blurred.rgb / max(blurred.a, 1e-4);
        behind = vec4f(colour, 1.0) * smoothstep(0.0, 0.05, blurred.a);
    } else if p.z > 0.5 {
        behind = vec4f(effect.p[1].rgb, 1.0);
    }
    return texel * keep + behind * (1.0 - keep);
}

// The picture with its subject cut out, premultiplied, for key.subject's blurred background.
@fragment
fn fs_subject_hole(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let subject = subject_share(field_at(position.xy / input_size()).r, effect.p[0].x);
    return load(position) * (1.0 - subject);
}

// body.outline: a band of colour p[0].x picture heights wide around the person, softened
// over p[0].y of its width (at least a pixel), drawn behind the person.
@fragment
fn fs_body_outline(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    let size = input_size();
    let field = field_at(position.xy / size);
    let width = max(effect.p[0].x, 0.0);
    let soft = max(clamp(effect.p[0].y, 0.0, 1.0) * width, 1.0 / size.y);
    let band = (1.0 - smoothstep(width - soft, width + soft, field.g)) * step(1e-6, width);
    let outline = vec4f(effect.p[1].rgb, 1.0) * band;
    return mix(over(outline, texel), texel, field.r);
}

// body.glow: light of colour p[1] spreading from the person's blurred mask, p[0].x strong,
// added around (not over) the person.
@fragment
fn fs_body_glow(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    let field = field_at(position.xy / input_size());
    let glow = clamp(2.0 * field.b * max(effect.p[0].x, 0.0), 0.0, 1.0) * (1.0 - field.r);
    return vec4f(texel.rgb + effect.p[1].rgb * glow, texel.a + glow * (1.0 - texel.a));
}

// body.shadow: the person's blurred mask moved by p[0].xy (texture coordinates), p[0].z
// opaque, in colour p[1], drawn behind the person.
@fragment
fn fs_body_shadow(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    let uv = position.xy / input_size();
    let person = field_at(uv).r;
    let shadow_uv = uv - effect.p[0].xy;
    let shadow_amount = select(0.0, field_at(shadow_uv).b, inside(shadow_uv));
    let a = clamp(shadow_amount * effect.p[0].z, 0.0, 1.0);
    let shade = vec4f(effect.p[1].rgb * a, a);
    return mix(over(shade, texel), texel, person);
}

// body.sparkle: sparkles drawn on the CPU (aux, red) add light of colour p[1].
@fragment
fn fs_body_sparkle(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    let c = clamp(textureSampleLevel(aux_texture, linear_sampler, position.xy / input_size(), 0.0).r, 0.0, 1.0);
    return vec4f(texel.rgb + effect.p[1].rgb * c, texel.a + c * (1.0 - texel.a));
}

// ---- color.deflicker and detail.denoise (ADR-0035) ----

// Scales linear light by p[0].x: color.deflicker's gain toward the neighbours' brightness.
@fragment
fn fs_gain(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    return vec4f(texel.rgb * max(effect.p[0].x, 0.0), texel.a);
}

// Edge-preserving smoothing (a bilateral filter, C. Tomasi and R. Manduchi, "Bilateral
// Filtering for Gray and Color Images", 1998): each pixel becomes the average of its
// neighbours within p[0].z pixels, weighted by distance (sigma p[0].y pixels) and by how
// close their sRGB-encoded colour is (sigma p[0].x), so noise smooths and edges stay. At
// p[0].x 0 the picture is unchanged.
@fragment
fn fs_bilateral(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = effect.p[0];
    let centre = load(position);
    if p.x <= 0.0 {
        return centre;
    }
    let top = vec2i(textureDimensions(input_texture)) - vec2i(1);
    let here = to_display(unpremultiply(centre));
    let radius = i32(p.z + 0.5);
    let spatial = max(p.y, 0.1);
    var sum = vec4f(0.0);
    var total = 0.0;
    for (var dy = -radius; dy <= radius; dy = dy + 1) {
        for (var dx = -radius; dx <= radius; dx = dx + 1) {
            let q = clamp(vec2i(position.xy) + vec2i(dx, dy), vec2i(0), top);
            let texel = textureLoad(input_texture, q, 0);
            let d = to_display(unpremultiply(texel)) - here;
            let w = exp(-f32(dx * dx + dy * dy) / (2.0 * spatial * spatial) - dot(d, d) / (2.0 * p.x * p.x));
            sum = sum + w * texel;
            total = total + w;
        }
    }
    return sum / max(total, 1e-6);
}
