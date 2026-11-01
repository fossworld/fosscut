// 3D objects (ADR-0040): meshes seen through the timeline's camera, lit by a sky light and
// a key light with a soft shadow, and the floor that catches it. Positions are in canvas
// pixels of the scene (x right, y down, z away). Output is premultiplied linear light.
//
// Shading follows the public metallic-roughness model of glTF 2.0 (Khronos, appendix B):
// Lambert diffuse, the GGX / Trowbridge-Reitz distribution (Walter et al., "Microfacet
// Models for Refraction through Rough Surfaces", EGSR 2007), the Smith-Schlick visibility
// with k = (roughness + 1)^2 / 8 (Karis, "Real Shading in Unreal Engine 4", SIGGRAPH 2013)
// and Schlick's Fresnel approximation (Schlick, Computer Graphics Forum 1994). The sky light
// is a hemisphere light from above and below. Shadows are a depth map from the key light
// filtered by percentage-closer filtering (Reeves, Salesin and Cook, SIGGRAPH 1987) over a
// fixed 9 by 9 grid of hardware-filtered comparisons, so they are soft and deterministic.

struct Scene {
    clip_from_object: mat4x4f,
    world_from_object: mat4x4f,
    normal_from_object: mat4x4f,
    light_from_world: mat4x4f,
    clip_from_world: mat4x4f,
    // xyz: the camera's eye.
    eye: vec4f,
    // xyz: unit direction towards the key light; w: 1 when it casts shadows.
    light_dir: vec4f,
    // rgb: the key light's linear colour times its intensity.
    light_color: vec4f,
    // rgb: the sky light from above and from below, times its strength.
    sky: vec4f,
    ground: vec4f,
    // x: one shadow-map texel in texture coordinates, y: the depth bias, z: how far
    // receivers are moved along their normal before the lookup (scene pixels), w: how dark
    // the shadow on a catching floor is.
    shadow: vec4f,
    // rgb: a drawn floor's linear colour; w: 1 for a floor that only catches the shadow.
    floor: vec4f,
};

@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

struct Surface {
    // rgb: linear colour; a: opacity.
    color: vec4f,
    // rgb: linear light given off, strength applied.
    emissive: vec4f,
    // x: metallic, y: roughness, z: texture mode (0 none, 1 printed over the colour by its
    // transparency, 2 multiplying the colour as glTF does), w: 1 when both sides are fronts.
    params: vec4f,
    // xy: how much of the surface's texture coordinates the texture covers, centred;
    // z: alpha cutoff (negative for none); w: 1 when blended.
    rect: vec4f,
};

@group(1) @binding(0) var<uniform> surface: Surface;
@group(1) @binding(1) var surface_texture: texture_2d<f32>;
@group(1) @binding(2) var surface_sampler: sampler;

const PI: f32 = 3.14159265358979;

struct VertexIn {
    @location(0) position: vec3f,
    @location(1) normal: vec3f,
    @location(2) uv: vec2f,
};

struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) world: vec3f,
    @location(1) normal: vec3f,
    @location(2) uv: vec2f,
};

@vertex
fn vs_object(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = scene.clip_from_object * vec4f(v.position, 1.0);
    out.world = (scene.world_from_object * vec4f(v.position, 1.0)).xyz;
    out.normal = (scene.normal_from_object * vec4f(v.normal, 0.0)).xyz;
    out.uv = v.uv;
    return out;
}

@vertex
fn vs_shadow(v: VertexIn) -> @builtin(position) vec4f {
    return scene.light_from_world * (scene.world_from_object * vec4f(v.position, 1.0));
}

@vertex
fn vs_floor(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = scene.clip_from_world * vec4f(v.position, 1.0);
    out.world = v.position;
    out.normal = v.normal;
    out.uv = v.uv;
    return out;
}

// How much of the key light reaches `world`: 1 lit, 0 in shadow.
fn lit(world: vec3f, normal: vec3f) -> f32 {
    if (scene.light_dir.w < 0.5) {
        return 1.0;
    }
    let moved = world + normal * scene.shadow.z;
    let p = scene.light_from_world * vec4f(moved, 1.0);
    let uv = vec2f(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    if (any(uv < vec2f(0.0)) || any(uv > vec2f(1.0))) {
        return 1.0;
    }
    let depth = clamp(p.z - scene.shadow.y, 0.0, 1.0);
    var sum = 0.0;
    for (var j = -4; j <= 4; j++) {
        for (var i = -4; i <= 4; i++) {
            let at = uv + vec2f(f32(i), f32(j)) * scene.shadow.x;
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, at, depth);
        }
    }
    return sum / 81.0;
}

