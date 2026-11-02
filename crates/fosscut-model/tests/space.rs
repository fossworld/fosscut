//! Clips placed in 3D and camera clips (ADR-0037): sparse JSON that older projects read
//! unchanged, keyframed evaluation, and validation.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use fosscut_model::{
    CameraContent, Clip, ClipContent, Effect, Id, IdKind, Interpolation, KeyValue, Keyframe,
    Project, TimelineFormat, Track, TrackKind, Transform, validate,
};
use fosscut_time::{Flicks, Rate};
use serde_json::json;

fn frames(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

fn project_with(kind: TrackKind, clip: Clip) -> Project {
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Space", format);
    let timeline = project.manifest.primary_timeline.clone();
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 1),
            kind,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![clip],
        });
    project
}

fn camera(content: CameraContent) -> Clip {
    Clip::new(
        Id::new(IdKind::Clip, 2),
        Flicks::ZERO,
        frames(60),
        ClipContent::Camera(Box::new(content)),
    )
}

fn codes(project: &Project) -> Vec<String> {
    validate(project).into_iter().map(|i| i.code).collect()
}

#[test]
fn transforms_in_3d_are_sparse_and_older_clips_read_unchanged() {
    // A clip written before 3D placement reads with the 2D defaults.
    let old: Transform = serde_json::from_value(json!({ "position": [0.1, 0.2] })).unwrap();
    assert!(!old.three_d && old.position_z == 0.0 && old.anchor == [0.0, 0.0]);
    assert!(!old.is_spatial());
    assert_eq!(
        serde_json::to_value(Transform::default()).unwrap(),
        json!({}),
        "defaults are omitted"
    );
    let placed = Transform {
        three_d: true,
        position_z: 0.5,
        rotation_x: 10.0,
        rotation_y: -20.0,
        anchor: [-0.5, 0.0],
        ..Transform::default()
    };
    let value = serde_json::to_value(&placed).unwrap();
    assert_eq!(
        value,
        json!({
            "three_d": true,
            "position_z": 0.5,
            "rotation_x": 10.0,
            "rotation_y": -20.0,
            "anchor": [-0.5, 0.0]
        })
    );
    assert_eq!(serde_json::from_value::<Transform>(value).unwrap(), placed);
    for spatial in [
        Transform {
            rotation_y: 1.0,
            ..Transform::default()
        },
        Transform {
            anchor: [0.1, 0.0],
            ..Transform::default()
        },
        Transform {
            three_d: true,
            ..Transform::default()
        },
    ] {
        assert!(spatial.is_spatial());
    }
}

#[test]
fn keyframes_move_clips_in_depth() {
    let mut clip = Clip::new(
        Id::new(IdKind::Clip, 3),
        Flicks::ZERO,
        frames(60),
        ClipContent::Adjustment,
    );
    let key = |frame: i64, value: KeyValue| Keyframe {
        time: frames(frame),
        value,
        interpolation: Interpolation::Linear,
    };
    clip.keyframes.insert(
        "transform.rotation_y".into(),
        vec![
            key(0, KeyValue::Number(-90.0)),
            key(30, KeyValue::Number(0.0)),
        ],
    );
    clip.keyframes.insert(
        "transform.anchor".into(),
        vec![key(0, KeyValue::Pair([-0.5, 0.0]))],
    );
    clip.transform.position_z = 0.25;
    let at = clip.transform_at(frames(15));
    assert!((at.rotation_y + 45.0).abs() < 1e-9);
    assert_eq!(at.anchor, [-0.5, 0.0]);
    assert_eq!(at.position_z, 0.25, "plain values stay without keyframes");
}

#[test]
fn camera_clips_round_trip_and_validate() {
    let content = CameraContent {
        field_of_view: 55.0,
        position_z: 0.3,
        orbit: [20.0, 5.0],
        depth_of_field: true,
        ..CameraContent::default()
    };
    let project = project_with(TrackKind::Video, camera(content.clone()));
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let timeline = &project.timelines[&project.manifest.primary_timeline];
    let json = serde_json::to_value(timeline).unwrap();
    let stored = &json["tracks"][0]["clips"][0];
    assert_eq!(stored["kind"], "camera");
    assert_eq!(stored["field_of_view"], 55.0);
    assert!(stored.get("transform").is_none() && stored.get("aperture").is_none());
    let back: fosscut_model::Timeline = serde_json::from_value(json).unwrap();
    assert_eq!(&back, timeline);

    // Only on video tracks.
    let audio = project_with(TrackKind::Audio, camera(CameraContent::default()));
    assert!(codes(&audio).contains(&"fosscut.model.track_kind".to_string()));
    // Values in range.
    let wide = project_with(
        TrackKind::Video,
        camera(CameraContent {
            field_of_view: 175.0,
            ..CameraContent::default()
        }),
    );
    assert_eq!(codes(&wide), ["fosscut.model.camera"]);
    // No picture for effects or masks, and no speed.
    let mut extra = camera(CameraContent::default());
    extra.masks.push(fosscut_model::Mask::new(
        Id::new(IdKind::Mask, 4),
        fosscut_model::MaskShape::Ellipse,
    ));
    let masked = project_with(TrackKind::Video, extra);
    assert_eq!(codes(&masked), ["fosscut.model.camera"]);
    let mut graded = camera(CameraContent::default());
    graded.effects.push(Effect {
        id: Id::new(IdKind::Effect, 5),
        effect_type: "color.adjust".into(),
        version: 1,
        enabled: true,
        params: Default::default(),
        keyframes: Default::default(),
    });
    let graded = project_with(TrackKind::Video, graded);
    assert!(codes(&graded).contains(&"fosscut.model.effect_target".to_string()));
    // Camera keyframes are checked against their ranges.
    let mut keyed = camera(CameraContent::default());
    keyed.keyframes.insert(
        "camera.aperture".into(),
        vec![Keyframe {
            time: Flicks::ZERO,
            value: KeyValue::Number(2.0),
            interpolation: Interpolation::Linear,
        }],
    );
    let keyed = project_with(TrackKind::Video, keyed);
    assert_eq!(codes(&keyed), ["fosscut.model.keyframe_value"]);
    // 3D placement must be finite.
    let mut lost = Clip::new(
        Id::new(IdKind::Clip, 6),
        Flicks::ZERO,
        frames(30),
        ClipContent::Adjustment,
    );
    lost.transform.rotation_x = f64::NAN;
    let lost = project_with(TrackKind::Video, lost);
    assert!(codes(&lost).contains(&"fosscut.model.transform".to_string()));
}
