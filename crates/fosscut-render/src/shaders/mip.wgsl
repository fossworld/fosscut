// One mip level from the level above it: a 2x2 box filter in linear light, so sources
// shrink without aliasing.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    return full_screen_position(index);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    // The bilinear sample at the centre of each 2x2 block of the source is their average.
    let uv = position.xy * 2.0 / vec2f(textureDimensions(source));
    return textureSampleLevel(source, linear_sampler, uv, 0.0);
}
