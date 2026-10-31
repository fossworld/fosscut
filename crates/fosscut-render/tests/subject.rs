//! Subject effects on the GPU (ADR-0033, items 3 and 4): `key.subject` and the body effects
//! on a drawn figure whose matte, mask and landmarks the test provider supplies, so no model
//! runs. Each effect changes the pixels its parameters say and nothing else, and without the
//! analysis it leaves the picture unchanged.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeMap;

use fosscut_model::{ParamValue, effect_type};
use fosscut_render::color::TransferFunction;
use fosscut_render::effects::PlannedEffect;
use fosscut_render::plan::{Layer, RenderPlan};
use fosscut_render::subject::MatteKind;
use fosscut_render::{FrameProvider, Image, Renderer};
use support::*;

const SIZE: (u32, u32) = (160, 90);
const MASK: (u32, u32) = (80, 45);

/// An effect with every parameter at its default except `values`.
fn effect(kind: &str, values: &[(&str, ParamValue)], time: f64) -> PlannedEffect {
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
        time,
    }
}

fn n(v: f64) -> ParamValue {
    ParamValue::Number(v)
}

fn name(v: &str) -> ParamValue {
    ParamValue::Name(v.into())
}

fn frames() -> Frames {
    Frames::default()
        .with(1, figure_frame(SIZE.0, SIZE.1))
        .with(2, solid(SIZE.0, SIZE.1, [145, 54, 34]))
        .with_matte(1, MatteKind::Person, figure_matte(MASK.0, MASK.1))
        .with_pose(1, figure_pose(MASK))
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

fn with(effects: Vec<PlannedEffect>) -> Layer {
    Layer {
        effects,
        ..Layer::new(source(1))
    }
}

fn rgb(image: &Image, x: u32, y: u32) -> [u8; 3] {
    let p = image.pixel(x, y);
    [p[0], p[1], p[2]]
}

fn near(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
}

/// Pixels inside the figure (its torso), three pixels outside its head on the right and the
/// left, past the mask's soft edge (the head spans x 74 to 86 on rows 12 to 24, with nothing else near), further right,
/// and far away.
const TORSO: (u32, u32) = (80, 45);
const RIGHT_OUTSIDE: (u32, u32) = (89, 18);
const LEFT_OUTSIDE: (u32, u32) = (70, 18);
const BEYOND: (u32, u32) = (95, 18);
const FAR: (u32, u32) = (20, 45);

#[test]
fn the_subject_key_keeps_the_figure_and_replaces_the_background() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let green = render(&mut renderer, vec![Layer::new(source(2))], &mut frames);
    let at = |image: &Image, (x, y): (u32, u32)| rgb(image, x, y);

    // Over a green track: the figure stays, the green shows around it.
    let keyed = render(
        &mut renderer,
        vec![
            Layer::new(source(2)),
            with(vec![effect("key.subject", &[], 0.0)]),
        ],
        &mut frames,
    );
    assert!(
        near(at(&keyed, TORSO), at(&plain, TORSO), 2),
        "the figure stays"
    );
    for point in [FAR, RIGHT_OUTSIDE, LEFT_OUTSIDE] {
        assert!(
            near(at(&keyed, point), at(&green, point), 3),
            "the background goes at {point:?}"
        );
    }

    // Inverted: the figure goes and the background stays.
    let inverted = render(
        &mut renderer,
        vec![
            Layer::new(source(2)),
            with(vec![effect(
                "key.subject",
                &[("invert", ParamValue::Flag(true))],
                0.0,
            )]),
        ],
        &mut frames,
    );
    assert!(near(at(&inverted, TORSO), at(&green, TORSO), 3));
    assert!(near(at(&inverted, FAR), at(&plain, FAR), 2));

    // A colour behind.
    let red = render(
        &mut renderer,
        vec![with(vec![effect(
            "key.subject",
            &[("background", name("color")), ("color", name("#FF0000"))],
            0.0,
        )])],
        &mut frames,
    );
    assert!(near(at(&red, FAR), [255, 0, 0], 2), "{:?}", at(&red, FAR));
    assert!(near(at(&red, TORSO), at(&plain, TORSO), 2));

    // The picture blurred behind: the checker is smoothed, the figure stays sharp.
    let blurred = render(
        &mut renderer,
        vec![with(vec![effect(
            "key.subject",
            &[("background", name("blur")), ("blur", n(0.05))],
            0.0,
        )])],
        &mut frames,
    );
    let contrast = |image: &Image| {
        let row: Vec<i32> = (4..36).map(|x| i32::from(image.pixel(x, 70)[1])).collect();
        row.iter().max().unwrap() - row.iter().min().unwrap()
    };
    assert!(
        contrast(&blurred) * 3 < contrast(&plain),
        "{} vs {}",
        contrast(&blurred),
        contrast(&plain)
    );
    assert!(near(at(&blurred, TORSO), at(&plain, TORSO), 2));

    // Without the matte nothing changes, whatever the parameters.
    let mut bare = Frames::default().with(1, figure_frame(SIZE.0, SIZE.1));
    let unchanged = render(
        &mut renderer,
        vec![with(vec![effect(
            "key.subject",
            &[("background", name("color"))],
            0.0,
        )])],
        &mut bare,
    );
    assert_eq!(
        unchanged,
        render(&mut renderer, vec![Layer::new(source(1))], &mut bare)
    );
    // The subject matte is another analysis; asking for it finds nothing here.
    let other = render(
        &mut renderer,
        vec![with(vec![effect(
            "key.subject",
            &[("source", name("subject"))],
            0.0,
        )])],
        &mut frames,
    );
    assert_eq!(other, plain);
}

