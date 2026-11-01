//! 3D objects (ADR-0040) on the CPU: meshes of the built-in solids, extruded text and
//! paths, grids of blocks and glTF models, their materials, and where an object, its floor
//! and its key light are in the scene. `gpu/objects.rs` draws them.
//!
//! # Spaces
//!
//! Meshes are built in object space, in fractions of the canvas width with x right, y down
//! and z away from the viewer, centred on their bounds (a grid of blocks stands on y = 0
//! instead). [`object_model`] scales them to canvas pixels and places them with the clip's
//! transform through [`crate::space::layer_model`], so objects and planar 3D layers share
//! one scene and one camera ([`crate::space::Camera::clip_from_world`]). A glTF model (y
//! up, z towards the viewer) is turned half a turn around x to match, centred on its
//! bounds and scaled so its longest side is the object's `size`.
//!
//! # Determinism
//!
//! Everything here is a pure function of the object's settings and the model file: the
//! grid's heights come from integer hashes (`splitmix64`, Steele, Lea and Flood, "Fast
//! splittable pseudorandom number generators", OOPSLA 2014), curves are flattened with a
//! fixed tolerance, and polygons are triangulated by ear clipping (`earcut`).

use std::sync::Arc;

use fosscut_model::{
    BlocksObject, Clip, Material, ModelContent, ModelInfo, ModelObject, Transform,
};
use fosscut_time::Flicks;
use kurbo::{BezPath, PathEl, Point};

use crate::plan::SourceRef;
use crate::space::{Mat4, Vec3, apply, layer_model, multiply};

/// Material slot of a box's (or block's) front face; extruded text and paths use it for
/// their front.
pub const FRONT: usize = 0;
/// Material slot of the back face.
pub const BACK: usize = 1;
/// Material slot of the left face.
pub const LEFT: usize = 2;
/// Material slot of the right face.
pub const RIGHT: usize = 3;
/// Material slot of the top face.
pub const TOP: usize = 4;
/// Material slot of the bottom face.
pub const BOTTOM: usize = 5;
/// Material slot of everything else: the object's own `material`.
pub const MAIN: usize = 6;

/// Segments of a quarter of a rounded edge.
const ROUND_STEPS: usize = 4;
/// Segments around a sphere, cylinder or torus.
const AROUND: usize = 64;
/// The font size text is laid out at before scaling, in layout units.
const LAYOUT_SIZE: f32 = 512.0;
/// The most textures one object prints.
pub const MAX_TEXTURES: usize = 8;

/// One vertex of a mesh, as the shader reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshVertex {
    /// Position in object space.
    pub position: [f32; 3],
    /// Unit normal, pointing out of the solid.
    pub normal: [f32; 3],
    /// Texture coordinate, 0 to 1 from the top left.
    pub uv: [f32; 2],
}

/// A run of triangles drawn with one material.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPart {
    /// First index.
    pub start: u32,
    /// Number of indices.
    pub count: u32,
    /// Its material slot: [`FRONT`] to [`MAIN`] for built-in solids, the file's material
    /// index for glTF models.
    pub slot: usize,
    /// The width and height of a flat face in object units, so a texture keeps its
    /// proportions on it; zero for surfaces a texture wraps around.
    pub face: [f32; 2],
}

/// Triangles in parts by material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    /// The vertices.
    pub vertices: Vec<MeshVertex>,
    /// Three indices per triangle.
    pub indices: Vec<u32>,
    /// Runs of indices by material.
    pub parts: Vec<MeshPart>,
}

impl Mesh {
    fn vertex(&mut self, position: Vec3, normal: Vec3, uv: [f64; 2]) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(MeshVertex {
            position: position.map(|v| v as f32),
            normal: normal.map(|v| v as f32),
            uv: uv.map(|v| v as f32),
        });
        index
    }

    fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.triangle(a, b, c);
        self.triangle(a, c, d);
    }

    /// Records the indices added since `start` as a part.
    fn close_part(&mut self, start: usize, slot: usize, face: [f64; 2]) {
        let count = self.indices.len() - start;
        if count > 0 {
            self.parts.push(MeshPart {
                start: start as u32,
                count: count as u32,
                slot,
                face: face.map(|v| v as f32),
            });
        }
    }

    /// Triangles drawn.
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// The smallest and largest corner of its bounds, or `None` when empty.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut low = [f64::INFINITY; 3];
        let mut high = [f64::NEG_INFINITY; 3];
        for v in &self.vertices {
            for axis in 0..3 {
                low[axis] = low[axis].min(f64::from(v.position[axis]));
                high[axis] = high[axis].max(f64::from(v.position[axis]));
            }
        }
        (low[0] <= high[0]).then_some((low, high))
    }

    /// Moves every vertex by `offset` and scales it by `scale` afterwards.
    fn transform(&mut self, offset: Vec3, scale: f64) {
        for v in &mut self.vertices {
            for (value, by) in v.position.iter_mut().zip(offset) {
                *value = ((f64::from(*value) + by) * scale) as f32;
            }
        }
    }

    /// Centres the mesh on its bounds in x and y (and z when `depth` is true).
    fn centre(&mut self, depth: bool) {
        if let Some((low, high)) = self.bounds() {
            let middle = |axis: usize| -(low[axis] + high[axis]) / 2.0;
            self.transform(
                [middle(0), middle(1), if depth { middle(2) } else { 0.0 }],
                1.0,
            );
        }
    }
}

/// A 3D object at one moment, as the planner gives it to the renderer. Its placement is
/// the layer's transform and its camera the layer's [`crate::space::LayerSpace`].
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectLayer {
    /// The object with its `model.*` keyframes applied.
    pub content: ModelContent,
    /// The image assets its materials print, each as its still frame, in the order of
    /// [`ModelContent::texture_assets`].
    pub textures: Vec<SourceRef>,
}

impl ObjectLayer {
    /// The object of `clip` (whose content is `content`) at `offset` from its start.
    pub fn of(clip: &Clip, content: &ModelContent, offset: Flicks) -> ObjectLayer {
        let mut content = content.clone();
        for property in fosscut_model::MODEL_PROPERTIES {
            if let Some(value) = clip.keyed(property.path, offset) {
                let value = match (value, property.min, property.max) {
                    (fosscut_model::KeyValue::Number(v), low, high) => {
                        fosscut_model::KeyValue::Number(
                            v.max(low.unwrap_or(f64::NEG_INFINITY))
                                .min(high.unwrap_or(f64::INFINITY)),
                        )
                    }
                    (other, _, _) => other,
                };
                content.set_value(property.path, value);
            }
        }
        let textures = content
            .texture_assets()
            .into_iter()
            .take(MAX_TEXTURES)
            .map(|asset| SourceRef {
                asset: asset.clone(),
                time: Flicks::ZERO,
            })
            .collect();
        ObjectLayer { content, textures }
    }
}

/// `splitmix64`'s mixing function: a well-spread 64-bit hash of `z`.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A number from 0 (included) to 1 (excluded) chosen by the seed, a lot and a choice.
fn unit(seed: u32, row: u32, column: u32, choice: u64) -> f64 {
    let lot = (u64::from(row) << 32) | u64::from(column);
    let hashed = mix(mix(mix(u64::from(seed)) ^ lot) ^ choice);
    (hashed >> 11) as f64 / (1u64 << 53) as f64
}

fn normalize(v: Vec3) -> Vec3 {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 1e-12 {
        [v[0] / length, v[1] / length, v[2] / length]
    } else {
        [0.0, 0.0, -1.0]
    }
}

/// The six faces of a box: their slot, the axis they are flat on and its sign, then the
/// axes and directions texture u and v run along, so pictures read the right way round
/// from outside.
const BOX_FACES: [(usize, usize, f64, usize, f64, usize, f64); 6] = [
    (FRONT, 2, -1.0, 0, 1.0, 1, 1.0),
    (BACK, 2, 1.0, 0, -1.0, 1, 1.0),
    (LEFT, 0, -1.0, 2, -1.0, 1, 1.0),
    (RIGHT, 0, 1.0, 2, 1.0, 1, 1.0),
    (TOP, 1, -1.0, 0, 1.0, 2, -1.0),
    (BOTTOM, 1, 1.0, 0, 1.0, 2, 1.0),
];

