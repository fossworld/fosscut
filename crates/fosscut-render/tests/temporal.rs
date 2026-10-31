//! Effects that read neighbouring frames (ADR-0035), on the GPU and measured against
//! references computed on the CPU: optical-flow slow motion against the true in-between
//! frame, noise reduction by variance and edge sharpness, flicker removal by brightness
//! variation, and motion blur against a box-filtered pan and an averaged moving shape.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeMap;

use fosscut_media::VideoFrame;
use fosscut_model::{ParamValue, Transform, effect_type};
use fosscut_render::color::TransferFunction;
use fosscut_render::effects::PlannedEffect;
use fosscut_render::plan::{Layer, Neighbours, RenderPlan, SourceRef};
use fosscut_render::{Image, Renderer};
use support::*;

const W: u32 = 320;
const H: u32 = 180;

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
        time: 0.0,
    }
}

fn n(v: f64) -> ParamValue {
    ParamValue::Number(v)
}

fn render(renderer: &mut Renderer, layer: Layer, frames: &mut Frames) -> Image {
    let plan = RenderPlan {
        canvas: (W, H),
        layers: vec![layer],
        skipped: Vec::new(),
    };
    renderer
        .render_image(&plan, frames, (W, H), TransferFunction::Srgb)
        .unwrap()
}

fn neighbours(before: &[u64], after: &[u64], per_frame: f64) -> Neighbours {
    Neighbours {
        before: before.iter().map(|&n| source(n)).collect(),
        after: after.iter().map(|&n| source(n)).collect(),
        per_frame,
    }
}

/// PSNR over the pixels at least `margin` from every edge.
fn psnr_inside(a: &Image, b: &Image, margin: u32) -> f64 {
    let crop = |image: &Image| {
        let mut rgba = Vec::new();
        for y in margin..image.height - margin {
            for x in margin..image.width - margin {
                rgba.extend_from_slice(&image.pixel(x, y));
            }
        }
        Image {
            width: image.width - 2 * margin,
            height: image.height - 2 * margin,
            rgba,
        }
    };
    crop(a).psnr(&crop(b)).unwrap()
}

/// The sRGB-encoded green of a pixel, 0 to 1.
fn green(image: &Image, x: u32, y: u32) -> f64 {
    f64::from(image.pixel(x, y)[1]) / 255.0
}

/// Mean linear luminance of an sRGB image.
fn luminance(image: &Image) -> f64 {
    let decode = |c: u8| f64::from(TransferFunction::Srgb.decode(f32::from(c) / 255.0));
    let mut sum = 0.0;
    for p in image.rgba.chunks(4) {
        sum += 0.2126 * decode(p[0]) + 0.7152 * decode(p[1]) + 0.0722 * decode(p[2]);
    }
    sum / (image.rgba.len() / 4) as f64
}

