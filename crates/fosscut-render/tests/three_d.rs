//! Layers placed in 3D on the GPU (ADR-0037): at rest they match the 2D path, runs of 3D
//! layers draw farthest first, camera clips dolly and orbit, the depth of field blurs by
//! distance with soft edges, masks follow the layer, and preview matches export.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_model::{
    Asset, AssetKind, CameraContent, Clip, ClipContent, Id, IdKind, Interpolation, KeyValue,
    Keyframe, Mask, MaskShape, MediaInfo, PathHints, Project, TextContent, TextStyle,
    TimelineFormat, Track, TrackKind, Transform, VideoInfo,
};
use fosscut_render::color::TransferFunction;
use fosscut_render::plan::{Layer, RenderPlan, plan_frame};
use fosscut_render::space::{Camera, place_layers};
use fosscut_render::{ExportGraph, Image, PreviewGraph, Renderer};
use fosscut_time::{Flicks, Rate};
use support::*;

const CANVAS: (u32, u32) = (1280, 720);
const SIZE: (u32, u32) = (320, 180);
const GRAY: [u8; 3] = [60, 128, 128];
const RED: [u8; 3] = [63, 102, 240];
const GREEN: [u8; 3] = [173, 42, 26];
const BLUE: [u8; 3] = [32, 240, 118];
const WHITE: [u8; 3] = [235, 128, 128];

fn render(renderer: &mut Renderer, plan: &RenderPlan, frames: &mut Frames) -> Image {
    renderer
        .render_image(plan, frames, SIZE, TransferFunction::Bt709)
        .unwrap()
}

fn plan(layers: Vec<Layer>) -> RenderPlan {
    RenderPlan {
        canvas: CANVAS,
        layers,
        skipped: Vec::new(),
    }
}

/// `layers` over a gray background, placed in 3D through the default camera.
fn placed(layers: Vec<Layer>) -> RenderPlan {
    let mut all = vec![Layer::new(source(9))];
    all.extend(layers);
    let mut plan = plan(all);
    place_layers(&mut plan, &Camera::default_for(CANVAS));
    plan
}

fn frames() -> Frames {
    Frames::default()
        .with(1, pattern(640, 360, 0))
        .with(2, solid(64, 36, RED))
        .with(3, solid(64, 36, GREEN))
        .with(4, solid(64, 36, BLUE))
        .with(5, solid(64, 36, WHITE))
        .with(9, solid(64, 36, GRAY))
}

fn card(n: u64, transform: Transform) -> Layer {
    Layer {
        transform,
        ..Layer::new(source(n))
    }
}

fn rgb(image: &Image, x: u32, y: u32) -> [u8; 3] {
    let p = image.pixel(x, y);
    [p[0], p[1], p[2]]
}

/// Which of red, green and blue dominates a pixel.
fn dominant(image: &Image, x: u32, y: u32) -> usize {
    let [r, g, b] = rgb(image, x, y);
    let channels = [r, g, b];
    (0..3).max_by_key(|&c| channels[c]).unwrap()
}

#[test]
fn layers_at_rest_on_the_canvas_plane_render_like_2d() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let mut ellipse = Mask::new(Id::new(IdKind::Mask, 1), MaskShape::Ellipse);
    ellipse.size = [0.8, 0.9];
    ellipse.feather = 0.2;
    let moved = |three_d: bool| Layer {
        transform: Transform {
            position: [0.15, -0.1],
            scale: 0.5,
            stretch: [1.1, 0.9],
            rotation: 12.0,
            flip_horizontal: true,
            three_d,
            ..Transform::default()
        },
        crop: [0.1, 0.0, 0.95, 0.9],
        opacity: 0.9,
        masks: vec![ellipse.clone()],
        ..Layer::new(source(1))
    };
    let title = |three_d: bool| Layer {
        transform: Transform {
            position: [-0.2, 0.3],
            rotation: -6.0,
            three_d,
            ..Transform::default()
        },
        ..Layer::text(TextContent {
            text: "Depth".into(),
            style: TextStyle {
                bold: true,
                ..TextStyle::default()
            },
            ..Default::default()
        })
    };
    let flat = render(
        &mut renderer,
        &placed(vec![moved(false), title(false)]),
        &mut frames,
    );
    let spatial = placed(vec![moved(true), title(true)]);
    assert!(spatial.layers[1].space.is_some() && spatial.layers[2].space.is_some());
    let deep = render(&mut renderer, &spatial, &mut frames);
    // The same corners, coordinates and sampling: identical pixels.
    assert_eq!(
        flat.psnr(&deep),
        Some(f64::INFINITY),
        "3D at rest matches 2D"
    );
    // An anchor alone does not move the picture.
    let anchored = render(
        &mut renderer,
        &placed(vec![card(
            1,
            Transform {
                anchor: [0.4, -0.3],
                ..Transform::default()
            },
        )]),
        &mut frames,
    );
    let plain = render(
        &mut renderer,
        &placed(vec![card(1, Transform::default())]),
        &mut frames,
    );
    assert_eq!(plain.psnr(&anchored), Some(f64::INFINITY), "anchor only");
}

