// Particle sprites (ADR-0038): one instanced quad per particle, drawn into a transparent
// linear-light texture of the output size. Shapes are analytic, with edges softened by at
// least one pixel so small and moving particles never shimmer. The output is premultiplied;
// the pipeline's blend state adds light up (`add`) or covers (`normal`).

struct Viewport {
    // x, y: output size in pixels.
    size: vec4f,
}

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var sprite_texture: texture_2d<f32>;
@group(0) @binding(2) var sprite_sampler: sampler;

struct Instance {
    // Centre in output pixels.
    @location(0) center: vec2f,
    // Half the quad's width (along the axis) and height, in pixels.
    @location(1) half_size: vec2f,
    // The quad's x axis: cos and sin of its clockwise angle.
    @location(2) axis: vec2f,
    // x: radius in pixels; y: a spark's half length in pixels.
    @location(3) extent: vec2f,
    // Linear colour and opacity, not premultiplied.
    @location(4) color: vec4f,
    // x: kind (0 dot, 1 spark, 2 star, 3 ring, 4 square, 5 image); y: softness;
    // z: star points; w: the width factor while tumbling (its sign mirrors images).
    @location(5) shape: vec4f,
}

struct Varyings {
    @builtin(position) position: vec4f,
    // Pixels from the centre along the quad's axes.
    @location(0) local: vec2f,
    @location(1) half_size: vec2f,
    @location(2) extent: vec2f,
    @location(3) color: vec4f,
    @location(4) shape: vec4f,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, instance: Instance) -> Varyings {
    let corner = vec2f(f32(index & 1u), f32((index >> 1u) & 1u)) * 2.0 - 1.0;
    let local = corner * instance.half_size;
    let across = vec2f(-instance.axis.y, instance.axis.x);
    let pixel = instance.center + instance.axis * local.x + across * local.y;
    var out: Varyings;
    out.position = vec4f(
        pixel.x / viewport.size.x * 2.0 - 1.0,
        1.0 - pixel.y / viewport.size.y * 2.0,
        0.0,
        1.0,
    );
    out.local = local;
    out.half_size = instance.half_size;
    out.extent = instance.extent;
    out.color = instance.color;
    out.shape = instance.shape;
    return out;
}

// Coverage of an edge at normalized distance `d` (1 on the edge) with softness `soft`
// (a fraction of the radius), at least `pixel` wide.
fn edge(d: f32, soft: f32, pixel: f32) -> f32 {
    let width = max(soft, pixel);
    return 1.0 - smoothstep(1.0 - width, 1.0, d);
}

@fragment
fn fs_main(in: Varyings) -> @location(0) vec4f {
    // Sampled before branching: implicit-derivative sampling needs uniform control flow.
    var uv = in.local / max(in.half_size, vec2f(0.5)) * 0.5 + 0.5;
    if in.shape.w < 0.0 {
        uv.x = 1.0 - uv.x;
    }
    let texel = textureSample(sprite_texture, sprite_sampler, uv).rgb;
    let kind = u32(in.shape.x + 0.5);
    let soft = in.shape.y;
    let radius = max(in.extent.x, 0.5);
    let pixel = 1.0 / radius;
    var coverage = 0.0;
    var rgb = in.color.rgb;
    switch kind {
        case 1u: {
            // A streak: a capsule along the axis, brightest at its head (+x).
            let half_length = in.extent.y;
            let along = clamp(in.local.x, -half_length, half_length);
            let d = length(vec2f(in.local.x - along, in.local.y)) / radius;
            let head = select(1.0, clamp((in.local.x + half_length) / (2.0 * half_length), 0.0, 1.0), half_length > 0.0);
            coverage = edge(d, soft, pixel) * mix(0.15, 1.0, head);
        }
        case 2u: {
            // A star: thin points around a small core.
            let points = max(in.shape.z, 3.0);
            let r = length(in.local) / radius;
            let angle = atan2(in.local.y, in.local.x);
            let spike = pow(abs(cos(angle * points * 0.5)), 10.0);
            let reach = mix(0.22, 1.0, spike);
            coverage = edge(r / reach, soft, pixel / reach);
        }
        case 3u: {
            // A ring: a band near the edge.
            let r = length(in.local) / radius;
            let band = abs(r - 0.82) / 0.18;
            coverage = edge(band, soft, pixel / 0.18);
        }
        case 4u: {
            // A rectangle, dimmed as it turns edge-on while it tumbles.
            let q = abs(in.local) / max(in.half_size, vec2f(0.5));
            let d = max(q.x, q.y);
            let narrow = min(in.half_size.x, in.half_size.y);
            coverage = edge(d, soft, 1.0 / max(narrow, 0.5));
            rgb = rgb * (0.55 + 0.45 * abs(in.shape.w));
        }
        case 5u: {
            // An image: its brightness covers, its colour tinted by the particle's.
            let luma = dot(texel, vec3f(0.2126, 0.7152, 0.0722));
            coverage = clamp(luma, 0.0, 1.0);
            rgb = texel * in.color.rgb / max(coverage, 1e-4);
        }
        default: {
            // A round dot.
            let r = length(in.local) / radius;
            coverage = edge(r, soft, pixel);
        }
    }
    let alpha = clamp(in.color.a * coverage, 0.0, 1.0);
    return vec4f(rgb * alpha, alpha);
}