#[test]
fn flow_slow_motion_matches_the_true_middle_frame_better_than_blending() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default()
        .with(1, frame_of(|x, y| scene(x, y, 0.0)))
        .with(2, frame_of(|x, y| scene(x, y, 1.0)))
        .with(3, frame_of(|x, y| scene(x, y, 0.5)))
        .with(4, frame_of(|x, y| scene(x, y, 0.25)));
    let between = |amount: f32, flow: bool, renderer: &mut Renderer, frames: &mut Frames| {
        let layer = Layer {
            next_frame: Some((source(2), amount)),
            flow,
            ..Layer::new(source(1))
        };
        render(renderer, layer, frames)
    };
    let truth = render(&mut renderer, Layer::new(source(3)), &mut frames);
    let flowed = between(0.5, true, &mut renderer, &mut frames);
    let blended = between(0.5, false, &mut renderer, &mut frames);
    // Frame edges show what neither frame has; compare inside them.
    let (flow_db, blend_db) = (
        psnr_inside(&flowed, &truth, 12),
        psnr_inside(&blended, &truth, 12),
    );
    eprintln!("middle frame: flow {flow_db:.1} dB, blend {blend_db:.1} dB");
    // Measured 39.2 dB against 30.1 dB on Metal: flow keeps both moving parts sharp where
    // blending shows each twice.
    assert!(
        flow_db >= blend_db + 6.0,
        "flow {flow_db:.1} dB, blend {blend_db:.1} dB"
    );
    assert!(flow_db >= 34.0, "flow {flow_db:.1} dB");

    // A quarter of the way, and from a flow kept the other way round: the same frame.
    let truth = render(&mut renderer, Layer::new(source(4)), &mut frames);
    let quarter = between(0.25, true, &mut renderer, &mut frames);
    let quarter_db = psnr_inside(&quarter, &truth, 12);
    eprintln!("quarter frame: flow {quarter_db:.1} dB");
    assert!(quarter_db >= 34.0, "{quarter_db:.1} dB");
    let reversed = render(
        &mut renderer,
        Layer {
            next_frame: Some((source(1), 0.75)),
            flow: true,
            ..Layer::new(source(2))
        },
        &mut frames,
    );
    let symmetric = reversed.psnr(&quarter).unwrap();
    assert!(
        symmetric >= 50.0,
        "the kept flow read backwards: {symmetric:.1} dB"
    );

    // At either end the in-between frame is the end frame.
    let first = render(&mut renderer, Layer::new(source(1)), &mut frames);
    let start = between(0.0, true, &mut renderer, &mut frames);
    assert!(start.psnr(&first).unwrap() >= 40.0);
}

/// A flat dark and a flat light half with a sharp vertical edge at x = 160, and noise seeded
/// by `seed`, moved right by `shift` pixels.
fn noisy(seed: u32, shift: f32, amplitude: f32) -> VideoFrame {
    frame_of(move |x, y| {
        let x = x - shift;
        let base = if x < 160.0 { 0.06 } else { 0.4 };
        // Texture the flow can follow, and noise it cannot.
        let texture = 0.04 * value_noise(x, y, 9.0, 3);
        let noise = (hash(x.floor() as i32, y.floor() as i32, seed) - 0.5) * amplitude;
        let v = base + texture;
        let encoded = TransferFunction::Srgb.encode(v) + noise;
        let l = TransferFunction::Srgb.decode(encoded.clamp(0.0, 1.0));
        [l, l, l]
    })
}

/// Variance of the green channel over a flat region.
fn variance(image: &Image, xs: std::ops::Range<u32>, ys: std::ops::Range<u32>) -> f64 {
    let values: Vec<f64> = ys
        .flat_map(|y| xs.clone().map(move |x| (x, y)))
        .map(|(x, y)| green(image, x, y))
        .collect();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
}

/// The step across the edge at x = 160 + `shift`: the difference of the row-averaged values
/// two pixels either side of it, and how many pixels the rise from 10% to 90% takes.
fn edge(image: &Image, shift: u32) -> (f64, u32) {
    let column = |x: u32| (40..140).map(|y| green(image, x, y)).sum::<f64>() / 100.0;
    let at = 160 + shift;
    let (low, high) = (column(at - 6), column(at + 5));
    let step = high - low;
    let rise = (at - 6..at + 6)
        .filter(|&x| {
            let v = (column(x) - low) / step;
            (0.1..0.9).contains(&v)
        })
        .count() as u32;
    (column(at + 1) - column(at - 2), rise)
}

