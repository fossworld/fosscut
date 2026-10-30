//! Effects on the GPU (ADR-0011, items 1 and 4): every effect leaves the picture alone at
//! its defaults, and each one moves pixels the way its parameters say, checked against
//! values computed on the CPU. Adjustment layers change only what is below them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use fosscut_media::VideoFrame;
use fosscut_model::{AssetRef, Id, IdKind, ParamValue, Transform, effect_type};
use fosscut_render::color::{TransferFunction, reference_yuv_to_linear};
use fosscut_render::effects::PlannedEffect;
use fosscut_render::lut::Lut3d;
use fosscut_render::plan::{Layer, LayerSource, RenderPlan, SourceRef};
use fosscut_render::{FrameProvider, Image, RenderError, Renderer};
use support::*;

const SIZE: (u32, u32) = (64, 36);
const GRAY: [u8; 3] = [126, 128, 128];
const ORANGE: [u8; 3] = [150, 70, 190];

/// Frames plus LUTs, for the LUT effect.
struct WithLuts {
    frames: Frames,
    luts: BTreeMap<Id, Arc<Lut3d>>,
}

impl FrameProvider for WithLuts {
    fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError> {
        self.frames.frame(source)
    }

    fn lut(&mut self, asset: &Id) -> Result<Arc<Lut3d>, RenderError> {
        self.luts
            .get(asset)
            .cloned()
            .ok_or(RenderError::InvalidLut {
                message: "missing".into(),
            })
    }
}

fn provider(frames: Frames) -> WithLuts {
    WithLuts {
        frames,
        luts: BTreeMap::new(),
    }
}

/// An effect with every parameter at its default except `values`.
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
        seed: 1,
    }
}

fn n(v: f64) -> ParamValue {
    ParamValue::Number(v)
}

fn render(renderer: &mut Renderer, layers: Vec<Layer>, frames: &mut dyn FrameProvider) -> Image {
    let plan = RenderPlan {
        canvas: SIZE,
        layers,
        skipped: Vec::new(),
    };
    renderer
        .render_image(&plan, frames, SIZE, TransferFunction::Srgb)
        .unwrap()
}

fn with(effects: Vec<PlannedEffect>, source_id: u64) -> Layer {
    Layer {
        effects,
        ..Layer::new(source(source_id))
    }
}

/// The sRGB-encoded value of a flat Y'CbCr color, 0 to 1.
fn encoded(yuv: [u8; 3]) -> [f64; 3] {
    reference_yuv_to_linear(&bt709_limited(), yuv[0], yuv[1], yuv[2])
        .unwrap()
        .map(|l| f64::from(TransferFunction::Srgb.encode(l)).clamp(0.0, 1.0))
}

fn linear(code: u8) -> f64 {
    let v = f64::from(code) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn rgb(image: &Image, x: u32, y: u32) -> [u8; 3] {
    let p = image.pixel(x, y);
    [p[0], p[1], p[2]]
}

fn near(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
}

#[test]
fn every_effect_leaves_the_picture_alone_at_rest() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let ramp = |x: u32, y: u32| {
        [
            16 + (x * 219 / 63) as u8,
            60 + (y * 3) as u8,
            200 - (x * 2) as u8,
        ]
    };
    let mut frames = provider(Frames::default().with(1, nv12(64, 36, bt709_limited(), ramp)));
    let lut = Id::new(IdKind::Asset, 9);
    frames
        .luts
        .insert(lut.clone(), Arc::new(Lut3d::identity(17)));
    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let neutral = [
        effect("color.adjust", &[]),
        effect("color.curves", &[]),
        effect("color.hsl", &[]),
        effect("color.wheels", &[]),
        effect(
            "color.lut",
            &[("lut", ParamValue::Asset(AssetRef { asset: lut }))],
        ),
        effect("color.vignette", &[]),
        effect("blur.gaussian", &[("radius", n(0.0))]),
        effect("detail.sharpen", &[("amount", n(0.0))]),
        effect("stylize.grain", &[("amount", n(0.0))]),
    ];
    for neutral in neutral {
        let kind = neutral.effect_type.clone();
        let image = render(&mut renderer, vec![with(vec![neutral], 1)], &mut frames);
        for (x, y) in [(3, 3), (20, 10), (40, 20), (60, 33)] {
            assert!(
                near(rgb(&image, x, y), rgb(&plain, x, y), 2),
                "{kind} at ({x}, {y}): {:?} vs {:?}",
                rgb(&image, x, y),
                rgb(&plain, x, y)
            );
        }
    }
}

