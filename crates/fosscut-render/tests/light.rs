//! Light effects on the GPU (ADR-0011): glows and streaks spread past their layer through
//! a padded picture that never moves the layer, sweeps light only visible pixels, and clip
//! animations plan their blur and glow so preview, frame grabs and export draw them alike.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_model::{
    Animation, Clip, ClipAnimations, ClipContent, Id, IdKind, ParamValue, Project, TextContent,
    TextStyle, TimelineFormat, Track, TrackKind, Transform,
};
use fosscut_render::color::TransferFunction;
use fosscut_render::effects::PlannedEffect;
use fosscut_render::plan::{Layer, RenderPlan, plan_frame};
use fosscut_render::{ExportGraph, Image, PreviewGraph, Renderer};
use fosscut_time::{Flicks, Rate};
use support::*;

const CANVAS: (u32, u32) = (320, 180);
const WHITE: [u8; 3] = [235, 128, 128];
const GREY: [u8; 3] = [126, 128, 128];

fn effect(kind: &str, values: &[(&str, ParamValue)]) -> PlannedEffect {
    let spec = fosscut_model::effect_type(kind).unwrap();
    let mut all: std::collections::BTreeMap<String, ParamValue> = spec
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

fn n(value: f64) -> ParamValue {
    ParamValue::Number(value)
}

/// A square of asset 1 at a third of the fitted size in the middle of the canvas: 60
/// pixels wide, from x 130 to 190 and y 60 to 120.
fn square(effects: Vec<PlannedEffect>) -> Layer {
    Layer {
        transform: Transform {
            scale: 1.0 / 3.0,
            ..Transform::default()
        },
        effects,
        ..Layer::new(source(1))
    }
}

fn render(renderer: &mut Renderer, layers: Vec<Layer>, frames: &mut Frames) -> Image {
    let plan = RenderPlan {
        canvas: CANVAS,
        layers,
        skipped: Vec::new(),
    };
    renderer
        .render_image(&plan, frames, CANVAS, TransferFunction::Srgb)
        .unwrap()
}

/// PSNR between `a` and `b` over the pixels of `a` well inside its picture: every pixel
/// within three of them shows something. A padded picture's outline is filtered in its own
/// pixels rather than cut by the quad, so edges may soften, but nothing inside may move.
fn interior_psnr(a: &Image, b: &Image) -> f64 {
    let shows = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && x < i64::from(a.width)
            && y < i64::from(a.height)
            && luma(a, x as u32, y as u32) > 0
    };
    let (mut sum, mut count) = (0.0f64, 0u64);
    for y in 0..a.height {
        for x in 0..a.width {
            let (cx, cy) = (i64::from(x), i64::from(y));
            if !(-3..=3).all(|dy| (-3..=3).all(|dx| shows(cx + dx, cy + dy))) {
                continue;
            }
            let (p, q) = (a.pixel(x, y), b.pixel(x, y));
            for c in 0..3 {
                let d = f64::from(p[c]) - f64::from(q[c]);
                sum += d * d;
                count += 1;
            }
        }
    }
    assert!(count > 3000, "enough of the picture is inside: {count}");
    let mse = sum / count as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    }
}

/// The smallest box holding every pixel brighter than black: left, top, right, bottom.
fn bounds(image: &Image) -> [u32; 4] {
    let mut b = [u32::MAX, u32::MAX, 0, 0];
    for y in 0..image.height {
        for x in 0..image.width {
            if luma(image, x, y) > 8 {
                b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
        }
    }
    b
}

fn luma(image: &Image, x: u32, y: u32) -> u32 {
    let p = image.pixel(x, y);
    (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) / 3
}

#[test]
fn glow_spreads_past_the_layer_on_every_side() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default().with(1, solid(64, 64, WHITE));
    let plain = render(&mut renderer, vec![square(vec![])], &mut frames);
    let glow = effect("light.glow", &[("radius", n(0.2))]);
    let lit = render(&mut renderer, vec![square(vec![glow.clone()])], &mut frames);
    // Outside the square: dark without the glow, lit with it, on all four sides.
    for (x, y) in [(195, 90), (124, 90), (160, 54), (160, 126)] {
        assert!(luma(&plain, x, y) < 3, "({x}, {y}) is outside the square");
        assert!(luma(&lit, x, y) > 40, "({x}, {y}): {}", luma(&lit, x, y));
    }
    // It fades with distance and leaves the far corners dark.
    assert!(luma(&lit, 195, 90) > luma(&lit, 215, 90));
    assert!(luma(&lit, 5, 5) < 3);
    assert_eq!(luma(&lit, 160, 90), 255, "the square stays white");
    // Only the light, without the layer: the inside of a large square still glows, but a
    // tinted glow is coloured where the picture was white.
    let only = effect(
        "light.glow",
        &[
            ("radius", n(0.2)),
            ("glow_only", ParamValue::Flag(true)),
            ("tint", n(1.0)),
            ("color", ParamValue::Name("#FF6600".into())),
        ],
    );
    let orange = render(&mut renderer, vec![square(vec![only])], &mut frames);
    let p = orange.pixel(160, 90);
    assert!(p[0] > 200 && p[2] < 30, "orange light alone: {p:?}");
    let p = orange.pixel(197, 90);
    assert!(p[0] > p[1] && p[1] > p[2], "orange outside too: {p:?}");
    // A glow on a layer below another still shows around it, over the canvas.
    let below = render(
        &mut renderer,
        vec![square(vec![glow]), square(vec![])],
        &mut frames,
    );
    assert!(luma(&below, 195, 90) > 40);
}