#[test]
fn the_outline_hugs_the_figure_from_behind() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let outlined = render(
        &mut renderer,
        vec![with(vec![effect(
            "body.outline",
            &[
                ("color", name("#FF00FF")),
                ("width", n(0.05)),
                ("softness", n(0.0)),
            ],
            0.0,
        )])],
        &mut frames,
    );
    let at = |image: &Image, (x, y): (u32, u32)| rgb(image, x, y);
    assert!(
        near(at(&outlined, RIGHT_OUTSIDE), [255, 0, 255], 8),
        "{:?}",
        at(&outlined, RIGHT_OUTSIDE)
    );
    assert!(near(at(&outlined, LEFT_OUTSIDE), [255, 0, 255], 8));
    assert!(
        near(at(&outlined, TORSO), at(&plain, TORSO), 2),
        "the person stays in front"
    );
    assert!(
        near(at(&outlined, BEYOND), at(&plain, BEYOND), 2),
        "beyond the width"
    );
    assert!(near(at(&outlined, FAR), at(&plain, FAR), 2));
    let none = render(
        &mut renderer,
        vec![with(vec![effect(
            "body.outline",
            &[("width", n(0.0))],
            0.0,
        )])],
        &mut frames,
    );
    assert!(
        near(at(&none, RIGHT_OUTSIDE), at(&plain, RIGHT_OUTSIDE), 2),
        "no width, no line"
    );
}

#[test]
fn the_glow_adds_light_around_the_figure() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let glowing = render(
        &mut renderer,
        vec![with(vec![effect(
            "body.glow",
            &[
                ("color", name("#00FF00")),
                ("size", n(0.05)),
                ("strength", n(2.0)),
            ],
            0.0,
        )])],
        &mut frames,
    );
    let green = |image: &Image, (x, y): (u32, u32)| image.pixel(x, y)[1];
    let gain = |point| i32::from(green(&glowing, point)) - i32::from(green(&plain, point));
    assert!(
        gain(RIGHT_OUTSIDE) > 60,
        "near the edge: {}",
        gain(RIGHT_OUTSIDE)
    );
    assert!(
        gain(BEYOND) > 10 && gain(BEYOND) < gain(RIGHT_OUTSIDE),
        "fading: {}",
        gain(BEYOND)
    );
    assert!(gain(FAR) < 4, "far away: {}", gain(FAR));
    assert!(
        near(
            rgb(&glowing, TORSO.0, TORSO.1),
            rgb(&plain, TORSO.0, TORSO.1),
            2
        ),
        "not over the person"
    );
    let off = render(
        &mut renderer,
        vec![with(vec![effect(
            "body.glow",
            &[("strength", n(0.0))],
            0.0,
        )])],
        &mut frames,
    );
    assert!(
        off.psnr(&plain).unwrap() > 50.0,
        "strength 0 changes nothing"
    );
}