#[test]
fn adjust_moves_light_balance_and_color() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = provider(
        Frames::default()
            .with(1, solid(64, 36, GRAY))
            .with(2, solid(64, 36, ORANGE)),
    );
    let gray = encoded(GRAY);
    let base = linear((gray[0] * 255.0).round() as u8);
    let brighter = render(
        &mut renderer,
        vec![with(
            vec![effect("color.adjust", &[("exposure", n(1.0))])],
            1,
        )],
        &mut frames,
    );
    let doubled = linear(brighter.pixel(32, 18)[0]);
    assert!(
        (doubled / base - 2.0).abs() < 0.08,
        "one stop doubles the light: {doubled} vs {base}"
    );

    let warm = render(
        &mut renderer,
        vec![with(
            vec![effect("color.adjust", &[("temperature", n(0.8))])],
            1,
        )],
        &mut frames,
    );
    let [r, g, b] = rgb(&warm, 32, 18);
    assert!(
        r > g && g > b,
        "warmer: more red, less blue: {:?}",
        [r, g, b]
    );
    let green = render(
        &mut renderer,
        vec![with(vec![effect("color.adjust", &[("tint", n(-0.8))])], 1)],
        &mut frames,
    );
    let [r, g, b] = rgb(&green, 32, 18);
    assert!(g > r && g > b, "negative tint is greener: {:?}", [r, g, b]);

    let gray_orange = render(
        &mut renderer,
        vec![with(
            vec![effect("color.adjust", &[("saturation", n(-1.0))])],
            2,
        )],
        &mut frames,
    );
    let [r, g, b] = rgb(&gray_orange, 32, 18);
    assert!(
        r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1,
        "no color left: {:?}",
        [r, g, b]
    );
    let flat = render(
        &mut renderer,
        vec![with(
            vec![effect("color.adjust", &[("contrast", n(-1.0))])],
            2,
        )],
        &mut frames,
    );
    assert!(
        near(rgb(&flat, 32, 18), [128, 128, 128], 2),
        "no contrast is mid gray"
    );
    let lifted = render(
        &mut renderer,
        vec![with(vec![effect("color.adjust", &[("blacks", n(1.0))])], 1)],
        &mut frames,
    );
    assert!(
        lifted.pixel(32, 18)[0] > (gray[0] * 255.0) as u8,
        "lifted blacks brighten"
    );
}

