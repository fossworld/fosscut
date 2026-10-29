// Draws one layer over the canvas in linear light with premultiplied alpha: LUT, mask and
// opacity (ADR-0008 item 2, ADR-0011 item 4). Layer textures are premultiplied: video is
// opaque, text carries its glyph coverage in alpha.

struct Layer {
    // x: opacity; y: mask (0 none, 1 rectangle, 2 ellipse); z: feather; w: invert (0 or 1).
    params: vec4f,
    // xy: mask centre; zw: mask size, as fractions of the layer.
    mask: vec4f,
    // x: LUT entries per axis, 0 for no LUT.
    lut: vec4f,
}

struct VertexOut {
    @builtin(position) position: vec4f,
    @location(0) local: vec2f,
    @location(1) uv: vec2f,
}

@group(0) @binding(0) var layer_texture: texture_2d<f32>;
@group(0) @binding(1) var layer_sampler: sampler;
@group(0) @binding(2) var lut_texture: texture_3d<f32>;
@group(0) @binding(3) var lut_sampler: sampler;
@group(0) @binding(4) var<uniform> layer: Layer;

@vertex
fn vs_main(
    @location(0) position: vec2f,
    @location(1) local: vec2f,
    @location(2) uv: vec2f,
) -> VertexOut {
    var out: VertexOut;
    out.position = vec4f(position, 0.0, 1.0);
    out.local = local;
    out.uv = uv;
    return out;
}

// Mask coverage: k is the distance from the centre in units of the mask's half size
// (per axis for rectangles, radial for ellipses), so 1 is the edge.
fn mask_alpha(local: vec2f) -> f32 {
    let kind = layer.params.y;
    if kind < 0.5 {
        return 1.0;
    }
    let half_size = max(layer.mask.zw * 0.5, vec2f(1e-6));
    let d = abs(local - layer.mask.xy) / half_size;
    var k = length(d);
    if kind < 1.5 {
        k = max(d.x, d.y);
    }
    let feather = layer.params.z;
    var inside = select(0.0, 1.0, k <= 1.0);
    if feather > 0.0 {
        inside = 1.0 - smoothstep(1.0 - feather, 1.0, k);
    }
    return select(inside, 1.0 - inside, layer.params.w > 0.5);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    let texel = textureSample(layer_texture, layer_sampler, in.uv);
    var color = texel.rgb / max(texel.a, 1e-6);
    let entries = layer.lut.x;
    if entries > 0.5 {
        // LUTs expect the display-referred signal: encode, look up, decode.
        let signal = clamp(encode_transfer(color, 0u), vec3f(0.0), vec3f(1.0));
        let coord = signal * ((entries - 1.0) / entries) + vec3f(0.5 / entries);
        color = decode_transfer(textureSampleLevel(lut_texture, lut_sampler, coord, 0.0).rgb, 0u);
    }
    let alpha = texel.a * clamp(layer.params.x, 0.0, 1.0) * mask_alpha(in.local);
    return vec4f(color * alpha, alpha);
}