/// Adds a box of `size` centred on `centre` with edges rounded by `bevel`, one part per
/// face.
fn add_box(mesh: &mut Mesh, centre: Vec3, size: Vec3, bevel: f64) {
    let half = size.map(|s| s.abs() / 2.0);
    let r = bevel.clamp(0.0, half.iter().copied().fold(f64::INFINITY, f64::min));
    // Grid coordinates along an axis of half-length `h`: the rounded strips at both ends
    // in steps, and the flat part between them.
    let coords = |h: f64| -> Vec<f64> {
        if r <= 0.0 {
            return vec![-h, h];
        }
        let mut values: Vec<f64> = (0..=ROUND_STEPS)
            .map(|i| -h + r * i as f64 / ROUND_STEPS as f64)
            .chain((0..=ROUND_STEPS).map(|i| h - r + r * i as f64 / ROUND_STEPS as f64))
            .collect();
        values.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        values
    };
    for (slot, axis, sign, u_axis, u_sign, v_axis, v_sign) in BOX_FACES {
        let start = mesh.indices.len();
        let (us, vs) = (coords(half[u_axis]), coords(half[v_axis]));
        let base = mesh.vertices.len() as u32;
        for &v in &vs {
            for &u in &us {
                let mut p = [0.0; 3];
                p[axis] = sign * half[axis];
                p[u_axis] = u_sign * u;
                p[v_axis] = v_sign * v;
                let mut normal = [0.0; 3];
                normal[axis] = sign;
                if r > 0.0 {
                    // Rounded box: the point on the box shrunk by the radius, pushed out
                    // by the radius along the direction from it.
                    let inner: Vec3 = std::array::from_fn(|a| p[a].clamp(r - half[a], half[a] - r));
                    normal = normalize(std::array::from_fn(|a| p[a] - inner[a]));
                    p = std::array::from_fn(|a| inner[a] + r * normal[a]);
                }
                let uv = [
                    (u + half[u_axis]) / (2.0 * half[u_axis]).max(1e-12),
                    (v + half[v_axis]) / (2.0 * half[v_axis]).max(1e-12),
                ];
                mesh.vertex(std::array::from_fn(|a| centre[a] + p[a]), normal, uv);
            }
        }
        let row = us.len() as u32;
        for j in 0..vs.len() as u32 - 1 {
            for i in 0..row - 1 {
                let a = base + j * row + i;
                mesh.quad(a, a + 1, a + row + 1, a + row);
            }
        }
        mesh.close_part(start, slot, [2.0 * half[u_axis], 2.0 * half[v_axis]]);
    }
}

/// A sphere of diameter `size`.
fn sphere(size: f64) -> Mesh {
    let mut mesh = Mesh::default();
    let r = size / 2.0;
    let (around, down) = (AROUND, AROUND / 2);
    for j in 0..=down {
        let theta = std::f64::consts::PI * j as f64 / down as f64;
        for i in 0..=around {
            let phi = std::f64::consts::TAU * i as f64 / around as f64;
            let n = [
                theta.sin() * phi.sin(),
                -theta.cos(),
                -theta.sin() * phi.cos(),
            ];
            mesh.vertex(
                n.map(|v| v * r),
                n,
                [i as f64 / around as f64, j as f64 / down as f64],
            );
        }
    }
    grid(&mut mesh, 0, around + 1, down + 1);
    mesh.close_part(0, MAIN, [0.0, 0.0]);
    mesh
}

/// Indices of a `columns` by `rows` grid of vertices starting at `base`.
fn grid(mesh: &mut Mesh, base: u32, columns: usize, rows: usize) {
    for j in 0..rows as u32 - 1 {
        for i in 0..columns as u32 - 1 {
            let a = base + j * columns as u32 + i;
            mesh.quad(a, a + 1, a + columns as u32 + 1, a + columns as u32);
        }
    }
}

/// An upright cylinder of `[diameter, height]`.
fn cylinder(size: [f64; 2]) -> Mesh {
    let mut mesh = Mesh::default();
    let (r, h) = (size[0] / 2.0, size[1] / 2.0);
    for (index, y) in [-h, h].into_iter().enumerate() {
        for i in 0..=AROUND {
            let phi = std::f64::consts::TAU * i as f64 / AROUND as f64;
            let n = [phi.sin(), 0.0, -phi.cos()];
            mesh.vertex(
                [r * n[0], y, r * n[2]],
                n,
                [i as f64 / AROUND as f64, index as f64],
            );
        }
    }
    grid(&mut mesh, 0, AROUND + 1, 2);
    for (y, ny) in [(-h, -1.0), (h, 1.0)] {
        let centre = mesh.vertex([0.0, y, 0.0], [0.0, ny, 0.0], [0.5, 0.5]);
        let ring = mesh.vertices.len() as u32;
        for i in 0..=AROUND {
            let phi = std::f64::consts::TAU * i as f64 / AROUND as f64;
            let (x, z) = (r * phi.sin(), -r * phi.cos());
            let v = if ny < 0.0 {
                (r - z) / (2.0 * r)
            } else {
                (z + r) / (2.0 * r)
            };
            mesh.vertex([x, y, z], [0.0, ny, 0.0], [(x + r) / (2.0 * r), v]);
        }
        for i in 0..AROUND as u32 {
            mesh.triangle(centre, ring + i, ring + i + 1);
        }
    }
    mesh.close_part(0, MAIN, [0.0, 0.0]);
    mesh
}

/// A flat rectangle facing the viewer.
fn plane(size: [f64; 2]) -> Mesh {
    let mut mesh = Mesh::default();
    let (w, h) = (size[0] / 2.0, size[1] / 2.0);
    let n = [0.0, 0.0, -1.0];
    let a = mesh.vertex([-w, -h, 0.0], n, [0.0, 0.0]);
    let b = mesh.vertex([w, -h, 0.0], n, [1.0, 0.0]);
    let c = mesh.vertex([w, h, 0.0], n, [1.0, 1.0]);
    let d = mesh.vertex([-w, h, 0.0], n, [0.0, 1.0]);
    mesh.quad(a, b, c, d);
    mesh.close_part(0, MAIN, size);
    mesh
}

/// A ring of outer diameter `size` and tube diameter `thickness`, facing the viewer.
fn torus(size: f64, thickness: f64) -> Mesh {
    let mut mesh = Mesh::default();
    let tube = (thickness / 2.0).min(size / 4.0);
    let ring = size / 2.0 - tube;
    let (around, across) = (AROUND * 3 / 2, AROUND / 2);
    for j in 0..=across {
        let psi = std::f64::consts::TAU * j as f64 / across as f64;
        for i in 0..=around {
            let phi = std::f64::consts::TAU * i as f64 / around as f64;
            let out = [phi.cos(), phi.sin(), 0.0];
            let n = [psi.cos() * out[0], psi.cos() * out[1], -psi.sin()];
            mesh.vertex(
                [
                    ring * out[0] + tube * n[0],
                    ring * out[1] + tube * n[1],
                    tube * n[2],
                ],
                n,
                [i as f64 / around as f64, j as f64 / across as f64],
            );
        }
    }
    grid(&mut mesh, 0, around + 1, across + 1);
    mesh.close_part(0, MAIN, [0.0, 0.0]);
    mesh
}

/// The heights of a grid's blocks by row and column: `None` for an empty lot. Heights are
/// spread by the square of a random number, then scaled by the growth.
pub fn block_heights(blocks: &BlocksObject) -> Vec<Vec<Option<f64>>> {
    let [low, high] = blocks.heights;
    (0..blocks.rows)
        .map(|row| {
            (0..blocks.columns)
                .map(|column| {
                    if unit(blocks.seed, row, column, 1) >= blocks.fill {
                        return None;
                    }
                    let u = unit(blocks.seed, row, column, 2);
                    let height = (low + (high - low) * u * u) * blocks.growth.clamp(0.0, 1.0);
                    (height > 0.0).then_some(height)
                })
                .collect()
        })
        .collect()
}

/// A grid of blocks standing on y = 0, centred in x and z, one part per face direction.
fn blocks(object: &BlocksObject) -> Mesh {
    let mut all = Mesh::default();
    let step = object.block + object.gap;
    let heights = block_heights(object);
    // Each block adds six parts; merge them by face afterwards.
    let mut faces: Vec<Mesh> = (0..6).map(|_| Mesh::default()).collect();
    for (row, line) in heights.iter().enumerate() {
        for (column, height) in line.iter().enumerate() {
            let Some(height) = height else { continue };
            let x = (column as f64 - (f64::from(object.columns) - 1.0) / 2.0) * step;
            let z = (row as f64 - (f64::from(object.rows) - 1.0) / 2.0) * step;
            let mut one = Mesh::default();
            add_box(
                &mut one,
                [x, -height / 2.0, z],
                [object.block, *height, object.block],
                0.0,
            );
            for part in &one.parts {
                let target = &mut faces[part.slot];
                let offset = target.vertices.len() as u32;
                let range = part.start as usize..(part.start + part.count) as usize;
                let used: Vec<u32> = one.indices[range].to_vec();
                let low = used.iter().copied().min().unwrap_or(0);
                let high = used.iter().copied().max().unwrap_or(0);
                target
                    .vertices
                    .extend_from_slice(&one.vertices[low as usize..=high as usize]);
                target.indices.extend(used.iter().map(|i| i - low + offset));
            }
        }
    }
    for (slot, face) in faces.into_iter().enumerate() {
        let start = all.indices.len();
        let offset = all.vertices.len() as u32;
        all.vertices.extend(face.vertices);
        all.indices.extend(face.indices.iter().map(|i| i + offset));
        all.close_part(start, slot, [0.0, 0.0]);
    }
    all
}

