//! Clip properties of format 0.3.0: stretch, flips, crop, blend modes, sound shaping,
//! deactivated clips and keyframes, with their sparse JSON, evaluation and validation.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use fosscut_model::{
    BlendMode, ChannelMode, Clip, ClipContent, Crop, Id, IdKind, Interpolation, KeyValue, Keyframe,
    Project, TextContent, TextStyle, TimelineFormat, Track, TrackKind, validate,
};
use fosscut_time::{Flicks, Rate};
use serde_json::json;

fn frames(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

fn title() -> Clip {
    Clip::new(
        Id::new(IdKind::Clip, 2),
        Flicks::ZERO,
        frames(90),
        ClipContent::Text(TextContent {
            text: "Hello".into(),
            style: TextStyle::default(),
        }),
    )
}

fn key(frame: i64, value: KeyValue) -> Keyframe {
    Keyframe {
        time: frames(frame),
        value,
        interpolation: Interpolation::Linear,
    }
}

fn project_with(clip: Clip) -> Project {
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Props", format);
    let timeline = project.manifest.primary_timeline.clone();
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 1),
            kind: TrackKind::Text,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![clip],
        });
    project
}

fn codes(clip: Clip) -> Vec<String> {
    validate(&project_with(clip))
        .into_iter()
        .map(|i| i.code)
        .collect()
}

#[test]
fn new_properties_are_sparse_and_round_trip() {
    let plain = serde_json::to_value(title()).unwrap();
    for field in [
        "crop",
        "blend",
        "audio",
        "enabled",
        "keyframes",
        "transform",
    ] {
        assert!(
            plain.get(field).is_none(),
            "{field} is omitted at its default"
        );
    }
    let mut clip = title();
    clip.transform.stretch = [1.5, 1.0];
    clip.transform.flip_horizontal = true;
    clip.crop.left = 0.1;
    clip.blend = BlendMode::SoftLight;
    clip.enabled = false;
    clip.audio.fade_in = frames(15);
    clip.audio.channels = ChannelMode::Left;
    clip.keyframes.insert(
        "opacity".into(),
        vec![
            key(0, KeyValue::Number(0.0)),
            Keyframe {
                interpolation: Interpolation::EASE_OUT,
                ..key(30, KeyValue::Number(1.0))
            },
        ],
    );
    let value = serde_json::to_value(&clip).unwrap();
    assert_eq!(
        value["transform"],
        json!({ "stretch": [1.5, 1.0], "flip_horizontal": true })
    );
    assert_eq!(value["crop"], json!({ "left": 0.1 }));
    assert_eq!(value["blend"], "soft_light");
    assert_eq!(value["enabled"], false);
    assert_eq!(
        value["audio"],
        json!({ "fade_in": frames(15).get(), "channels": "left" })
    );
    assert_eq!(
        value["keyframes"]["opacity"][0],
        json!({ "time": 0, "value": 0.0 })
    );
    let back: Clip = serde_json::from_value(value).unwrap();
    assert_eq!(back, clip);
    assert!(validate(&project_with(clip)).is_empty());
}

#[test]
fn keyframed_values_replace_plain_ones() {
    let mut clip = title();
    clip.transform.scale = 2.0;
    clip.opacity = 0.3;
    clip.keyframes.insert(
        "transform.position".into(),
        vec![
            key(0, KeyValue::Pair([0.0, 0.0])),
            key(30, KeyValue::Pair([0.5, -0.5])),
        ],
    );
    clip.keyframes.insert(
        "opacity".into(),
        vec![
            key(10, KeyValue::Number(0.0)),
            key(20, KeyValue::Number(2.0)),
        ],
    );
    let t = clip.transform_at(frames(15));
    assert_eq!(t.position, [0.25, -0.25]);
    assert_eq!(t.scale, 2.0, "no keyframes: the plain value");
    assert_eq!(t.size_factors(), [2.0, 2.0]);
    assert_eq!(clip.opacity_at(frames(0)), 0.0, "held before the first key");
    assert_eq!(clip.opacity_at(frames(15)), 1.0, "clamped to 1");
    assert_eq!(clip.volume_at(frames(5)), 1.0);
    assert_eq!(clip.pan_at(frames(5)), 0.0);
    assert_eq!(clip.crop_at(frames(5)), Crop::default());
    clip.shift_keyframes(Flicks::ZERO - frames(10));
    assert_eq!(
        clip.opacity_at(frames(5)),
        1.0,
        "the animation moved 10 frames earlier"
    );
}