#[test]
fn the_shadow_falls_where_its_offset_points() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let shadowed = render(
        &mut renderer,
        vec![with(vec![effect(
            "body.shadow",
            &[
                ("offset", ParamValue::Pair([0.06, 0.0])),
                ("blur", n(0.002)),
                ("opacity", n(1.0)),
            ],
            0.0,
        )])],
        &mut frames,
    );
    let luma = |image: &Image, (x, y): (u32, u32)| {
        let [r, g, b] = rgb(image, x, y);
        u32::from(r) + u32::from(g) + u32::from(b)
    };
    assert!(
        luma(&shadowed, RIGHT_OUTSIDE) * 4 < luma(&plain, RIGHT_OUTSIDE),
        "the shadow falls right"
    );
    assert!(
        near(
            rgb(&shadowed, BEYOND.0, BEYOND.1),
            rgb(&plain, BEYOND.0, BEYOND.1),
            2
        ),
        "only as far as the offset"
    );
    assert!(
        near(
            rgb(&shadowed, LEFT_OUTSIDE.0, LEFT_OUTSIDE.1),
            rgb(&plain, LEFT_OUTSIDE.0, LEFT_OUTSIDE.1),
            2
        ),
        "not left"
    );
    assert!(
        near(
            rgb(&shadowed, TORSO.0, TORSO.1),
            rgb(&plain, TORSO.0, TORSO.1),
            2
        ),
        "behind the person"
    );
    assert!(near(
        rgb(&shadowed, FAR.0, FAR.1),
        rgb(&plain, FAR.0, FAR.1),
        2
    ));
}

#[test]
fn sparkles_twinkle_along_the_body_the_same_way_every_time() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let sparkle = |time: f64| {
        with(vec![effect(
            "body.sparkle",
            &[("amount", n(1.0)), ("size", n(0.05))],
            time,
        )])
    };
    let first = render(&mut renderer, vec![sparkle(0.4)], &mut frames);
    let again = render(&mut renderer, vec![sparkle(0.4)], &mut frames);
    let later = render(&mut renderer, vec![sparkle(1.1)], &mut frames);
    assert_eq!(first, again, "a frame renders the same every time");
    assert_ne!(first, later, "they move on");
    // Brighter pixels, all near the figure: none in the outer columns.
    let mut brighter = 0;
    for y in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            let lift = first.pixel(x, y)[0] as i32 - plain.pixel(x, y)[0] as i32;
            if lift > 20 {
                brighter += 1;
                assert!(
                    (40..120).contains(&x),
                    "a sparkle at ({x}, {y}) is far from the body"
                );
            }
        }
    }
    assert!(brighter > 50, "{brighter} pixels sparkle");
    let none = render(
        &mut renderer,
        vec![with(vec![effect(
            "body.sparkle",
            &[("amount", n(0.0))],
            0.4,
        )])],
        &mut frames,
    );
    assert_eq!(none, plain);
    // Without a pose, or with no one in it, nothing is drawn.
    let mut nobody = Frames::default().with(1, figure_frame(SIZE.0, SIZE.1));
    let mut pose = figure_pose(MASK);
    pose.presence = 0.1;
    nobody = nobody.with_pose(1, pose);
    let empty = render(&mut renderer, vec![sparkle(0.4)], &mut nobody);
    assert_eq!(
        empty,
        render(&mut renderer, vec![Layer::new(source(1))], &mut nobody)
    );
}

#[test]
fn several_subject_effects_on_one_clip_each_read_their_own_field() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = frames();
    let plain = render(&mut renderer, vec![Layer::new(source(1))], &mut frames);
    let both = render(
        &mut renderer,
        vec![with(vec![
            effect(
                "body.glow",
                &[
                    ("color", name("#00FF00")),
                    ("size", n(0.08)),
                    ("strength", n(2.0)),
                ],
                0.0,
            ),
            effect(
                "body.outline",
                &[
                    ("color", name("#FF00FF")),
                    ("width", n(0.05)),
                    ("softness", n(0.0)),
                ],
                0.0,
            ),
        ])],
        &mut frames,
    );
    // The outline is drawn over the glow near the edge; the glow still reaches past it.
    let edge = rgb(&both, RIGHT_OUTSIDE.0, RIGHT_OUTSIDE.1);
    assert!(edge[0] > 200 && edge[2] > 200, "the outline: {edge:?}");
    assert!(
        both.pixel(BEYOND.0, BEYOND.1)[1] > plain.pixel(BEYOND.0, BEYOND.1)[1] + 10,
        "the glow"
    );
}
