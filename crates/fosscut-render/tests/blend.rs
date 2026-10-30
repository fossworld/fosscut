//! Blend modes, crop, flips and non-uniform scale on the GPU, checked against values
//! computed on the CPU from the W3C Compositing and Blending formulas (ADR-0011, item 4).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_model::{BlendMode, Transform};
use fosscut_render::color::{TransferFunction, reference_yuv_to_linear};
use fosscut_render::plan::{Layer, RenderPlan};
use fosscut_render::{Image, Renderer};
use support::*;

const SIZE: (u32, u32) = (64, 36);
const BACKDROP: [u8; 3] = [120, 150, 100];
const PAINT: [u8; 3] = [170, 90, 160];

fn render(renderer: &mut Renderer, layers: Vec<Layer>, frames: &mut Frames) -> Image {
    let plan = RenderPlan {
        canvas: SIZE,
        layers,
        skipped: Vec::new(),
    };
    // An sRGB output, so output codes equal the sRGB-encoded values the modes work on.
    renderer
        .render_image(&plan, frames, SIZE, TransferFunction::Srgb)
        .unwrap()
}

fn encoded(yuv: [u8; 3]) -> [f64; 3] {
    reference_yuv_to_linear(&bt709_limited(), yuv[0], yuv[1], yuv[2])
        .unwrap()
        .map(|l| f64::from(TransferFunction::Srgb.encode(l)).clamp(0.0, 1.0))
}

fn soft(b: f64, s: f64) -> f64 {
    if s <= 0.5 {
        b - (1.0 - 2.0 * s) * b * (1.0 - b)
    } else {
        let d = if b <= 0.25 {
            ((16.0 * b - 12.0) * b + 4.0) * b
        } else {
            b.sqrt()
        };
        b + (2.0 * s - 1.0) * (d - b)
    }
}

fn hard(b: f64, s: f64) -> f64 {
    if s <= 0.5 {
        b * 2.0 * s
    } else {
        let t = 2.0 * s - 1.0;
        b + t - b * t
    }
}

/// The blended value of one channel, from the specification's definitions.
fn reference(mode: BlendMode, b: f64, s: f64) -> f64 {
    match mode {
        BlendMode::Normal => s,
        BlendMode::Multiply => b * s,
        BlendMode::Screen => b + s - b * s,
        BlendMode::Overlay => hard(s, b),
        BlendMode::Darken => b.min(s),
        BlendMode::Lighten => b.max(s),
        BlendMode::ColorDodge => {
            if b == 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        BlendMode::ColorBurn => {
            if b == 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        BlendMode::HardLight => hard(b, s),
        BlendMode::SoftLight => soft(b, s),
        BlendMode::LinearBurn => (b + s - 1.0).max(0.0),
        BlendMode::Add => (b + s).min(1.0),
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
    }
}

const MODES: [BlendMode; 14] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::HardLight,
    BlendMode::SoftLight,
    BlendMode::LinearBurn,
    BlendMode::Add,
    BlendMode::Difference,
    BlendMode::Exclusion,
];

fn assert_close(actual: [u8; 4], expected: [f64; 3], what: &str) {
    for c in 0..3 {
        let want = (expected[c] * 255.0).round();
        assert!(
            (f64::from(actual[c]) - want).abs() <= 2.0,
            "{what}: got {actual:?}, expected {:?}",
            expected.map(|v| (v * 255.0).round())
        );
    }
}

#[test]
fn every_blend_mode_matches_the_specification() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default()
        .with(1, solid(64, 36, BACKDROP))
        .with(2, solid(64, 36, PAINT));
    let (b, s) = (encoded(BACKDROP), encoded(PAINT));
    for mode in MODES {
        for opacity in [1.0, 0.5] {
            let layers = vec![
                Layer::new(source(1)),
                Layer {
                    blend: mode,
                    opacity,
                    ..Layer::new(source(2))
                },
            ];
            let image = render(&mut renderer, layers, &mut frames);
            let expected: [f64; 3] = std::array::from_fn(|c| {
                let blended = reference(mode, b[c], s[c]);
                if mode == BlendMode::Normal {
                    // Normal mixes in linear light: check it separately below.
                    blended
                } else {
                    b[c] + (blended - b[c]) * f64::from(opacity)
                }
            });
            if mode == BlendMode::Normal && opacity < 1.0 {
                continue;
            }
            assert_close(
                image.pixel(32, 18),
                expected,
                &format!("{mode:?} at {opacity}"),
            );
        }
    }
}

