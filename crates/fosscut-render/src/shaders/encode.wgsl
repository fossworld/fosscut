// The linear canvas to the output's transfer function (ADR-0008, item 4).

@group(0) @binding(0) var canvas: texture_2d<f32>;
@group(0) @binding(1) var<uniform> output: vec4u;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    return full_screen_position(index);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let linear = textureLoad(canvas, vec2i(position.xy), 0).rgb;
    return vec4f(encode_transfer(linear, output.x), 1.0);
}