#[test]
fn padding_never_moves_or_resizes_the_picture() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    // A glow whose light is far too faint to see still pads the picture.
    let faint = effect("light.glow", &[("radius", n(0.1)), ("intensity", n(1e-6))]);
    let mut frames = Frames::default()
        .with(1, pattern(96, 64, 0))
        .with(2, pattern(64, 96, 7));
    let placements = [
        Transform {
            position: [0.1, -0.05],
            scale: 0.6,
            rotation: 17.0,
            ..Transform::default()
        },
        Transform {
            scale: 0.5,
            stretch: [1.4, 0.8],
            flip_horizontal: true,
            ..Transform::default()
        },
        Transform {
            scale: 0.7,
            flip_vertical: true,
            rotation: -30.0,
            ..Transform::default()
        },
    ];
    for (index, transform) in placements.iter().enumerate() {
        for crop in [[0.0, 0.0, 1.0, 1.0], [0.1, 0.0, 0.8, 0.7]] {
            for n in [1, 2] {
                let layer = |effects: Vec<PlannedEffect>| Layer {
                    transform: transform.clone(),
                    crop,
                    effects,
                    ..Layer::new(source(n))
                };
                let plain = render(&mut renderer, vec![layer(vec![])], &mut frames);
                let padded = render(&mut renderer, vec![layer(vec![faint.clone()])], &mut frames);
                let psnr = interior_psnr(&plain, &padded);
                let what = format!("placement {index}, crop {crop:?}, asset {n}");
                assert!(psnr > 45.0, "{what}: {psnr:.1} dB");
                let (p, q) = (bounds(&plain), bounds(&padded));
                // Within the softening of an edge: half a source pixel, two output pixels.
                assert!(
                    p.iter().zip(q).all(|(a, b)| a.abs_diff(b) <= 2),
                    "{what}: {p:?} vs {q:?}"
                );
            }
        }
    }
    // A source stored on its side and turned for display keeps its place too.
    let turned = |effects: Vec<PlannedEffect>| Layer {
        rotation: 90,
        transform: Transform {
            scale: 0.5,
            ..Transform::default()
        },
        effects,
        ..Layer::new(source(1))
    };
    let plain = render(&mut renderer, vec![turned(vec![])], &mut frames);
    let padded = render(&mut renderer, vec![turned(vec![faint])], &mut frames);
    assert!(interior_psnr(&plain, &padded) > 45.0);
    let (p, q) = (bounds(&plain), bounds(&padded));
    assert!(
        p.iter().zip(q).all(|(a, b)| a.abs_diff(b) <= 2),
        "{p:?} vs {q:?}"
    );
}

#[test]
fn cropped_edges_cut_the_light_and_thresholds_pick_highlights() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default().with(1, solid(64, 64, WHITE));
    let glow = effect("light.glow", &[("radius", n(0.2))]);
    // The right half cropped away: light still spreads left, but not past the crop.
    let cropped = Layer {
        crop: [0.0, 0.0, 0.5, 1.0],
        ..square(vec![glow])
    };
    let image = render(&mut renderer, vec![cropped], &mut frames);
    assert!(luma(&image, 124, 90) > 40, "left of the picture");
    assert!(luma(&image, 165, 90) < 3, "right of the crop");
    // A ramp from black to white with a high threshold: only its bright end glows.
    let mut frames = Frames::default().with(1, pattern(64, 64, 0));
    let high = effect(
        "light.glow",
        &[
            ("radius", n(0.1)),
            ("threshold", n(0.9)),
            ("glow_only", ParamValue::Flag(true)),
        ],
    );
    let image = render(&mut renderer, vec![square(vec![high])], &mut frames);
    assert!(luma(&image, 135, 90) < 3, "the dark end gives no light");
    assert!(luma(&image, 186, 90) > 20, "the bright end does");
}

