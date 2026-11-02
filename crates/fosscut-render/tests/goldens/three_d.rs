//! Golden frames of layers placed in 3D (ADR-0037), a module of `goldens.rs`: a card
//! turned with perspective, a stack of cards sorted by depth, a camera dolly and orbit,
//! the depth of field, and a tilted card with a mask and an effect in its own space.

use fosscut_model::{
    Asset, AssetKind, CameraContent, Clip, ClipContent, Id, IdKind, Interpolation, KeyValue,
    Keyframe, MaskShape, MediaInfo, ParamValue, PathHints, Project, TimelineFormat, Track,
    TrackKind, Transform, VideoInfo,
};
use fosscut_render::plan::{Layer, RenderPlan, plan_frame};
use fosscut_render::space::{Camera, place_layers};
use fosscut_time::{Flicks, Rate};

use super::support::*;
use super::{check, planned};

const CANVAS: (u32, u32) = (1920, 1080);

/// Colours of the cards, as BT.709 limited Y'CbCr: coral, teal, gold, violet.
const COLORS: [[u8; 3]; 4] = [
    [120, 110, 190],
    [130, 150, 80],
    [180, 50, 150],
    [80, 170, 160],
];

fn frames() -> Frames {
    let mut frames = Frames::default().with(1, pattern(640, 360, 0)).with(
        9,
        nv12(320, 180, bt709_limited(), |_, y| {
            [(30 + y / 6) as u8, 132, 124]
        }),
    );
    for (i, color) in COLORS.iter().enumerate() {
        frames = frames.with(2 + i as u64, solid(64, 36, *color));
    }
    frames
}

/// `layers` over a dark gradient, seen through `camera`.
fn scene(layers: Vec<Layer>, camera: &Camera) -> RenderPlan {
    let mut all = vec![Layer::new(source(9))];
    all.extend(layers);
    let mut plan = RenderPlan {
        canvas: CANVAS,
        layers: all,
        skipped: Vec::new(),
    };
    place_layers(&mut plan, camera);
    plan
}

fn spatial(n: u64, transform: Transform) -> Layer {
    Layer {
        transform: Transform {
            three_d: true,
            ..transform
        },
        ..Layer::new(source(n))
    }
}

/// The test pattern turned 40 degrees around its vertical axis: its right side recedes,
/// shorter than its left, with straight edges and the pattern foreshortened along them.
#[test]
fn three_d_card() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let card = spatial(
        1,
        Transform {
            scale: 0.6,
            rotation_y: 40.0,
            ..Transform::default()
        },
    );
    let plan = scene(vec![card], &Camera::default_for(CANVAS));
    check("three-d-card", &mut renderer, &plan, &mut frames());
}

/// Cards placed by depth along a diagonal, in shuffled track order: each nearer card covers
/// the farther ones, and farther cards look smaller and closer to the middle.
fn stack() -> Vec<Layer> {
    let card = |n: u64, depth: f64, x: f64, y: f64| {
        spatial(
            n,
            Transform {
                scale: 0.32,
                position: [x, y],
                position_z: depth,
                rotation_y: -18.0,
                ..Transform::default()
            },
        )
    };
    vec![
        card(3, 0.4, 0.0, 0.0),
        card(2, 0.0, -0.22, 0.14),
        card(5, 1.2, 0.3, -0.18),
        card(4, 0.8, 0.16, -0.1),
    ]
}

#[test]
fn three_d_stack() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let plan = scene(stack(), &Camera::default_for(CANVAS));
    check("three-d-stack", &mut renderer, &plan, &mut frames());
}

/// The stack seen with depth of field focused on the teal card (0.4 deep): the coral card
/// in front and the gold and violet cards behind blur with their distance, edges included.
#[test]
fn three_d_depth_of_field() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let camera = Camera::from_content(
        CANVAS,
        &CameraContent {
            point_of_interest_z: 0.4,
            depth_of_field: true,
            aperture: 0.25,
            ..CameraContent::default()
        },
    );
    let plan = scene(stack(), &camera);
    check(
        "three-d-depth-of-field",
        &mut renderer,
        &plan,
        &mut frames(),
    );
}

