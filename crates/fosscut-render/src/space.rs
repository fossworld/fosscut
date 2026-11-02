//! Layers placed in 3D (ADR-0037): cameras, the projection of a layer seen through one,
//! the order of 3D layers by depth, and the blur of the depth of field.
//!
//! # Coordinates
//!
//! The scene is measured in pixels of the timeline's canvas: the origin at the canvas
//! centre, x to the right, y downwards and z away from the viewer. A clip at
//! `transform.position` `[px, py]` and `transform.position_z` `pz` sits at
//! `(px * width, py * height, pz * width)`. The camera looks from its eye towards its
//! target; view space has x to the right of the picture, y down it and z along the view.
//! [`Camera::projection`] maps view space to clip space with `w = z / f`, where `f` is the
//! focal length in canvas pixels, so a point on the plane `f` in front of the camera keeps
//! `w = 1` and lands where a 2D layer would. The default camera ([`Camera::default_for`])
//! sits at that distance in front of the z = 0 plane, so a 3D layer at z = 0 with no turn
//! renders exactly like the 2D path.
//!
//! # Layers in the scene
//!
//! The planner gives a layer a [`LayerSpace`] when its transform is spatial
//! ([`fosscut_model::Transform::is_spatial`]): 3D layers (`three_d`) see the timeline's
//! camera, other spatial layers the default camera on their own. Each run of consecutive
//! 3D layers in a plan is drawn farthest first by [`LayerSpace::depth`], keeping track
//! order between equal depths; any other layer between them ends the run, as in classic
//! motion-design tools. Layers are flat, so intersecting layers are not split.
//!
//! # Other layer kinds
//!
//! A future layer kind that draws real 3D objects (glTF models, primitives) takes part
//! the same way: the planner sets its `space` with `scene: true` and a `depth` (for
//! example the view depth of its bounding box centre from [`Camera::view_depth`]), and
//! [`order_by_depth`] sorts it among the planar layers of its run. To draw, it renders its
//! triangles with [`Camera::clip_from_world`] (and a depth buffer of its own) into a
//! canvas-sized texture that is composited like any layer. [`camera_at`] gives the camera
//! of a timeline at a time for anything that needs it outside a plan.

use fosscut_model::{CameraContent, ClipContent, Project, Timeline, TrackKind, Transform};
use fosscut_time::Flicks;

use crate::plan::{Layer, LayerSource, RenderPlan};

/// A point or direction in scene coordinates.
pub type Vec3 = [f64; 3];

/// A 4x4 matrix in rows, applied to column vectors: `p' = M p`.
pub type Mat4 = [[f64; 4]; 4];

/// The nearest a point can be to the camera and still be drawn, as a fraction of the
/// canvas width; nearer parts of a layer are clipped.
pub const NEAR_PLANE: f64 = 0.01;

/// The identity matrix.
pub const IDENTITY: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// `a * b`: first `b`, then `a`.
pub fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (row, out_row) in out.iter_mut().enumerate() {
        for (column, value) in out_row.iter_mut().enumerate() {
            *value = (0..4).map(|k| a[row][k] * b[k][column]).sum();
        }
    }
    out
}