#[test]
fn noise_reduction_lowers_variance_and_keeps_edges_sharp() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let amplitude = 0.16;
    let mut frames = Frames::default()
        .with(1, noisy(11, 0.0, amplitude))
        .with(2, noisy(12, 0.0, amplitude))
        .with(3, noisy(13, 0.0, amplitude))
        .with(9, noisy(0, 0.0, 0.0));
    let clean = render(&mut renderer, Layer::new(source(9)), &mut frames);
    let plain = render(&mut renderer, Layer::new(source(1)), &mut frames);
    let denoised = |strength: f64, temporal: bool, renderer: &mut Renderer, frames: &mut Frames| {
        let layer = Layer {
            effects: vec![effect(
                "detail.denoise",
                &[
                    ("strength", n(strength)),
                    ("temporal", ParamValue::Flag(temporal)),
                ],
            )],
            neighbours: neighbours(&[2], &[3], 1.0),
            ..Layer::new(source(1))
        };
        render(renderer, layer, frames)
    };
    let spatial = denoised(0.6, false, &mut renderer, &mut frames);
    let temporal = denoised(0.6, true, &mut renderer, &mut frames);
    let none = denoised(0.0, true, &mut renderer, &mut frames);
    assert_eq!(none, plain, "strength 0 leaves the picture alone");

    let flat = |image: &Image| {
        (variance(image, 20..140, 30..150) + variance(image, 180..300, 30..150)) / 2.0
    };
    let (v_plain, v_spatial, v_temporal, v_clean) =
        (flat(&plain), flat(&spatial), flat(&temporal), flat(&clean));
    eprintln!(
        "variance: noisy {v_plain:.6}, spatial {v_spatial:.6}, temporal {v_temporal:.6}, clean {v_clean:.6}"
    );
    // The noise is what the noisy frame has beyond the clean picture's own texture.
    let noise = |v: f64| (v - v_clean).max(0.0);
    assert!(noise(v_spatial) <= 0.4 * noise(v_plain), "spatial");
    assert!(noise(v_temporal) <= 0.25 * noise(v_plain), "temporal");
    assert!(v_temporal < v_spatial, "the neighbours help");

    let (step_clean, rise_clean) = edge(&clean, 0);
    for (what, image) in [("spatial", &spatial), ("temporal", &temporal)] {
        let (step, rise) = edge(image, 0);
        eprintln!(
            "{what} edge: step {step:.3} (clean {step_clean:.3}), rise {rise} px (clean {rise_clean})"
        );
        assert!(
            step >= 0.9 * step_clean,
            "{what}: the edge keeps its contrast"
        );
        assert!(rise <= rise_clean + 1, "{what}: the edge stays sharp");
    }
}

#[test]
fn temporal_noise_reduction_follows_motion() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    // The picture moves right 3 pixels a frame; each frame has its own noise.
    let amplitude = 0.12;
    let mut frames = Frames::default()
        .with(1, noisy(21, 3.0, amplitude))
        .with(2, noisy(22, 0.0, amplitude))
        .with(3, noisy(23, 6.0, amplitude))
        .with(9, noisy(0, 3.0, 0.0));
    let clean = render(&mut renderer, Layer::new(source(9)), &mut frames);
    let with = |temporal: bool, renderer: &mut Renderer, frames: &mut Frames| {
        let layer = Layer {
            effects: vec![effect(
                "detail.denoise",
                &[
                    ("strength", n(0.6)),
                    ("temporal", ParamValue::Flag(temporal)),
                ],
            )],
            neighbours: neighbours(&[2], &[3], 1.0),
            ..Layer::new(source(1))
        };
        render(renderer, layer, frames)
    };
    let spatial = with(false, &mut renderer, &mut frames);
    let temporal = with(true, &mut renderer, &mut frames);
    let (s_db, t_db) = (
        psnr_inside(&spatial, &clean, 10),
        psnr_inside(&temporal, &clean, 10),
    );
    eprintln!("moving: spatial {s_db:.1} dB, temporal {t_db:.1} dB against the clean frame");
    assert!(t_db >= s_db + 1.0, "aligned neighbours remove more noise");
    let (step_clean, _) = edge(&clean, 3);
    let (step, rise) = edge(&temporal, 3);
    assert!(
        step >= 0.9 * step_clean && rise <= 3,
        "no ghost of the edge: {step:.3}, {rise}"
    );
}

/// A textured still picture at `gain` times its brightness.
fn lit(gain: f32) -> VideoFrame {
    frame_of(move |x, y| {
        let v = 0.05 + 0.5 * value_noise(x, y, 11.0, 5) + if x > 200.0 { 0.2 } else { 0.0 };
        [v * gain, v * gain * 0.9, v * gain * 0.8]
    })
}