#[test]
fn curves_hsl_wheels_and_luts_change_what_they_target() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    // Orange on the left half, blue on the right.
    let split = |x: u32, _| if x < 32 { ORANGE } else { [80, 200, 110] };
    let mut frames = provider(
        Frames::default()
            .with(1, solid(64, 36, GRAY))
            .with(2, nv12(64, 36, bt709_limited(), split))
            .with(3, solid(64, 36, [50, 128, 128])),
    );
    let gray = encoded(GRAY);
    let inverted = render(
        &mut renderer,
        vec![with(
            vec![effect(
                "color.curves",
                &[("master", ParamValue::Points(vec![[0.0, 1.0], [1.0, 0.0]]))],
            )],
            1,
        )],
        &mut frames,
    );
    let want = ((1.0 - gray[0]) * 255.0).round() as u8;
    assert!(
        inverted.pixel(32, 18)[0].abs_diff(want) <= 2,
        "inverted gray"
    );

    let plain = render(&mut renderer, vec![Layer::new(source(2))], &mut frames);
    let calm_orange = render(
        &mut renderer,
        vec![with(
            vec![effect(
                "color.hsl",
                &[("orange_saturation", n(-1.0)), ("red_saturation", n(-1.0))],
            )],
            2,
        )],
        &mut frames,
    );
    let [r, g, b] = rgb(&calm_orange, 10, 18);
    assert!(
        r.abs_diff(b) < 40,
        "the orange half loses its color: {:?}",
        [r, g, b]
    );
    assert!(
        near(rgb(&calm_orange, 54, 18), rgb(&plain, 54, 18), 2),
        "blue is untouched"
    );

    let cool_highlights = render(
        &mut renderer,
        vec![with(
            vec![effect(
                "color.wheels",
                &[("gain", ParamValue::Pair([-0.5, -0.866]))],
            )],
            1,
        )],
        &mut frames,
    );
    let [r, _, b] = rgb(&cool_highlights, 32, 18);
    assert!(
        b > r + 10,
        "a gain push towards blue: {:?}",
        rgb(&cool_highlights, 32, 18)
    );

    // An inverting LUT at half intensity lands in between.
    let lut = Id::new(IdKind::Asset, 9);
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for b in [0.0, 1.0] {
        for g in [0.0, 1.0] {
            for r in [0.0, 1.0] {
                cube.push_str(&format!("{} {} {}\n", 1.0 - r, 1.0 - g, 1.0 - b));
            }
        }
    }
    frames
        .luts
        .insert(lut.clone(), Arc::new(Lut3d::parse_cube(&cube).unwrap()));
    let lut_ref = ParamValue::Asset(AssetRef { asset: lut });
    let full = render(
        &mut renderer,
        vec![with(
            vec![effect("color.lut", &[("lut", lut_ref.clone())])],
            3,
        )],
        &mut frames,
    );
    let half = render(
        &mut renderer,
        vec![with(
            vec![effect(
                "color.lut",
                &[("lut", lut_ref), ("intensity", n(0.5))],
            )],
            3,
        )],
        &mut frames,
    );
    let dark = encoded([50, 128, 128]);
    let (before, after, middle) = (
        linear((dark[0] * 255.0).round() as u8),
        linear(full.pixel(32, 18)[0]),
        linear(half.pixel(32, 18)[0]),
    );
    assert!(
        after > before + 0.1,
        "dark gray inverts to light: {before} to {after}"
    );
    assert!(
        ((before + after) / 2.0 - middle).abs() < 0.02,
        "half way in linear light: {before} {middle} {after}"
    );
    let missing = renderer.render_image(
        &RenderPlan {
            canvas: SIZE,
            layers: vec![with(
                vec![effect(
                    "color.lut",
                    &[(
                        "lut",
                        ParamValue::Asset(AssetRef {
                            asset: Id::new(IdKind::Asset, 77),
                        }),
                    )],
                )],
                1,
            )],
            skipped: Vec::new(),
        },
        &mut frames,
        SIZE,
        TransferFunction::Srgb,
    );
    assert_eq!(missing.unwrap_err().code(), "fosscut.render.invalid_lut");
}