#[test]
fn runs_of_3d_layers_draw_farthest_first_and_2d_layers_end_a_run() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let at = |depth: f64| Transform {
        three_d: true,
        scale: 0.5,
        position_z: depth,
        ..Transform::default()
    };
    // In track order the farthest card is on top; in depth the nearest wins.
    let cards = vec![card(2, at(0.0)), card(3, at(0.3)), card(4, at(0.6))];
    let image = render(&mut renderer, &placed(cards), &mut frames);
    assert_eq!(
        dominant(&image, 160, 90),
        0,
        "red, the nearest, is in front"
    );
    // A 2D layer between 3D layers ends the run: the cards above it draw after it.
    let split = vec![
        card(2, at(0.0)),
        card(
            5,
            Transform {
                scale: 0.2,
                ..Transform::default()
            },
        ),
        card(3, at(0.3)),
        card(4, at(0.6)),
    ];
    let image = render(&mut renderer, &placed(split), &mut frames);
    assert_eq!(
        dominant(&image, 160, 90),
        1,
        "green is the nearest of the run above the 2D layer"
    );
}

#[test]
fn turned_cards_recede_with_perspective_and_masks_follow_them() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let turned = card(
        5,
        Transform {
            three_d: true,
            scale: 0.6,
            rotation_y: 50.0,
            ..Transform::default()
        },
    );
    let image = render(&mut renderer, &placed(vec![turned]), &mut frames);
    // The column height of the white card near each side: the right side is farther.
    let height = |x: u32| (0..180).filter(|y| rgb(&image, x, *y)[0] > 150).count();
    let camera = Camera::default_for(CANVAS);
    let (half_w, half_h) = (1280.0 * 0.3, 720.0 * 0.3);
    let turn = 50f64.to_radians();
    let left = camera
        .to_canvas([-half_w * turn.cos(), -half_h, -half_w * turn.sin()])
        .unwrap();
    let right = camera
        .to_canvas([half_w * turn.cos(), -half_h, half_w * turn.sin()])
        .unwrap();
    let column = |x: f64| (x / 4.0).round() as u32;
    let (near, far) = (height(column(left[0]) + 3), height(column(right[0]) - 3));
    assert!(near > far + 10, "near side {near} px, far side {far} px");
    // The top corner is where the projection puts it, within a pixel or two.
    let expected = (right[1] / 4.0).round() as i64;
    let top = (0..180)
        .find(|y| rgb(&image, column(right[0]) - 1, *y)[0] > 150)
        .unwrap() as i64;
    assert!(
        (top - expected).abs() <= 2,
        "top {top}, expected {expected}"
    );
    // A mask in the layer's own coordinates: the corners go, the middle stays.
    let mut ellipse = Mask::new(Id::new(IdKind::Mask, 1), MaskShape::Ellipse);
    ellipse.size = [0.7, 0.7];
    let masked = Layer {
        masks: vec![ellipse],
        transform: Transform {
            three_d: true,
            scale: 0.6,
            rotation_x: 40.0,
            ..Transform::default()
        },
        ..Layer::new(source(5))
    };
    let image = render(&mut renderer, &placed(vec![masked]), &mut frames);
    assert!(rgb(&image, 160, 90)[0] > 200, "the middle shows");
    // The picture's point at 8% across and 92% down, outside the ellipse, tilted back.
    let (x, y) = ((0.08 - 0.5) * 768.0, (0.92 - 0.5) * 432.0);
    let tilt = 40f64.to_radians();
    let corner = camera
        .to_canvas([x, y * tilt.cos(), -y * tilt.sin()])
        .unwrap();
    let [r, ..] = rgb(&image, (corner[0] / 4.0) as u32, (corner[1] / 4.0) as u32);
    assert!(
        r < 80,
        "a corner outside the ellipse shows the background: {r}"
    );
}

