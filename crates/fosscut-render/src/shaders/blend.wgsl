// Combines a layer drawn on its own with the canvas below it using a blend mode
// (ADR-0011, item 4). Both inputs are linear light; the formulas are the separable modes of
// the W3C Compositing and Blending specification, applied to sRGB-encoded values as
// designers expect, then decoded again. The canvas is opaque, so the result is
// (1 - alpha) * below + alpha * B(below, layer), with alpha the layer's coverage times its
// opacity and mask.

@group(0) @binding(0) var below_texture: texture_2d<f32>;
@group(0) @binding(1) var layer_texture: texture_2d<f32>;
// x: mode, numbered as BlendMode in plan order (0 normal, 1 multiply, ...).
@group(0) @binding(2) var<uniform> blend: vec4u;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    return full_screen_position(index);
}

fn screen(b: vec3f, s: vec3f) -> vec3f {
    return b + s - b * s;
}

fn hard_light(b: vec3f, s: vec3f) -> vec3f {
    return select(screen(b, 2.0 * s - 1.0), b * 2.0 * s, s <= vec3f(0.5));
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if b <= 0.0 {
        return 0.0;
    }
    if s >= 1.0 {
        return 1.0;
    }
    return min(1.0, b / (1.0 - s));
}

fn color_burn(b: f32, s: f32) -> f32 {
    if b >= 1.0 {
        return 1.0;
    }
    if s <= 0.0 {
        return 0.0;
    }
    return 1.0 - min(1.0, (1.0 - b) / s);
}

fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        return b - (1.0 - 2.0 * s) * b * (1.0 - b);
    }
    var d = sqrt(b);
    if b <= 0.25 {
        d = ((16.0 * b - 12.0) * b + 4.0) * b;
    }
    return b + (2.0 * s - 1.0) * (d - b);
}

fn mixed(mode: u32, b: vec3f, s: vec3f) -> vec3f {
    switch mode {
        case 1u: { return b * s; }
        case 2u: { return screen(b, s); }
        case 3u: { return hard_light(s, b); }
        case 4u: { return min(b, s); }
        case 5u: { return max(b, s); }
        case 6u: { return vec3f(color_dodge(b.r, s.r), color_dodge(b.g, s.g), color_dodge(b.b, s.b)); }
        case 7u: { return vec3f(color_burn(b.r, s.r), color_burn(b.g, s.g), color_burn(b.b, s.b)); }
        case 8u: { return hard_light(b, s); }
        case 9u: { return vec3f(soft_light(b.r, s.r), soft_light(b.g, s.g), soft_light(b.b, s.b)); }
        case 10u: { return max(b + s - 1.0, vec3f(0.0)); }
        case 11u: { return min(b + s, vec3f(1.0)); }
        case 12u: { return abs(b - s); }
        case 13u: { return b + s - 2.0 * b * s; }
        default: { return s; }
    }
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let p = vec2i(position.xy);
    let below = textureLoad(below_texture, p, 0);
    let layer = textureLoad(layer_texture, p, 0);
    if layer.a <= 0.0 {
        return below;
    }
    let b = clamp(encode_transfer(below.rgb, 1u), vec3f(0.0), vec3f(1.0));
    let s = clamp(encode_transfer(layer.rgb / layer.a, 1u), vec3f(0.0), vec3f(1.0));
    let a = clamp(layer.a, 0.0, 1.0);
    let result = mix(b, mixed(blend.x, b, s), a);
    return vec4f(decode_transfer(result, 1u), 1.0);
}
