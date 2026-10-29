//! Where a layer lands on the canvas: fit, transform and source rotation (ADR-0004
//! transform semantics), computed on the CPU so it is exact and testable.

use bytemuck::{Pod, Zeroable};
use fosscut_model::Transform;

/// One corner of a layer quad as the composite shader receives it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    /// Position in normalized device coordinates (x right, y up, -1 to 1).
    pub position: [f32; 2],
    /// Position within the layer as displayed, 0 to 1 (x right, y down); masks use it.
    pub local: [f32; 2],
    /// Texture coordinate in the stored frame, 0 to 1.
    pub uv: [f32; 2],
}

/// Clockwise display rotation of a source, from the model's counterclockwise degrees
/// (FFmpeg's display-matrix convention), rounded to a quarter turn.
pub fn clockwise_rotation(counterclockwise_degrees: i32) -> u16 {
    let quarter_turns = ((-f64::from(counterclockwise_degrees)) / 90.0).round() as i64;
    (quarter_turns.rem_euclid(4) * 90) as u16
}

/// Texture coordinate in the stored frame for a point of the displayed layer, for a source
/// that must be rotated `rotation` degrees clockwise for display.
pub fn stored_uv(local: [f32; 2], rotation: u16) -> [f32; 2] {
    let [x, y] = local;
    match rotation {
        90 => [y, 1.0 - x],
        180 => [1.0 - x, 1.0 - y],
        270 => [1.0 - y, x],
        _ => [x, y],
    }
}

/// The four corners (top-left, top-right, bottom-left, bottom-right, ready for a triangle
/// strip) of a `width` x `height` stored frame, displayed rotated `rotation` degrees
/// clockwise, fitted inside a `canvas` and moved by `transform`.
pub fn layer_quad(
    canvas: (u32, u32),
    source: (u32, u32),
    rotation: u16,
    transform: &Transform,
) -> [Vertex; 4] {
    let (canvas_w, canvas_h) = (f64::from(canvas.0), f64::from(canvas.1));
    let (mut w, mut h) = (f64::from(source.0.max(1)), f64::from(source.1.max(1)));
    if rotation == 90 || rotation == 270 {
        std::mem::swap(&mut w, &mut h);
    }
    // "Fitted size": the largest size that shows the whole frame on the canvas.
    let fit = (canvas_w / w).min(canvas_h / h);
    quad(canvas, (w * fit, h * fit), (0.0, 0.0), rotation, transform)
}

/// The corners of a layer drawn at its natural `size` in canvas pixels (no fitting),
/// placed so that the point `offset` canvas pixels from the layer's centre sits at the
/// canvas centre moved by `transform`. Text layers use it to centre their layout box
/// rather than their pixel-cropped raster.
pub fn placed_quad(
    canvas: (u32, u32),
    size: (f64, f64),
    offset: (f64, f64),
    transform: &Transform,
) -> [Vertex; 4] {
    quad(canvas, size, offset, 0, transform)
}

