// Y'CbCr planes to linear-light RGB at the source's own size (ADR-0008, item 4).

struct Convert {
    // Rows of the 3x4 matrix from color.rs: R'G'B' = M * [y, cb, cr, 1].
    row0: vec4f,
    row1: vec4f,
    row2: vec4f,
    // x: 0 = NV12 (interleaved chroma), 1 = three planes; y: source transfer.
    mode: vec4u,
}

@group(0) @binding(0) var luma: texture_2d<f32>;
@group(0) @binding(1) var chroma_a: texture_2d<f32>;
@group(0) @binding(2) var chroma_b: texture_2d<f32>;
@group(0) @binding(3) var chroma_sampler: sampler;
@group(0) @binding(4) var<uniform> params: Convert;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    return full_screen_position(index);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let y = textureLoad(luma, vec2i(position.xy), 0).r;
    // Chroma is sampled bilinearly at the luma pixel's centre (centre-sited upsampling).
    let uv = position.xy / vec2f(textureDimensions(luma));
    var cb: f32;
    var cr: f32;
    if params.mode.x == 0u {
        let c = textureSampleLevel(chroma_a, chroma_sampler, uv, 0.0).rg;
        cb = c.r;
        cr = c.g;
    } else {
        cb = textureSampleLevel(chroma_a, chroma_sampler, uv, 0.0).r;
        cr = textureSampleLevel(chroma_b, chroma_sampler, uv, 0.0).r;
    }
    let yuv = vec4f(y, cb, cr, 1.0);
    let rgb = clamp(
        vec3f(dot(params.row0, yuv), dot(params.row1, yuv), dot(params.row2, yuv)),
        vec3f(0.0),
        vec3f(1.0),
    );
    return vec4f(decode_transfer(rgb, params.mode.y), 1.0);
}