// The sky light arriving from `direction`: the sky's colour from above, the ground's from
// below.
fn hemisphere(direction: vec3f) -> vec3f {
    let up = clamp(0.5 - 0.5 * direction.y, 0.0, 1.0);
    return mix(scene.ground.rgb, scene.sky.rgb, up);
}

fn distribution(n_dot_h: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

fn visibility(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let gv = n_dot_v / (n_dot_v * (1.0 - k) + k);
    let gl = n_dot_l / (n_dot_l * (1.0 - k) + k);
    return gv * gl;
}

fn fresnel(f0: vec3f, cosine: f32) -> vec3f {
    return f0 + (vec3f(1.0) - f0) * pow(1.0 - cosine, 5.0);
}

@fragment
fn fs_object(in: VertexOut) -> @location(0) vec4f {
    let v = normalize(scene.eye.xyz - in.world);
    var n = normalize(in.normal);
    if (surface.params.w > 0.5 && dot(n, v) < 0.0) {
        n = -n;
    }
    let uv = (in.uv - vec2f(0.5)) / surface.rect.xy + vec2f(0.5);
    let texel = textureSample(surface_texture, surface_sampler, uv);
    var base = surface.color.rgb;
    var alpha = surface.color.a;
    let mode = surface.params.z;
    if (mode > 1.5) {
        base = base * texel.rgb;
        alpha = alpha * texel.a;
    } else if (mode > 0.5) {
        let inside = select(0.0, 1.0, all(uv >= vec2f(0.0)) && all(uv <= vec2f(1.0)));
        let cover = texel.a * inside;
        base = texel.rgb * inside + base * (1.0 - cover);
    }
    if (surface.rect.z >= 0.0 && alpha < surface.rect.z) {
        discard;
    }
    if (surface.rect.w < 0.5) {
        alpha = 1.0;
    }
    let metallic = clamp(surface.params.x, 0.0, 1.0);
    let roughness = clamp(surface.params.y, 0.045, 1.0);
    let f0 = mix(vec3f(0.04), base, metallic);
    let diffuse = base * (1.0 - metallic);
    let n_dot_v = max(dot(n, v), 1e-4);
    let l = scene.light_dir.xyz;
    let n_dot_l = max(dot(n, l), 0.0);
    var color = vec3f(0.0);
    if (n_dot_l > 0.0) {
        let h = normalize(l + v);
        let f = fresnel(f0, max(dot(v, h), 0.0));
        let a = roughness * roughness;
        let specular = distribution(max(dot(n, h), 0.0), a)
            * visibility(n_dot_v, n_dot_l, roughness) * f / (4.0 * n_dot_v * n_dot_l + 1e-4);
        let kd = vec3f(1.0) - f;
        color = (kd * diffuse + PI * specular) * scene.light_color.rgb * n_dot_l
            * lit(in.world, n);
    }
    // The sky light: diffuse from the hemisphere around the normal, and a reflection of it
    // that blurs towards the diffuse light as the surface gets rougher.
    let ambient_f = f0 + (max(vec3f(1.0 - roughness), f0) - f0) * pow(1.0 - n_dot_v, 5.0);
    let reflected = mix(hemisphere(reflect(-v, n)), hemisphere(n), roughness);
    color += diffuse * (vec3f(1.0) - ambient_f) * hemisphere(n) + reflected * ambient_f;
    color += surface.emissive.rgb;
    return vec4f(color * alpha, alpha);
}

@fragment
fn fs_floor(in: VertexOut) -> @location(0) vec4f {
    let n = vec3f(0.0, -1.0, 0.0);
    // The floor fades out towards its edge, so it never ends in a line.
    let fade = 1.0 - smoothstep(0.6, 1.0, length(in.uv - vec2f(0.5)) * 2.0);
    let n_dot_l = max(dot(n, scene.light_dir.xyz), 0.0);
    let light = select(1.0, lit(in.world, n), n_dot_l > 0.0);
    if (scene.floor.w > 0.5) {
        let dark = scene.shadow.w * (1.0 - light) * fade;
        return vec4f(0.0, 0.0, 0.0, dark);
    }
    let color = scene.floor.rgb * (scene.light_color.rgb * n_dot_l * light + hemisphere(n));
    return vec4f(color * fade, fade);
}