#[test]
fn fades_rise_and_fall_on_a_quarter_sine() {
    let mut clip = title();
    clip.audio.fade_in = frames(30);
    clip.audio.fade_out = frames(30);
    assert_eq!(clip.fade_at(Flicks::ZERO), 0.0);
    let half = clip.fade_at(frames(15));
    assert!(
        (half - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12,
        "{half}"
    );
    assert_eq!(clip.fade_at(frames(45)), 1.0);
    assert_eq!(clip.fade_at(frames(90)), 0.0, "silent at the end");
    clip.audio.fade_in = frames(80);
    assert!(
        clip.fade_at(frames(70)) < clip.fade_at(frames(60)),
        "overlapping: the lower wins"
    );
}

#[test]
fn crop_keeps_a_valid_region() {
    let crop = Crop {
        left: 0.2,
        right: 0.3,
        top: 0.1,
        bottom: 0.0,
    };
    assert_eq!(crop.kept(), [0.2, 0.1, 0.7, 1.0]);
    let wild = Crop {
        left: 0.9,
        right: 0.9,
        top: f64::NAN,
        bottom: 2.0,
    };
    let [l, t, r, b] = wild.kept();
    assert!(r >= l && b >= t, "never inside out");
    assert_eq!((t, b), (0.0, 0.0));
}

#[test]
fn each_property_rule_reports_its_code() {
    let mut bad = title();
    bad.transform.stretch = [0.0, 1.0];
    assert_eq!(codes(bad), ["fosscut.model.transform"]);
    let mut bad = title();
    bad.transform.rotation = f64::INFINITY;
    assert_eq!(codes(bad), ["fosscut.model.transform"]);
    let mut bad = title();
    bad.crop = Crop {
        left: 0.6,
        right: 0.5,
        ..Crop::default()
    };
    assert_eq!(codes(bad), ["fosscut.model.crop"]);
    let mut bad = title();
    bad.audio.fade_out = Flicks::ZERO - frames(1);
    assert_eq!(codes(bad), ["fosscut.model.fade"]);
    let mut bad = title();
    bad.audio.pan = 1.5;
    assert_eq!(codes(bad), ["fosscut.model.pan"]);

    let keyed = |path: &str, keys: Vec<Keyframe>| {
        let mut clip = title();
        clip.keyframes.insert(path.into(), keys);
        codes(clip)
    };
    assert_eq!(
        keyed("blend", vec![key(0, KeyValue::Number(1.0))]),
        ["fosscut.model.keyframe_path"]
    );
    assert_eq!(keyed("opacity", vec![]), ["fosscut.model.keyframes"]);
    assert_eq!(
        keyed(
            "opacity",
            vec![key(5, KeyValue::Number(1.0)), key(5, KeyValue::Number(0.0))]
        ),
        ["fosscut.model.keyframes"]
    );
    assert_eq!(
        keyed("transform.position", vec![key(0, KeyValue::Number(1.0))]),
        ["fosscut.model.keyframe_value"]
    );
    assert_eq!(
        keyed("opacity", vec![key(0, KeyValue::Number(1.5))]),
        ["fosscut.model.keyframe_value"]
    );
    assert_eq!(
        keyed(
            "transform.rotation",
            vec![Keyframe {
                interpolation: Interpolation::Bezier([1.5, 0.0, 0.5, 1.0]),
                ..key(0, KeyValue::Number(0.0))
            }]
        ),
        ["fosscut.model.keyframe_easing"]
    );
    assert!(
        keyed("transform.scale", vec![key(0, KeyValue::Number(0.0))]).is_empty(),
        "an animation may grow from nothing"
    );
}

#[test]
fn each_effect_rule_reports_its_code() {
    use fosscut_model::{AssetRef, Effect, ParamValue};
    let effect = |kind: &str, params: Vec<(&str, ParamValue)>| Effect {
        id: Id::new(IdKind::Effect, 9),
        effect_type: kind.into(),
        version: 1,
        enabled: true,
        params: params
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        keyframes: Default::default(),
    };
    let with = |effect: Effect| {
        let mut clip = title();
        clip.effects.push(effect);
        codes(clip)
    };
    let n = ParamValue::Number;
    assert!(with(effect("color.adjust", vec![("exposure", n(1.0))])).is_empty());
    assert_eq!(
        with(effect("color.glow", vec![])),
        ["fosscut.model.effect_type"]
    );
    let mut newer = effect("color.adjust", vec![]);
    newer.version = 2;
    assert_eq!(with(newer), ["fosscut.model.effect_version"]);
    let mut bad_id = effect("color.adjust", vec![]);
    bad_id.id = Id::new(IdKind::Clip, 9);
    assert_eq!(with(bad_id), ["fosscut.model.id_kind"]);
    for (case, params) in [
        ("unknown parameter", vec![("glow", n(1.0))]),
        ("out of range", vec![("exposure", n(9.0))]),
        ("wrong kind", vec![("exposure", ParamValue::Flag(true))]),
    ] {
        assert_eq!(
            with(effect("color.adjust", params)),
            ["fosscut.model.effect_param"],
            "{case}"
        );
    }
    assert_eq!(
        with(effect(
            "color.wheels",
            vec![("lift", ParamValue::Pair([2.0, 0.0]))]
        )),
        ["fosscut.model.effect_param"]
    );
    assert_eq!(
        with(effect(
            "color.curves",
            vec![("red", ParamValue::Points(vec![[0.5, 0.0], [0.4, 1.0]]))]
        )),
        ["fosscut.model.effect_param"],
        "points out of order"
    );
    assert_eq!(
        with(effect("color.lut", vec![])),
        ["fosscut.model.effect_param"],
        "a LUT effect needs its LUT"
    );
    assert_eq!(
        with(effect(
            "color.lut",
            vec![(
                "lut",
                ParamValue::Asset(AssetRef {
                    asset: Id::new(IdKind::Asset, 4)
                })
            )]
        )),
        ["fosscut.model.effect_param"],
        "a missing asset"
    );
    let mut keyed = effect("color.adjust", vec![]);
    keyed
        .keyframes
        .insert("exposure".into(), vec![key(0, KeyValue::Number(9.0))]);
    keyed.keyframes.insert(
        "contrast".into(),
        vec![key(3, KeyValue::Number(0.0)), key(3, KeyValue::Number(0.1))],
    );
    let mut keyed_codes = with(keyed);
    keyed_codes.sort();
    assert_eq!(
        keyed_codes,
        ["fosscut.model.keyframe_value", "fosscut.model.keyframes"]
    );
    let mut curve_keys = effect("color.curves", vec![]);
    curve_keys
        .keyframes
        .insert("red".into(), vec![key(0, KeyValue::Number(0.5))]);
    assert_eq!(with(curve_keys), ["fosscut.model.keyframe_path"]);
    let mut sound = Clip::new(
        Id::new(IdKind::Clip, 3),
        Flicks::ZERO,
        frames(10),
        ClipContent::Text(TextContent {
            text: "x".into(),
            style: TextStyle::default(),
        }),
    );
    sound.effects.push(effect("color.adjust", vec![]));
    sound.effects.push(effect("color.adjust", vec![]));
    assert_eq!(codes(sound), ["fosscut.model.duplicate_id"]);
}