/// A project with a pattern card (3D or not), a 2D card, and a camera clip.
fn space_project(three_d: bool, camera: Clip) -> (Project, Id) {
    let format = TimelineFormat {
        width: CANVAS.0,
        height: CANVAS.1,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 7), "Space", format);
    for n in [1, 5, 9] {
        project.media.assets.push(Asset {
            folder: None,
            id: asset(n),
            kind: AssetKind::Video,
            name: format!("frame-{n}.mp4"),
            path: PathHints::default(),
            identity: None,
            info: MediaInfo {
                duration: Some(Rate::FPS_30.frames_to_flicks(300)),
                video: Some(VideoInfo {
                    width: 640,
                    height: 360,
                    rate: Some(Rate::FPS_30),
                    codec: None,
                    rotation: 0,
                }),
                ..Default::default()
            },
        });
    }
    project.manifest.next_id = 100;
    let timeline = project.manifest.primary_timeline.clone();
    let clip = |id: u64, n: u64, transform: Transform| Clip {
        transform,
        ..Clip::new(
            Id::new(IdKind::Clip, id),
            Flicks::ZERO,
            Rate::FPS_30.frames_to_flicks(60),
            ClipContent::Video {
                asset: asset(n),
                source_in: Flicks::ZERO,
            },
        )
    };
    let tracks = [
        (20, clip(30, 9, Transform::default())),
        (
            21,
            clip(
                31,
                1,
                Transform {
                    three_d,
                    scale: 0.4,
                    position_z: 0.2,
                    ..Transform::default()
                },
            ),
        ),
        (
            22,
            clip(
                32,
                5,
                Transform {
                    scale: 0.1,
                    position: [0.4, -0.4],
                    ..Transform::default()
                },
            ),
        ),
        (23, camera),
    ];
    for (id, clip) in tracks {
        project
            .timelines
            .get_mut(&timeline)
            .unwrap()
            .tracks
            .push(Track {
                id: Id::new(IdKind::Track, id),
                kind: TrackKind::Video,
                name: None,
                muted: false,
                hidden: false,
                locked: false,
                caption_style: None,
                clips: vec![clip],
            });
    }
    (project, timeline)
}

fn key(frame: i64, value: KeyValue) -> Keyframe {
    Keyframe {
        time: Rate::FPS_30.frames_to_flicks(frame),
        value,
        interpolation: Interpolation::EASE_IN_OUT,
    }
}

/// A camera clip over the first two seconds that pushes in, orbits and focuses.
fn moving_camera(depth_of_field: bool) -> Clip {
    let mut camera = Clip::new(
        Id::new(IdKind::Clip, 40),
        Flicks::ZERO,
        Rate::FPS_30.frames_to_flicks(60),
        ClipContent::Camera(Box::new(CameraContent {
            depth_of_field,
            aperture: 0.4,
            point_of_interest_z: 0.2,
            ..CameraContent::default()
        })),
    );
    camera.keyframes.insert(
        "camera.position_z".into(),
        vec![
            key(0, KeyValue::Number(0.0)),
            key(59, KeyValue::Number(0.6)),
        ],
    );
    camera.keyframes.insert(
        "camera.orbit".into(),
        vec![
            key(0, KeyValue::Pair([0.0, 0.0])),
            key(59, KeyValue::Pair([30.0, 10.0])),
        ],
    );
    camera
}

/// Columns of the canvas where the middle row is the pattern card (not gray or white).
fn card_columns(image: &Image) -> Vec<u32> {
    (0..image.width)
        .filter(|x| {
            let [r, g, b] = rgb(image, *x, image.height / 2);
            let spread = r.max(g).max(b) - r.min(g).min(b);
            spread > 25
        })
        .collect()
}

#[test]
fn camera_clips_dolly_and_orbit_3d_layers_only() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let (project, timeline) = space_project(true, moving_camera(false));
    let start = plan_frame(&project, &timeline, Flicks::ZERO).unwrap();
    let end = plan_frame(&project, &timeline, Rate::FPS_30.frames_to_flicks(59)).unwrap();
    let wide = render(&mut renderer, &start, &mut frames);
    let close = render(&mut renderer, &end, &mut frames);
    let (before, after) = (card_columns(&wide).len(), card_columns(&close).len());
    assert!(before > 40, "{before}");
    assert!(
        after as f64 > before as f64 * 1.15,
        "pushed in: {before} -> {after}"
    );
    // The 2D card in the corner does not see the camera.
    for (x, y) in [(288, 18), (300, 24)] {
        assert_eq!(rgb(&wide, x, y), rgb(&close, x, y), "2D at {x},{y}");
    }
    // Orbiting to the right, the card's right side comes nearer: taller than its left.
    let columns = card_columns(&close);
    let height = |x: u32| {
        (0..close.height)
            .filter(|y| {
                let [r, g, b] = rgb(&close, x, *y);
                r.max(g).max(b) - r.min(g).min(b) > 25
            })
            .count()
    };
    let (first, last) = (columns[2], columns[columns.len() - 3]);
    assert!(
        height(last) > height(first),
        "{first}: {}, {last}: {}",
        height(first),
        height(last)
    );
    // The same timeline with a 2D card ignores the camera.
    let (flat, timeline) = space_project(false, moving_camera(false));
    let a = render(
        &mut renderer,
        &plan_frame(&flat, &timeline, Flicks::ZERO).unwrap(),
        &mut frames,
    );
    let b = render(
        &mut renderer,
        &plan_frame(&flat, &timeline, Rate::FPS_30.frames_to_flicks(59)).unwrap(),
        &mut frames,
    );
    assert_eq!(a.psnr(&b), Some(f64::INFINITY));
}