#[test]
fn flicker_removal_evens_brightness_and_keeps_fades() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    const COUNT: u64 = 24;
    let deflicker = |gains: &dyn Fn(u64) -> f32, renderer: &mut Renderer| {
        let mut frames = Frames::default();
        for i in 0..COUNT {
            frames = frames.with(i + 1, lit(gains(i)));
        }
        let (mut before, mut after) = (Vec::new(), Vec::new());
        for i in 0..COUNT {
            let reach = |d: i64| -> Vec<u64> {
                (1..=3)
                    .map(|k| i as i64 + d * k)
                    .take_while(|&j| (0..COUNT as i64).contains(&j))
                    .map(|j| j as u64 + 1)
                    .collect()
            };
            let layer = |effects| Layer {
                effects,
                neighbours: Neighbours {
                    before: reach(-1).into_iter().map(source).collect(),
                    after: reach(1).into_iter().map(source).collect(),
                    per_frame: 1.0,
                },
                ..Layer::new(source(i + 1))
            };
            before.push(luminance(&render(renderer, layer(Vec::new()), &mut frames)));
            let fixed = layer(vec![effect("color.deflicker", &[("window", n(6.0))])]);
            after.push(luminance(&render(renderer, fixed, &mut frames)));
        }
        (before, after)
    };
    // Frames 3 to 20 have all three neighbours on both sides.
    let interior = 3..(COUNT as usize - 3);
    let variation = |levels: &[f64]| {
        let pairs = interior.start..interior.end - 1;
        pairs
            .clone()
            .map(|i| (levels[i + 1] - levels[i]).abs())
            .sum::<f64>()
            / pairs.len() as f64
    };

    // 5% brighter and darker every other frame over a slow fade from full to 70%.
    let fade = |i: u64| 1.0 - 0.3 * i as f32 / (COUNT - 1) as f32;
    let flicker = |i: u64| fade(i) * if i.is_multiple_of(2) { 1.05 } else { 0.95 };
    let (flickering, fixed) = deflicker(&flicker, &mut renderer);
    let (faded, kept) = deflicker(&fade, &mut renderer);
    let (v_before, v_after, v_fade) =
        (variation(&flickering), variation(&fixed), variation(&faded));
    eprintln!(
        "frame-to-frame brightness change: flickering {v_before:.5}, deflickered {v_after:.5}, the fade alone {v_fade:.5}"
    );
    // The flicker is what changes beyond the fade's own steady change.
    let drop = 1.0 - (v_after - v_fade).abs() / (v_before - v_fade);
    eprintln!("flicker reduced by {:.1}%", 100.0 * drop);
    assert!(
        drop >= 0.8,
        "flicker drops by at least 80%: {:.1}%",
        100.0 * drop
    );
    for i in interior {
        let relative = |a: f64, b: f64| (a - b).abs() / b;
        assert!(
            relative(kept[i], faded[i]) < 0.01,
            "frame {i} keeps the fade"
        );
        assert!(
            relative(fixed[i], faded[i]) < 0.02,
            "frame {i} follows the fade"
        );
    }
}

/// The background of [`scene`] alone at time `t`, moving left 4 pixels a frame.
fn pan(x: f32, y: f32, t: f32) -> [f32; 3] {
    let bx = x + 12.0 * t;
    let n = 0.6 * value_noise(bx, y, 7.0, 1) + 0.4 * value_noise(bx, y, 3.0, 2);
    let stripe = if (bx / 23.0).floor() as i32 % 2 == 0 {
        0.15
    } else {
        0.0
    };
    let band = if (60.0..64.0).contains(&y) { 0.3 } else { 0.0 };
    [
        0.1 + 0.5 * n + stripe + band,
        0.15 + 0.55 * n + band,
        0.3 + 0.4 * n * n + band,
    ]
}

