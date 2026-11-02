// Layers placed in 3D (ADR-0037): the vertex stage for composite.wgsl's fragment stage.
// Corners arrive in homogeneous clip coordinates (w is the depth over the focal length),
// so the rasterizer interpolates the picture's coordinates with perspective, and the 4x
// multisampled canvas antialiases the edges. Masks, LUTs and opacity work as for flat
// layers, in the layer's own coordinates.

@vertex
fn vs_projected(
    @location(0) position: vec4f,
    @location(1) local: vec2f,
    @location(2) uv: vec2f,
) -> VertexOut {
    var out: VertexOut;
    out.position = position;
    out.local = local;
    out.uv = uv;
    return out;
}
