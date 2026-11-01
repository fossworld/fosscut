//! Particle layers (ADR-0038) through the planner and the GPU: a frame renders the same
//! however it is reached, the seed changes the particles, sprites draw their shapes where
//! the emitter puts them, and motion blur streaks them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeMap;

use fosscut_model::{
    Burst, Clip, ClipContent, Id, IdKind, Interpolation, KeyValue, Keyframe, ParamValue,
    ParticleBlend, ParticleContent, Project, SpriteKind, TimelineFormat, Track, TrackKind,
};
use fosscut_render::color::TransferFunction;
use fosscut_render::effects::PlannedEffect;
use fosscut_render::particles::ParticleLayer;
use fosscut_render::plan::{Layer, LayerSource, RenderPlan, plan_frame};
use fosscut_render::{Image, Renderer};
use fosscut_time::{Flicks, Rate};
use support::*;

const SIZE: (u32, u32) = (320, 180);
const CANVAS: (u32, u32) = (1920, 1080);

fn at(seconds: f64) -> Flicks {
    Flicks::from_seconds_f64(seconds).unwrap()
}

fn layer(content: ParticleContent, seconds: f64) -> ParticleLayer {
    ParticleLayer {
        content,
        keyframes: BTreeMap::new(),
        time: at(seconds),
        frame: Rate::FPS_30.frame_duration(),
        sprite: None,
    }
}

fn plan(layers: Vec<Layer>) -> RenderPlan {
    RenderPlan {
        canvas: CANVAS,
        layers,
        skipped: Vec::new(),
    }
}

fn render(renderer: &mut Renderer, plan: &RenderPlan, frames: &mut Frames) -> Image {
    renderer
        .render_image(plan, frames, SIZE, TransferFunction::Bt709)
        .unwrap()
}

fn luma(image: &Image, x: u32, y: u32) -> u32 {
    let [r, g, b, _] = image.pixel(x, y);
    (u32::from(r) * 2 + u32::from(g) * 7 + u32::from(b)) / 10
}

/// A single still particle at `position` (fractions from the canvas centre).
fn still(position: [f64; 2], size: f64, kind: SpriteKind) -> ParticleContent {
    let mut content = ParticleContent {
        rate: 0.0,
        speed: 0.0,
        size,
        life: 10.0,
        opacity_over_life: vec![1.0],
        bursts: vec![Burst {
            at: Flicks::ZERO,
            count: 1,
        }],
        ..ParticleContent::default()
    };
    content.emitter.position = position;
    content.sprite.kind = kind;
    content.sprite.softness = 0.1;
    content
}

fn busy() -> ParticleContent {
    let mut content = fosscut_core::particle_preset("sparks").unwrap().content;
    content.turbulence = 0.1;
    content.bursts.push(Burst {
        at: at(0.4),
        count: 200,
    });
    content
}

#[test]
fn frames_render_the_same_however_they_are_reached() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default();
    let shown = |t: f64| plan(vec![Layer::particles(layer(busy(), t))]);
    let alone = render(&mut renderer, &shown(1.25), &mut frames);
    assert!(alone.rgba.chunks(4).any(|p| p[0] > 200), "sparks show");
    for t in [0.2, 0.9, 2.0, 1.2] {
        render(&mut renderer, &shown(t), &mut frames);
    }
    let after = render(&mut renderer, &shown(1.25), &mut frames);
    assert_eq!(after.rgba, alone.rgba, "the same frame after others");
    let Some(mut fresh) = support::renderer() else {
        return;
    };
    render(&mut fresh, &shown(3.0), &mut frames);
    assert_eq!(
        render(&mut fresh, &shown(1.25), &mut frames).rgba,
        alone.rgba
    );
}

#[test]
fn another_seed_gives_other_particles_alike() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default();
    let mut content = fosscut_core::particle_preset("confetti").unwrap().content;
    let first = render(
        &mut renderer,
        &plan(vec![Layer::particles(layer(content.clone(), 1.0))]),
        &mut frames,
    );
    content.seed += 1;
    let second = render(
        &mut renderer,
        &plan(vec![Layer::particles(layer(content, 1.0))]),
        &mut frames,
    );
    let differing = first
        .rgba
        .chunks(4)
        .zip(second.rgba.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(differing > 500, "{differing} pixels differ");
    let mean = |image: &Image| {
        image.rgba.chunks(4).map(|p| u64::from(p[0])).sum::<u64>() as f64
            / (image.rgba.len() / 4) as f64
    };
    let (a, b) = (mean(&first), mean(&second));
    assert!((a - b).abs() < 0.35 * a.max(b), "alike: {a} and {b}");
}

