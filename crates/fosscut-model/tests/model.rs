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
        caption_style: None,
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
        caption_style: None,
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

fn primary(project: &mut Project) -> &mut Timeline {
    let id = project.manifest.primary_timeline.clone();
    project.timelines.get_mut(&id).unwrap()
}

/// Every rule in `validate`, each broken on its own, reports its code at the right path.
#[test]
fn each_validation_rule_reports_its_code() {
    type Breakage = fn(&mut Project);
    let cases: [(&str, &str, Breakage); 17] = [
        ("project", "fosscut.model.id_kind", |p| {
            p.manifest.id = Id::new(IdKind::Clip, 1);
        }),
        ("project", "fosscut.model.missing_timeline", |p| {
            p.manifest.primary_timeline = Id::new(IdKind::Timeline, 99);
        }),
        ("project", "fosscut.model.timeline_list", |p| {
            p.manifest.timelines.clear();
        }),
        ("asset_0001", "fosscut.model.asset_duration", |p| {
            p.media.assets[0].info.duration = None;
        }),
        ("timeline_0000", "fosscut.model.id_mismatch", |p| {
            primary(p).id = Id::new(IdKind::Timeline, 7);
        }),
        ("timeline_0000", "fosscut.model.canvas", |p| {
            primary(p).format.width = 0;
        }),
        (
            "timeline_0000/track_0002",
            "fosscut.model.duplicate_id",
            |p| {
                primary(p).tracks[1].id = primary(p).tracks[0].id.clone();
            },
        ),
        (
            "timeline_0000/track_0002/clip_0004",
            "fosscut.model.negative_start",
            |p| {
                primary(p).tracks[0].clips[0].start = frames(-1);
            },
        ),
        (
            "timeline_0000/track_0002/clip_0004",
            "fosscut.model.empty_clip",
            |p| {
                primary(p).tracks[0].clips[0].duration = Flicks::ZERO;
            },
        ),
        (
            "timeline_0000/track_0002/clip_0004",
            "fosscut.model.opacity",
            |p| {
                primary(p).tracks[0].clips[0].opacity = 1.5;
            },
        ),
        (
            "timeline_0000/track_0002/clip_0004",
            "fosscut.model.volume",
            |p| {
                primary(p).tracks[0].clips[0].volume = f64::NAN;
            },
        ),
        (
            "timeline_0000/track_0003/clip_0006",
            "fosscut.model.empty_text",
            |p| {
                primary(p).tracks[1].clips[0].content = ClipContent::Text(TextContent {
                    text: "  ".into(),
                    style: TextStyle::default(),
                });
            },
        ),
        (
            "timeline_0000/track_0002/clip_0004",
            "fosscut.model.asset_kind",
            |p| {
                p.media.assets[0].kind = AssetKind::Audio;
            },
        ),
        (
            "timeline_0000/track_0002/clip_0004",
            "fosscut.model.asset_kind",
            |p| {
                let asset = p.media.assets[0].id.clone();
                primary(p).tracks[0].clips[0].content = ClipContent::Image { asset };
            },
        ),
        (
            "timeline_0000/track_0002/clip_0004",
            "fosscut.model.source_range",
            |p| {
                let asset = p.media.assets[0].id.clone();
                primary(p).tracks[0].clips[0].content = ClipContent::Video {
                    asset,
                    source_in: frames(-1),
                };
            },
        ),
        (
            "timeline_0000/track_0002/clip_0004",
            "fosscut.model.track_kind",
            |p| {
                let asset = p.media.assets[0].id.clone();
                primary(p).tracks[0].clips[0].content = ClipContent::Audio {
                    asset,
                    source_in: Flicks::ZERO,
                };
            },
        ),
        (
            "timeline_0000/track_0006/clip_0007",
            "fosscut.model.asset_kind",
            |p| {
                p.media.assets[0].info.audio = None;
                let asset = p.media.assets[0].id.clone();
                primary(p).tracks.push(Track {
                    id: Id::new(IdKind::Track, 6),
                    kind: TrackKind::Audio,
                    name: None,
                    muted: false,
                    hidden: false,
                    locked: false,
                    caption_style: None,
                    clips: vec![Clip {
                        id: Id::new(IdKind::Clip, 7),
                        start: Flicks::ZERO,
                        duration: frames(30),
                        content: ClipContent::Audio {
                            asset,
                            source_in: Flicks::ZERO,
                        },
                        transform: Transform::default(),
                        opacity: 1.0,
                        volume: 1.0,
                    }],
                });
            },
        ),
    ];
    for (path, code, breakage) in cases {
        let mut project = round_one_project();
        breakage(&mut project);
        let issues = validate(&project);
        assert!(
            issues.iter().any(|i| i.code == code && i.path == path),
            "expected {code} at {path}, got {issues:?}"
        );
        let shown = issues.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert!(
            shown
                .iter()
                .any(|line| line.starts_with(path) && line.ends_with(&format!("({code})"))),
            "{shown:?}"
        );
    }
}

