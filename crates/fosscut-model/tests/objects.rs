//! 3D objects in the project format (ADR-0040): model assets and object clips round-trip
//! sparsely through a bundle, validation names what is missing or misused, and the
//! `model.*` properties are in the closed vocabulary.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use fosscut_model::{
    Asset, AssetKind, Clip, ClipContent, Id, IdKind, MediaInfo, MemoryStore, ModelContent,
    ModelInfo, PathHints, Project, TimelineFormat, Track, TrackKind, VideoInfo, validate,
};
use fosscut_time::{Flicks, Rate};
use serde_json::json;

fn model_asset() -> Asset {
    Asset {
        id: Id::new(IdKind::Asset, 1),
        kind: AssetKind::Model,
        name: "house.glb".into(),
        path: PathHints {
            relative: Some("models/0123456789abcdef/house.glb".into()),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            model: Some(ModelInfo {
                meshes: 2,
                triangles: 18,
                materials: 2,
                textures: 0,
                size: [1.1, 1.6, 1.1],
                generator: Some("a tool".into()),
                copyright: None,
            }),
            ..MediaInfo::default()
        },
        folder: None,
    }
}

fn image_asset() -> Asset {
    Asset {
        id: Id::new(IdKind::Asset, 2),
        kind: AssetKind::Image,
        name: "logo.png".into(),
        path: PathHints::default(),
        identity: None,
        info: MediaInfo {
            video: Some(VideoInfo {
                width: 256,
                height: 256,
                rate: None,
                codec: Some("png".into()),
                rotation: 0,
            }),
            ..MediaInfo::default()
        },
        folder: None,
    }
}

fn object(json: serde_json::Value) -> ClipContent {
    ClipContent::Model {
        model: Box::new(serde_json::from_value::<ModelContent>(json).unwrap()),
    }
}

fn project(assets: Vec<Asset>, kind: TrackKind, content: ClipContent) -> Project {
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Ad", format);
    project.media.assets = assets;
    project.manifest.next_id = 10;
    let timeline = project.manifest.primary_timeline.clone();
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 3),
            kind,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![Clip::new(
                Id::new(IdKind::Clip, 4),
                Flicks::ZERO,
                Flicks::from_seconds(2).unwrap(),
                content,
            )],
        });
    project
}

fn codes(project: &Project) -> Vec<String> {
    validate(project).into_iter().map(|i| i.code).collect()
}

#[test]
fn model_assets_and_object_clips_round_trip_sparsely() {
    let asset = model_asset();
    let stored = serde_json::to_value(&asset).unwrap();
    assert_eq!(stored["kind"], "model");
    assert_eq!(stored["info"]["model"]["triangles"], 18);
    assert!(stored["info"]["model"].get("copyright").is_none());
    assert_eq!(serde_json::from_value::<Asset>(stored).unwrap(), asset);

    let content = object(json!({
        "object": {"kind": "asset", "asset": "asset_0001"},
        "material": {"texture": "asset_0002", "roughness": 0.3},
        "floor": {}
    }));
    let project = project(
        vec![asset, image_asset()],
        TrackKind::Video,
        content.clone(),
    );
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    let loaded = Project::load(&store).unwrap();
    assert_eq!(loaded.media, project.media);
    assert_eq!(loaded.timelines, project.timelines);
    let clip = serde_json::to_value(&loaded.timelines.values().next().unwrap().tracks[0].clips[0])
        .unwrap();
    assert_eq!(
        clip["model"],
        json!({
            "object": {"kind": "asset", "asset": "asset_0001"},
            "material": {"texture": "asset_0002", "roughness": 0.3},
            "floor": {}
        })
    );
    // The model asset is the clip's asset; its texture is found through the object.
    assert_eq!(content.asset().unwrap().as_str(), "asset_0001");
    assert_eq!(content.kind_name(), "model");
}

#[test]
fn validation_names_what_is_missing_or_misused() {
    let solid = || object(json!({}));
    assert!(codes(&project(vec![], TrackKind::Video, solid())).is_empty());
    // Objects live on video tracks.
    let codes_audio = codes(&project(vec![], TrackKind::Audio, solid()));
    assert!(
        codes_audio.contains(&"fosscut.model.track_kind".to_string()),
        "{codes_audio:?}"
    );
    // Their assets must exist and be of the right kind.
    let missing = object(json!({"object": {"kind": "asset", "asset": "asset_0001"}}));
    assert!(
        codes(&project(vec![], TrackKind::Video, missing.clone()))
            .contains(&"fosscut.model.missing_asset".to_string())
    );
    let not_a_model = codes(&project(
        vec![image_asset()],
        TrackKind::Video,
        object(json!({"object": {"kind": "asset", "asset": "asset_0002"}})),
    ));
    assert!(
        not_a_model
            .iter()
            .any(|c| c == "fosscut.model.object" || c == "fosscut.model.asset_kind"),
        "{not_a_model:?}"
    );
    let texture = object(json!({"material": {"texture": "asset_0001"}}));
    assert!(
        codes(&project(vec![model_asset()], TrackKind::Video, texture))
            .contains(&"fosscut.model.object".to_string())
    );
    // A model asset needs what its file holds.
    let mut empty = model_asset();
    empty.info.model = None;
    assert!(
        codes(&project(vec![empty], TrackKind::Video, missing))
            .contains(&"fosscut.model.asset_model".to_string())
    );
    // Values out of range.
    let bad = object(json!({"light": {"intensity": 50}}));
    assert!(
        codes(&project(vec![], TrackKind::Video, bad))
            .contains(&"fosscut.model.object".to_string())
    );
    // An unknown object kind from a newer FossCut fails to read instead of being guessed.
    assert!(
        serde_json::from_value::<ModelContent>(json!({"object": {"kind": "hologram"}})).is_err()
    );
    assert!(serde_json::from_value::<AssetKind>(json!("hologram")).is_err());
}

#[test]
fn model_properties_are_in_the_vocabulary() {
    let properties = fosscut_model::clip_properties();
    for path in [
        "model.emissive_strength",
        "model.light.azimuth",
        "model.light.elevation",
        "model.light.intensity",
        "model.light.ambient",
        "model.shadow",
        "model.depth",
        "model.growth",
    ] {
        let property = properties.iter().find(|p| p.path == path).unwrap();
        assert!(property.animatable, "{path}");
        assert_eq!(property.applies, fosscut_model::Applies::Model, "{path}");
    }
    // A keyframed setting reads through the clip like any other.
    let content = object(json!({"object": {"kind": "blocks"}}));
    let mut clip = Clip::new(
        Id::new(IdKind::Clip, 1),
        Flicks::ZERO,
        Flicks::from_seconds(2).unwrap(),
        content,
    );
    clip.keyframes.insert(
        "model.growth".into(),
        vec![
            fosscut_model::Keyframe {
                time: Flicks::ZERO,
                value: fosscut_model::KeyValue::Number(0.0),
                interpolation: fosscut_model::Interpolation::default(),
            },
            fosscut_model::Keyframe {
                time: Flicks::from_seconds(2).unwrap(),
                value: fosscut_model::KeyValue::Number(1.0),
                interpolation: fosscut_model::Interpolation::default(),
            },
        ],
    );
    let halfway = clip.keyed("model.growth", Flicks::from_seconds(1).unwrap());
    assert_eq!(halfway, Some(fosscut_model::KeyValue::Number(0.5)));
}