/// Closed rings of points, flattened from `path` within `tolerance`.
fn rings(path: &BezPath, tolerance: f64) -> Vec<Vec<[f64; 2]>> {
    let mut rings: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut current: Vec<[f64; 2]> = Vec::new();
    let finish = |current: &mut Vec<[f64; 2]>, rings: &mut Vec<Vec<[f64; 2]>>| {
        let mut ring = std::mem::take(current);
        ring.dedup_by(|a, b| (a[0] - b[0]).hypot(a[1] - b[1]) < tolerance * 1e-3);
        while ring.len() > 1 {
            let (first, last) = (ring[0], ring[ring.len() - 1]);
            if (first[0] - last[0]).hypot(first[1] - last[1]) < tolerance * 1e-3 {
                ring.pop();
            } else {
                break;
            }
        }
        if ring.len() >= 3 && area(&ring).abs() > tolerance * tolerance * 1e-3 {
            rings.push(ring);
        }
    };
    kurbo::flatten(
        path.elements().iter().copied(),
        tolerance,
        |element| match element {
            PathEl::MoveTo(p) => {
                finish(&mut current, &mut rings);
                current.push([p.x, p.y]);
            }
            PathEl::LineTo(p) => current.push([p.x, p.y]),
            PathEl::ClosePath => finish(&mut current, &mut rings),
            PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => current.push([p.x, p.y]),
        },
    );
    finish(&mut current, &mut rings);
    rings
}

/// The signed area of a ring by the shoelace formula: positive for one turning direction,
/// negative for the other.
fn area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

/// True when `point` lies inside `ring` (even-odd crossing test).
fn inside(point: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let mut odd = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + n - 1) % n]);
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            odd = !odd;
        }
    }
    odd
}

/// Polygons with holes: each outer ring (positive area) with the holes (negative area)
/// directly inside it, by nesting depth as the even-odd rule fills them.
fn polygons(mut rings: Vec<Vec<[f64; 2]>>) -> Vec<Vec<Vec<[f64; 2]>>> {
    let depths: Vec<usize> = (0..rings.len())
        .map(|i| {
            (0..rings.len())
                .filter(|&j| j != i && inside(rings[i][0], &rings[j]))
                .count()
        })
        .collect();
    for (ring, depth) in rings.iter_mut().zip(&depths) {
        let outer = depth % 2 == 0;
        if (area(ring) > 0.0) != outer {
            ring.reverse();
        }
    }
    let mut groups: Vec<(usize, Vec<Vec<[f64; 2]>>)> = Vec::new();
    for i in (0..rings.len()).filter(|&i| depths[i].is_multiple_of(2)) {
        groups.push((i, vec![rings[i].clone()]));
    }
    for i in (0..rings.len()).filter(|&i| !depths[i].is_multiple_of(2)) {
        let parent = groups
            .iter_mut()
            .filter(|(outer, _)| {
                depths[*outer] + 1 == depths[i] && inside(rings[i][0], &rings[*outer])
            })
            .min_by(|a, b| area(&rings[a.0]).total_cmp(&area(&rings[b.0])));
        if let Some((_, group)) = parent {
            group.push(rings[i].clone());
        }
    }
    groups.into_iter().map(|(_, group)| group).collect()
}

/// The outward normal of the edge from `a` to `b` of a ring oriented as [`polygons`]
/// leaves it.
fn edge_normal(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
    let length = ex.hypot(ey).max(1e-12);
    [ey / length, -ex / length]
}

/// Extrudes the closed outlines of `path` (object units, y down) by `depth` along z,
/// chamfering the front and back edges by `bevel`; curves are flattened within
/// `tolerance`. The front cap is [`FRONT`], the back cap [`BACK`], sides and bevels
/// [`MAIN`]. Without depth only the front cap is made.
pub fn extrude(path: &BezPath, tolerance: f64, depth: f64, bevel: f64) -> Mesh {
    let groups = polygons(rings(path, tolerance));
    let mut mesh = Mesh::default();
    if groups.is_empty() {
        return mesh;
    }
    let all = groups.iter().flatten().flatten();
    let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in all {
        for axis in 0..2 {
            low[axis] = low[axis].min(p[axis]);
            high[axis] = high[axis].max(p[axis]);
        }
    }
    let span = [(high[0] - low[0]).max(1e-12), (high[1] - low[1]).max(1e-12)];
    let planar = |p: [f64; 2]| [(p[0] - low[0]) / span[0], (p[1] - low[1]) / span[1]];
    let depth = depth.max(0.0);
    let bevel = bevel.clamp(0.0, depth / 2.0);
    let (front, back) = (-depth / 2.0, depth / 2.0);
    // Corners sharper than this keep separate normals on each side.
    let smooth = 35f64.to_radians().cos();

    // Each ring's points moved inwards by the bevel along their mitred normals.
    let inset = |ring: &[[f64; 2]]| -> Vec<[f64; 2]> {
        let n = ring.len();
        (0..n)
            .map(|i| {
                let (prev, here, next) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
                let (a, b) = (edge_normal(prev, here), edge_normal(here, next));
                let sum = [a[0] + b[0], a[1] + b[1]];
                let length = sum[0].hypot(sum[1]);
                let m = if length < 1e-9 {
                    b
                } else {
                    [sum[0] / length, sum[1] / length]
                };
                let reach = bevel / (m[0] * b[0] + m[1] * b[1]).max(0.35);
                [here[0] - m[0] * reach, here[1] - m[1] * reach]
            })
            .collect()
    };

    let mut earcut = earcut::Earcut::new();
    let mut triangles: Vec<u32> = Vec::new();
    for group in &groups {
        let insets: Vec<Vec<[f64; 2]>> = group.iter().map(|ring| inset(ring)).collect();
        let mut holes: Vec<u32> = Vec::new();
        let mut count = 0;
        for ring in &group[..group.len() - 1] {
            count += ring.len();
            holes.push(count as u32);
        }
        // Caps: the inset outline, triangulated as it is, since moving the points of the
        // outline's own triangles could fold thin ones over the holes.
        let capped: Vec<[f64; 2]> = insets.iter().flatten().copied().collect();
        earcut.earcut(capped.iter().copied(), &holes, &mut triangles);
        let caps: &[(usize, f64, f64)] = if depth > 0.0 {
            &[(FRONT, front, -1.0), (BACK, back, 1.0)]
        } else {
            &[(FRONT, 0.0, -1.0)]
        };
        for &(slot, z, nz) in caps {
            let start = mesh.indices.len();
            let base = mesh.vertices.len() as u32;
            for p in &capped {
                let mut uv = planar(*p);
                if nz > 0.0 {
                    uv[0] = 1.0 - uv[0];
                }
                mesh.vertex([p[0], p[1], z], [0.0, 0.0, nz], uv);
            }
            for [a, b, c] in triangles.as_chunks::<3>().0 {
                mesh.triangle(base + a, base + b, base + c);
            }
            mesh.close_part(start, slot, span);
        }
        if depth <= 0.0 {
            continue;
        }
        // Walls and bevels, edge by edge.
        let start = mesh.indices.len();
        for (ring, inner) in group.iter().zip(&insets) {
            let n = ring.len();
            let normals: Vec<[f64; 2]> = (0..n)
                .map(|i| edge_normal(ring[i], ring[(i + 1) % n]))
                .collect();
            // The normal at the `end` (0 start, 1 end) of edge `i`: shared with the
            // neighbouring edge across a shallow corner.
            let corner = |i: usize, end: usize| -> [f64; 2] {
                let own = normals[i];
                let other = if end == 0 {
                    normals[(i + n - 1) % n]
                } else {
                    normals[(i + 1) % n]
                };
                if own[0] * other[0] + own[1] * other[1] >= smooth {
                    let sum = [own[0] + other[0], own[1] + other[1]];
                    let length = sum[0].hypot(sum[1]).max(1e-12);
                    [sum[0] / length, sum[1] / length]
                } else {
                    own
                }
            };
            let mut along = 0.0;
            for i in 0..n {
                let j = (i + 1) % n;
                let length = (ring[j][0] - ring[i][0]).hypot(ring[j][1] - ring[i][1]);
                let (u0, u1) = (along / span[0], (along + length) / span[0]);
                along += length;
                let (n0, n1) = (corner(i, 0), corner(i, 1));
                let v = |z: f64| (z - front) / depth;
                // Bands from front to back, each from one edge to the other.
                let mut bands: Vec<(Edge, Edge)> = Vec::new();
                let outline = [ring[i], ring[j]];
                if bevel > 0.0 {
                    let inner_edge = [inner[i], inner[j]];
                    bands.push((
                        Edge::new(inner_edge, front, -1.0),
                        Edge::new(outline, front + bevel, -1.0),
                    ));
                    bands.push((
                        Edge::new(outline, back - bevel, 1.0),
                        Edge::new(inner_edge, back, 1.0),
                    ));
                }
                if back - bevel > front + bevel + 1e-12 {
                    bands.push((
                        Edge::new(outline, front + bevel, 0.0),
                        Edge::new(outline, back - bevel, 0.0),
                    ));
                }
                for (
                    Edge {
                        points: near,
                        z: z0,
                        tilt: tilt0,
                    },
                    Edge {
                        points: far,
                        z: z1,
                        tilt: tilt1,
                    },
                ) in bands
                {
                    let normal = |side: [f64; 2], tilt: f64| normalize([side[0], side[1], tilt]);
                    let a =
                        mesh.vertex([near[0][0], near[0][1], z0], normal(n0, tilt0), [u0, v(z0)]);
                    let b =
                        mesh.vertex([near[1][0], near[1][1], z0], normal(n1, tilt0), [u1, v(z0)]);
                    let c = mesh.vertex([far[1][0], far[1][1], z1], normal(n1, tilt1), [u1, v(z1)]);
                    let d = mesh.vertex([far[0][0], far[0][1], z1], normal(n0, tilt1), [u0, v(z1)]);
                    mesh.quad(a, b, c, d);
                }
            }
        }
        mesh.close_part(start, MAIN, [0.0, 0.0]);
    }
    merge_parts(mesh)
}