#[test]
fn sweeps_light_visible_pixels_as_they_pass() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default().with(1, solid(64, 64, GREY));
    let sweep = |position: f64| {
        effect(
            "light.sweep",
            &[
                ("position", n(position)),
                ("angle", n(0.0)),
                ("width", n(0.3)),
                ("softness", n(0.5)),
            ],
        )
    };
    let plain = render(&mut renderer, vec![square(vec![])], &mut frames);
    let before = render(&mut renderer, vec![square(vec![sweep(0.0)])], &mut frames);
    let after = render(&mut renderer, vec![square(vec![sweep(1.0)])], &mut frames);
    assert!(plain.psnr(&before).unwrap() > 50.0, "not yet entered");
    assert!(plain.psnr(&after).unwrap() > 50.0, "already left");
    let middle = render(&mut renderer, vec![square(vec![sweep(0.5)])], &mut frames);
    assert!(
        luma(&middle, 160, 90) > luma(&plain, 160, 90) + 60,
        "the band"
    );
    assert_eq!(luma(&middle, 133, 90), luma(&plain, 133, 90), "left of it");
    assert_eq!(luma(&middle, 160, 50), 0, "transparent parts stay dark");
    // Moving the position moves the band from left to right.
    let early = render(&mut renderer, vec![square(vec![sweep(0.35)])], &mut frames);
    assert!(luma(&early, 150, 90) > luma(&early, 175, 90));
    // At 90 degrees it moves down: a level band.
    let level = effect(
        "light.sweep",
        &[("position", n(0.5)), ("angle", n(90.0)), ("width", n(0.2))],
    );
    let image = render(&mut renderer, vec![square(vec![level])], &mut frames);
    assert!(luma(&image, 135, 90) > luma(&image, 135, 65) + 40);
}

#[test]
fn streaks_run_along_their_angle() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    // A white square 45 pixels wide (138 to 182); streaks as long as the frame's width at
    // the square's own size reach 80 pixels on each side at its quarter scale.
    let mut frames = Frames::default().with(1, solid(32, 32, WHITE));
    let small = |effects| Layer {
        transform: Transform {
            scale: 0.25,
            ..Transform::default()
        },
        effects,
        ..Layer::new(source(1))
    };
    let level = effect(
        "light.streaks",
        &[("length", n(1.0)), ("threshold", n(0.5)), ("tint", n(0.0))],
    );
    let image = render(&mut renderer, vec![small(vec![level])], &mut frames);
    assert!(
        luma(&image, 200, 90) > 20,
        "right: {}",
        luma(&image, 200, 90)
    );
    assert!(luma(&image, 120, 90) > 20, "left");
    assert!(luma(&image, 160, 140) < 3, "not below");
    assert!(luma(&image, 200, 90) > luma(&image, 240, 90), "fading");
    assert!(
        luma(&image, 240, 90) > luma(&image, 290, 90),
        "fading further"
    );
    let steep = effect(
        "light.streaks",
        &[
            ("length", n(1.0)),
            ("threshold", n(0.5)),
            ("angle", n(90.0)),
        ],
    );
    let image = render(&mut renderer, vec![small(vec![steep])], &mut frames);
    assert!(luma(&image, 160, 140) > 20, "below");
    assert!(luma(&image, 200, 90) < 3, "not to the side");
    // Dark pictures under the threshold give no streaks.
    let mut dark = Frames::default().with(1, solid(32, 32, GREY));
    let image = render(
        &mut renderer,
        vec![small(vec![effect("light.streaks", &[("length", n(1.0))])])],
        &mut dark,
    );
    assert!(luma(&image, 200, 90) < 3);
}

fn titled(animations: ClipAnimations) -> (Project, Id) {
    let format = TimelineFormat {
        width: 640,
        height: 360,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Light", format);
    let timeline = project.manifest.primary_timeline.clone();
    let mut title = Clip::new(
        Id::new(IdKind::Clip, 2),
        Flicks::ZERO,
        Rate::FPS_30.frames_to_flicks(60),
        ClipContent::Text(Box::new(TextContent {
            text: "Glow".into(),
            style: TextStyle {
                size: 0.2,
                bold: true,
                ..TextStyle::default()
            },
            ..Default::default()
        })),
    );
    title.animations = animations;
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 3),
            kind: TrackKind::Text,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![title],
        });
    project.manifest.next_id = 10;
    (project, timeline)
}

