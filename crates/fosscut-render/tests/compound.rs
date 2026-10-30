//! Compound clips on the GPU (ADR-0025): a nested plan renders into its own picture,
//! placed like a frame of its canvas, with a transparent background.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_model::Transform;
use fosscut_render::color::TransferFunction;
use fosscut_render::plan::{Layer, LayerSource, RenderPlan};
use fosscut_render::{Image, Renderer};
use support::*;

const SIZE: (u32, u32) = (64, 36);
const WHITE: [u8; 3] = [235, 128, 128];
const RED: [u8; 3] = [63, 102, 240];

fn render(renderer: &mut Renderer, layers: Vec<Layer>, frames: &mut Frames) -> Image {
    let plan = RenderPlan {
        canvas: SIZE,
        layers,
        skipped: Vec::new(),
    };
    renderer
        .render_image(&plan, frames, SIZE, TransferFunction::Srgb)
        .unwrap()
}

fn rgb(image: &Image, x: u32, y: u32) -> [u8; 3] {
    let p = image.pixel(x, y);
    [p[0], p[1], p[2]]
}

/// A nested plan of the same canvas showing `layers`.
fn nested(layers: Vec<Layer>, transform: Transform) -> Layer {
    Layer {
        source: LayerSource::Nested(Box::new(RenderPlan {
            canvas: SIZE,
            layers,
            skipped: Vec::new(),
        })),
        transform,
        ..Layer::new(source(1))
    }
}

#[test]
fn a_compound_clip_shows_its_timeline_placed_like_a_picture() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default()
        .with(1, solid(64, 36, WHITE))
        .with(2, solid(64, 36, RED));
    // At rest, the compound clip looks exactly like its contents drawn directly.
    let direct = render(&mut renderer, vec![Layer::new(source(2))], &mut frames);
    let wrapped = render(
        &mut renderer,
        vec![nested(vec![Layer::new(source(2))], Transform::default())],
        &mut frames,
    );
    assert_eq!(rgb(&wrapped, 32, 18), rgb(&direct, 32, 18));
    // Half size over white: its empty canvas is transparent, so white shows around it.
    let half = Transform {
        scale: 0.5,
        ..Transform::default()
    };
    let small = Layer {
        transform: Transform {
            scale: 0.5,
            ..Transform::default()
        },
        ..Layer::new(source(2))
    };
    let placed = render(
        &mut renderer,
        vec![Layer::new(source(1)), nested(vec![small], half)],
        &mut frames,
    );
    assert!(
        rgb(&placed, 2, 2).iter().all(|c| *c >= 250),
        "white outside"
    );
    assert!(rgb(&placed, 32, 18)[1] < 10, "red in the middle");
    assert!(
        rgb(&placed, 22, 12).iter().all(|c| *c >= 250),
        "white inside its picture but outside the small clip within it"
    );
    // Two levels deep, next to a sibling: each keeps its own picture.
    let deep = nested(
        vec![nested(vec![Layer::new(source(2))], Transform::default())],
        Transform {
            position: [-0.25, 0.0],
            scale: 0.5,
            ..Transform::default()
        },
    );
    let sibling = nested(
        vec![Layer::new(source(1))],
        Transform {
            position: [0.25, 0.0],
            scale: 0.5,
            ..Transform::default()
        },
    );
    let both = render(&mut renderer, vec![deep, sibling], &mut frames);
    assert!(rgb(&both, 16, 18)[1] < 10, "red on the left");
    assert!(
        rgb(&both, 48, 18).iter().all(|c| *c >= 250),
        "white on the right"
    );
}