#[test]
fn sprites_draw_their_shapes_where_the_emitter_is() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default();
    // A dot a quarter of the canvas width right of the centre: x = 240, y = 90 at 320x180.
    let dot = render(
        &mut renderer,
        &plan(vec![Layer::particles(layer(
            still([0.25, 0.0], 0.2, SpriteKind::Dot),
            0.5,
        ))]),
        &mut frames,
    );
    assert!(luma(&dot, 240, 90) > 240, "{:?}", dot.pixel(240, 90));
    assert_eq!(luma(&dot, 80, 90), 0, "nothing elsewhere");
    assert_eq!(luma(&dot, 240, 90 + 30), 0, "a dot 36 pixels across");
    // A ring is dark in the middle and bright on its rim (radius 18 pixels).
    let ring = render(
        &mut renderer,
        &plan(vec![Layer::particles(layer(
            still([0.0, 0.0], 0.2, SpriteKind::Ring),
            0.5,
        ))]),
        &mut frames,
    );
    assert!(luma(&ring, 160, 90) < 20);
    assert!(luma(&ring, 160 + 15, 90) > 150, "{:?}", ring.pixel(175, 90));
    // A star reaches out along its points and not between them.
    let mut star = still([0.0, 0.0], 0.4, SpriteKind::Star);
    star.sprite.softness = 0.0;
    let star = render(
        &mut renderer,
        &plan(vec![Layer::particles(layer(star, 0.5))]),
        &mut frames,
    );
    assert!(luma(&star, 160 + 28, 90) > 120, "along a point");
    assert!(luma(&star, 160 + 20, 90 + 20) < 40, "between points");
    // A spark streaks along its direction of travel, brightest at its head.
    let mut spark = still([0.0, 0.0], 0.03, SpriteKind::Spark);
    spark.speed = 1.0;
    spark.direction = 0.0;
    spark.spread = 0.0;
    spark.sprite.stretch = 0.2;
    let streak = render(
        &mut renderer,
        &plan(vec![Layer::particles(layer(spark, 0.1))]),
        &mut frames,
    );
    // Centre at x = 160 + 0.1 * 180 = 178; 0.2 canvas heights (36 pixels) long.
    let row: Vec<u32> = (140..200).map(|x| luma(&streak, x, 90)).collect();
    let lit = row.iter().filter(|v| **v > 30).count();
    assert!(lit > 25, "{row:?}");
    assert!(luma(&streak, 178, 90 + 8) < 20, "thin across");
    assert!(
        luma(&streak, 194, 90) > luma(&streak, 162, 90),
        "bright head"
    );
}

#[test]
fn light_adds_up_where_normal_particles_cover() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default();
    let pair = |blend: ParticleBlend| {
        let mut content = still([0.0, 0.0], 0.2, SpriteKind::Dot);
        content.bursts[0].count = 2;
        content.opacity = 0.4;
        content.colors = vec![fosscut_model::Color::parse("#808080").unwrap()];
        content.blend = blend;
        plan(vec![Layer::particles(layer(content, 0.5))])
    };
    let added = render(&mut renderer, &pair(ParticleBlend::Add), &mut frames);
    let covered = render(&mut renderer, &pair(ParticleBlend::Normal), &mut frames);
    assert!(
        luma(&added, 160, 90) > luma(&covered, 160, 90) + 10,
        "{} and {}",
        luma(&added, 160, 90),
        luma(&covered, 160, 90)
    );
}

