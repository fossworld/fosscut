//! Format tests for ADR-0004: round trips, validation rules and document size.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use fosscut_model::*;
use fosscut_time::{Flicks, Rate};
use proptest::prelude::*;

fn frames(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

/// The round 1 audit edit: an 8 s clip split at frame 118 and a 3 s title.
fn round_one_project() -> Project {
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 0x2f3a), "FossCut audit", format);
    let asset = project.allocate_id(IdKind::Asset);
    project.media.assets.push(Asset {
        id: asset.clone(),
        kind: AssetKind::Video,
        name: "test-pattern.mp4".into(),
        path: PathHints {
            relative: Some("../media/test-pattern.mp4".into()),
            absolute: None,
        },
        identity: Some(ContentIdentity {
            size: 1_234_567,
            sample_blake3: "ab".repeat(32),
        }),
        info: MediaInfo {
            duration: Some(frames(240)),
            container: Some("mov,mp4,m4a,3gp,3g2,mj2".into()),
            video: Some(VideoInfo {
                width: 1280,
                height: 720,
                rate: Some(Rate::FPS_30),
                codec: Some("h264".into()),
                rotation: 0,
            }),
            audio: Some(AudioInfo {
                sample_rate: 48_000,
                channels: 1,
                codec: Some("aac".into()),
            }),
        },
    });
    let video_track = project.allocate_id(IdKind::Track);
    let text_track = project.allocate_id(IdKind::Track);
    let clip_a = project.allocate_id(IdKind::Clip);
    let clip_b = project.allocate_id(IdKind::Clip);
    let title = project.allocate_id(IdKind::Clip);
    let timeline_id = project.manifest.primary_timeline.clone();
    let timeline = project.timelines.get_mut(&timeline_id).unwrap();
    timeline.tracks.push(Track {
        id: video_track,
        kind: TrackKind::Video,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        clips: vec![
            Clip {
                id: clip_a,
                start: Flicks::ZERO,
                duration: frames(118),
                content: ClipContent::Video {
                    asset: asset.clone(),
                    source_in: Flicks::ZERO,
                },
                transform: Transform::default(),
                opacity: 1.0,
                volume: 1.0,
            },
            Clip {
                id: clip_b,
                start: frames(118),
                duration: frames(122),
                content: ClipContent::Video {
                    asset,
                    source_in: frames(118),
                },
                transform: Transform::default(),
                opacity: 1.0,
                volume: 1.0,
            },
        ],
    });
    timeline.tracks.push(Track {
        id: text_track,
        kind: TrackKind::Text,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        clips: vec![Clip {
            id: title,
            start: Flicks::ZERO,
            duration: frames(90),
            content: ClipContent::Text(TextContent {
                text: "FossCut audit".into(),
                style: TextStyle::default(),
            }),
            transform: Transform::default(),
            opacity: 1.0,
            volume: 1.0,
        }],
    });
    project
}

fn first_clips(project: &mut Project) -> &mut Vec<Clip> {
    let id = project.manifest.primary_timeline.clone();
    &mut project.timelines.get_mut(&id).unwrap().tracks[0].clips
}

#[test]
fn round_one_project_is_valid_and_round_trips() {
    let project = round_one_project();
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    assert_eq!(
        store.paths(),
        vec!["media.json", "project.json", "timelines/timeline_0000.json"]
    );
    assert_eq!(Project::load(&store).unwrap(), project);
}

#[test]
fn round_one_project_is_under_two_kilobytes() {
    let project = round_one_project();
    let size: usize = project
        .document_keys()
        .iter()
        .map(|key| {
            serde_json::to_vec(&project.document_value(key).unwrap())
                .unwrap()
                .len()
        })
        .sum();
    assert!(size < 2048, "compact project size is {size} bytes");
}

#[test]
fn defaults_are_omitted() {
    let project = round_one_project();
    let id = project.manifest.primary_timeline.clone();
    let json = serde_json::to_string(&project.timelines[&id]).unwrap();
    for omitted in [
        "transform",
        "opacity",
        "volume",
        "muted",
        "\"source_in\":0",
        "sample_rate",
    ] {
        assert!(
            !json.contains(omitted),
            "{omitted} should be omitted: {json}"
        );
    }
}