/// One edge of a band of an extrusion's wall or bevel: its two points, their depth, and
/// how far its normal tilts towards the front (-1) or back (1).
struct Edge {
    points: [[f64; 2]; 2],
    z: f64,
    tilt: f64,
}

impl Edge {
    fn new(points: [[f64; 2]; 2], z: f64, tilt: f64) -> Edge {
        Edge { points, z, tilt }
    }
}

/// Joins parts of the same slot that follow each other in the index list.
fn merge_parts(mut mesh: Mesh) -> Mesh {
    // Reorder the indices by slot, keeping the order within a slot.
    let mut by_slot: Vec<(usize, Vec<u32>, [f32; 2])> = Vec::new();
    for part in &mesh.parts {
        let range = part.start as usize..(part.start + part.count) as usize;
        match by_slot.iter_mut().find(|(slot, _, _)| *slot == part.slot) {
            Some((_, indices, _)) => indices.extend_from_slice(&mesh.indices[range]),
            None => by_slot.push((part.slot, mesh.indices[range].to_vec(), part.face)),
        }
    }
    mesh.indices.clear();
    mesh.parts.clear();
    for (slot, indices, face) in by_slot {
        let start = mesh.indices.len() as u32;
        mesh.indices.extend(indices);
        mesh.parts.push(MeshPart {
            start,
            count: mesh.indices.len() as u32 - start,
            slot,
            face,
        });
    }
    mesh
}

/// The outlines of `text` laid out by `engine`, in layout units at [`LAYOUT_SIZE`] with y
/// down, each line centred on x = 0.
pub(crate) fn text_path(
    engine: &mut crate::text::TextEngine,
    text: &fosscut_model::TextObject,
) -> BezPath {
    use cosmic_text::{Attrs, Buffer, Command, Family, Metrics, Shaping, Style, Weight};
    let family = text
        .font
        .clone()
        .unwrap_or_else(|| fosscut_model::BUNDLED_FONT.to_string());
    let attrs = Attrs::new()
        .family(Family::Name(&family))
        .weight(if text.bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        })
        .style(if text.italic {
            Style::Italic
        } else {
            Style::Normal
        });
    let mut buffer = Buffer::new(
        &mut engine.fonts,
        Metrics::new(LAYOUT_SIZE, LAYOUT_SIZE * 1.15),
    );
    buffer.set_size(None, None);
    buffer.set_text(&text.text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(&mut engine.fonts, false);
    let mut path = BezPath::new();
    let runs: Vec<(f32, f32, Vec<cosmic_text::LayoutGlyph>)> = buffer
        .layout_runs()
        .map(|run| (run.line_y, run.line_w, run.glyphs.to_vec()))
        .collect();
    for (line_y, line_w, glyphs) in runs {
        let shift = -line_w / 2.0;
        for glyph in glyphs {
            let key = glyph.physical((0.0, 0.0), 1.0).cache_key;
            let Some(commands) = engine.glyphs.get_outline_commands(&mut engine.fonts, key) else {
                continue;
            };
            let ox = f64::from(glyph.x + glyph.font_size * glyph.x_offset + shift);
            let oy = f64::from(line_y + glyph.y - glyph.font_size * glyph.y_offset);
            let point = |x: f32, y: f32| Point::new(ox + f64::from(x), oy - f64::from(y));
            for command in commands.iter().copied() {
                match command {
                    Command::MoveTo(p) => path.move_to(point(p.x, p.y)),
                    Command::LineTo(p) => path.line_to(point(p.x, p.y)),
                    Command::QuadTo(c, p) => path.quad_to(point(c.x, c.y), point(p.x, p.y)),
                    Command::CurveTo(a, b, p) => {
                        path.curve_to(point(a.x, a.y), point(b.x, b.y), point(p.x, p.y))
                    }
                    Command::Close => path.close_path(),
                }
            }
        }
    }
    path
}

/// The mesh of a built-in object in object units; `None` for model assets, whose mesh
/// comes from their file, and for text, which needs the renderer's fonts.
pub fn solid_mesh(object: &ModelObject) -> Option<Mesh> {
    Some(match object {
        ModelObject::Asset(_) | ModelObject::Text(_) => return None,
        ModelObject::Box(o) => {
            let mut mesh = Mesh::default();
            add_box(&mut mesh, [0.0; 3], o.size, o.bevel);
            mesh
        }
        ModelObject::Sphere(o) => sphere(o.size),
        ModelObject::Cylinder(o) => cylinder(o.size),
        ModelObject::Plane(o) => plane(o.size),
        ModelObject::Torus(o) => torus(o.size, o.thickness),
        ModelObject::Path(o) => {
            let path = BezPath::from_svg(&o.path).unwrap_or_default();
            let bounds = kurbo::Shape::bounding_box(&path);
            let width = bounds.width().max(bounds.height()).max(1e-12);
            let scale = if bounds.width() > 1e-12 {
                o.size / bounds.width()
            } else {
                o.size / width
            };
            let scaled = kurbo::Affine::scale(scale) * path;
            let mut mesh = extrude(&scaled, o.size * 1e-3, o.depth, o.bevel);
            mesh.centre(false);
            mesh
        }
        ModelObject::Blocks(o) => blocks(o),
    })
}

/// The mesh of extruded text in object units, centred on its bounds.
pub(crate) fn text_mesh(
    engine: &mut crate::text::TextEngine,
    text: &fosscut_model::TextObject,
) -> Mesh {
    let scale = text.size / f64::from(LAYOUT_SIZE);
    let path = kurbo::Affine::scale(scale) * text_path(engine, text);
    let mut mesh = extrude(&path, text.size * 2e-3, text.depth, text.bevel);
    mesh.centre(false);
    mesh
}

/// A material as the shader uses it, resolved from the object's settings or the model
/// file.
#[derive(Clone, Debug, PartialEq)]
pub struct Shading {
    /// Linear colour and opacity.
    pub color: [f32; 4],
    /// What it prints: an index into the layer's textures (built-in materials) or into
    /// the model's images (file materials).
    pub texture: Option<Texture>,
    /// How much of a flat face a built-in material's texture covers.
    pub texture_size: f32,
    /// 0 to 1.
    pub metallic: f32,
    /// 0 to 1.
    pub roughness: f32,
    /// Linear light it gives off by itself, strength applied.
    pub emissive: [f32; 3],
    /// Drawn with blending rather than as a solid.
    pub blend: bool,
    /// File materials: coverage below which a pixel is cut away (`MASK`).
    pub cutoff: Option<f32>,
    /// Both sides are shaded as fronts.
    pub double_sided: bool,
}

/// Which texture a material prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Texture {
    /// An image asset of the layer, printed over the colour by its transparency.
    Asset(usize),
    /// An image of the model file, multiplying the colour (glTF's base colour texture).
    File(usize),
}

impl Shading {
    /// A built-in material, with the layer's texture list to find its texture in.
    pub fn of(material: &Material, layer: &ObjectLayer, emissive_strength: f64) -> Shading {
        let [r, g, b, _] = crate::text::linear_color(material.color.as_str());
        let emissive = material
            .emissive
            .as_ref()
            .map(|c| crate::text::linear_color(c.as_str()))
            .map_or([0.0; 3], |[r, g, b, _]| {
                let strength = (material.emissive_strength * emissive_strength) as f32;
                [r * strength, g * strength, b * strength]
            });
        let opacity = material.opacity.clamp(0.0, 1.0) as f32;
        Shading {
            color: [r, g, b, opacity],
            texture: material.texture.as_ref().and_then(|asset| {
                layer
                    .textures
                    .iter()
                    .position(|s| &s.asset == asset)
                    .map(Texture::Asset)
            }),
            texture_size: material.texture_size.clamp(0.05, 1.0) as f32,
            metallic: material.metallic.clamp(0.0, 1.0) as f32,
            roughness: material.roughness.clamp(0.0, 1.0) as f32,
            emissive,
            blend: opacity < 1.0,
            cutoff: None,
            double_sided: false,
        }
    }
}

