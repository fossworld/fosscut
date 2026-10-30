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

// Mixes the original (aux) with the effected input by p[0].x: an adjustment layer's opacity.
@fragment
fn fs_mix(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let original = textureLoad(aux_texture, vec2i(position.xy), 0);
    return mix(original, load(position), clamp(effect.p[0].x, 0.0, 1.0));
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
