//! Format 0.15.0 (ADR-0035): optical-flow slow motion and the effects that read
//! neighbouring frames, with their sparse JSON, defaults and validation.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use fosscut_model::{
    Asset, AssetKind, Clip, ClipContent, Effect, EffectCategory, Id, IdKind, Interpolate,
    Interpolation, KeyValue, Keyframe, MediaInfo, ParamKind, ParamValue, PathHints, Project, Speed,
    TimelineFormat, Track, TrackKind, VideoInfo, effect_type, validate,
};
use fosscut_time::{Flicks, Rate};
use serde_json::json;

fn frames(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

/// A project with one 3 s video clip of a 10 s, 30 fps asset.
fn project_with(clip: Clip) -> Project {
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Temporal", format);
    project.media.assets.push(Asset {
        folder: None,
        id: Id::new(IdKind::Asset, 1),
        kind: AssetKind::Video,
        name: "clip.mp4".into(),
        path: PathHints::default(),
        identity: None,
        info: MediaInfo {
            duration: Some(frames(300)),
            video: Some(VideoInfo {
                width: 1920,
                height: 1080,
                rate: Some(Rate::FPS_30),
                codec: None,
                rotation: 0,
            }),
            ..Default::default()
        },
    });
    project.manifest.next_id = 20;
    let timeline = project.manifest.primary_timeline.clone();
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 2),
            kind: TrackKind::Video,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![clip],
        });
    project
}

fn video() -> Clip {
    Clip::new(
        Id::new(IdKind::Clip, 3),
        Flicks::ZERO,
        frames(90),
        ClipContent::Video {
            asset: Id::new(IdKind::Asset, 1),
            source_in: Flicks::ZERO,
        },
    )
}

fn codes(clip: Clip) -> Vec<String> {
    validate(&project_with(clip))
        .into_iter()
        .map(|i| i.code)
        .collect()
}

fn effect(kind: &str, params: Vec<(&str, ParamValue)>) -> Effect {
    Effect {
        id: Id::new(IdKind::Effect, 9),
        effect_type: kind.into(),
        version: 1,
        enabled: true,
        params: params
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        keyframes: Default::default(),
    }
}

#[test]
fn flow_slow_motion_is_stored_sparsely_and_validates() {
    let speed = Speed {
        interpolate: Interpolate::Flow,
        ..Speed::constant(0.25)
    };
    let stored = serde_json::to_value(&speed).unwrap();
    assert_eq!(stored, json!({"rate": 0.25, "interpolate": "flow"}));
    let back: Speed = serde_json::from_value(stored).unwrap();
    assert_eq!(back, speed);
    assert_eq!(
        serde_json::to_value(Speed::constant(0.25)).unwrap(),
        json!({"rate": 0.25}),
        "nearest, the default, is left out"
    );
    let unknown: Result<Speed, _> =
        serde_json::from_value(json!({"rate": 0.5, "interpolate": "magic"}));
    assert!(
        unknown.is_err(),
        "unknown ways of interpolating are refused"
    );

    let mut clip = video();
    clip.speed = Some(speed);
    assert!(codes(clip.clone()).is_empty(), "{:?}", codes(clip));
}