/// The materials of `layer`'s mesh by slot: [`FRONT`] to [`MAIN`] for built-in solids, the
/// file's materials then its default material for glTF models (or the object's own
/// material everywhere when it has one).
pub fn shadings(layer: &ObjectLayer, model: Option<&ModelData>) -> Vec<Shading> {
    let content = &layer.content;
    let strength = content.emissive_strength;
    let main = content.material.clone().unwrap_or_default();
    match (model, &content.material) {
        // The file's own materials.
        (Some(model), None) => model
            .materials
            .iter()
            .map(|m| Shading {
                emissive: m.emissive.map(|v| v * strength as f32),
                ..m.clone()
            })
            .collect(),
        // The object's own material on every part of the model.
        (Some(model), Some(_)) => vec![Shading::of(&main, layer, strength); model.materials.len()],
        // Faces, then the object's material.
        (None, _) => content
            .faces
            .all()
            .iter()
            .map(|face| Shading::of(face.unwrap_or(&main), layer, strength))
            .chain(std::iter::once(Shading::of(&main, layer, strength)))
            .collect(),
    }
}

/// A decoded image of a model file: RGBA, 8 bits per channel, sRGB, straight alpha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, rows from the top.
    pub rgba: Vec<u8>,
}

/// A glTF model loaded for drawing: one mesh in the file's units, turned to FossCut's axes
/// (x right, y down, z away), with a part per primitive whose slot is its material.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelData {
    /// The mesh; slots index `materials`.
    pub mesh: Mesh,
    /// The file's materials, then the glTF default material.
    pub materials: Vec<Shading>,
    /// The images its materials print.
    pub images: Vec<Arc<ModelImage>>,
    /// What the file holds.
    pub info: ModelInfo,
    /// A hash of the mesh and the number of images, made once when loading, for keying
    /// what the GPU keeps of it.
    pub key: u64,
}

/// Turns glTF's axes (y up, z towards the viewer) into FossCut's (y down, z away).
fn turned(v: [f32; 3]) -> [f32; 3] {
    [v[0], -v[1], -v[2]]
}

/// A column-major 4x4 matrix times a point (`w` 1) or a direction (`w` 0).
fn transformed(m: &[[f32; 4]; 4], v: [f32; 3], w: f32) -> [f32; 3] {
    std::array::from_fn(|row| {
        m[0][row] * v[0] + m[1][row] * v[1] + m[2][row] * v[2] + m[3][row] * w
    })
}

fn column_product(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|column| {
        std::array::from_fn(|row| (0..4).map(|k| a[k][row] * b[column][k]).sum())
    })
}

fn determinant3(m: &[[f32; 4]; 4]) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2])
        - m[1][0] * (m[0][1] * m[2][2] - m[2][1] * m[0][2])
        + m[2][0] * (m[0][1] * m[1][2] - m[1][1] * m[0][2])
}

/// The normal of a direction, for normal matrices: the inverse transpose of the upper
/// 3x3, up to scale (its cofactor matrix), applied to `n`.
fn normal_through(m: &[[f32; 4]; 4], n: [f32; 3]) -> [f32; 3] {
    let c =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[c0][r0] * m[c1][r1] - m[c1][r0] * m[c0][r1];
    let cof = [
        [c(1, 2, 1, 2), -c(1, 2, 0, 2), c(1, 2, 0, 1)],
        [-c(0, 2, 1, 2), c(0, 2, 0, 2), -c(0, 2, 0, 1)],
        [c(0, 1, 1, 2), -c(0, 1, 0, 2), c(0, 1, 0, 1)],
    ];
    let out: [f32; 3] = std::array::from_fn(|row| (0..3).map(|k| cof[row][k] * n[k]).sum());
    let length = (out[0] * out[0] + out[1] * out[1] + out[2] * out[2]).sqrt();
    if length > 1e-12 {
        out.map(|v| v / length)
    } else {
        [0.0, 0.0, -1.0]
    }
}

fn rgba8(image: &gltf::image::Data) -> ModelImage {
    use gltf::image::Format;
    let pixels = (image.width * image.height) as usize;
    let p = &image.pixels;
    let float = |bytes: &[u8]| -> u8 {
        let v = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        // Linear float images are written as sRGB 8-bit, like the PNGs and JPEGs.
        let encoded = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    let mut rgba = Vec::with_capacity(pixels * 4);
    for i in 0..pixels {
        let pixel: [u8; 4] = match image.format {
            Format::R8 => [p[i], p[i], p[i], 255],
            Format::R8G8 => [p[2 * i], p[2 * i], p[2 * i], p[2 * i + 1]],
            Format::R8G8B8 => [p[3 * i], p[3 * i + 1], p[3 * i + 2], 255],
            Format::R8G8B8A8 => [p[4 * i], p[4 * i + 1], p[4 * i + 2], p[4 * i + 3]],
            Format::R16 => [p[2 * i + 1], p[2 * i + 1], p[2 * i + 1], 255],
            Format::R16G16 => [p[4 * i + 1], p[4 * i + 1], p[4 * i + 1], p[4 * i + 3]],
            Format::R16G16B16 => [p[6 * i + 1], p[6 * i + 3], p[6 * i + 5], 255],
            Format::R16G16B16A16 => [p[8 * i + 1], p[8 * i + 3], p[8 * i + 5], p[8 * i + 7]],
            Format::R32G32B32FLOAT => {
                let at = 12 * i;
                [
                    float(&p[at..]),
                    float(&p[at + 4..]),
                    float(&p[at + 8..]),
                    255,
                ]
            }
            Format::R32G32B32A32FLOAT => {
                let at = 16 * i;
                let alpha = f32::from_le_bytes([p[at + 12], p[at + 13], p[at + 14], p[at + 15]]);
                [
                    float(&p[at..]),
                    float(&p[at + 4..]),
                    float(&p[at + 8..]),
                    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
                ]
            }
        };
        rgba.extend_from_slice(&pixel);
    }
    ModelImage {
        width: image.width,
        height: image.height,
        rgba,
    }
}

impl ModelData {
    /// Loads a `.glb` file, or a `.gltf` file with the buffers and images it names beside
    /// it.
    pub fn load(path: &std::path::Path) -> Result<ModelData, String> {
        let (document, buffers, images) = gltf::import(path).map_err(|e| e.to_string())?;
        ModelData::from_parts(&document, &buffers, &images)
    }

    /// Reads a `.glb` file, or a `.gltf` file whose buffers and images are embedded, from
    /// memory.
    pub fn from_slice(data: &[u8]) -> Result<ModelData, String> {
        let (document, buffers, images) = gltf::import_slice(data).map_err(|e| e.to_string())?;
        ModelData::from_parts(&document, &buffers, &images)
    }

    fn from_parts(
        document: &gltf::Document,
        buffers: &[gltf::buffer::Data],
        images: &[gltf::image::Data],
    ) -> Result<ModelData, String> {
        let mut materials: Vec<Shading> = document
            .materials()
            .map(|m| {
                let pbr = m.pbr_metallic_roughness();
                let [r, g, b, a] = pbr.base_color_factor();
                let blend = m.alpha_mode() == gltf::material::AlphaMode::Blend;
                let cutoff = (m.alpha_mode() == gltf::material::AlphaMode::Mask)
                    .then(|| m.alpha_cutoff().unwrap_or(0.5));
                Shading {
                    color: [r, g, b, if blend || cutoff.is_some() { a } else { 1.0 }],
                    texture: pbr
                        .base_color_texture()
                        .map(|info| Texture::File(info.texture().source().index())),
                    texture_size: 1.0,
                    metallic: pbr.metallic_factor().clamp(0.0, 1.0),
                    roughness: pbr.roughness_factor().clamp(0.0, 1.0),
                    emissive: m.emissive_factor(),
                    blend,
                    cutoff,
                    double_sided: m.double_sided(),
                }
            })
            .collect();
        // glTF's default material: white, fully metallic and fully rough.
        let default = materials.len();
        materials.push(Shading {
            color: [1.0; 4],
            texture: None,
            texture_size: 1.0,
            metallic: 1.0,
            roughness: 1.0,
            emissive: [0.0; 3],
            blend: false,
            cutoff: None,
            double_sided: false,
        });
        let scene = document
            .default_scene()
            .or_else(|| document.scenes().next())
            .ok_or("the file has no scene")?;
        let mut mesh = Mesh::default();
        let mut meshes = 0u32;
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let mut pending: Vec<(gltf::Node, [[f32; 4]; 4])> =
            scene.nodes().map(|n| (n, identity)).collect();
        let mut visited = 0usize;
        while let Some((node, parent)) = pending.pop() {
            visited += 1;
            if visited > 100_000 {
                return Err("the scene's nodes repeat".into());
            }
            let world = column_product(&parent, &node.transform().matrix());
            for child in node.children() {
                pending.push((child, world));
            }
            let Some(node_mesh) = node.mesh() else {
                continue;
            };
            meshes += 1;
            let mirrored = determinant3(&world) < 0.0;
            for primitive in node_mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let Some(positions) = reader.read_positions() else {
                    continue;
                };
                let positions: Vec<[f32; 3]> = positions.collect();
                let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(Iterator::collect);
                let uvs: Option<Vec<[f32; 2]>> =
                    reader.read_tex_coords(0).map(|t| t.into_f32().collect());
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(indices) => indices.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                if indices.iter().any(|&i| i as usize >= positions.len()) {
                    return Err("a primitive's indices point past its vertices".into());
                }
                let base = mesh.vertices.len() as u32;
                let start = mesh.indices.len();
                let mut smooth = vec![[0.0f32; 3]; positions.len()];
                if normals.is_none() {
                    for t in indices.as_chunks::<3>().0 {
                        let [a, b, c] = t.map(|i| positions[i as usize]);
                        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                        let n = [
                            e1[1] * e2[2] - e1[2] * e2[1],
                            e1[2] * e2[0] - e1[0] * e2[2],
                            e1[0] * e2[1] - e1[1] * e2[0],
                        ];
                        for &i in t {
                            for axis in 0..3 {
                                smooth[i as usize][axis] += n[axis];
                            }
                        }
                    }
                }
                for (i, position) in positions.iter().enumerate() {
                    let normal = normals.as_ref().map_or(smooth[i], |n| n[i]);
                    mesh.vertices.push(MeshVertex {
                        position: turned(transformed(&world, *position, 1.0)),
                        normal: turned(normal_through(&world, normal)),
                        uv: uvs.as_ref().map_or([0.0, 0.0], |uv| uv[i]),
                    });
                }
                for t in indices.as_chunks::<3>().0 {
                    let (b, c) = if mirrored { (t[2], t[1]) } else { (t[1], t[2]) };
                    mesh.triangle(base + t[0], base + b, base + c);
                }
                let slot = primitive.material().index().unwrap_or(default);
                mesh.close_part(start, slot, [0.0, 0.0]);
            }
        }
        let size = mesh.bounds().map_or([0.0; 3], |(low, high)| {
            [high[0] - low[0], high[1] - low[1], high[2] - low[2]]
        });
        let asset = &document.as_json().asset;
        let info = ModelInfo {
            meshes,
            triangles: mesh.triangles() as u64,
            materials: document.materials().count() as u32,
            textures: images.len() as u32,
            size,
            generator: asset.generator.clone(),
            copyright: asset.copyright.clone(),
        };
        if mesh.indices.is_empty() {
            return Err("the file's scene draws no triangles".into());
        }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytemuck::cast_slice::<MeshVertex, u8>(&mesh.vertices).hash(&mut hasher);
            mesh.indices.hash(&mut hasher);
            images.len().hash(&mut hasher);
            hasher.finish()
        };
        Ok(ModelData {
            mesh,
            materials,
            images: images.iter().map(|i| Arc::new(rgba8(i))).collect(),
            info,
            key,
        })
    }

    /// The offset and scale that centre the model on its bounds and make its longest side
    /// `size` object units.
    pub fn fit(&self, size: f64) -> (Vec3, f64) {
        match self.mesh.bounds() {
            Some((low, high)) => {
                let longest = (0..3).map(|a| high[a] - low[a]).fold(0.0, f64::max);
                let centre: Vec3 = std::array::from_fn(|a| -(low[a] + high[a]) / 2.0);
                (centre, if longest > 1e-12 { size / longest } else { 1.0 })
            }
            None => ([0.0; 3], 1.0),
        }
    }
}