/// Sum of absolute differences between horizontal neighbours in a window: how sharp it
/// is.
fn detail(image: &Image, x0: u32, y0: u32, w: u32, h: u32) -> f64 {
    let mut sum = 0.0;
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let (a, b) = (rgb(image, x, y), rgb(image, x + 1, y));
            sum += (0..3)
                .map(|c| (f64::from(a[c]) - f64::from(b[c])).abs())
                .sum::<f64>();
        }
    }
    sum
}

#[test]
fn depth_of_field_blurs_by_distance_from_the_focus() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let checker = |n: u64| {
        nv12(320, 180, bt709_limited(), move |x, y| {
            let on = (x / 16 + y / 16 + n as u32).is_multiple_of(2);
            if on { [200, 128, 128] } else { [40, 128, 128] }
        })
    };
    frames = frames.with(6, checker(0)).with(7, checker(1));
    let cards = || {
        vec![
            card(
                7,
                Transform {
                    three_d: true,
                    scale: 0.4,
                    position: [0.25, 0.0],
                    position_z: 1.2,
                    ..Transform::default()
                },
            ),
            card(
                6,
                Transform {
                    three_d: true,
                    scale: 0.3,
                    position: [-0.25, 0.0],
                    ..Transform::default()
                },
            ),
        ]
    };
    let scene = |camera: &Camera| {
        let mut all = vec![Layer::new(source(9))];
        all.extend(cards());
        let mut plan = plan(all);
        place_layers(&mut plan, camera);
        plan
    };
    let sharp = render(
        &mut renderer,
        &scene(&Camera::default_for(CANVAS)),
        &mut frames,
    );
    let focused = Camera::from_content(
        CANVAS,
        &CameraContent {
            depth_of_field: true,
            aperture: 0.3,
            ..CameraContent::default()
        },
    );
    let plan = scene(&focused);
    assert!(plan.layers[1].space.as_ref().unwrap().defocus > 0.002);
    assert_eq!(plan.layers[2].space.as_ref().unwrap().defocus, 0.0);
    let soft = render(&mut renderer, &plan, &mut frames);
    // Windows inside each card, from where the camera puts their centres.
    let window = |point: [f64; 3]| {
        let at = Camera::default_for(CANVAS).to_canvas(point).unwrap();
        ((at[0] / 4.0) as u32 - 8, (at[1] / 4.0) as u32 - 8)
    };
    let (nx, ny) = window([-320.0, 0.0, 0.0]);
    let (fx, fy) = window([320.0, 0.0, 1.2 * 1280.0]);
    let near = detail(&soft, nx, ny, 16, 16) / detail(&sharp, nx, ny, 16, 16);
    let far = detail(&soft, fx, fy, 16, 16) / detail(&sharp, fx, fy, 16, 16);
    assert!(near > 0.95, "the card in focus stays sharp: {near:.2}");
    assert!(far < 0.6, "the far card is blurred: {far:.2}");
    // Its edges soften too: just outside the far card the background brightens a little.
    let far_card = Camera::default_for(CANVAS)
        .to_canvas([320.0 + 1280.0 * 0.2, 0.0, 1.2 * 1280.0])
        .unwrap();
    let outside = ((far_card[0] / 4.0) as u32 + 2, 90);
    let gray = rgb(&sharp, outside.0, outside.1)[0];
    let blurred = rgb(&soft, outside.0, outside.1)[0];
    assert!(blurred > gray, "soft edge: {gray} -> {blurred}");
}