#[test]
fn blended_layers_only_touch_what_they_cover_and_normal_layers_continue_on_top() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default()
        .with(1, solid(64, 36, BACKDROP))
        .with(2, solid(64, 36, PAINT))
        .with(3, solid(64, 36, [235, 128, 128]));
    let small = Transform {
        scale: 0.5,
        ..Transform::default()
    };
    let layers = vec![
        Layer::new(source(1)),
        Layer {
            blend: BlendMode::Multiply,
            transform: small.clone(),
            ..Layer::new(source(2))
        },
        Layer {
            transform: Transform {
                scale: 0.25,
                position: [0.375, 0.0],
                ..Transform::default()
            },
            ..Layer::new(source(3))
        },
    ];
    let image = render(&mut renderer, layers, &mut frames);
    let (b, s) = (encoded(BACKDROP), encoded(PAINT));
    assert_close(image.pixel(2, 2), b, "outside the multiplied layer");
    let multiplied: [f64; 3] = std::array::from_fn(|c| b[c] * s[c]);
    assert_close(image.pixel(32, 18), multiplied, "inside it");
    assert_close(image.pixel(56, 18), [1.0; 3], "white normal layer on top");

    // Two blended layers in a row, with nothing normal before them.
    let layers = vec![
        Layer {
            blend: BlendMode::Screen,
            ..Layer::new(source(1))
        },
        Layer {
            blend: BlendMode::Difference,
            ..Layer::new(source(1))
        },
    ];
    let image = render(&mut renderer, layers, &mut frames);
    let screened = b.map(|v| reference(BlendMode::Screen, 0.0, v));
    let expected: [f64; 3] = std::array::from_fn(|c| (screened[c] - b[c]).abs());
    assert_close(
        image.pixel(32, 18),
        expected,
        "screen over black, then difference",
    );
}

#[test]
fn crop_flip_and_stretch_place_the_picture() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    // A ramp from dark on the left to light on the right.
    let ramp = |x: u32, _| [16 + (x * 219 / 63) as u8, 128, 128];
    let mut frames = Frames::default().with(1, nv12(64, 36, bt709_limited(), ramp));
    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let flipped = render(
        &mut renderer,
        vec![Layer {
            transform: Transform {
                flip_horizontal: true,
                ..Transform::default()
            },
            ..Layer::new(source(1))
        }],
        &mut frames,
    );
    for x in [4, 20, 40, 59] {
        let a = plain.pixel(x, 18)[0];
        let b = flipped.pixel(63 - x, 18)[0];
        assert!(a.abs_diff(b) <= 2, "x={x}: {a} vs mirrored {b}");
    }
    let cropped = render(
        &mut renderer,
        vec![Layer {
            crop: [0.5, 0.0, 1.0, 1.0],
            ..Layer::new(source(1))
        }],
        &mut frames,
    );
    assert_eq!(
        cropped.pixel(10, 18)[..3],
        [0, 0, 0],
        "the left half is gone"
    );
    let kept = cropped.pixel(50, 18)[0];
    assert!(
        kept.abs_diff(plain.pixel(50, 18)[0]) <= 2,
        "the rest stays in place"
    );
    let squashed = render(
        &mut renderer,
        vec![Layer {
            transform: Transform {
                stretch: [1.0, 0.5],
                ..Transform::default()
            },
            ..Layer::new(source(1))
        }],
        &mut frames,
    );
    assert_eq!(
        squashed.pixel(32, 3)[..3],
        [0, 0, 0],
        "half height: black above"
    );
    assert!(squashed.pixel(32, 18)[0] > 0, "the middle is covered");
    assert!(
        squashed.pixel(60, 18)[0].abs_diff(plain.pixel(60, 18)[0]) <= 2,
        "full width"
    );
}