/// From object space to the scene: object units to canvas pixels (the canvas width per
/// unit), the model's own fit for a glTF model (`fit`: offset, then scale), then the
/// clip's placement by [`layer_model`]. Depth scales with the uniform `scale`; the
/// anchor is a fraction of the object's width and height (`size`, object units).
pub fn object_model(
    canvas: (u32, u32),
    transform: &Transform,
    size: [f64; 2],
    fit: (Vec3, f64),
) -> Mat4 {
    let w = f64::from(canvas.0.max(1));
    let units = w * fit.1;
    let (o, s) = (fit.0, units);
    let to_pixels: Mat4 = [
        [s, 0.0, 0.0, o[0] * s],
        [0.0, s, 0.0, o[1] * s],
        [0.0, 0.0, s * transform.scale, o[2] * s * transform.scale],
        [0.0, 0.0, 0.0, 1.0],
    ];
    multiply(
        &layer_model(canvas, transform, (size[0] * w, size[1] * w)),
        &to_pixels,
    )
}

/// The level of an object's floor in the scene (its y in canvas pixels): below the lowest
/// point of the object at rest (placed and scaled, not turned) by the floor's gap.
/// `bounds` are the object's in object units.
pub fn floor_level(
    canvas: (u32, u32),
    transform: &Transform,
    bounds: (Vec3, Vec3),
    gap: f64,
) -> f64 {
    let rest = Transform {
        rotation: 0.0,
        rotation_x: 0.0,
        rotation_y: 0.0,
        ..transform.clone()
    };
    let w = f64::from(canvas.0.max(1));
    let (low, high) = bounds;
    let at_rest = object_model(
        canvas,
        &rest,
        [high[0] - low[0], high[1] - low[1]],
        ([0.0; 3], 1.0),
    );
    let bottom = apply(&at_rest, [0.0, high[1], 0.0]);
    bottom[1] + gap * w
}

/// The corners of the box `bounds` placed by `model`, in the scene.
pub fn placed_corners(model: &Mat4, bounds: (Vec3, Vec3)) -> Vec<Vec3> {
    let (low, high) = bounds;
    (0..8)
        .map(|i| {
            let p = [
                if i & 1 == 0 { low[0] } else { high[0] },
                if i & 2 == 0 { low[1] } else { high[1] },
                if i & 4 == 0 { low[2] } else { high[2] },
            ];
            let q = apply(model, p);
            [q[0], q[1], q[2]]
        })
        .collect()
}

