//! Smooth slow motion on the GPU (ADR-0023 item 8): a layer mixes in the next source
//! frame by how far its time lies towards it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_render::color::TransferFunction;
use fosscut_render::plan::{Layer, RenderPlan};
use fosscut_render::{Image, Renderer};
use support::*;

const SIZE: (u32, u32) = (64, 36);
const WHITE: [u8; 3] = [235, 128, 128];
const RED: [u8; 3] = [63, 102, 240];

fn render(renderer: &mut Renderer, layer: Layer, frames: &mut Frames) -> Image {
    let plan = RenderPlan {
        canvas: SIZE,
        layers: vec![layer],
        skipped: Vec::new(),
    };
    renderer
        .render_image(&plan, frames, SIZE, TransferFunction::Srgb)
        .unwrap()
}

#[test]
fn in_between_frames_mix_the_two_source_frames() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default()
        .with(1, solid(64, 36, WHITE))
        .with(2, solid(64, 36, RED));
    let at = |amount: f32, renderer: &mut Renderer, frames: &mut Frames| {
        let layer = Layer {
            next_frame: Some((source(2), amount)),
            ..Layer::new(source(1))
        };
        let image = render(renderer, layer, frames);
        let p = image.pixel(32, 18);
        [p[0], p[1], p[2]]
    };
    let none = at(0.0, &mut renderer, &mut frames);
    let all = at(1.0, &mut renderer, &mut frames);
    let half = at(0.5, &mut renderer, &mut frames);
    assert!(none.iter().all(|c| *c >= 250), "the first frame: {none:?}");
    assert!(all[0] >= 250 && all[1] <= 5, "the second frame: {all:?}");
    // Half of white and red in light: green and blue near 188 in sRGB.
    assert!(half[0] >= 250 && (180..=196).contains(&half[1]), "{half:?}");
    // A next frame of another size is ignored rather than stretched.
    let mut odd = Frames::default()
        .with(1, solid(64, 36, WHITE))
        .with(2, solid(32, 18, RED));
    let skipped = at(0.5, &mut renderer, &mut odd);
    assert!(skipped.iter().all(|c| *c >= 250), "{skipped:?}");
}
