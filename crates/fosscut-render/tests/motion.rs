//! Transitions, clip animations and the moving and stylizing effects on the GPU (ADR-0011,
//! items 1 and 4), checked against values that follow from their definitions.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeMap;

use fosscut_model::{
    Animation, Clip, ClipAnimations, ClipContent, Id, IdKind, ParamValue, Project, TimelineFormat,
    Track, TrackKind, Transition, effect_type,
};
use fosscut_render::color::TransferFunction;
use fosscut_render::effects::PlannedEffect;
use fosscut_render::plan::{Layer, LayerSource, RenderPlan, TransitionLayer, plan_frame};
use fosscut_render::{Image, Renderer};
use fosscut_time::Rate;
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

fn transition(kind: &str, progress: f64, params: &[(&str, &str)]) -> Layer {
    Layer {
        source: LayerSource::Transition(Box::new(TransitionLayer {
            outgoing: Layer::new(source(1)),
            incoming: Layer::new(source(2)),
            kind: kind.into(),
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), ParamValue::Name(v.to_string())))
                .collect(),
            progress,
        })),
        ..Layer::new(source(1))
    }
}

fn rgb(image: &Image, x: u32, y: u32) -> [u8; 3] {
    let p = image.pixel(x, y);
    [p[0], p[1], p[2]]
}

const W: [u8; 3] = [255, 255, 255];
const R: [u8; 3] = [255, 2, 0];

/// Asserts two colors are within a couple of code values.
fn same(a: [u8; 3], b: [u8; 3], what: &str) {
    assert!(
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3),
        "{what}: {a:?} vs {b:?}"
    );
}

#[test]
fn transitions_go_from_one_clip_to_the_other() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default()
        .with(1, solid(64, 36, WHITE))
        .with(2, solid(64, 36, RED));
    for kind in fosscut_model::transition_types().iter().map(|t| t.name) {
        let start = render(&mut renderer, vec![transition(kind, 0.0, &[])], &mut frames);
        let end = render(&mut renderer, vec![transition(kind, 1.0, &[])], &mut frames);
        same(
            rgb(&start, 32, 18),
            W,
            &format!("{kind} starts on the first clip"),
        );
        same(rgb(&end, 32, 18), R, &format!("{kind} ends on the second"));
    }
    let half = render(
        &mut renderer,
        vec![transition("dissolve", 0.5, &[])],
        &mut frames,
    );
    assert!(
        rgb(&half, 32, 18)[1] > 150 && rgb(&half, 32, 18)[1] < 250,
        "half white, half red in light"
    );
    let dip = render(
        &mut renderer,
        vec![transition("dip", 0.5, &[])],
        &mut frames,
    );
    same(rgb(&dip, 32, 18), [0, 0, 0], "through black in the middle");
    let wipe = render(
        &mut renderer,
        vec![transition("wipe", 0.5, &[("direction", "left")])],
        &mut frames,
    );
    assert_eq!(
        (rgb(&wipe, 4, 18), rgb(&wipe, 60, 18)),
        (W, R),
        "the edge moves left from the right"
    );
    let up = render(
        &mut renderer,
        vec![transition("wipe", 0.5, &[("direction", "up")])],
        &mut frames,
    );
    same(rgb(&up, 32, 2), W, "the top is still the first clip");
    same(rgb(&up, 32, 34), R, "the edge moves up from the bottom");
    let push = render(
        &mut renderer,
        vec![transition("push", 0.25, &[("direction", "left")])],
        &mut frames,
    );
    assert_eq!(
        (rgb(&push, 10, 18), rgb(&push, 60, 18)),
        (W, R),
        "a quarter in from the right"
    );
    let iris = render(
        &mut renderer,
        vec![transition("iris", 0.4, &[])],
        &mut frames,
    );
    assert_eq!(
        (rgb(&iris, 32, 18), rgb(&iris, 1, 1)),
        (R, W),
        "opening from the middle"
    );
}