/// An orthographic view from the key light, towards `-direction`, covering the sphere
/// around `centre` of `radius`, as a wgpu clip matrix: x and y to -1..1 across the sphere,
/// depth from 0 on the light's side of the sphere to 1 far beyond it, so floors under the
/// object are inside.
pub fn light_view(direction: Vec3, centre: Vec3, radius: f64) -> Mat4 {
    let forward = normalize(direction.map(|v| -v));
    let helper = if forward[1].abs() > 0.99 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let cross = |a: Vec3, b: Vec3| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let right = normalize(cross(helper, forward));
    let up = cross(forward, right);
    let dot = |a: Vec3, b: Vec3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let r = radius.max(1e-6);
    // Depth runs over the sphere and eight radii beyond it.
    let (near, far) = (-r, 9.0 * r);
    let row = |axis: Vec3, scale: f64| {
        [
            axis[0] * scale,
            axis[1] * scale,
            axis[2] * scale,
            -dot(axis, centre) * scale,
        ]
    };
    let depth = |axis: Vec3| {
        let s = 1.0 / (far - near);
        [
            axis[0] * s,
            axis[1] * s,
            axis[2] * s,
            (-dot(axis, centre) - near) * s,
        ]
    };
    [
        row(right, 1.0 / r),
        row(up, 1.0 / r),
        depth(forward),
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// A small glTF model made by FossCut for tests (its own geometry, free to use): a house of
/// a box body one unit wide, deep and high standing on y = 0, with flat normals and texture
/// coordinates, and a pyramid roof 0.6 high without normals, each in a node of its own under
/// a parent node and with a material of its own. With `texture` (PNG bytes) the walls print
/// it. Returns the JSON document and its binary buffer: `buffer` names the buffer's file
/// (`None` leaves the buffer for a `.glb`), `image` the texture's file (`None` keeps the
/// image in the buffer).
#[doc(hidden)]
pub fn fixture_gltf(
    texture: Option<&[u8]>,
    buffer: Option<&str>,
    image: Option<&str>,
) -> (String, Vec<u8>) {
    let mut bin: Vec<u8> = Vec::new();
    fn put(bin: &mut Vec<u8>, values: &[f32]) {
        for v in values {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    // Body: six faces of four corners, glTF axes (y up, z towards the viewer).
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    let corners = [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for (n, u, v) in faces {
        for (a, b) in corners {
            let p: [f32; 3] = std::array::from_fn(|k| n[k] * 0.5 + u[k] * a * 0.5 + v[k] * b * 0.5);
            positions.extend_from_slice(&[p[0], p[1] + 0.5, p[2]]);
            normals.extend_from_slice(&n);
            uvs.extend_from_slice(&[(a + 1.0) / 2.0, (1.0 - b) / 2.0]);
        }
    }
    put(&mut bin, &positions);
    put(&mut bin, &normals);
    put(&mut bin, &uvs);
    for face in 0..6u16 {
        for i in [0u16, 1, 2, 0, 2, 3] {
            bin.extend_from_slice(&(face * 4 + i).to_le_bytes());
        }
    }
    // Roof: four sloping sides and a base of two triangles, unindexed and without normals.
    let apex = [0.0f32, 1.6, 0.0];
    let base = [
        [-0.55f32, 1.0, -0.55],
        [0.55, 1.0, -0.55],
        [0.55, 1.0, 0.55],
        [-0.55, 1.0, 0.55],
    ];
    let mut roof: Vec<f32> = Vec::new();
    for i in 0..4 {
        let (a, b) = (base[i], base[(i + 1) % 4]);
        // Counterclockwise seen from outside.
        roof.extend_from_slice(&b);
        roof.extend_from_slice(&a);
        roof.extend_from_slice(&apex);
    }
    for [a, b, c] in [[0, 1, 2], [0, 2, 3]] {
        roof.extend_from_slice(&base[a]);
        roof.extend_from_slice(&base[b]);
        roof.extend_from_slice(&base[c]);
    }
    let roof_start = bin.len();
    put(&mut bin, &roof);
    let image_start = bin.len();
    let embedded = texture.filter(|_| image.is_none());
    if let Some(png) = embedded {
        bin.extend_from_slice(png);
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
    }
    let view = |offset: usize, length: usize, target: &str| {
        format!(r#"{{"buffer": 0, "byteOffset": {offset}, "byteLength": {length}{target}}}"#)
    };
    let mut views = vec![
        view(0, 288, r#", "target": 34962"#),
        view(288, 288, r#", "target": 34962"#),
        view(576, 192, r#", "target": 34962"#),
        view(768, 72, r#", "target": 34963"#),
        view(roof_start, 216, r#", "target": 34962"#),
    ];
    if let Some(png) = embedded {
        views.push(view(image_start, png.len(), ""));
    }
    let accessors = [
        r#"{"bufferView": 0, "componentType": 5126, "count": 24, "type": "VEC3", "min": [-0.5, 0, -0.5], "max": [0.5, 1, 0.5]}"#,
        r#"{"bufferView": 1, "componentType": 5126, "count": 24, "type": "VEC3"}"#,
        r#"{"bufferView": 2, "componentType": 5126, "count": 24, "type": "VEC2"}"#,
        r#"{"bufferView": 3, "componentType": 5123, "count": 36, "type": "SCALAR"}"#,
        r#"{"bufferView": 4, "componentType": 5126, "count": 18, "type": "VEC3", "min": [-0.55, 1, -0.55], "max": [0.55, 1.6, 0.55]}"#,
    ];
    let wall_texture = if texture.is_some() {
        r#", "baseColorTexture": {"index": 0}"#
    } else {
        ""
    };
    let images = match (texture, image) {
        (None, _) => String::new(),
        (Some(_), Some(uri)) => {
            format!(r#", "images": [{{"uri": "{uri}"}}], "textures": [{{"source": 0}}]"#)
        }
        (Some(_), None) => format!(
            r#", "images": [{{"bufferView": {}, "mimeType": "image/png"}}], "textures": [{{"source": 0}}]"#,
            views.len() - 1
        ),
    };
    let uri = buffer.map_or(String::new(), |uri| format!(r#", "uri": "{uri}""#));
    let json = format!(
        r#"{{"asset": {{"version": "2.0", "generator": "FossCut test fixture", "copyright": "The FossCut Contributors"}},
"scene": 0,
"scenes": [{{"nodes": [0]}}],
"nodes": [{{"children": [1, 2], "translation": [0, -0.8, 0]}}, {{"mesh": 0}}, {{"mesh": 1}}],
"meshes": [
  {{"primitives": [{{"attributes": {{"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}}, "indices": 3, "material": 0}}]}},
  {{"primitives": [{{"attributes": {{"POSITION": 4}}, "material": 1}}]}}
],
"materials": [
  {{"pbrMetallicRoughness": {{"baseColorFactor": [0.9, 0.75, 0.5, 1], "metallicFactor": 0, "roughnessFactor": 0.8{wall_texture}}}}},
  {{"pbrMetallicRoughness": {{"baseColorFactor": [0.7, 0.12, 0.08, 1], "metallicFactor": 0, "roughnessFactor": 0.5}}}}
],
"buffers": [{{"byteLength": {length}{uri}}}],
"bufferViews": [{views}],
"accessors": [{accessors}]{images}}}"#,
        length = bin.len(),
        views = views.join(", "),
        accessors = accessors.join(", "),
    );
    (json, bin)
}

/// [`fixture_gltf`] as one `.glb` file.
#[doc(hidden)]
pub fn fixture_glb(texture: Option<&[u8]>) -> Vec<u8> {
    let (json, bin) = fixture_gltf(texture, None, None);
    let mut json = json.into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    glb
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every normal is a unit vector and points away from `centre(position)`, the point
    /// of the solid's core nearest to it.
    fn closed_and_outward(mesh: &Mesh, centre: impl Fn(Vec3) -> Vec3) {
        for v in &mesh.vertices {
            let n = v.normal.map(f64::from);
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((length - 1.0).abs() < 1e-4, "{n:?}");
            let position = v.position.map(f64::from);
            let core = centre(position);
            let p: Vec3 = std::array::from_fn(|a| position[a] - core[a]);
            assert!(n[0] * p[0] + n[1] * p[1] + n[2] * p[2] > -1e-6, "{v:?}");
        }
        assert!(
            mesh.indices
                .iter()
                .all(|&i| (i as usize) < mesh.vertices.len())
        );
        let covered: u32 = mesh.parts.iter().map(|p| p.count).sum();
        assert_eq!(covered as usize, mesh.indices.len());
    }

    #[test]
    fn solids_have_their_size_and_outward_normals() {
        let cases = [
            (
                r#"{"kind": "box", "size": [0.3, 0.2, 0.1]}"#,
                [0.3, 0.2, 0.1],
            ),
            (
                r#"{"kind": "box", "size": [0.3, 0.2, 0.1], "bevel": 0.02}"#,
                [0.3, 0.2, 0.1],
            ),
            (r#"{"kind": "sphere", "size": 0.5}"#, [0.5, 0.5, 0.5]),
            (
                r#"{"kind": "cylinder", "size": [0.2, 0.4]}"#,
                [0.2, 0.4, 0.2],
            ),
            (
                r#"{"kind": "torus", "size": 0.4, "thickness": 0.1}"#,
                [0.4, 0.4, 0.1],
            ),
        ];
        for (json, size) in cases {
            let object: ModelObject = serde_json::from_str(json).unwrap();
            let mesh = solid_mesh(&object).unwrap();
            let (low, high) = mesh.bounds().unwrap();
            for axis in 0..3 {
                assert!(
                    ((high[axis] - low[axis]) - size[axis]).abs() < 1e-3,
                    "{json}: {low:?} {high:?}"
                );
                assert!((high[axis] + low[axis]).abs() < 1e-6, "{json} is centred");
            }
            if object.kind_name() == "torus" {
                // The middle of the tube nearest to the point.
                closed_and_outward(&mesh, |p| {
                    let length = p[0].hypot(p[1]).max(1e-12);
                    [0.15 * p[0] / length, 0.15 * p[1] / length, 0.0]
                });
            } else {
                closed_and_outward(&mesh, |_| [0.0; 3]);
            }
        }
        // A box has a part per face, each with its face's size.
        let object: ModelObject =
            serde_json::from_str(r#"{"kind": "box", "size": [0.3, 0.2, 0.1]}"#).unwrap();
        let mesh = solid_mesh(&object).unwrap();
        let slots: Vec<usize> = mesh.parts.iter().map(|p| p.slot).collect();
        assert_eq!(slots, vec![FRONT, BACK, LEFT, RIGHT, TOP, BOTTOM]);
        assert_eq!(mesh.parts[0].face, [0.3, 0.2]);
        assert_eq!(mesh.parts[TOP].face, [0.3, 0.1]);
    }

    #[test]
    fn a_bevelled_box_rounds_its_corners() {
        let mut mesh = Mesh::default();
        add_box(&mut mesh, [0.0; 3], [1.0, 1.0, 1.0], 0.2);
        // No vertex lies further out than the corner sphere allows.
        let limit = 0.3f64 * 3f64.sqrt() + 0.2;
        for v in &mesh.vertices {
            let p = v.position.map(f64::from);
            assert!((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() <= limit + 1e-6);
        }
        assert!(
            mesh.vertices
                .iter()
                .all(|v| v.position.iter().all(|c| c.abs() <= 0.5 + 1e-6))
        );
    }

    #[test]
    fn extrusion_caps_holes_and_bevels() {
        // A square with a square hole.
        let path = BezPath::from_svg("M0 0 L10 0 L10 10 L0 10 Z M3 3 L7 3 L7 7 L3 7 Z").unwrap();
        let flat = extrude(&path, 0.01, 0.0, 0.0);
        assert_eq!(flat.parts.len(), 1, "only a front cap without depth");
        // The cap covers the square minus the hole: 100 - 16.
        let covered: f64 = flat
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| flat.vertices[i as usize].position);
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() as f64 / 2.0
            })
            .sum();
        assert!((covered - 84.0).abs() < 1e-3, "{covered}");

        let solid = extrude(&path, 0.01, 2.0, 0.5);
        let slots: Vec<usize> = solid.parts.iter().map(|p| p.slot).collect();
        assert_eq!(slots, vec![FRONT, BACK, MAIN]);
        let (low, high) = solid.bounds().unwrap();
        assert!((low[2] + 1.0).abs() < 1e-6 && (high[2] - 1.0).abs() < 1e-6);
        // The caps are inset by the bevel; the walls keep the outline.
        let front = &solid.parts[0];
        let cap: Vec<[f32; 3]> = solid.indices
            [front.start as usize..(front.start + front.count) as usize]
            .iter()
            .map(|&i| solid.vertices[i as usize].position)
            .collect();
        assert!(cap.iter().all(|p| p[0] >= 0.49 && p[0] <= 9.51));
        // Wall normals point out of the ring and into the hole.
        for v in &solid.vertices {
            if v.normal[2].abs() < 1e-6 {
                let [x, y, _] = v.position;
                let n = v.normal;
                let outer = x <= 0.01 || x >= 9.99 || y <= 0.01 || y >= 9.99;
                let towards_middle = n[0] * (5.0 - x) + n[1] * (5.0 - y);
                if outer {
                    assert!(towards_middle < 0.0, "{v:?}");
                } else {
                    assert!(towards_middle > 0.0, "{v:?}");
                }
            }
        }
    }

    /// The front cap's area, and the area its outline encloses (the even-odd area of the
    /// flattened rings).
    fn cap_and_outline(mesh: &Mesh, path: &BezPath) -> (f64, f64) {
        let front = mesh.parts.iter().find(|p| p.slot == FRONT).unwrap();
        let cap: f64 = mesh.indices[front.start as usize..(front.start + front.count) as usize]
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.vertices[i as usize].position);
                f64::from((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum();
        let outline: f64 = polygons(rings(path, 0.5))
            .iter()
            .flatten()
            .map(|ring| area(ring))
            .sum();
        (cap, outline)
    }

    #[test]
    fn glyph_caps_cover_their_outlines_exactly() {
        let mut engine = crate::text::TextEngine::new();
        for text in ["O", "B", "e", "8", "%", "@"] {
            let object: fosscut_model::TextObject =
                serde_json::from_value(serde_json::json!({"text": text, "bold": true})).unwrap();
            let path = text_path(&mut engine, &object);
            let mesh = extrude(&path, 0.5, 10.0, 0.0);
            let (cap, outline) = cap_and_outline(&mesh, &path);
            assert!(outline > 1000.0, "{text}: {outline}");
            assert!(
                (cap - outline).abs() < outline * 1e-3,
                "{text}: cap {cap}, outline {outline}"
            );
            // With a bevel the cap shrinks, and none of its triangles folds over.
            let bevelled = extrude(&path, 0.5, 10.0, 8.0);
            let front = bevelled.parts.iter().find(|p| p.slot == FRONT).unwrap();
            let signed: Vec<f64> = bevelled.indices
                [front.start as usize..(front.start + front.count) as usize]
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| {
                    let [a, b, c] = t.map(|i| bevelled.vertices[i as usize].position);
                    f64::from((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]))
                })
                .collect();
            let turning = signed.iter().filter(|a| **a > 1e-6).count();
            let against = signed.iter().filter(|a| **a < -1e-6).count();
            assert!(
                turning == 0 || against == 0,
                "{text}: {turning} and {against}"
            );
            let (smaller, _) = cap_and_outline(&bevelled, &path);
            assert!(smaller < cap, "{text}");
        }
    }

    #[test]
    fn text_extrudes_in_the_bundled_font() {
        let mut engine = crate::text::TextEngine::new();
        let text: fosscut_model::TextObject = serde_json::from_str(
            r#"{"text": "OB\nA", "size": 0.1, "depth": 0.02, "bevel": 0.002}"#,
        )
        .unwrap();
        let mesh = text_mesh(&mut engine, &text);
        assert!(mesh.triangles() > 200, "{}", mesh.triangles());
        let (low, high) = mesh.bounds().unwrap();
        assert!((low[0] + high[0]).abs() < 1e-6 && (low[1] + high[1]).abs() < 1e-6);
        assert!((high[2] - low[2] - 0.02).abs() < 1e-6);
        // Two lines of capitals about 0.07 high each, with the gap between.
        assert!(
            high[1] - low[1] > 0.12 && high[1] - low[1] < 0.2,
            "{low:?} {high:?}"
        );
        let slots: Vec<usize> = mesh.parts.iter().map(|p| p.slot).collect();
        assert_eq!(slots, vec![FRONT, BACK, MAIN]);
    }

    #[test]
    fn grids_are_chosen_by_their_seed() {
        let object = BlocksObject {
            fill: 0.7,
            ..BlocksObject::default()
        };
        let heights = block_heights(&object);
        assert_eq!(heights, block_heights(&object), "the same every time");
        let other = block_heights(&BlocksObject {
            seed: 1,
            ..object.clone()
        });
        assert_ne!(heights, other);
        let filled = heights.iter().flatten().filter(|h| h.is_some()).count();
        assert!((30..=60).contains(&filled), "{filled} of 64");
        for height in heights.iter().flatten().flatten() {
            assert!((0.02..=0.2).contains(height));
        }
        let mesh = blocks(&object);
        let (low, high) = mesh.bounds().unwrap();
        assert!(high[1].abs() < 1e-6, "they stand on y = 0");
        assert!(low[1] < -0.02);
        let grown = block_heights(&BlocksObject {
            growth: 0.5,
            ..object.clone()
        });
        for (half, full) in grown.iter().flatten().zip(heights.iter().flatten()) {
            assert_eq!(
                half.map(|h| (h * 2.0 * 1e9).round()),
                full.map(|h| (h * 1e9).round())
            );
        }
    }

    #[test]
    fn a_glb_loads_with_its_nodes_materials_and_axes() {
        let model = ModelData::from_slice(&fixture_glb(None)).unwrap();
        assert_eq!(model.info.meshes, 2);
        assert_eq!(model.info.triangles, 12 + 6);
        assert_eq!(model.info.materials, 2);
        assert_eq!(
            model.info.generator.as_deref(),
            Some("FossCut test fixture")
        );
        // The parent node moves both down 0.8; glTF's up is FossCut's -y.
        let (low, high) = model.mesh.bounds().unwrap();
        assert!(
            (high[1] - 0.8).abs() < 1e-5 && (low[1] + 0.8).abs() < 1e-5,
            "{low:?} {high:?}"
        );
        assert!((model.info.size[1] - 1.6).abs() < 1e-5);
        let slots: Vec<usize> = model.mesh.parts.iter().map(|p| p.slot).collect();
        assert_eq!(slots.len(), 2);
        assert!(slots.contains(&0) && slots.contains(&1));
        // Computed roof normals point out of the roof.
        let roof = model.mesh.parts.iter().find(|p| p.slot == 1).unwrap();
        for &i in &model.mesh.indices[roof.start as usize..(roof.start + roof.count) as usize] {
            let v = model.mesh.vertices[i as usize];
            // From a point inside the pyramid.
            let (p, n) = (v.position, v.normal);
            let outward = n[0] * p[0] + n[1] * (p[1] + 0.35) + n[2] * p[2];
            assert!(outward > 0.0, "{v:?}");
        }
        assert_eq!(model.materials.len(), 3, "the file's two and the default");
        let (offset, scale) = model.fit(0.4);
        assert!((scale - 0.4 / 1.6).abs() < 1e-6);
        assert!(offset[1].abs() < 1e-6);
        assert!(ModelData::from_slice(b"not a model").is_err());
    }

    #[test]
    fn objects_are_placed_in_canvas_pixels_and_floors_lie_under_them() {
        let canvas = (1920, 1080);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        let placed = Transform {
            position: [0.1, 0.2],
            position_z: 0.5,
            scale: 2.0,
            ..Transform::default()
        };
        let m = object_model(canvas, &placed, [0.2, 0.1], ([0.0; 3], 1.0));
        // The origin goes to the position; an object unit is a canvas width, and depth
        // scales with the uniform scale like width and height.
        let origin = apply(&m, [0.0; 3]);
        assert!(close(origin[0], 192.0) && close(origin[1], 216.0) && close(origin[2], 960.0));
        let corner = apply(&m, [0.1, 0.05, 0.1]);
        assert!(close(corner[0], 192.0 + 384.0) && close(corner[2], 960.0 + 384.0));
        // The floor lies under the lowest point at rest, whatever the object's turn.
        let bounds = ([-0.1, -0.05, -0.1], [0.1, 0.05, 0.1]);
        let level = floor_level(canvas, &placed, bounds, 0.01);
        assert!(close(level, 216.0 + 0.05 * 1920.0 * 2.0 + 19.2), "{level}");
        let turned = Transform {
            rotation_x: 40.0,
            rotation_y: 25.0,
            ..placed.clone()
        };
        assert!(close(floor_level(canvas, &turned, bounds, 0.01), level));
        // A fitted model is moved by its offset before scaling.
        let fitted = object_model(
            canvas,
            &Transform::default(),
            [1.0, 1.0],
            ([0.0, -0.8, 0.0], 0.25),
        );
        assert!(close(apply(&fitted, [0.0, 0.8, 0.0])[1], 0.0));
    }

    #[test]
    fn the_light_view_frames_the_object() {
        let view = light_view([0.0, -1.0, 0.0], [10.0, 20.0, 30.0], 5.0);
        // From above: the centre maps to the middle at depth near 0.1, the top of the sphere
        // to depth 0.
        let centre = apply(&view, [10.0, 20.0, 30.0]);
        assert!(centre[0].abs() < 1e-9 && centre[1].abs() < 1e-9);
        assert!((centre[2] - 0.1).abs() < 1e-9);
        let top = apply(&view, [10.0, 15.0, 30.0]);
        assert!(top[2].abs() < 1e-9);
        let side = apply(&view, [15.0, 20.0, 30.0]);
        assert!((side[0].abs() - 1.0).abs() < 1e-9);
    }
}