/// Averages each 2x2 block of an image in linear light, as the preview's mipmaps do.
fn half(image: &Image) -> Image {
    let (w, h) = (image.width / 2, image.height / 2);
    let bt709 = TransferFunction::Bt709;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let linear: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                    .iter()
                    .map(|(dx, dy)| {
                        let code = image.pixel(2 * x + dx, 2 * y + dy)[c];
                        bt709.decode(f32::from(code) / 255.0)
                    })
                    .sum::<f32>()
                    / 4.0;
                rgba.push((bt709.encode(linear).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }
    Image {
        width: w,
        height: h,
        rgba,
    }
}

#[test]
fn preview_matches_export_for_a_moving_3d_scene() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (project, timeline) = space_project(true, moving_camera(true));
    let export = ExportGraph::new(&project, &timeline).unwrap();
    let preview = PreviewGraph::new(&project, &timeline, (640, 640)).unwrap();
    let mut provider = Frames::default();
    for n in [1, 5, 9] {
        provider.moving.insert(asset(n), (1280, 720));
    }
    for frame in [0, 30, 59] {
        let time = Rate::FPS_30.frames_to_flicks(frame);
        let full = export
            .render(&mut renderer, &project, &mut provider, time)
            .unwrap();
        let small = preview
            .grab(&mut renderer, &project, &mut provider, time)
            .unwrap();
        // As in parity.rs, plus the depth of field: each resolution blurs through the mip
        // levels its own size calls for, so 38 dB (still invisible) leaves room for that,
        // while a missing, misplaced or unblurred layer drops far below it.
        let psnr = half(&full).psnr(&small).unwrap();
        eprintln!("3D parity at frame {frame}: {psnr:.1} dB");
        assert!(psnr > 38.0, "frame {frame}: PSNR {psnr:.1} dB");
    }
}

#[test]
fn compound_clips_render_flat_and_are_placed_in_3d() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let turned = Transform {
        three_d: true,
        scale: 0.6,
        rotation_y: 50.0,
        ..Transform::default()
    };
    // The white picture inside a compound clip, turned, looks like the picture turned.
    let inner = plan(vec![Layer::new(source(5))]);
    let nested = Layer {
        source: fosscut_render::plan::LayerSource::Nested(Box::new(inner)),
        transform: turned.clone(),
        ..Layer::new(source(5))
    };
    let placed_nested = placed(vec![nested]);
    assert!(placed_nested.layers[1].space.is_some());
    let compound = render(&mut renderer, &placed_nested, &mut frames);
    let direct = render(&mut renderer, &placed(vec![card(5, turned)]), &mut frames);
    let psnr = direct.psnr(&compound).unwrap();
    assert!(psnr > 36.0, "{psnr:.1} dB");
}

#[test]
fn motion_blur_samples_3d_placements() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let mut blur = |on: bool| {
        let mut camera = Clip::new(
            Id::new(IdKind::Clip, 40),
            Flicks::ZERO,
            Rate::FPS_30.frames_to_flicks(60),
            ClipContent::Camera(Box::default()),
        );
        camera.enabled = false;
        let (mut project, timeline) = space_project(true, camera);
        let card = &mut project.timelines.get_mut(&timeline).unwrap().tracks[1].clips[0];
        // A fast turn: 180 degrees in a fifth of a second, 30 degrees a frame.
        card.keyframes.insert(
            "transform.rotation_y".into(),
            vec![
                key(0, KeyValue::Number(-90.0)),
                key(6, KeyValue::Number(90.0)),
            ],
        );
        if on {
            card.effects.push(fosscut_model::Effect {
                id: Id::new(IdKind::Effect, 50),
                effect_type: "motion.blur".into(),
                version: 1,
                enabled: true,
                params: [("amount".to_string(), fosscut_model::ParamValue::Number(1.0))].into(),
                keyframes: Default::default(),
            });
        }
        let plan = plan_frame(&project, &timeline, Rate::FPS_30.frames_to_flicks(2)).unwrap();
        let layer = plan.layers[1].clone();
        (render(&mut renderer, &plan, &mut frames), layer)
    };
    let (sharp, still) = blur(false);
    let (streaked, moving) = blur(true);
    assert!(still.shutter.is_empty());
    let turns: Vec<f64> = moving.shutter.iter().map(|t| t.rotation_y).collect();
    assert!(turns.len() > 2 && turns.first() < turns.last(), "{turns:?}");
    assert!(
        moving.space.is_some(),
        "the shutter is projected through the camera"
    );
    let psnr = sharp.psnr(&streaked).unwrap();
    assert!(psnr < 35.0, "the blur shows: {psnr:.1} dB");
}