#[test]
fn the_planner_centres_transitions_on_the_cut_and_applies_animations() {
    let format = TimelineFormat {
        width: 320,
        height: 180,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let frames_of = |n: i64| Rate::FPS_30.frames_to_flicks(n);
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Cut", format);
    let title = |id: u64, start: i64, text: &str| {
        Clip::new(
            Id::new(IdKind::Clip, id),
            frames_of(start),
            frames_of(30),
            ClipContent::Text(Box::new(fosscut_model::TextContent {
                text: text.into(),
                style: fosscut_model::TextStyle::default(),
                ..Default::default()
            })),
        )
    };
    let mut first = title(2, 0, "One");
    first.transition = Some(Transition {
        transition_type: "slide".into(),
        duration: frames_of(10),
        params: BTreeMap::new(),
    });
    first.animations = ClipAnimations {
        entrance: Some(Animation {
            preset: "slide_left".into(),
            duration: frames_of(10),
            delay: fosscut_time::Flicks::ZERO,
        }),
        ..Default::default()
    };
    let second = title(3, 30, "Two");
    let timeline = project.manifest.primary_timeline.clone();
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 4),
            kind: TrackKind::Text,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![first, second],
        });
    let at = |n: i64| plan_frame(&project, &timeline, frames_of(n)).unwrap();
    let layer = &at(0).layers[0];
    assert!(
        (layer.transform.position[0] - 1.0).abs() < 1e-9,
        "starts off to the right"
    );
    assert_eq!(at(20).layers[0].transform.position, [0.0, 0.0], "at rest");
    let LayerSource::Transition(t) = &at(30).layers[0].source else {
        panic!("a transition over the cut");
    };
    assert!((t.progress - 0.5).abs() < 1e-9, "centred on the cut");
    assert_eq!(t.kind, "slide");
    assert!(matches!(&t.outgoing.source, LayerSource::Text(c) if c.content.text == "One"));
    assert!(matches!(&t.incoming.source, LayerSource::Text(c) if c.content.text == "Two"));
    assert!(
        matches!(at(25).layers[0].source, LayerSource::Transition(_)),
        "from five frames before"
    );
    assert!(
        matches!(at(24).layers[0].source, LayerSource::Text(_)),
        "not earlier"
    );
    assert!(
        matches!(at(35).layers[0].source, LayerSource::Text(_)),
        "over after five frames"
    );
}

fn effect(kind: &str, values: &[(&str, f64)]) -> PlannedEffect {
    let spec = effect_type(kind).unwrap();
    let mut all: BTreeMap<String, ParamValue> = spec
        .params
        .iter()
        .map(|p| (p.name.to_string(), p.default.value()))
        .collect();
    for (name, value) in values {
        all.insert(name.to_string(), ParamValue::Number(*value));
    }
    PlannedEffect {
        effect_type: kind.into(),
        values: all,
        seed: 7,
        time: 0.3,
    }
}

#[test]
fn stylizing_and_moving_effects_rest_at_zero_and_act_above_it() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let ramp = |x: u32, y: u32| {
        [
            16 + (x * 219 / 63) as u8,
            90 + (y * 2) as u8,
            160 - (x / 2) as u8,
        ]
    };
    let mut frames = Frames::default().with(1, nv12(64, 36, bt709_limited(), ramp));
    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let with = |effects: Vec<PlannedEffect>| Layer {
        effects,
        ..Layer::new(source(1))
    };
    for kind in [
        "stylize.glitch",
        "stylize.rgb_split",
        "stylize.vhs",
        "stylize.invert",
        "motion.shake",
        "motion.zoom_blur",
        "motion.flash",
        "motion.rainbow",
        "motion.zoom_pulse",
    ] {
        let image = render(
            &mut renderer,
            vec![with(vec![effect(kind, &[("amount", 0.0)])])],
            &mut frames,
        );
        for (x, y) in [(5, 5), (32, 18), (58, 30)] {
            let (a, b) = (rgb(&image, x, y), rgb(&plain, x, y));
            assert!(
                a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= 2),
                "{kind} at rest: {a:?} vs {b:?}"
            );
        }
    }
    let negative = render(
        &mut renderer,
        vec![with(vec![effect("stylize.invert", &[])])],
        &mut frames,
    );
    let [a, b] = [rgb(&negative, 10, 10)[0], rgb(&plain, 10, 10)[0]];
    assert!(
        (i32::from(a) + i32::from(b) - 255).abs() <= 2,
        "inverted: {a} and {b}"
    );
    let mirrored = render(
        &mut renderer,
        vec![with(vec![effect("stylize.mirror", &[])])],
        &mut frames,
    );
    assert_eq!(
        rgb(&mirrored, 60, 18),
        rgb(&mirrored, 3, 18),
        "the left half reflected"
    );
    let flat = render(
        &mut renderer,
        vec![with(vec![effect("stylize.posterize", &[("levels", 2.0)])])],
        &mut frames,
    );
    assert!(
        rgb(&flat, 32, 18).iter().all(|c| *c == 0 || *c == 255),
        "two levels: {:?}",
        rgb(&flat, 32, 18)
    );
    let blocks = render(
        &mut renderer,
        vec![with(vec![effect("stylize.pixelate", &[("size", 0.25)])])],
        &mut frames,
    );
    assert_eq!(rgb(&blocks, 1, 1), rgb(&blocks, 7, 7), "one block");
    let flash = render(
        &mut renderer,
        vec![Layer {
            effects: vec![PlannedEffect {
                time: 0.0,
                ..effect("motion.flash", &[("amount", 1.0)])
            }],
            ..Layer::new(source(1))
        }],
        &mut frames,
    );
    same(rgb(&flash, 20, 20), W, "a full flash on the beat is white");
}