/// Corners of a layer `size` canvas pixels large (as displayed), whose point `offset`
/// from its centre is placed by `transform`.
fn quad(
    canvas: (u32, u32),
    size: (f64, f64),
    offset: (f64, f64),
    rotation: u16,
    transform: &Transform,
) -> [Vertex; 4] {
    let (canvas_w, canvas_h) = (f64::from(canvas.0), f64::from(canvas.1));
    let (half_w, half_h) = (
        size.0 * transform.scale / 2.0,
        size.1 * transform.scale / 2.0,
    );
    let center = (
        canvas_w / 2.0 + transform.position[0] * canvas_w,
        canvas_h / 2.0 + transform.position[1] * canvas_h,
    );
    // Clockwise on screen: with y pointing down, the standard rotation matrix turns
    // clockwise.
    let (sin, cos) = transform.rotation.to_radians().sin_cos();
    let corner = |lx: f64, ly: f64| {
        let (dx, dy) = (
            (2.0 * lx - 1.0) * half_w - offset.0 * transform.scale,
            (2.0 * ly - 1.0) * half_h - offset.1 * transform.scale,
        );
        let x = center.0 + dx * cos - dy * sin;
        let y = center.1 + dx * sin + dy * cos;
        let local = [lx as f32, ly as f32];
        Vertex {
            position: [
                (2.0 * x / canvas_w - 1.0) as f32,
                (1.0 - 2.0 * y / canvas_h) as f32,
            ],
            local,
            uv: stored_uv(local, rotation),
        }
    };
    [
        corner(0.0, 0.0),
        corner(1.0, 0.0),
        corner(0.0, 1.0),
        corner(1.0, 1.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5
    }

    #[test]
    fn matching_aspect_fills_the_canvas() {
        let quad = layer_quad((1920, 1080), (1280, 720), 0, &Transform::default());
        assert!(close(quad[0].position, [-1.0, 1.0]));
        assert!(close(quad[3].position, [1.0, -1.0]));
        assert_eq!(quad[0].uv, [0.0, 0.0]);
        assert_eq!(quad[3].uv, [1.0, 1.0]);
    }

    #[test]
    fn other_aspects_are_fitted_inside() {
        // A 4:3 frame on a 16:9 canvas: full height, centred, pillarboxed.
        let quad = layer_quad((1920, 1080), (640, 480), 0, &Transform::default());
        assert!(close(quad[0].position, [-0.75, 1.0]));
        assert!(close(quad[3].position, [0.75, -1.0]));
        // A portrait phone frame stored landscape and rotated 90 degrees on a vertical canvas.
        let quad = layer_quad((1080, 1920), (1920, 1080), 90, &Transform::default());
        assert!(close(quad[0].position, [-1.0, 1.0]));
        assert!(close(quad[3].position, [1.0, -1.0]));
        assert!(
            close(quad[0].uv, [0.0, 1.0]),
            "top-left shows the stored bottom-left"
        );
    }

    #[test]
    fn position_scale_and_rotation_follow_the_model() {
        let moved = Transform {
            position: [0.25, -0.25],
            scale: 0.5,
            rotation: 0.0,
        };
        let quad = layer_quad((1920, 1080), (1920, 1080), 0, &moved);
        // Half size, centre moved a quarter canvas right and up.
        assert!(close(quad[0].position, [0.0, 1.0]));
        assert!(close(quad[3].position, [1.0, 0.0]));

        let turned = Transform {
            rotation: 90.0,
            ..Transform::default()
        };
        let quad = layer_quad((1000, 1000), (1000, 1000), 0, &turned);
        // Clockwise quarter turn: the top-left corner moves to the top-right.
        assert!(close(quad[0].position, [1.0, 1.0]));
        assert!(close(quad[1].position, [1.0, -1.0]));
    }

    #[test]
    fn placed_layers_keep_their_natural_size() {
        let quad = placed_quad(
            (1920, 1080),
            (480.0, 108.0),
            (0.0, 0.0),
            &Transform::default(),
        );
        assert!(close(quad[0].position, [-0.25, 0.1]));
        assert!(close(quad[3].position, [0.25, -0.1]));
        // An anchor 96 px right of the layer centre is what lands at the canvas centre.
        let shifted = placed_quad(
            (1920, 1080),
            (480.0, 108.0),
            (96.0, 0.0),
            &Transform::default(),
        );
        assert!(close(shifted[0].position, [-0.35, 0.1]));
        let lower = Transform {
            position: [0.0, 0.35],
            scale: 2.0,
            rotation: 0.0,
        };
        let quad = placed_quad((1920, 1080), (480.0, 108.0), (0.0, 0.0), &lower);
        assert!(close(quad[0].position, [-0.5, -0.5]));
        assert_eq!(quad[3].uv, [1.0, 1.0]);
    }

    #[test]
    fn rotation_conventions_convert_and_map_texture_coordinates() {
        assert_eq!(clockwise_rotation(0), 0);
        assert_eq!(clockwise_rotation(-90), 90);
        assert_eq!(clockwise_rotation(90), 270);
        assert_eq!(clockwise_rotation(180), 180);
        assert_eq!(clockwise_rotation(-269), 270);
        for rotation in [0, 90, 180, 270] {
            let corners: Vec<[f32; 2]> = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]
                .iter()
                .map(|c| stored_uv(*c, rotation))
                .collect();
            let mut sorted = corners.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            sorted.dedup();
            assert_eq!(
                sorted.len(),
                4,
                "rotation {rotation} maps corners to corners"
            );
        }
        assert_eq!(stored_uv([1.0, 0.0], 90), [0.0, 0.0]);
        assert_eq!(stored_uv([0.0, 0.0], 270), [1.0, 0.0]);
        assert_eq!(stored_uv([0.0, 0.0], 180), [1.0, 1.0]);
    }
}