fn entering(preset: &str) -> ClipAnimations {
    ClipAnimations {
        entrance: Some(Animation {
            preset: preset.into(),
            duration: Rate::FPS_30.frames_to_flicks(20),
            delay: Rate::FPS_30.frames_to_flicks(4),
        }),
        ..Default::default()
    }
}

#[test]
fn animations_plan_their_blur_and_glow_and_preview_matches_export() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let at = |frame: i64| Rate::FPS_30.frames_to_flicks(frame);
    let (glowing, timeline) = titled(entering("glow_in"));
    let kinds = |project: &Project, frame: i64| -> Vec<String> {
        plan_frame(project, &timeline, at(frame)).unwrap().layers[0]
            .effects
            .iter()
            .map(|e| e.effect_type.clone())
            .collect()
    };
    assert!(
        kinds(&glowing, 2).is_empty(),
        "waiting for its delay: no light yet"
    );
    assert_eq!(kinds(&glowing, 14), ["light.glow"], "flaring halfway");
    assert!(kinds(&glowing, 30).is_empty(), "at rest");
    let (blurred, _) = titled(entering("blur_in"));
    assert_eq!(kinds(&blurred, 6), ["animation.blur"]);
    assert!(kinds(&blurred, 30).is_empty());

    // The flare lights the canvas around the title, preview and export alike.
    let export = ExportGraph::new(&glowing, &timeline).unwrap();
    let preview = PreviewGraph::new(&glowing, &timeline, (320, 320)).unwrap();
    let mut provider = Frames::default();
    let full = export
        .render(&mut renderer, &glowing, &mut provider, at(14))
        .unwrap();
    let rest = export
        .render(&mut renderer, &glowing, &mut provider, at(30))
        .unwrap();
    let small = preview
        .grab(&mut renderer, &glowing, &mut provider, at(14))
        .unwrap();
    // Around the letters, where the title at rest leaves the canvas dark: light only while
    // it flares.
    let lit = (0..rest.height)
        .flat_map(|y| (0..rest.width).map(move |x| (x, y)))
        .filter(|&(x, y)| luma(&rest, x, y) == 0 && luma(&full, x, y) > 10)
        .count();
    assert!(lit > 2000, "lit around the title: {lit} pixels");
    let psnr = half(&full).psnr(&small).unwrap();
    eprintln!("animated glow parity: {psnr:.1} dB");
    // Text rasterizes at each size (as in parity.rs, 33 dB for plain text); the glow is
    // measured on the canvas, so a misplaced or differently sized glow costs far more.
    assert!(psnr > 33.0, "PSNR {psnr:.1} dB");
}

/// Averages each 2x2 block of an image in linear light, as a half-size render does.
fn half(image: &Image) -> Image {
    let (w, h) = (image.width / 2, image.height / 2);
    let decode =
        |code: u8| fosscut_render::color::TransferFunction::Bt709.decode(f32::from(code) / 255.0);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let linear: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                    .iter()
                    .map(|(dx, dy)| decode(image.pixel(2 * x + dx, 2 * y + dy)[c]))
                    .sum::<f32>()
                    / 4.0;
                let code = TransferFunction::Bt709.encode(linear).clamp(0.0, 1.0) * 255.0;
                rgba.push(code.round() as u8);
            }
        }
    }
    Image {
        width: w,
        height: h,
        rgba,
    }
}