#[test]
fn motion_blur_follows_the_flow_and_the_shutter() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    // A pan of 12 pixels a frame: with a half-frame shutter, a 6-pixel box blur along it.
    let mut frames = Frames::default()
        .with(1, frame_of(|x, y| pan(x, y, 0.0)))
        .with(2, frame_of(|x, y| pan(x, y, -1.0)))
        .with(3, frame_of(|x, y| pan(x, y, 1.0)));
    let samples = 64;
    let reference = frame_of(|x, y| {
        let mut sum = [0.0f32; 3];
        for i in 0..samples {
            let t = (i as f32 + 0.5) / samples as f32 - 0.5;
            let c = pan(x, y, 0.5 * t);
            for k in 0..3 {
                sum[k] += c[k] / samples as f32;
            }
        }
        sum
    });
    frames = frames.with(9, reference);
    let blurred = |amount: f64, renderer: &mut Renderer, frames: &mut Frames| {
        let layer = Layer {
            effects: vec![effect(
                "motion.blur",
                &[("amount", n(amount)), ("samples", n(24.0))],
            )],
            neighbours: neighbours(&[2], &[3], 1.0),
            ..Layer::new(source(1))
        };
        render(renderer, layer, frames)
    };
    let sharp = render(&mut renderer, Layer::new(source(1)), &mut frames);
    let truth = render(&mut renderer, Layer::new(source(9)), &mut frames);
    let half = blurred(0.5, &mut renderer, &mut frames);
    let (blur_db, sharp_db) = (
        psnr_inside(&half, &truth, 16),
        psnr_inside(&sharp, &truth, 16),
    );
    eprintln!("pan blur: {blur_db:.1} dB against the box-filtered pan, {sharp_db:.1} dB unblurred");
    assert!(blur_db >= sharp_db + 5.0 && blur_db >= 30.0);
    assert_eq!(
        blurred(0.0, &mut renderer, &mut frames),
        sharp,
        "a closed shutter blurs nothing"
    );
    // The band across the pan stays sharp: motion is horizontal only.
    let band = |image: &Image| green(image, 100, 61) - green(image, 100, 57);
    assert!(
        band(&half) >= 0.9 * band(&sharp),
        "{} vs {}",
        band(&half),
        band(&sharp)
    );
}

#[test]
fn a_moving_layer_streaks_along_its_shutter_path() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let white = [235, 128, 128];
    let mut frames = Frames::default()
        .with(1, solid(32, 32, white))
        .with(2, solid(W, H, [16, 128, 128]));
    let at = |x: f64| Transform {
        position: [x, 0.0],
        scale: 0.2,
        ..Transform::default()
    };
    // Eight placements from 0.1 to 0.3 of the canvas to the right of centre.
    let path: Vec<Transform> = (0..8).map(|i| at(0.1 + 0.2 * f64::from(i) / 7.0)).collect();
    let square = |shutter: Vec<Transform>| Layer {
        transform: at(0.2),
        shutter,
        ..Layer::new(source(1))
    };
    let plan = |layer: Layer| RenderPlan {
        canvas: (W, H),
        layers: vec![Layer::new(source(2)), layer],
        skipped: Vec::new(),
    };
    let image = |layer: Layer, renderer: &mut Renderer, frames: &mut Frames| {
        renderer
            .render_image(&plan(layer), frames, (W, H), TransferFunction::Linear)
            .unwrap()
    };
    let streak = image(square(path.clone()), &mut renderer, &mut frames);
    // The reference: each placement drawn alone, averaged in linear light.
    let mut sum = vec![0.0f64; (W * H * 4) as usize];
    for transform in &path {
        let alone = image(
            square(Vec::new()).with_transform(transform),
            &mut renderer,
            &mut frames,
        );
        for (s, v) in sum.iter_mut().zip(&alone.rgba) {
            *s += f64::from(*v) / path.len() as f64;
        }
    }
    let averaged = Image {
        width: W,
        height: H,
        rgba: sum.iter().map(|v| v.round() as u8).collect(),
    };
    let db = streak.psnr(&averaged).unwrap();
    eprintln!("streak against the average of its placements: {db:.1} dB");
    assert!(db >= 40.0, "{db:.1} dB");
    // The streak reaches both ends of the path, fainter than the still square.
    let still = image(square(Vec::new()), &mut renderer, &mut frames);
    let centre_y = H / 2;
    let left_end = (W as f64 * (0.5 + 0.1 - 0.05)) as u32 + 2;
    assert!(
        streak.pixel(left_end, centre_y)[1] > 20,
        "the streak starts at the first placement"
    );
    assert!(
        still.pixel(left_end, centre_y)[1] < 20,
        "where the still square is not"
    );
    assert!(streak.pixel(W / 2 + W / 5, centre_y)[1] < still.pixel(W / 2 + W / 5, centre_y)[1]);
}