#[test]
fn vignette_grain_blur_and_sharpen() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let edge = |x: u32, _| {
        if x < 32 {
            [40, 128, 128]
        } else {
            [200, 128, 128]
        }
    };
    let mut frames = provider(
        Frames::default()
            .with(1, solid(64, 36, GRAY))
            .with(2, nv12(64, 36, bt709_limited(), edge)),
    );
    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let vignette = render(
        &mut renderer,
        vec![with(
            vec![effect("color.vignette", &[("amount", n(1.0))])],
            1,
        )],
        &mut frames,
    );
    assert!(
        vignette.pixel(1, 1)[0] < plain.pixel(1, 1)[0] / 2,
        "dark corners"
    );
    assert!(
        near(rgb(&vignette, 32, 18), rgb(&plain, 32, 18), 1),
        "a clear middle"
    );

    let grain = |seed: u32, renderer: &mut Renderer, frames: &mut WithLuts| {
        let mut effect = effect("stylize.grain", &[("amount", n(0.8))]);
        effect.seed = seed;
        render(renderer, vec![with(vec![effect], 1)], frames)
    };
    let a = grain(1, &mut renderer, &mut frames);
    let b = grain(1, &mut renderer, &mut frames);
    let c = grain(2, &mut renderer, &mut frames);
    assert_eq!(a.rgba, b.rgba, "the same frame renders the same grain");
    assert_ne!(a.rgba, c.rgba, "another frame, other grain");
    let mean = |image: &Image| {
        image.rgba.chunks(4).map(|p| f64::from(p[0])).sum::<f64>() / f64::from(SIZE.0 * SIZE.1)
    };
    assert!(
        (mean(&a) - f64::from(plain.pixel(0, 0)[0])).abs() < 4.0,
        "grain keeps the level"
    );
    let spread = a.rgba.chunks(4).map(|p| p[0]).max().unwrap()
        - a.rgba.chunks(4).map(|p| p[0]).min().unwrap();
    assert!(spread > 10, "visible grain: {spread}");

    let sharp = render(&mut renderer, vec![Layer::new(source(2))], &mut frames);
    let blurred = render(
        &mut renderer,
        vec![with(
            vec![effect("blur.gaussian", &[("radius", n(0.1))])],
            2,
        )],
        &mut frames,
    );
    assert!(
        blurred.pixel(31, 18)[0] > sharp.pixel(31, 18)[0] + 20,
        "the edge spreads"
    );
    assert!(blurred.pixel(32, 18)[0] < sharp.pixel(32, 18)[0] - 20);
    assert!(
        blurred.pixel(2, 18)[0].abs_diff(sharp.pixel(2, 18)[0]) <= 6,
        "far from the edge little changes"
    );
    let wide = render(
        &mut renderer,
        vec![with(
            vec![effect("blur.gaussian", &[("radius", n(0.2))])],
            2,
        )],
        &mut frames,
    );
    let row: Vec<u8> = (0..64).map(|x| wide.pixel(x, 18)[0]).collect();
    assert!(
        row.windows(2).all(|w| w[1] + 1 >= w[0]),
        "a smooth ramp: {row:?}"
    );
    let sharpened = render(
        &mut renderer,
        vec![with(
            vec![effect(
                "detail.sharpen",
                &[("amount", n(1.5)), ("radius", n(0.02))],
            )],
            2,
        )],
        &mut frames,
    );
    assert!(
        sharpened.pixel(30, 18)[0] < sharp.pixel(30, 18)[0],
        "darker before the edge"
    );
    assert!(
        sharpened.pixel(33, 18)[0] > sharp.pixel(33, 18)[0],
        "brighter after it"
    );
}

#[test]
fn adjustment_layers_change_only_what_is_below() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = provider(
        Frames::default()
            .with(1, solid(64, 36, ORANGE))
            .with(2, solid(64, 36, [235, 128, 128])),
    );
    let adjustment = |opacity: f32| Layer {
        source: LayerSource::Adjustment,
        opacity,
        effects: vec![effect("color.adjust", &[("saturation", n(-1.0))])],
        ..Layer::new(source(1))
    };
    let small_white = Layer {
        transform: Transform {
            scale: 0.25,
            position: [0.375, 0.0],
            ..Transform::default()
        },
        ..Layer::new(source(2))
    };
    let tinted_above = Layer {
        effects: vec![effect("color.adjust", &[("temperature", n(1.0))])],
        ..small_white.clone()
    };
    let image = render(
        &mut renderer,
        vec![Layer::new(source(1)), adjustment(1.0), tinted_above],
        &mut frames,
    );
    let [r, g, b] = rgb(&image, 10, 18);
    assert!(
        r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1,
        "graded to gray below: {:?}",
        [r, g, b]
    );
    let [r, _, b] = rgb(&image, 56, 18);
    assert!(
        r > b + 20,
        "the layer above keeps its own warm effect: {:?}",
        rgb(&image, 56, 18)
    );

    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let half = render(
        &mut renderer,
        vec![Layer::new(source(1)), adjustment(0.5)],
        &mut frames,
    );
    let full = render(
        &mut renderer,
        vec![Layer::new(source(1)), adjustment(1.0)],
        &mut frames,
    );
    for c in 0..3 {
        // Opacity mixes linear light, as it does for normal layers.
        let middle = (linear(plain.pixel(10, 18)[c]) + linear(full.pixel(10, 18)[c])) / 2.0;
        assert!(
            (linear(half.pixel(10, 18)[c]) - middle).abs() < 0.01,
            "half opacity mixes channel {c}"
        );
    }
    let alone = render(&mut renderer, vec![adjustment(1.0)], &mut frames);
    assert_eq!(
        rgb(&alone, 10, 18),
        [0, 0, 0],
        "with nothing below, black stays black"
    );
}