#[test]
fn motion_blur_streaks_fast_particles() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default();
    let mut fast = still([-0.2, 0.0], 0.03, SpriteKind::Dot);
    fast.speed = 3.0;
    fast.direction = 0.0;
    fast.spread = 0.0;
    let sharp = Layer::particles(layer(fast.clone(), 0.2));
    let spec = fosscut_model::effect_type("motion.blur").unwrap();
    let mut values: BTreeMap<String, ParamValue> = spec
        .params
        .iter()
        .map(|p| (p.name.to_string(), p.default.value()))
        .collect();
    values.insert("amount".into(), ParamValue::Number(1.0));
    values.insert("samples".into(), ParamValue::Number(16.0));
    let blurred = Layer {
        effects: vec![PlannedEffect {
            effect_type: "motion.blur".into(),
            values,
            seed: 0,
            time: 0.2,
        }],
        ..Layer::particles(layer(fast, 0.2))
    };
    let width = |image: &Image| (0..SIZE.0).filter(|x| luma(image, *x, 90) > 8).count();
    let sharp = render(&mut renderer, &plan(vec![sharp]), &mut frames);
    let blurred = render(&mut renderer, &plan(vec![blurred]), &mut frames);
    // In a 30 fps frame it moves 0.1 canvas heights, 18 pixels.
    assert!(
        width(&blurred) >= width(&sharp) + 12,
        "{} and {}",
        width(&blurred),
        width(&sharp)
    );
}

#[test]
fn image_sprites_take_their_brightness_as_coverage() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    // A white square image, and a black one.
    let mut frames = Frames::default()
        .with(1, solid(64, 64, [235, 128, 128]))
        .with(2, solid(64, 64, [16, 128, 128]));
    let sprite = |n: u64| {
        let mut content = still([0.0, 0.0], 0.2, SpriteKind::Image);
        content.sprite.asset = Some(asset(n));
        content.colors = vec![fosscut_model::Color::parse("#FF8000").unwrap()];
        ParticleLayer {
            sprite: Some(source(n)),
            ..layer(content, 0.5)
        }
    };
    let white = render(
        &mut renderer,
        &plan(vec![Layer::particles(sprite(1))]),
        &mut frames,
    );
    let [r, g, b, _] = white.pixel(160, 90);
    assert!(
        r > 200 && g > 80 && g < 200 && b < 30,
        "tinted: {r} {g} {b}"
    );
    assert!(luma(&white, 160 + 14, 90 + 14) > 100, "a square image");
    let black = render(
        &mut renderer,
        &plan(vec![Layer::particles(sprite(2))]),
        &mut frames,
    );
    assert!(luma(&black, 160, 90) < 5);
}

#[test]
fn the_planner_turns_particle_clips_into_layers() {
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Particles", format);
    let timeline = project.manifest.primary_timeline.clone();
    let mut clip = Clip::new(
        Id::new(IdKind::Clip, 7),
        at(1.0),
        at(4.0),
        ClipContent::Particles {
            particles: Box::new(busy()),
        },
    );
    let key = Keyframe {
        time: Flicks::ZERO,
        value: KeyValue::Number(5.0),
        interpolation: Interpolation::Linear,
    };
    clip.keyframes.insert("particles.rate".into(), vec![key]);
    clip.keyframes
        .insert("opacity".into(), vec![key_value(0.5)]);
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 3),
            kind: TrackKind::Video,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![clip],
        });
    assert!(
        plan_frame(&project, &timeline, at(0.5))
            .unwrap()
            .layers
            .is_empty()
    );
    let planned = plan_frame(&project, &timeline, at(2.5)).unwrap();
    let layer = &planned.layers[0];
    assert_eq!(layer.opacity, 0.5);
    let LayerSource::Particles(particles) = &layer.source else {
        panic!("{:?}", layer.source);
    };
    assert_eq!(particles.time, at(1.5));
    assert_eq!(particles.frame, Rate::FPS_30.frame_duration());
    assert_eq!(
        particles.keyframes.keys().collect::<Vec<_>>(),
        vec!["particles.rate"],
        "only the emitter's own keyframes"
    );
    assert!(layer.frame_source().is_none());
    // A different moment keys the frame cache differently; the same one alike.
    let again = plan_frame(&project, &timeline, at(2.5)).unwrap();
    let later = plan_frame(&project, &timeline, at(2.6)).unwrap();
    assert_eq!(again.cache_key_bytes(SIZE), planned.cache_key_bytes(SIZE));
    assert_ne!(later.cache_key_bytes(SIZE), planned.cache_key_bytes(SIZE));
}

fn key_value(value: f64) -> Keyframe {
    Keyframe {
        time: Flicks::ZERO,
        value: KeyValue::Number(value),
        interpolation: Interpolation::Linear,
    }
}