trait WithTransform {
    fn with_transform(self, transform: &Transform) -> Self;
}

impl WithTransform for Layer {
    fn with_transform(mut self, transform: &Transform) -> Layer {
        self.transform = transform.clone();
        self
    }
}

#[test]
fn neighbours_missing_or_frozen_leave_the_picture_alone() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default()
        .with(1, frame_of(|x, y| pan(x, y, 0.0)))
        .with(3, frame_of(|x, y| pan(x, y, 1.0)));
    let plain = render(&mut renderer, Layer::new(source(1)), &mut frames);
    let with = |effects: Vec<PlannedEffect>,
                neighbours: Neighbours,
                renderer: &mut Renderer,
                frames: &mut Frames| {
        render(
            renderer,
            Layer {
                effects,
                neighbours,
                ..Layer::new(source(1))
            },
            frames,
        )
    };
    let blur = || vec![effect("motion.blur", &[("amount", n(1.0))])];
    // A freeze frame does not move: no blur.
    let frozen = with(
        blur(),
        neighbours(&[], &[3], 0.0),
        &mut renderer,
        &mut frames,
    );
    assert_eq!(frozen, plain);
    // Without neighbours, nothing to blur along.
    let alone = with(blur(), Neighbours::default(), &mut renderer, &mut frames);
    assert_eq!(alone, plain);
    // A neighbour the source cannot supply is left out, and one side is enough.
    let one_side = with(
        blur(),
        neighbours(&[7], &[3], 1.0),
        &mut renderer,
        &mut frames,
    );
    assert!(
        one_side.psnr(&plain).unwrap() < 40.0,
        "the next frame alone gives the motion"
    );
    // Flicker removal needs neighbours on both sides.
    let deflicker = vec![effect("color.deflicker", &[])];
    let edge = with(
        deflicker,
        neighbours(&[], &[3], 1.0),
        &mut renderer,
        &mut frames,
    );
    assert_eq!(edge, plain);
}

#[test]
fn neighbour_frames_are_asked_for_in_time_order() {
    use fosscut_render::{FrameProvider, RenderError};
    use fosscut_time::{Flicks, Rate};
    struct Recorder {
        inner: Frames,
        asked: Vec<Flicks>,
    }
    impl FrameProvider for Recorder {
        fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError> {
            self.asked.push(source.time);
            self.inner.frame(source)
        }
    }
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut recorder = Recorder {
        inner: Frames::default(),
        asked: Vec::new(),
    };
    recorder.inner.moving.insert(asset(1), (W, H));
    let at = |n: i64| SourceRef {
        asset: asset(1),
        time: Rate::FPS_30.frames_to_flicks(n),
    };
    let layer = Layer {
        effects: vec![
            effect("color.deflicker", &[("window", n(4.0))]),
            effect("motion.blur", &[]),
        ],
        neighbours: Neighbours {
            before: vec![at(9), at(8)],
            after: vec![at(11), at(12)],
            per_frame: 1.0,
        },
        next_frame: Some((at(11), 0.5)),
        flow: true,
        ..Layer::new(at(10))
    };
    let plan = RenderPlan {
        canvas: (W, H),
        layers: vec![layer],
        skipped: Vec::new(),
    };
    renderer
        .render_image(&plan, &mut recorder, (W, H), TransferFunction::Srgb)
        .unwrap();
    let times: Vec<i64> = recorder
        .asked
        .iter()
        .map(|t| Rate::FPS_30.frame_at(*t))
        .collect();
    assert_eq!(times, [8, 9, 10, 11, 12], "each frame once, earliest first");
}