/// `m` applied to the point `p` (w = 1), in homogeneous coordinates.
pub fn apply(m: &Mat4, p: Vec3) -> [f64; 4] {
    let mut out = [0.0; 4];
    for (row, value) in out.iter_mut().enumerate() {
        *value = m[row][0] * p[0] + m[row][1] * p[1] + m[row][2] * p[2] + m[row][3];
    }
    out
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalized(v: Vec3) -> Option<Vec3> {
    let length = dot(v, v).sqrt();
    (length > 1e-9 && length.is_finite()).then(|| [v[0] / length, v[1] / length, v[2] / length])
}

fn translation(t: Vec3) -> Mat4 {
    [
        [1.0, 0.0, 0.0, t[0]],
        [0.0, 1.0, 0.0, t[1]],
        [0.0, 0.0, 1.0, t[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn scaling(x: f64, y: f64) -> Mat4 {
    [
        [x, 0.0, 0.0, 0.0],
        [0.0, y, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Turns clockwise on the canvas (y down) by `degrees` around the z axis: the clip's
/// `rotation`.
pub fn rotation_z(degrees: f64) -> Mat4 {
    let (s, c) = degrees.to_radians().sin_cos();
    [
        [c, -s, 0.0, 0.0],
        [s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Tilts by `degrees` around the x axis; positive moves the top (negative y) away from
/// the viewer (positive z): the clip's `rotation_x`.
pub fn rotation_x(degrees: f64) -> Mat4 {
    let (s, c) = degrees.to_radians().sin_cos();
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, c, s, 0.0],
        [0.0, -s, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Turns by `degrees` around the y axis; positive moves the right side (positive x) away
/// from the viewer: the clip's `rotation_y`.
pub fn rotation_y(degrees: f64) -> Mat4 {
    let (s, c) = degrees.to_radians().sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// The distance in front of the z = 0 plane at which a camera with a `field_of_view`
/// (degrees across the canvas width) sees exactly the canvas `width`: its focal length in
/// canvas pixels.
pub fn framing_distance(width: f64, field_of_view: f64) -> f64 {
    let half = (field_of_view.clamp(1.0, 170.0) / 2.0).to_radians();
    width / 2.0 / half.tan()
}

/// What the depth of field keeps sharp and how strongly it blurs the rest, in canvas
/// pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    /// Distance from the camera to the plane in focus, along the view.
    pub distance: f64,
    /// Diameter of the lens opening.
    pub aperture: f64,
    /// Largest blur radius.
    pub max_blur: f64,
}

/// A camera at one moment, in scene coordinates (canvas pixels, see the module
/// documentation).
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    /// The timeline's canvas size in pixels.
    pub canvas: (u32, u32),
    /// Where the camera is.
    pub eye: Vec3,
    /// The point it looks at.
    pub target: Vec3,
    /// Degrees the picture turns clockwise around the view axis.
    pub roll: f64,
    /// Degrees across the canvas width.
    pub field_of_view: f64,
    /// The depth of field, when it is on.
    pub focus: Option<Focus>,
}

impl Camera {
    /// The default camera of a `canvas`: it looks straight at the z = 0 plane from the
    /// distance where that plane exactly fills the frame, so 3D layers at z = 0 render as
    /// they would in 2D.
    pub fn default_for(canvas: (u32, u32)) -> Camera {
        Camera::from_content(canvas, &CameraContent::default())
    }

    /// The camera a camera clip's content describes on a `canvas` (ADR-0037).
    pub fn from_content(canvas: (u32, u32), content: &CameraContent) -> Camera {
        let (w, h) = (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1)));
        let fov = content.field_of_view.clamp(1.0, 170.0);
        let distance = framing_distance(w, fov);
        let target = [
            content.point_of_interest[0] * w,
            content.point_of_interest[1] * h,
            content.point_of_interest_z * w,
        ];
        let placed = [
            content.position[0] * w,
            content.position[1] * h,
            -distance + content.position_z * w,
        ];
        // The orbit turns the camera around its target: pitch first, then yaw.
        let [yaw, pitch] = content.orbit;
        let turn = multiply(&rotation_y(yaw), &rotation_x(pitch));
        let relative = apply(&turn, sub(placed, target));
        let eye = [
            target[0] + relative[0],
            target[1] + relative[1],
            target[2] + relative[2],
        ];
        let mut camera = Camera {
            canvas,
            eye,
            target,
            roll: content.roll,
            field_of_view: fov,
            focus: None,
        };
        if content.depth_of_field {
            let distance = camera.view_depth(target) + content.focus_offset * w;
            camera.focus = Some(Focus {
                distance: distance.max(NEAR_PLANE * w),
                aperture: content.aperture.max(0.0) * w,
                max_blur: content.max_blur.max(0.0) * w,
            });
        }
        camera
    }

    /// The focal length in canvas pixels: how far in front of the camera a plane is seen
    /// at its own size.
    pub fn focal_length(&self) -> f64 {
        framing_distance(f64::from(self.canvas.0.max(1)), self.field_of_view)
    }

    /// The camera's axes in scene coordinates: right, down and forward (the view
    /// direction), with the roll applied.
    pub fn axes(&self) -> [Vec3; 3] {
        let forward = normalized(sub(self.target, self.eye)).unwrap_or([0.0, 0.0, 1.0]);
        // Screen down follows the scene's y axis unless the camera looks straight along it.
        let right = normalized(cross([0.0, 1.0, 0.0], forward))
            .or_else(|| normalized(cross(forward, [0.0, 0.0, 1.0])))
            .unwrap_or([1.0, 0.0, 0.0]);
        let down = cross(forward, right);
        let (s, c) = self.roll.to_radians().sin_cos();
        let turned_right = [
            right[0] * c - down[0] * s,
            right[1] * c - down[1] * s,
            right[2] * c - down[2] * s,
        ];
        let turned_down = [
            right[0] * s + down[0] * c,
            right[1] * s + down[1] * c,
            right[2] * s + down[2] * c,
        ];
        [turned_right, turned_down, forward]
    }

    /// Scene to view space: x right, y down and z forward from the eye.
    pub fn view(&self) -> Mat4 {
        let [r, d, f] = self.axes();
        let row = |axis: Vec3| [axis[0], axis[1], axis[2], -dot(axis, self.eye)];
        [row(r), row(d), row(f), [0.0, 0.0, 0.0, 1.0]]
    }

    /// View space to clip space for wgpu: x and y to normalized device coordinates of the
    /// canvas after dividing by `w = z / f`, and depth from 0 at the near plane towards 1
    /// far away. Nothing is drawn behind the near plane.
    pub fn projection(&self) -> Mat4 {
        let (w, h) = (
            f64::from(self.canvas.0.max(1)),
            f64::from(self.canvas.1.max(1)),
        );
        let f = self.focal_length();
        let near = NEAR_PLANE * w;
        [
            [2.0 / w, 0.0, 0.0, 0.0],
            [0.0, -2.0 / h, 0.0, 0.0],
            [0.0, 0.0, 1.0 / f, -near / f],
            [0.0, 0.0, 1.0 / f, 0.0],
        ]
    }

    /// Scene to clip space: [`Camera::projection`] after [`Camera::view`].
    pub fn clip_from_world(&self) -> Mat4 {
        multiply(&self.projection(), &self.view())
    }

    /// How far in front of the camera `point` is, along the view.
    pub fn view_depth(&self, point: Vec3) -> f64 {
        let [_, _, forward] = self.axes();
        dot(sub(point, self.eye), forward)
    }

    /// Where `point` appears on the canvas, in canvas pixels from its top-left corner, or
    /// `None` when it is behind the near plane.
    pub fn to_canvas(&self, point: Vec3) -> Option<[f64; 2]> {
        let clip = apply(&self.clip_from_world(), point);
        let w = f64::from(self.canvas.0.max(1));
        if clip[3] * self.focal_length() < NEAR_PLANE * w {
            return None;
        }
        let (x, y) = (clip[0] / clip[3], clip[1] / clip[3]);
        Some([
            (x + 1.0) / 2.0 * w,
            (1.0 - y) / 2.0 * f64::from(self.canvas.1.max(1)),
        ])
    }

    /// The blur radius in canvas pixels of something `depth` in front of the camera: the
    /// circle of confusion of a thin lens (half its diameter) as the camera's projection
    /// shows it, up to the largest blur; 0 without depth of field.
    pub fn blur_radius(&self, depth: f64) -> f64 {
        let Some(focus) = &self.focus else {
            return 0.0;
        };
        let depth = depth.max(NEAR_PLANE * f64::from(self.canvas.0.max(1)));
        let diameter = self.focal_length() * focus.aperture * (depth - focus.distance).abs()
            / (depth * focus.distance);
        (diameter / 2.0).min(focus.max_blur)
    }
}

/// Where a layer sits in 3D and the camera it is seen through (ADR-0037).
#[derive(Clone, Debug, PartialEq)]
pub struct LayerSpace {
    /// The camera: the timeline's for 3D layers, the default camera for other spatial
    /// layers.
    pub camera: Camera,
    /// True for a layer of the timeline's 3D scene: it is sorted by depth with the 3D
    /// layers next to it and blurred by the depth of field.
    pub scene: bool,
    /// How far in front of the camera the layer's position point is, along the view, in
    /// canvas pixels; runs of scene layers draw the largest first.
    pub depth: f64,
    /// The depth of field's blur radius as a fraction of the canvas width; 0 when sharp.
    pub defocus: f64,
}

/// The model matrix of a layer placed by `transform` on a `canvas`: from the layer's own
/// plane, in canvas pixels from its placed point (its centre, or a text layer's layout box
/// centre; x right, y down), to the scene. `size` is the layer's displayed size before
/// scaling, which the anchor is a fraction of. The picture is scaled and stretched, turned
/// in its plane (`rotation`), tilted (`rotation_x`) and turned (`rotation_y`) around its
/// anchor, then moved to its position and depth.
pub fn layer_model(canvas: (u32, u32), transform: &Transform, size: (f64, f64)) -> Mat4 {
    let (w, h) = (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1)));
    let [sx, sy] = transform.size_factors();
    let pivot = [
        transform.anchor[0] * size.0,
        transform.anchor[1] * size.1,
        0.0,
    ];
    let position = [
        transform.position[0] * w + pivot[0],
        transform.position[1] * h + pivot[1],
        transform.position_z * w,
    ];
    let turn = multiply(
        &rotation_y(transform.rotation_y),
        &multiply(
            &rotation_x(transform.rotation_x),
            &rotation_z(transform.rotation),
        ),
    );
    let placed = multiply(&translation(position), &multiply(&turn, &scaling(sx, sy)));
    multiply(&placed, &translation([-pivot[0], -pivot[1], 0.0]))
}

/// The position point of a layer placed by `transform` on a `canvas`, in the scene.
pub fn position_point(canvas: (u32, u32), transform: &Transform) -> Vec3 {
    let w = f64::from(canvas.0.max(1));
    [
        transform.position[0] * w,
        transform.position[1] * f64::from(canvas.1.max(1)),
        transform.position_z * w,
    ]
}

/// The camera of `timeline` at `time`: the enabled camera clip covering `time` on the
/// highest visible video track, or the default camera of its canvas when there is none.
pub fn camera_at(timeline: &Timeline, time: Flicks) -> Camera {
    let canvas = (timeline.format.width, timeline.format.height);
    timeline
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Video && !track.hidden)
        .filter_map(|track| {
            track.clips.iter().find(|clip| {
                clip.enabled
                    && clip.start <= time
                    && time < clip.end()
                    && matches!(clip.content, ClipContent::Camera(_))
            })
        })
        .next_back()
        .and_then(|clip| clip.camera_at(time - clip.start))
        .map_or_else(
            || Camera::default_for(canvas),
            |content| Camera::from_content(canvas, &content),
        )
}

/// The camera of the timeline `timeline` of `project` at `time`; `None` for an unknown
/// timeline.
pub fn project_camera_at(
    project: &Project,
    timeline: &fosscut_model::Id,
    time: Flicks,
) -> Option<Camera> {
    project.timelines.get(timeline).map(|t| camera_at(t, time))
}

/// Gives every spatial layer of `plan` (and of its transitions' sides) its place in 3D:
/// 3D layers seen through `camera`, other spatial layers through the default camera, with
/// their depth and depth-of-field blur; then orders each run of 3D layers by depth. The
/// planner calls it with the timeline's camera; plans built by hand can call it too.
pub fn place_layers(plan: &mut RenderPlan, camera: &Camera) {
    let flat = Camera::default_for(plan.canvas);
    for layer in &mut plan.layers {
        place_layer(layer, plan.canvas, camera, &flat);
    }
    order_by_depth(&mut plan.layers);
}

fn place_layer(layer: &mut Layer, canvas: (u32, u32), camera: &Camera, flat: &Camera) {
    match &mut layer.source {
        LayerSource::Transition(transition) => {
            place_layer(&mut transition.outgoing, canvas, camera, flat);
            place_layer(&mut transition.incoming, canvas, camera, flat);
            return;
        }
        LayerSource::Adjustment => return,
        _ => {}
    }
    let spatial = layer.transform.is_spatial() || layer.shutter.iter().any(Transform::is_spatial);
    if !spatial {
        return;
    }
    let scene = layer.transform.three_d;
    let camera = if scene { camera } else { flat };
    let depth = camera.view_depth(position_point(canvas, &layer.transform));
    let defocus = if scene {
        camera.blur_radius(depth) / f64::from(canvas.0.max(1))
    } else {
        0.0
    };
    layer.space = Some(LayerSpace {
        camera: camera.clone(),
        scene,
        depth,
        defocus,
    });
}

/// Sorts each run of consecutive 3D scene layers farthest first, keeping their order
/// where depths are equal; every other layer stays where it is and ends a run.
pub fn order_by_depth(layers: &mut [Layer]) {
    let in_scene = |layer: &Layer| layer.space.as_ref().is_some_and(|s| s.scene);
    let mut start = 0;
    while start < layers.len() {
        if !in_scene(&layers[start]) {
            start += 1;
            continue;
        }
        let end = layers[start..]
            .iter()
            .position(|l| !in_scene(l))
            .map_or(layers.len(), |p| start + p);
        layers[start..end].sort_by(|a, b| {
            let depth = |l: &Layer| l.space.as_ref().map_or(0.0, |s| s.depth);
            depth(b).total_cmp(&depth(a))
        });
        start = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::SourceRef;
    use fosscut_model::{Id, IdKind};

    const CANVAS: (u32, u32) = (1920, 1080);

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn the_default_camera_frames_the_canvas_plane_like_2d() {
        let camera = Camera::default_for(CANVAS);
        assert!(close(
            camera.focal_length(),
            960.0 / 20f64.to_radians().tan()
        ));
        assert!(close(camera.eye[2], -camera.focal_length()));
        for point in [
            [0.0, 0.0, 0.0],
            [-960.0, -540.0, 0.0],
            [960.0, 540.0, 0.0],
            [123.0, -45.5, 0.0],
        ] {
            let clip = apply(&camera.clip_from_world(), point);
            assert!(close(clip[3], 1.0), "w is exactly 1 on the canvas plane");
            assert!(close(clip[0], point[0] / 960.0));
            assert!(close(clip[1], -point[1] / 540.0));
            assert!(
                (0.0..1.0).contains(&(clip[2] / clip[3])),
                "inside the depth range"
            );
            let at = camera.to_canvas(point).unwrap();
            assert!(close(at[0], point[0] + 960.0) && close(at[1], point[1] + 540.0));
        }
        // Farther away is smaller, by the focal length over the distance.
        let f = camera.focal_length();
        let far = camera.to_canvas([480.0, 0.0, 1920.0]).unwrap();
        assert!(close(far[0] - 960.0, 480.0 * f / (f + 1920.0)));
        assert!(camera.to_canvas([0.0, 0.0, -f - 10.0]).is_none(), "behind");
    }

    #[test]
    fn layers_turn_around_their_anchor_in_the_documented_directions() {
        let size = (400.0, 200.0);
        let corner = |transform: &Transform, p: Vec3| {
            let m = layer_model(CANVAS, transform, size);
            let q = apply(&m, p);
            [q[0], q[1], q[2]]
        };
        // Positive rotation_y moves the right edge away; positive rotation_x the top edge.
        let turned = Transform {
            rotation_y: 30.0,
            ..Transform::default()
        };
        assert!(corner(&turned, [200.0, 0.0, 0.0])[2] > 0.0);
        let tilted = Transform {
            rotation_x: 30.0,
            ..Transform::default()
        };
        assert!(corner(&tilted, [0.0, -100.0, 0.0])[2] > 0.0);
        // Rotation stays clockwise on the canvas.
        let rotated = Transform {
            rotation: 90.0,
            ..Transform::default()
        };
        let p = corner(&rotated, [200.0, 0.0, 0.0]);
        assert!(close(p[0], 0.0) && close(p[1], 200.0));
        // An anchor on the left edge: the edge stays put while the picture turns and
        // scales, and moving the anchor alone does not move the picture.
        let hinged = Transform {
            anchor: [-0.5, 0.0],
            rotation_y: 60.0,
            scale: 1.5,
            position: [0.1, 0.0],
            ..Transform::default()
        };
        let edge = corner(&hinged, [-200.0, 50.0, 0.0]);
        assert!(
            close(edge[0], 192.0 - 200.0) && close(edge[2], 0.0),
            "{edge:?}"
        );
        let moved_anchor = Transform {
            anchor: [0.3, -0.2],
            ..Transform::default()
        };
        let q = corner(&moved_anchor, [10.0, 20.0, 0.0]);
        assert!(close(q[0], 10.0) && close(q[1], 20.0) && close(q[2], 0.0));
        // Depth is a fraction of the width.
        let deep = Transform {
            position_z: 0.5,
            ..Transform::default()
        };
        assert!(close(corner(&deep, [0.0, 0.0, 0.0])[2], 960.0));
    }

    #[test]
    fn cameras_dolly_orbit_roll_and_focus() {
        let pushed = Camera::from_content(
            CANVAS,
            &CameraContent {
                position_z: 0.25,
                ..CameraContent::default()
            },
        );
        let flat = Camera::default_for(CANVAS);
        assert!(close(pushed.eye[2], flat.eye[2] + 480.0));
        // Pushing in makes the canvas plane larger.
        let edge = pushed.to_canvas([480.0, 0.0, 0.0]).unwrap();
        assert!(edge[0] - 960.0 > 480.0);
        // Orbiting to the right keeps looking at the target from its right.
        let orbit = Camera::from_content(
            CANVAS,
            &CameraContent {
                orbit: [90.0, 0.0],
                ..CameraContent::default()
            },
        );
        assert!(close(orbit.eye[0], flat.focal_length()) && orbit.eye[2].abs() < 1e-6);
        assert!(close(
            orbit.view_depth([0.0, 0.0, 0.0]),
            flat.focal_length()
        ));
        let [right, ..] = orbit.axes();
        assert!(
            right[2] > 0.99,
            "the camera's right now points away: {right:?}"
        );
        let up = Camera::from_content(
            CANVAS,
            &CameraContent {
                orbit: [0.0, 30.0],
                ..CameraContent::default()
            },
        );
        assert!(up.eye[1] < 0.0, "pitch moves the camera up");
        // Roll turns the picture clockwise.
        let rolled = Camera::from_content(
            CANVAS,
            &CameraContent {
                roll: 90.0,
                ..CameraContent::default()
            },
        );
        let p = rolled.to_canvas([100.0, 0.0, 0.0]).unwrap();
        assert!(close(p[0], 960.0) && close(p[1], 640.0), "{p:?}");
        // Looking straight down, or with the eye on the target, still gives a frame.
        let down = Camera {
            eye: [0.0, -1000.0, 0.0],
            ..flat.clone()
        };
        let [right, below, forward] = down.axes();
        assert!(close(forward[1], 1.0) && close(dot(right, right), 1.0));
        assert!(close(dot(below, below), 1.0) && close(dot(right, forward), 0.0));
        let lost = Camera {
            eye: [0.0, 0.0, 0.0],
            ..flat.clone()
        };
        assert_eq!(lost.axes()[2], [0.0, 0.0, 1.0]);
        // Depth of field: sharp at the point of interest, blurrier farther from it, up to
        // the largest blur.
        let focused = Camera::from_content(
            CANVAS,
            &CameraContent {
                depth_of_field: true,
                ..CameraContent::default()
            },
        );
        let f = focused.focal_length();
        assert_eq!(focused.blur_radius(f), 0.0);
        let behind = focused.blur_radius(f + 960.0);
        let farther = focused.blur_radius(f + 1920.0);
        assert!(behind > 1.0 && farther > behind, "{behind} {farther}");
        assert!(focused.blur_radius(f * 0.3) <= 0.02 * 1920.0 + 1e-9);
        assert_eq!(flat.blur_radius(f + 960.0), 0.0, "no depth of field");
        let racked = Camera::from_content(
            CANVAS,
            &CameraContent {
                depth_of_field: true,
                focus_offset: 0.5,
                ..CameraContent::default()
            },
        );
        assert!(racked.blur_radius(f + 960.0) < 1e-9, "focus moved behind");
    }

    fn layer(depth: f64, three_d: bool) -> Layer {
        Layer {
            transform: Transform {
                three_d,
                position_z: depth,
                ..Transform::default()
            },
            ..Layer::new(SourceRef {
                asset: Id::new(IdKind::Asset, 1),
                time: Flicks::ZERO,
            })
        }
    }

    #[test]
    fn runs_of_3d_layers_draw_farthest_first() {
        let mut plan = RenderPlan::empty(CANVAS);
        plan.layers = vec![
            layer(0.2, true),
            layer(0.6, true),
            layer(0.0, false),
            layer(0.1, true),
            layer(0.5, true),
            layer(0.5, true),
            layer(0.0, true),
        ];
        // The two equal depths keep their order: mark them apart.
        plan.layers[5].opacity = 0.5;
        place_layers(&mut plan, &Camera::default_for(CANVAS));
        let depths: Vec<f64> = plan.layers.iter().map(|l| l.transform.position_z).collect();
        assert_eq!(depths, [0.6, 0.2, 0.0, 0.5, 0.5, 0.1, 0.0]);
        assert_eq!(plan.layers[3].opacity, 1.0, "stable for equal depths");
        assert!(plan.layers[2].space.is_none(), "2D layers stay flat");
        let space = plan.layers[0].space.as_ref().unwrap();
        assert!(space.scene && space.defocus == 0.0);
        // A 2D layer turned in depth sees the default camera, not the scene's.
        let mut plan = RenderPlan::empty(CANVAS);
        plan.layers = vec![Layer {
            transform: Transform {
                rotation_y: 20.0,
                ..Transform::default()
            },
            ..layer(0.0, false)
        }];
        let orbiting = Camera::from_content(
            CANVAS,
            &CameraContent {
                orbit: [30.0, 0.0],
                depth_of_field: true,
                ..CameraContent::default()
            },
        );
        place_layers(&mut plan, &orbiting);
        let space = plan.layers[0].space.as_ref().unwrap();
        assert!(!space.scene);
        assert_eq!(space.camera, Camera::default_for(CANVAS));
    }
}
