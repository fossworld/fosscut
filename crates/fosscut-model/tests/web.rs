//! Web compositions in the project format (ADR-0039): web assets round-trip sparsely, video
//! clips show them, and validation names what is missing or misused.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use fosscut_model::{
    Asset, AssetKind, Clip, ClipContent, Id, IdKind, MediaInfo, MemoryStore, PathHints, Project,
    TimelineFormat, Track, TrackKind, VideoInfo, validate,
};
use fosscut_time::{Flicks, Rate};
use serde_json::json;

fn web_asset() -> Asset {
    Asset {
        id: Id::new(IdKind::Asset, 1),
        kind: AssetKind::Web,
        name: "promo".into(),
        path: PathHints {
            relative: Some("web/0123456789abcdef/index.html".into()),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            duration: Some(Flicks::from_seconds(6).unwrap()),
            container: Some("html".into()),
            video: Some(VideoInfo {
                width: 1080,
                height: 1920,
                rate: Some(Rate::FPS_30),
                codec: Some("html".into()),
                rotation: 0,
            }),
            ..MediaInfo::default()
        },
        folder: None,
    }
}

fn project(asset: Asset, content: ClipContent) -> Project {
    let format = TimelineFormat {
        width: 1080,
        height: 1920,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Ad", format);
    project.media.assets.push(asset);
    project.manifest.next_id = 10;
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
            clips: vec![Clip::new(
                Id::new(IdKind::Clip, 3),
                Flicks::ZERO,
                Flicks::from_seconds(2).unwrap(),
                content,
            )],
        });
    project
}

fn video() -> ClipContent {
    ClipContent::Video {
        asset: Id::new(IdKind::Asset, 1),
        source_in: Flicks::from_seconds(1).unwrap(),
    }
}

fn codes(project: &Project) -> Vec<String> {
    validate(project).into_iter().map(|i| i.code).collect()
}

#[test]
fn web_assets_round_trip_and_read_sparsely() {
    let asset = web_asset();
    let stored = serde_json::to_value(&asset).unwrap();
    assert_eq!(stored["kind"], "web");
    assert_eq!(stored["info"]["video"]["rate"], "30");
    assert!(stored["info"].get("audio").is_none(), "no empty fields");
    let back: Asset = serde_json::from_value(stored).unwrap();
    assert_eq!(back, asset);

    let project = project(asset, video());
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    let loaded = Project::load(&store).unwrap();
    assert_eq!(loaded.media, project.media);
    assert_eq!(loaded.timelines, project.timelines);
    assert!(AssetKind::Web.is_video() && AssetKind::Video.is_video());
    assert!(!AssetKind::Image.is_video());
}

#[test]
fn web_assets_need_a_size_and_length_and_show_in_video_clips_only() {
    let mut no_size = web_asset();
    no_size.info.video = None;
    assert!(codes(&project(no_size, video())).contains(&"fosscut.model.asset_web".to_string()));

    let mut no_length = web_asset();
    no_length.info.duration = None;
    assert!(
        codes(&project(no_length, video())).contains(&"fosscut.model.asset_duration".to_string())
    );

    let image = ClipContent::Image {
        asset: Id::new(IdKind::Asset, 1),
    };
    let codes = codes(&project(web_asset(), image));
    assert!(codes.iter().any(|c| c.contains("kind")), "{codes:?}");

    // An unknown kind from a newer FossCut fails to read instead of being guessed.
    assert!(serde_json::from_value::<AssetKind>(json!("hologram")).is_err());
}