/// A decoder of a moving pattern that counts the frames it decodes and the times it seeks
/// back, as a real decoder must when asked for an earlier frame.
struct Counting {
    info: fosscut_media::VideoStreamInfo,
    position: i64,
    decoded: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    seeks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl fosscut_media::VideoDecoder for Counting {
    fn info(&self) -> &fosscut_media::VideoStreamInfo {
        &self.info
    }

    fn acceleration(&self) -> fosscut_media::Acceleration {
        fosscut_media::Acceleration::Software
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, fosscut_media::MediaError> {
        use std::sync::atomic::Ordering;
        let rate = fosscut_time::Rate::FPS_30;
        let n = self.position;
        self.position += 1;
        self.decoded.fetch_add(1, Ordering::SeqCst);
        let mut frame = pattern(64, 36, (n as u32 * 3) % 64);
        frame.pts = rate.frames_to_flicks(n);
        frame.duration = rate.frame_duration();
        Ok(Some(frame))
    }

    fn frame_at(
        &mut self,
        time: fosscut_time::Flicks,
    ) -> Result<VideoFrame, fosscut_media::MediaError> {
        use std::sync::atomic::Ordering;
        let target = fosscut_time::Rate::FPS_30.frame_at(time);
        if target < self.position {
            self.seeks.fetch_add(1, Ordering::SeqCst);
            self.position = target;
        }
        while self.position < target {
            self.next_frame()?;
        }
        Ok(self.next_frame()?.unwrap())
    }
}

#[test]
fn decoders_read_forward_while_neighbours_are_rendered_in_order() {
    use fosscut_render::DecoderFrames;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let Some(mut renderer) = renderer() else {
        return;
    };
    let rate = fosscut_time::Rate::FPS_30;
    let run = |window: usize, renderer: &mut Renderer| {
        let (decoded, seeks) = (
            std::sync::Arc::new(AtomicUsize::new(0)),
            std::sync::Arc::new(AtomicUsize::new(0)),
        );
        let decoder = Counting {
            info: fosscut_media::VideoStreamInfo {
                index: 0,
                codec: "test".into(),
                width: 64,
                height: 36,
                rate: Some(rate),
                time_base: (1, 30),
                duration: None,
                frame_count: None,
                rotation: 0,
                color: bt709_limited(),
                source_pixel_format: "nv12".into(),
            },
            position: 0,
            decoded: decoded.clone(),
            seeks: seeks.clone(),
        };
        let mut frames = DecoderFrames::new();
        frames.insert(asset(1), Box::new(decoder));
        frames.set_window_bytes(window);
        let at = |n: i64| SourceRef {
            asset: asset(1),
            time: rate.frames_to_flicks(n)
                + fosscut_time::Flicks::new(rate.frame_duration().get() / 2),
        };
        let mut images = Vec::new();
        for i in 1..=12 {
            let layer = Layer {
                effects: vec![
                    effect("detail.denoise", &[]),
                    effect("color.deflicker", &[("window", n(4.0))]),
                ],
                neighbours: Neighbours {
                    before: (1..=2)
                        .map(|k| at(i - k))
                        .filter(|s| s.time.get() >= 0)
                        .collect(),
                    after: (1..=2).map(|k| at(i + k)).collect(),
                    per_frame: 1.0,
                },
                ..Layer::new(at(i))
            };
            let plan = RenderPlan {
                canvas: (64, 36),
                layers: vec![layer],
                skipped: Vec::new(),
            };
            images.push(
                renderer
                    .render_image(&plan, &mut frames, (64, 36), TransferFunction::Srgb)
                    .unwrap(),
            );
        }
        (
            decoded.load(Ordering::SeqCst),
            seeks.load(Ordering::SeqCst),
            frames.window_bytes(),
            images,
        )
    };
    let (decoded, seeks, kept, images) = run(fosscut_render::WINDOW_BYTES, &mut renderer);
    // Frames 0 to 14, each decoded once, never seeking back.
    assert_eq!((decoded, seeks), (15, 0));
    assert!(kept > 0 && kept <= fosscut_render::WINDOW_BYTES);
    let (decoded, seeks, kept, others) = run(0, &mut renderer);
    assert!(
        seeks > 0 && decoded > 15,
        "without the window: {decoded} decoded, {seeks} seeks"
    );
    assert_eq!(kept, 0);
    assert_eq!(images, others, "the same frames either way");
}
