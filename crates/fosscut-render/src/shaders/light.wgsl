// Light effects (ADR-0011): light.glow, light.sweep, light.streaks and the padding that lets
// light spread past a layer's edges. Appended to effects.wgsl, whose bindings, `load` and
// colour helpers it uses; the CPU side and the sources of the formulas are in light.rs.
// Pictures are premultiplied linear light. Nothing here is random.

// Copies the layer's picture into the middle of a larger, transparent one. p[0]: padding
// left and top, then the picture's width and height, in pixels.
@fragment
fn fs_light_pad(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let at = vec2i(position.xy) - vec2i(effect.p[0].xy);
    let size = vec2i(effect.p[0].zw);
    if any(at < vec2i(0)) || any(at >= size) {
        return vec4f(0.0);
    }
    return textureLoad(input_texture, at, 0);
}

fn light_brightest(c: vec3f) -> f32 {
    return max(max(c.r, c.g), c.b);
}

// The light that glows or streaks: parts at least as bright as the threshold (p[0].x, on
// the brightest sRGB-encoded channel, with a soft knee 0.1 below it), turned towards the
// tint colour (p[1].rgb, linear) by p[0].y while keeping their brightest channel.
@fragment
fn fs_light_bright(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return vec4f(0.0);
    }
    let color = unpremultiply(texel);
    let threshold = effect.p[0].x;
    var weight = 1.0;
    if threshold > 0.0 {
        weight = smoothstep(threshold - 0.1, threshold, light_brightest(to_display(color)));
    }
    let tinted = mix(color, effect.p[1].rgb * light_brightest(color), effect.p[0].y);
    return vec4f(max(tinted, vec3f(0.0)) * texel.a * weight, texel.a * weight);
}

// One pass of streaks: the brightest of the samples along a direction on both sides, each
// weighted by an exponential fall-off with its distance. p[0].xy: one step in texture
// coordinates; p[0].z: the level of detail sampled; p[0].w: steps on each side; p[1].x: the
// fall-off per step. Two passes (fine, then coarse) multiply their weights into one curve.
@fragment
fn fs_light_streak(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let uv = position.xy / vec2f(textureDimensions(input_texture));
    let step = effect.p[0].xy;
    let lod = effect.p[0].z;
    let taps = i32(effect.p[0].w);
    let falloff = effect.p[1].x;
    var best = textureSampleLevel(input_texture, linear_sampler, uv, lod);
    for (var k = 1; k <= taps; k = k + 1) {
        let w = exp(-falloff * f32(k));
        let ahead = textureSampleLevel(input_texture, linear_sampler, uv + step * f32(k), lod);
        let behind = textureSampleLevel(input_texture, linear_sampler, uv - step * f32(k), lod);
        best = max(best, max(ahead, behind) * w);
    }
    return best;
}

// Adds light to the picture: p[0].x of the light in aux_texture plus p[0].y of the light
// in mask_texture, times the intensity p[0].z. Where the picture is opaque the light adds
// to it; over transparent parts the light covers as much as its brightest channel, so it
// shows on the layers below. p[0].w is 1 to show the light alone.
@fragment
fn fs_light_add(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let picture = load(position);
    let at = vec2i(position.xy);
    let a = textureLoad(aux_texture, at, 0);
    let b = textureLoad(mask_texture, at, 0);
    let light = max((a.rgb * effect.p[0].x + b.rgb * effect.p[0].y) * effect.p[0].z, vec3f(0.0));
    let coverage = clamp(light_brightest(light), 0.0, 1.0);
    if effect.p[0].w > 0.5 {
        return vec4f(light, coverage);
    }
    return vec4f(picture.rgb + light, picture.a + coverage * (1.0 - picture.a));
}

// A band of light moving across the picture's visible pixels. p[0]: position (0 just
// before the band enters, 1 just after it leaves), the direction it moves in radians
// clockwise from left to right, its width as a fraction of the distance it travels, and
// the softness of its edges. p[1]: intensity, then the colour (linear). p[2]: where the
// picture lies in a padded one, left, top, width and height in pixels.
@fragment
fn fs_light_sweep(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let texel = load(position);
    if texel.a <= 0.0 {
        return texel;
    }
    let p = effect.p[0];
    let dir = vec2f(cos(p.y), sin(p.y));
    let content = effect.p[2];
    let along = dot(position.xy - content.xy, dir);
    // The picture's extent along the direction, from its four corners.
    let corners = vec4f(0.0, content.z * dir.x, content.w * dir.y, content.z * dir.x + content.w * dir.y);
    let low = min(min(corners.x, corners.y), min(corners.z, corners.w));
    let high = max(max(corners.x, corners.y), max(corners.z, corners.w));
    let half_width = 0.5 * max(p.z, 1e-4) * max(high - low, 1.0);
    let centre = mix(low - half_width, high + half_width, p.x);
    let distance = abs(along - centre);
    let inner = half_width * (1.0 - p.w);
    var profile = select(0.0, 1.0, distance <= half_width);
    if half_width - inner > 1e-3 {
        profile = 1.0 - smoothstep(inner, half_width, distance);
    }
    let light = effect.p[1].yzw * effect.p[1].x * profile;
    return vec4f(texel.rgb + light * texel.a, texel.a);
}