#[test]
fn timelines_find_tracks_and_clips() {
    let mut project = round_one_project();
    let timeline = primary(&mut project);
    let track = timeline.tracks[1].id.clone();
    let title = timeline.tracks[1].clips[0].id.clone();
    assert_eq!(
        timeline.track(&track).map(|t| t.kind),
        Some(TrackKind::Text)
    );
    assert_eq!(timeline.find_clip(&title).map(|(index, _)| index), Some(1));
    assert!(timeline.find_clip(&Id::new(IdKind::Clip, 99)).is_none());
    timeline.track_mut(&track).unwrap().muted = true;
    assert!(timeline.tracks[1].muted);
    assert!(timeline.track(&Id::new(IdKind::Track, 99)).is_none());
}

#[test]
fn colours_are_normalised_and_validated() {
    assert_eq!(Color::parse("#ff8800").unwrap().as_str(), "#FF8800");
    assert_eq!(Color::parse("#ff880080").unwrap().as_str(), "#FF880080");
    for bad in ["ff8800", "#ff88", "#gg8800", ""] {
        assert!(Color::parse(bad).is_err(), "{bad} should be rejected");
    }
    let json: Result<Color, _> = serde_json::from_str("\"red\"");
    assert!(json.is_err());
    assert_eq!(
        serde_json::to_string(&Color::parse("#00ff00").unwrap()).unwrap(),
        "\"#00FF00\""
    );
}

