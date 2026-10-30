//! Masks and keying on the GPU (ADR-0011, item 4): masks decide what of a layer shows,
//! follow the picture as displayed, combine, soften, limit adjustment layers, and the
//! chroma key removes its color with spill suppression.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeMap;

use fosscut_model::{Id, IdKind, Mask, MaskMode, MaskShape, ParamValue, effect_type};
use fosscut_render::color::TransferFunction;
use fosscut_render::effects::PlannedEffect;
use fosscut_render::plan::{Layer, LayerSource, RenderPlan};
use fosscut_render::{Image, Renderer};
use support::*;

const SIZE: (u32, u32) = (64, 36);
const WHITE: [u8; 3] = [235, 128, 128];
const GREEN: [u8; 3] = [173, 42, 26];

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

fn mask(shape: MaskShape) -> Mask {
    Mask::new(Id::new(IdKind::Mask, 1), shape)
}

fn effect(kind: &str, values: &[(&str, ParamValue)]) -> PlannedEffect {
    let spec = effect_type(kind).unwrap();
    let mut all: BTreeMap<String, ParamValue> = spec
        .params
        .iter()
        .map(|p| (p.name.to_string(), p.default.value()))
        .collect();
    for (name, value) in values {
        all.insert(name.to_string(), value.clone());
    }
    PlannedEffect {
        effect_type: kind.into(),
        values: all,
        seed: 0,
        time: 0.0,
    }
}

fn red(image: &Image, x: u32, y: u32) -> u8 {
    image.pixel(x, y)[0]
}

#[test]
fn masks_show_only_what_they_cover() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default().with(1, solid(64, 36, WHITE));
    let masked = |masks: Vec<Mask>| Layer {
        masks,
        ..Layer::new(source(1))
    };
    let box_ = render(
        &mut renderer,
        vec![masked(vec![mask(MaskShape::Rectangle)])],
        &mut frames,
    );
    assert_eq!(red(&box_, 32, 18), 255, "inside");
    assert_eq!(red(&box_, 4, 18), 0, "outside the middle half");
    assert_eq!(red(&box_, 32, 2), 0);

    let left = Mask {
        center: [0.25, 0.5],
        size: [0.5, 1.0],
        ..mask(MaskShape::Rectangle)
    };
    let hole = Mask {
        size: [0.2, 0.2],
        center: [0.25, 0.5],
        mode: MaskMode::Subtract,
        ..mask(MaskShape::Ellipse)
    };
    let cut = render(
        &mut renderer,
        vec![masked(vec![left.clone(), hole])],
        &mut frames,
    );
    assert_eq!(
        (red(&cut, 4, 4), red(&cut, 16, 18), red(&cut, 50, 18)),
        (255, 0, 0)
    );
    let soft = render(
        &mut renderer,
        vec![masked(vec![Mask {
            feather: 1.0,
            ..mask(MaskShape::Ellipse)
        }])],
        &mut frames,
    );
    let edge = red(&soft, 48, 18);
    assert!(edge > 40 && edge < 230, "a soft edge: {edge}");
    let off = render(
        &mut renderer,
        vec![masked(vec![Mask {
            enabled: false,
            ..left
        }])],
        &mut frames,
    );
    assert_eq!(red(&off, 60, 18), 255, "a disabled mask hides nothing");
}

#[test]
fn masks_follow_the_displayed_picture() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    // A portrait source stored landscape and turned a quarter for display: the mask's left
    // half is the displayed picture's left half.
    let plan = RenderPlan {
        canvas: (36, 64),
        layers: vec![Layer {
            rotation: 90,
            masks: vec![Mask {
                center: [0.25, 0.5],
                size: [0.5, 1.0],
                ..mask(MaskShape::Rectangle)
            }],
            ..Layer::new(source(1))
        }],
        skipped: Vec::new(),
    };
    let mut frames = Frames::default().with(1, solid(64, 36, WHITE));
    let image = renderer
        .render_image(&plan, &mut frames, (36, 64), TransferFunction::Srgb)
        .unwrap();
    assert_eq!((red(&image, 6, 32), red(&image, 30, 32)), (255, 0));
}

#[test]
fn adjustment_masks_limit_the_adjustment() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default().with(1, solid(64, 36, [150, 70, 190]));
    let gray = Layer {
        source: LayerSource::Adjustment,
        effects: vec![effect(
            "color.adjust",
            &[("saturation", ParamValue::Number(-1.0))],
        )],
        masks: vec![Mask {
            center: [0.75, 0.5],
            size: [0.5, 1.0],
            ..mask(MaskShape::Rectangle)
        }],
        ..Layer::new(source(1))
    };
    let image = render(
        &mut renderer,
        vec![Layer::new(source(1)), gray],
        &mut frames,
    );
    let [r, g, b, _] = image.pixel(56, 18);
    assert!(
        r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1,
        "gray inside the mask"
    );
    let [r, _, b, _] = image.pixel(8, 18);
    assert!(r > b + 40, "color outside it: {:?}", image.pixel(8, 18));
}

#[test]
fn the_chroma_key_removes_its_color() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    // Green with a white square in the middle, over a red background layer.
    let screen = |x: u32, y: u32| {
        if (24..40).contains(&x) && (10..26).contains(&y) {
            WHITE
        } else {
            GREEN
        }
    };
    let mut frames = Frames::default()
        .with(1, solid(64, 36, [63, 102, 240]))
        .with(2, nv12(64, 36, bt709_limited(), screen));
    let keyed = Layer {
        effects: vec![effect("key.chroma", &[])],
        ..Layer::new(source(2))
    };
    let image = render(
        &mut renderer,
        vec![Layer::new(source(1)), keyed],
        &mut frames,
    );
    let [r, g, _, _] = image.pixel(4, 4);
    assert!(
        r > 150 && g < 80,
        "the background shows through the green: {:?}",
        image.pixel(4, 4)
    );
    assert_eq!(
        &image.pixel(32, 18)[..3],
        &[255, 255, 255],
        "the white square stays"
    );
    let blue_key = Layer {
        effects: vec![effect(
            "key.chroma",
            &[("color", ParamValue::Name("#0000FF".into()))],
        )],
        ..Layer::new(source(2))
    };
    let image = render(
        &mut renderer,
        vec![Layer::new(source(1)), blue_key],
        &mut frames,
    );
    assert!(image.pixel(4, 4)[1] > 150, "keying blue leaves the green");
}