/// The pattern tilted back and turned, pixelated and masked by a soft star in its own
/// plane: the blocks and the star recede with the picture.
#[test]
fn three_d_mask_and_effect() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut star = fosscut_model::Mask::new(Id::new(IdKind::Mask, 1), MaskShape::Star);
    star.size = [0.9, 0.9];
    star.feather = 0.1;
    let card = Layer {
        masks: vec![star],
        effects: vec![planned(
            "stylize.pixelate",
            &[("size", ParamValue::Number(0.1))],
        )],
        ..spatial(
            1,
            Transform {
                scale: 0.7,
                rotation_x: 45.0,
                rotation_y: -20.0,
                ..Transform::default()
            },
        )
    };
    let plan = scene(vec![card], &Camera::default_for(CANVAS));
    check(
        "three-d-mask-and-effect",
        &mut renderer,
        &plan,
        &mut frames(),
    );
}

/// A timeline with a tilted map (the pattern lying back), three cards standing on it at
/// different depths (the middle one at the map's own depth, after it in track order), and
/// a camera clip that pushes in over the first second and orbits over the second.
fn camera_project() -> (Project, Id) {
    let format = TimelineFormat {
        width: CANVAS.0,
        height: CANVAS.1,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 3), "Camera", format);
    for n in 1..=9 {
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
    let length = Rate::FPS_30.frames_to_flicks(60);
    let clip = |id: u64, n: u64, transform: Transform| Clip {
        transform,
        ..Clip::new(
            Id::new(IdKind::Clip, id),
            Flicks::ZERO,
            length,
            ClipContent::Video {
                asset: asset(n),
                source_in: Flicks::ZERO,
            },
        )
    };
    // Cards whose bottom edges rest on the map: the map's surface rises by cos 72 / sin 72
    // of the canvas height for every canvas width it recedes from its centre.
    let standing = |depth: f64, x: f64| {
        let floor = 0.18 - (depth - 0.3) * (1920.0 / 1080.0) / 72f64.to_radians().tan();
        Transform {
            three_d: true,
            scale: 0.22,
            position: [x, floor - 0.11],
            position_z: depth,
            ..Transform::default()
        }
    };
    let key = |frame: i64, value: KeyValue| Keyframe {
        time: Rate::FPS_30.frames_to_flicks(frame),
        value,
        interpolation: Interpolation::EASE_IN_OUT,
    };
    let mut camera = Clip::new(
        Id::new(IdKind::Clip, 40),
        Flicks::ZERO,
        length,
        ClipContent::Camera(Box::new(CameraContent {
            point_of_interest_z: 0.3,
            ..CameraContent::default()
        })),
    );
    camera.keyframes.insert(
        "camera.position_z".into(),
        vec![
            key(0, KeyValue::Number(0.0)),
            key(30, KeyValue::Number(0.45)),
        ],
    );
    camera.keyframes.insert(
        "camera.orbit".into(),
        vec![
            key(30, KeyValue::Pair([0.0, 0.0])),
            key(59, KeyValue::Pair([35.0, 12.0])),
        ],
    );
    let tracks = vec![
        clip(30, 9, Transform::default()),
        clip(
            31,
            1,
            Transform {
                three_d: true,
                scale: 0.9,
                position: [0.0, 0.18],
                position_z: 0.3,
                rotation_x: 72.0,
                ..Transform::default()
            },
        ),
        clip(32, 2, standing(0.05, -0.2)),
        clip(33, 3, standing(0.3, 0.05)),
        clip(34, 4, standing(0.55, 0.24)),
        camera,
    ];
    for (i, clip) in tracks.into_iter().enumerate() {
        project
            .timelines
            .get_mut(&project.manifest.primary_timeline.clone())
            .unwrap()
            .tracks
            .push(Track {
                id: Id::new(IdKind::Track, 20 + i as u64),
                kind: TrackKind::Video,
                name: None,
                muted: false,
                hidden: false,
                locked: false,
                caption_style: None,
                clips: vec![clip],
            });
    }
    let timeline = project.manifest.primary_timeline.clone();
    (project, timeline)
}

/// Halfway, the camera has pushed in: the map and the cards are larger, the near card
/// most.
#[test]
fn three_d_camera_dolly() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (project, timeline) = camera_project();
    let plan = plan_frame(&project, &timeline, Rate::FPS_30.frames_to_flicks(30)).unwrap();
    check("three-d-camera-dolly", &mut renderer, &plan, &mut frames());
}

/// At the end, the camera has orbited to the right and up around the middle card: the
/// cards are seen from their right, the map from higher up.
#[test]
fn three_d_camera_orbit() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let (project, timeline) = camera_project();
    let plan = plan_frame(&project, &timeline, Rate::FPS_30.frames_to_flicks(59)).unwrap();
    check("three-d-camera-orbit", &mut renderer, &plan, &mut frames());
}