#[test]
fn directory_store_reports_missing_files_and_appends() {
    let root = std::env::temp_dir().join(format!("fosscut-store-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut store = DirStore::new(&root);
    assert_eq!(store.root(), root.as_path());
    assert!(!store.exists("history/ops.jsonl"));
    let missing = store.read("history/ops.jsonl").unwrap_err();
    assert!(matches!(missing, StoreError::NotFound(ref path) if path == "history/ops.jsonl"));
    store.append("history/ops.jsonl", b"one\n").unwrap();
    store.append("history/ops.jsonl", b"two\n").unwrap();
    assert_eq!(store.read("history/ops.jsonl").unwrap(), b"one\ntwo\n");
    assert!(store.exists("history/ops.jsonl"));
    std::fs::remove_dir_all(&root).unwrap();
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

/// The round 1 project with a caption track holding one styled page.
fn captioned_project() -> Project {
    let mut project = round_one_project();
    let track = project.allocate_id(IdKind::Track);
    let clip = project.allocate_id(IdKind::Clip);
    let timeline_id = project.manifest.primary_timeline.clone();
    let style = CaptionStyle {
        weight: FontWeight::Black,
        uppercase: true,
        animators: vec![
            Animator::WordHighlight(Animated {
                params: WordHighlight {
                    scale: 1.1,
                    ..WordHighlight::default()
                },
                ..Animated::default()
            }),
            Animator::PageFade(Animated::default()),
        ],
        ..CaptionStyle::default()
    };
    project
        .timelines
        .get_mut(&timeline_id)
        .unwrap()
        .tracks
        .push(Track {
            id: track,
            kind: TrackKind::Caption,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: Some(style),
            clips: vec![Clip {
                id: clip,
                start: frames(12),
                duration: frames(30),
                content: ClipContent::Caption(CaptionContent {
                    lines: vec![
                        CaptionLine {
                            words: vec![
                                CaptionWord {
                                    text: "This".into(),
                                    start: Flicks::ZERO,
                                    end: frames(6),
                                    emphasis: false,
                                },
                                CaptionWord {
                                    text: "changes".into(),
                                    start: frames(6),
                                    end: frames(15),
                                    emphasis: true,
                                },
                            ],
                        },
                        CaptionLine {
                            words: vec![CaptionWord {
                                text: "everything.".into(),
                                start: frames(16),
                                end: frames(28),
                                emphasis: false,
                            }],
                        },
                    ],
                }),
                transform: Transform::default(),
                opacity: 1.0,
                volume: 1.0,
            }],
        });
    project
}

fn caption_track(project: &mut Project) -> &mut Track {
    primary(project)
        .tracks
        .iter_mut()
        .find(|t| t.kind == TrackKind::Caption)
        .unwrap()
}

fn caption(project: &mut Project) -> &mut CaptionContent {
    match &mut caption_track(project).clips[0].content {
        ClipContent::Caption(caption) => caption,
        _ => unreachable!(),
    }
}

#[test]
fn captions_round_trip_sparsely() {
    let project = captioned_project();
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    assert_eq!(Project::load(&store).unwrap(), project);
    let timeline = &project.timelines[&project.manifest.primary_timeline];
    let json = serde_json::to_value(timeline).unwrap();
    let track = &json["tracks"][2];
    assert_eq!(track["kind"], "caption");
    assert_eq!(
        track["caption_style"],
        serde_json::json!({
            "weight": "black",
            "uppercase": true,
            "animators": [
                { "type": "word.highlight", "version": 1, "enabled": true,
                  "params": { "color": "#FFE14D", "scale": 1.1 } },
                { "type": "page.fade", "version": 1, "enabled": true }
            ]
        })
    );
    let page = &track["clips"][0];
    assert_eq!(page["kind"], "caption");
    assert_eq!(page["lines"][0]["words"][1]["emphasis"], true);
    assert!(page["lines"][0]["words"][0].get("emphasis").is_none());
    let content = match &timeline.tracks[2].clips[0].content {
        ClipContent::Caption(caption) => caption,
        _ => unreachable!(),
    };
    assert_eq!(content.text(), "This changes\neverything.");
    assert_eq!(content.words().count(), 3);
    assert!(timeline.tracks[2].clips[0].content.asset().is_none());
    assert_eq!(timeline.tracks[2].clips[0].content.kind_name(), "caption");
    assert_eq!(
        safe_margins(1080, 1920),
        [0.06, 0.10, 0.06, 0.22],
        "tall canvases keep clear of platform controls"
    );
    assert_eq!(safe_margins(1920, 1080), [0.05; 4]);
}

#[test]
fn older_formats_load_and_save_as_the_current_version() {
    let project = round_one_project();
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&store.read(MANIFEST_PATH).unwrap()).unwrap();
    assert_eq!(manifest["format_version"], FORMAT_VERSION);
    manifest["format_version"] = "0.1.0".into();
    store
        .write_atomic(MANIFEST_PATH, manifest.to_string().as_bytes())
        .unwrap();
    let loaded = Project::load(&store).unwrap();
    assert_eq!(loaded.manifest.format_version, "0.2.0");
    assert_eq!(loaded, project, "nothing else changes");
    assert!(READABLE_VERSIONS.contains(&"0.1.0"));
}

#[test]
fn caption_rules_report_their_codes() {
    let codes = |project: &Project| -> Vec<String> {
        validate(project).into_iter().map(|i| i.code).collect()
    };

    let mut unordered = captioned_project();
    caption(&mut unordered).lines[0].words[1].start = frames(3);
    assert_eq!(codes(&unordered), ["fosscut.model.caption_words"]);

    let mut outside = captioned_project();
    caption(&mut outside).lines[1].words[0].end = frames(31);
    assert_eq!(codes(&outside), ["fosscut.model.caption_words"]);

    let mut blank = captioned_project();
    caption(&mut blank).lines[1].words[0].text = "  ".into();
    assert_eq!(codes(&blank), ["fosscut.model.caption_words"]);

    let mut empty = captioned_project();
    caption(&mut empty).lines[1].words.clear();
    assert_eq!(codes(&empty), ["fosscut.model.caption_words"]);

    let mut wrong_track = captioned_project();
    caption_track(&mut wrong_track).kind = TrackKind::Text;
    assert_eq!(
        codes(&wrong_track),
        ["fosscut.model.caption_style", "fosscut.model.track_kind"]
    );

    let mut sizes = captioned_project();
    let style = caption_track(&mut sizes).caption_style.as_mut().unwrap();
    style.size = 0.0;
    style.stroke.width = -1.0;
    style.background.radius = f64::NAN;
    assert_eq!(codes(&sizes), ["fosscut.model.caption_style"; 3]);

    let mut versions = captioned_project();
    let style = caption_track(&mut versions).caption_style.as_mut().unwrap();
    style.animators = vec![
        Animator::WordPop(Animated {
            version: 2,
            ..Animated::default()
        }),
        Animator::PagePop(Animated {
            params: PagePop {
                from: 0.0,
                ..PagePop::default()
            },
            ..Animated::default()
        }),
    ];
    assert_eq!(
        codes(&versions),
        [
            "fosscut.model.animator_version",
            "fosscut.model.caption_style"
        ]
    );
    let issue = &validate(&versions)[0];
    assert!(issue.message.contains("word.pop version 2"), "{issue}");

    // Animators read sparsely and keep their type names.
    let sparse: Animator = serde_json::from_str(r#"{ "type": "word.karaoke" }"#).unwrap();
    assert_eq!(
        (sparse.type_name(), sparse.version(), sparse.enabled()),
        ("word.karaoke", 1, true)
    );
    for (name, _) in Animator::TYPES {
        let animator: Animator =
            serde_json::from_value(serde_json::json!({ "type": name, "enabled": false })).unwrap();
        assert_eq!(animator.type_name(), name);
        assert!(!animator.enabled());
    }
    assert!(serde_json::from_str::<Animator>(r#"{ "type": "word.wobble" }"#).is_err());
}
