// The linear canvas to the output's transfer function (ADR-0008, item 4).

@group(0) @binding(0) var canvas: texture_2d<f32>;
@group(0) @binding(1) var<uniform> output: vec4u;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    return full_screen_position(index);
}

// output.y = 1 keeps the canvas's alpha: a compound clip's picture (ADR-0025), whose empty
// areas let what is under it show.
@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = textureLoad(canvas, vec2i(position.xy), 0);
    let alpha = select(1.0, texel.a, output.y == 1u);
    return vec4f(encode_transfer(texel.rgb, output.x), alpha);
}