/// A picture-in-picture video with a glow and streaks: the padded picture renders the same
/// in the preview (half size) and the export (full size, then halved).
#[test]
fn padded_light_matches_between_preview_and_export() {
    use fosscut_model::{Asset, AssetKind, Effect, MediaInfo, PathHints, VideoInfo};
    let Some(mut renderer) = renderer() else {
        return;
    };
    let format = TimelineFormat {
        width: 1280,
        height: 720,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Light parity", format);
    project.media.assets.push(Asset {
        folder: None,
        id: asset(1),
        kind: AssetKind::Video,
        name: "pattern.mp4".into(),
        path: PathHints::default(),
        identity: None,
        info: MediaInfo {
            duration: Some(Rate::FPS_30.frames_to_flicks(90)),
            video: Some(VideoInfo {
                width: 1280,
                height: 720,
                rate: Some(Rate::FPS_30),
                codec: None,
                rotation: 0,
            }),
            ..Default::default()
        },
    });
    let timeline = project.manifest.primary_timeline.clone();
    let effect = |id: u64, kind: &str, params: &[(&str, ParamValue)]| Effect {
        id: Id::new(IdKind::Effect, id),
        effect_type: kind.into(),
        version: 1,
        enabled: true,
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        keyframes: Default::default(),
    };
    let mut clip = Clip::new(
        Id::new(IdKind::Clip, 2),
        Flicks::ZERO,
        Rate::FPS_30.frames_to_flicks(60),
        ClipContent::Video {
            asset: asset(1),
            source_in: Flicks::ZERO,
        },
    );
    clip.transform = Transform {
        position: [0.1, 0.05],
        scale: 0.4,
        rotation: 8.0,
        ..Transform::default()
    };
    clip.effects = vec![
        effect(
            3,
            "light.glow",
            &[("radius", n(0.06)), ("intensity", n(2.0))],
        ),
        effect(4, "light.streaks", &[("threshold", n(0.6))]),
        effect(5, "light.sweep", &[("position", n(0.4))]),
    ];
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 6),
            kind: TrackKind::Video,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![clip],
        });
    project.manifest.next_id = 10;
    assert!(fosscut_model::validate(&project).is_empty());
    let export = ExportGraph::new(&project, &timeline).unwrap();
    let preview = PreviewGraph::new(&project, &timeline, (640, 640)).unwrap();
    let mut provider = Frames::default();
    provider.moving.insert(asset(1), (1280, 720));
    let time = Rate::FPS_30.frames_to_flicks(10);
    let full = export
        .render(&mut renderer, &project, &mut provider, time)
        .unwrap();
    let small = preview
        .grab(&mut renderer, &project, &mut provider, time)
        .unwrap();
    // The picture spans x 224 to 480 of the preview's middle row; the glow lights the
    // canvas left of it, in both.
    assert!(luma(&small, 212, 198) > 5, "{}", luma(&small, 212, 198));
    assert!(luma(&full, 424, 396) > 5);
    let psnr = half(&full).psnr(&small).unwrap();
    eprintln!("padded light parity: {psnr:.1} dB");
    assert!(psnr > 40.0, "PSNR {psnr:.1} dB");
}

/// A layer turned in 3D (ADR-0037) keeps its place when its picture is padded, and its
/// glow spreads past it in perspective.
#[test]
fn padded_layers_project_in_three_d() {
    use fosscut_render::space::{Camera, place_layers};
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default().with(1, pattern(96, 64, 0));
    let mut turned = |effects: Vec<PlannedEffect>| {
        let mut plan = RenderPlan {
            canvas: CANVAS,
            layers: vec![Layer {
                transform: Transform {
                    scale: 0.6,
                    rotation_y: 35.0,
                    ..Transform::default()
                },
                effects,
                ..Layer::new(source(1))
            }],
            skipped: Vec::new(),
        };
        place_layers(&mut plan, &Camera::default_for(CANVAS));
        assert!(plan.layers[0].space.is_some(), "placed in 3D");
        renderer
            .render_image(&plan, &mut frames, CANVAS, TransferFunction::Srgb)
            .unwrap()
    };
    let plain = turned(vec![]);
    let faint = turned(vec![effect(
        "light.glow",
        &[("radius", n(0.1)), ("intensity", n(1e-6))],
    )]);
    assert!(interior_psnr(&plain, &faint) > 45.0);
    let (p, q) = (bounds(&plain), bounds(&faint));
    assert!(
        p.iter().zip(q).all(|(a, b)| a.abs_diff(b) <= 2),
        "{p:?} vs {q:?}"
    );
    let lit = turned(vec![effect("light.glow", &[("radius", n(0.1))])]);
    let outside = (0..plain.height)
        .flat_map(|y| (0..plain.width).map(move |x| (x, y)))
        .filter(|&(x, y)| luma(&plain, x, y) == 0 && luma(&lit, x, y) > 10)
        .count();
    assert!(outside > 500, "light around the turned picture: {outside}");
}

/// On an adjustment layer light effects work on everything below it, within the canvas.
#[test]
fn adjustment_layers_glow_what_is_below() {
    use fosscut_render::plan::LayerSource;
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default().with(1, solid(64, 64, WHITE));
    let adjustment = Layer {
        source: LayerSource::Adjustment,
        effects: vec![effect("light.glow", &[("radius", n(0.05))])],
        ..Layer::new(source(1))
    };
    let plain = render(&mut renderer, vec![square(vec![])], &mut frames);
    let lit = render(&mut renderer, vec![square(vec![]), adjustment], &mut frames);
    assert!(luma(&plain, 195, 90) < 3);
    assert!(luma(&lit, 195, 90) > 40, "{}", luma(&lit, 195, 90));
    assert_eq!(luma(&lit, 160, 90), 255);
}