#[test]
fn temporal_effects_are_registered_with_their_parameters() {
    let expect = |name: &str, category, params: &[(&str, ParamKind)]| {
        let spec = effect_type(name).unwrap();
        assert_eq!(spec.category, category, "{name}");
        assert_eq!(spec.version, 1);
        assert!(!spec.is_audio());
        let found: Vec<(&str, ParamKind)> = spec.params.iter().map(|p| (p.name, p.kind)).collect();
        assert_eq!(found, params, "{name}");
        spec
    };
    let blur = expect(
        "motion.blur",
        EffectCategory::Motion,
        &[
            ("amount", ParamKind::Number),
            ("samples", ParamKind::Number),
        ],
    );
    assert_eq!(blur.param("amount").unwrap().max, Some(1.0));
    assert_eq!(
        (
            blur.param("samples").unwrap().min,
            blur.param("samples").unwrap().max
        ),
        (Some(2.0), Some(32.0))
    );
    let denoise = expect(
        "detail.denoise",
        EffectCategory::Detail,
        &[
            ("strength", ParamKind::Number),
            ("temporal", ParamKind::Bool),
        ],
    );
    assert!(!denoise.param("temporal").unwrap().animatable);
    let deflicker = expect(
        "color.deflicker",
        EffectCategory::Color,
        &[
            ("strength", ParamKind::Number),
            ("window", ParamKind::Number),
        ],
    );
    assert_eq!(deflicker.param("window").unwrap().unit, "frames");

    // Every parameter has a default, so a bare effect is complete.
    let values = effect("detail.denoise", vec![])
        .values_at(Flicks::ZERO)
        .unwrap();
    assert_eq!(values["strength"], ParamValue::Number(0.5));
    assert_eq!(values["temporal"], ParamValue::Flag(true));
    let values = effect("color.deflicker", vec![])
        .values_at(Flicks::ZERO)
        .unwrap();
    assert_eq!(values["window"], ParamValue::Number(6.0));
    let values = effect("motion.blur", vec![])
        .values_at(Flicks::ZERO)
        .unwrap();
    assert_eq!(values["amount"], ParamValue::Number(0.5));
    assert_eq!(values["samples"], ParamValue::Number(12.0));
}

#[test]
fn temporal_effects_validate_their_ranges_and_keyframes() {
    let n = ParamValue::Number;
    let with = |effect: Effect| {
        let mut clip = video();
        clip.effects.push(effect);
        codes(clip)
    };
    for good in [
        effect(
            "motion.blur",
            vec![("amount", n(1.0)), ("samples", n(32.0))],
        ),
        effect(
            "detail.denoise",
            vec![("strength", n(1.0)), ("temporal", ParamValue::Flag(false))],
        ),
        effect("color.deflicker", vec![("window", n(2.0))]),
    ] {
        let kind = good.effect_type.clone();
        assert!(with(good).is_empty(), "{kind}");
    }
    for (bad, what) in [
        (
            effect("motion.blur", vec![("amount", n(1.5))]),
            "a shutter longer than a frame",
        ),
        (
            effect("motion.blur", vec![("samples", n(1.0))]),
            "fewer than two samples",
        ),
        (
            effect("detail.denoise", vec![("temporal", n(1.0))]),
            "temporal is on or off",
        ),
        (
            effect("color.deflicker", vec![("window", n(17.0))]),
            "a window past 16 frames",
        ),
    ] {
        assert_eq!(with(bad), ["fosscut.model.effect_param"], "{what}");
    }
    // Strength and shutter animate; the temporal switch does not.
    let key = |frame: i64, value: f64| Keyframe {
        time: frames(frame),
        value: KeyValue::Number(value),
        interpolation: Interpolation::Linear,
    };
    let mut ramp = effect("detail.denoise", vec![]);
    ramp.keyframes
        .insert("strength".into(), vec![key(0, 0.0), key(30, 1.0)]);
    assert!(with(ramp.clone()).is_empty());
    assert_eq!(
        ramp.values_at(frames(15)).unwrap()["strength"],
        ParamValue::Number(0.5)
    );
    let mut switched = effect("detail.denoise", vec![]);
    switched
        .keyframes
        .insert("temporal".into(), vec![key(0, 0.0)]);
    assert_eq!(with(switched), ["fosscut.model.keyframe_path"]);
    let mut opening = effect("motion.blur", vec![]);
    opening
        .keyframes
        .insert("amount".into(), vec![key(0, 0.0), key(30, 2.0)]);
    assert_eq!(with(opening), ["fosscut.model.keyframe_value"]);
}