#[test]
fn directory_store_round_trips() {
    let root = std::env::temp_dir().join(format!("fosscut-model-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = round_one_project();
    let mut store = DirStore::new(root.join("Audit.fosscut"));
    project.save_all(&mut store).unwrap();
    assert_eq!(Project::load(&store).unwrap(), project);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn overlapping_clips_are_rejected() {
    let mut project = round_one_project();
    first_clips(&mut project)[1].start = frames(100);
    let codes: Vec<String> = validate(&project).into_iter().map(|i| i.code).collect();
    assert!(
        codes.contains(&"fosscut.model.overlap".to_string()),
        "{codes:?}"
    );
}

#[test]
fn reading_past_the_asset_end_is_rejected() {
    let mut project = round_one_project();
    first_clips(&mut project)[1].duration = frames(200);
    let codes: Vec<String> = validate(&project).into_iter().map(|i| i.code).collect();
    assert!(
        codes.contains(&"fosscut.model.source_range".to_string()),
        "{codes:?}"
    );
}

#[test]
fn clips_must_match_their_track_kind() {
    let mut project = round_one_project();
    let id = project.manifest.primary_timeline.clone();
    let timeline = project.timelines.get_mut(&id).unwrap();
    let title = timeline.tracks[1].clips.remove(0);
    timeline.tracks[0].clips.clear();
    timeline.tracks[0].clips.push(title);
    let codes: Vec<String> = validate(&project).into_iter().map(|i| i.code).collect();
    assert!(
        codes.contains(&"fosscut.model.track_kind".to_string()),
        "{codes:?}"
    );
}

#[test]
fn missing_assets_and_duplicate_ids_are_rejected() {
    let mut project = round_one_project();
    project.media.assets.clear();
    let clips = first_clips(&mut project);
    clips[1].id = clips[0].id.clone();
    let codes: Vec<String> = validate(&project).into_iter().map(|i| i.code).collect();
    assert!(
        codes.contains(&"fosscut.model.missing_asset".to_string()),
        "{codes:?}"
    );
    assert!(
        codes.contains(&"fosscut.model.duplicate_id".to_string()),
        "{codes:?}"
    );
}

#[test]
fn newer_formats_are_refused() {
    let project = round_one_project();
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&store.read(MANIFEST_PATH).unwrap()).unwrap();
    manifest["format_version"] = "9.0.0".into();
    store
        .write_atomic(MANIFEST_PATH, manifest.to_string().as_bytes())
        .unwrap();
    let error = Project::load(&store).unwrap_err();
    assert_eq!(error.code(), "fosscut.project.unsupported_version");
}

#[test]
fn document_keys_parse_their_paths() {
    let key = DocumentKey::Timeline(Id::new(IdKind::Timeline, 3));
    assert_eq!(DocumentKey::from_path(&key.path()), Some(key));
    assert_eq!(
        DocumentKey::from_path("project.json"),
        Some(DocumentKey::Manifest)
    );
    assert_eq!(DocumentKey::from_path("other.json"), None);
}

proptest! {
    #[test]
    fn generated_timelines_round_trip(lengths in proptest::collection::vec((1i64..300, 0i64..30), 1..20)) {
        let mut project = round_one_project();
        let mut clips = Vec::new();
        let mut position = 0;
        for (length, gap) in lengths {
            position += gap;
            let id = project.allocate_id(IdKind::Clip);
            clips.push(Clip {
                id,
                start: frames(position),
                duration: frames(length),
                content: ClipContent::Text(TextContent { text: format!("clip {position}"), style: TextStyle::default() }),
                transform: Transform { position: [0.1, -0.2], scale: 1.5, rotation: 12.0 },
                opacity: 0.5,
                volume: 1.0,
            });
            position += length;
        }
        let id = project.manifest.primary_timeline.clone();
        project.timelines.get_mut(&id).unwrap().tracks[1].clips = clips;
        prop_assert!(validate(&project).is_empty());
        let mut store = MemoryStore::new();
        project.save_all(&mut store).unwrap();
        prop_assert_eq!(Project::load(&store).unwrap(), project);
    }
}
