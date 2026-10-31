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
        folder: None,
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
            lut: None,
            ..Default::default()
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
            Clip::new(
                clip_a,
                Flicks::ZERO,
                frames(118),
                ClipContent::Video {
                    asset: asset.clone(),
                    source_in: Flicks::ZERO,
                },
            ),
            Clip::new(
                clip_b,
                frames(118),
                frames(122),
                ClipContent::Video {
                    asset,
                    source_in: frames(118),
                },
            ),
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
        clips: vec![Clip::new(
            title,
            Flicks::ZERO,
            frames(90),
            ClipContent::Text(Box::new(TextContent {
                text: "FossCut audit".into(),
                style: TextStyle::default(),
                ..Default::default()
            })),
        )],
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
                primary(p).tracks[1].clips[0].content = ClipContent::Text(Box::new(TextContent {
                    text: "  ".into(),
                    style: TextStyle::default(),
                    ..Default::default()
                }));
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
                    clips: vec![Clip::new(
                        Id::new(IdKind::Clip, 7),
                        Flicks::ZERO,
                        frames(30),
                        ClipContent::Audio {
                            asset,
                            source_in: Flicks::ZERO,
                        },
                    )],
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
                transform: Transform { position: [0.1, -0.2], scale: 1.5, rotation: 12.0, ..Transform::default() },
                opacity: 0.5,
                ..Clip::new(id, frames(position), frames(length), ClipContent::Text(Box::new(TextContent { text: format!("clip {position}"), style: TextStyle::default() , ..Default::default() })))
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
            clips: vec![Clip::new(
                clip,
                frames(12),
                frames(30),
                ClipContent::Caption(CaptionContent {
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
                    ..Default::default()
                }),
            )],
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
    for older in [
        "0.1.0", "0.2.0", "0.3.0", "0.4.0", "0.5.0", "0.6.0", "0.7.0", "0.8.0", "0.9.0", "0.10.0",
        "0.11.0", "0.12.0", "0.13.0", "0.14.0",
    ] {
        manifest["format_version"] = older.into();
        store
            .write_atomic(MANIFEST_PATH, manifest.to_string().as_bytes())
            .unwrap();
        let loaded = Project::load(&store).unwrap();
        assert_eq!(loaded.manifest.format_version, FORMAT_VERSION, "{older}");
        assert_eq!(loaded, project, "nothing else changes in {older}");
        assert!(READABLE_VERSIONS.contains(&older));
    }
    assert_eq!(FORMAT_VERSION, "0.15.0");
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

#[test]
fn transitions_and_animations_round_trip_and_are_checked() {
    let mut project = round_one_project();
    let clip = &mut first_clips(&mut project)[0];
    let mut params = std::collections::BTreeMap::new();
    params.insert("direction".to_string(), ParamValue::Name("up".into()));
    clip.transition = Some(Transition {
        transition_type: "push".into(),
        duration: frames(12),
        params,
    });
    clip.animations = ClipAnimations {
        entrance: Some(Animation {
            preset: "pop".into(),
            duration: frames(15),
        }),
        combo: Some(Animation {
            preset: "swing".into(),
            duration: frames(30),
        }),
        ..Default::default()
    };
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    assert_eq!(Project::load(&store).unwrap(), project);
    let id = project.manifest.primary_timeline.clone();
    let json = serde_json::to_value(&project.timelines[&id]).unwrap();
    let saved = &json["tracks"][0]["clips"][0];
    assert_eq!(saved["transition"]["type"], "push");
    assert_eq!(saved["animations"]["in"]["preset"], "pop");
    assert!(saved["animations"].get("out").is_none(), "{saved}");
    assert!(
        json["tracks"][0]["clips"][1].get("animations").is_none(),
        "clips without animations store none"
    );

    type Breakage = fn(&mut Clip);
    let cases: [(&str, &str, Breakage); 7] = [
        ("unknown type", "fosscut.model.transition", |c| {
            c.transition.as_mut().unwrap().transition_type = "morph".into();
        }),
        ("bad choice", "fosscut.model.transition", |c| {
            c.transition
                .as_mut()
                .unwrap()
                .params
                .insert("direction".into(), ParamValue::Name("sideways".into()));
        }),
        ("unknown parameter", "fosscut.model.transition", |c| {
            c.transition
                .as_mut()
                .unwrap()
                .params
                .insert("speed".into(), ParamValue::Number(1.0));
        }),
        ("longer than the clip", "fosscut.model.transition", |c| {
            c.transition.as_mut().unwrap().duration = c.duration + frames(1);
        }),
        ("a combo as an entrance", "fosscut.model.animation", |c| {
            c.animations.entrance.as_mut().unwrap().preset = "pulse".into();
        }),
        ("an entrance as a combo", "fosscut.model.animation", |c| {
            c.animations.combo.as_mut().unwrap().preset = "fade".into();
        }),
        ("no time", "fosscut.model.animation", |c| {
            c.animations.entrance.as_mut().unwrap().duration = Flicks::ZERO;
        }),
    ];
    for (case, code, breakage) in cases {
        let mut broken = project.clone();
        breakage(&mut first_clips(&mut broken)[0]);
        let codes: Vec<String> = validate(&broken).into_iter().map(|i| i.code).collect();
        assert_eq!(codes, vec![code.to_string()], "{case}");
    }
}

#[test]
fn speed_round_trips_and_is_checked() {
    let mut project = round_one_project();
    let clip = &mut first_clips(&mut project)[0];
    clip.speed = Some(Speed {
        curve: vec![
            SpeedPoint { at: 0.0, rate: 0.5 },
            SpeedPoint { at: 1.0, rate: 1.5 },
        ],
        pitch: Pitch::Shift,
        ..Speed::default()
    });
    assert_eq!(clip.source_length(), clip.duration, "a mean rate of 1");
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    assert_eq!(Project::load(&store).unwrap(), project);
    let id = project.manifest.primary_timeline.clone();
    let json = serde_json::to_value(&project.timelines[&id]).unwrap();
    let saved = &json["tracks"][0]["clips"][0]["speed"];
    assert_eq!(saved["pitch"], "shift");
    assert!(
        saved.get("rate").is_none() && saved.get("reverse").is_none(),
        "{saved}"
    );

    type Breakage = fn(&mut Clip);
    let cases: [(&str, &str, Breakage); 7] = [
        ("too slow", "fosscut.model.speed", |c| {
            c.speed = Some(Speed::constant(0.01));
        }),
        ("too fast", "fosscut.model.speed", |c| {
            c.speed = Some(Speed::constant(101.0));
        }),
        ("rate and curve", "fosscut.model.speed", |c| {
            c.speed.as_mut().unwrap().rate = 2.0;
        }),
        ("one point", "fosscut.model.speed", |c| {
            c.speed.as_mut().unwrap().curve.truncate(1);
        }),
        ("out of order", "fosscut.model.speed", |c| {
            c.speed.as_mut().unwrap().curve.reverse();
        }),
        ("on a text clip", "fosscut.model.speed", |c| {
            c.content = ClipContent::Text(Box::new(TextContent {
                text: "Hi".into(),
                style: TextStyle::default(),
                ..Default::default()
            }));
        }),
        (
            "more source than the media has",
            "fosscut.model.source_range",
            |c| {
                c.speed = Some(Speed::constant(50.0));
            },
        ),
    ];
    for (case, code, breakage) in cases {
        let mut broken = project.clone();
        breakage(&mut first_clips(&mut broken)[0]);
        let codes: Vec<String> = validate(&broken).into_iter().map(|i| i.code).collect();
        assert!(codes.contains(&code.to_string()), "{case}: {codes:?}");
    }
    // A freeze uses no source; reversing uses the same range.
    let mut frozen = project.clone();
    first_clips(&mut frozen)[0].speed = Some(Speed::constant(0.0));
    assert!(validate(&frozen).is_empty());
    assert_eq!(first_clips(&mut frozen)[0].source_length(), Flicks::ZERO);
}

#[test]
fn clips_map_source_through_speed_and_split_into_parts() {
    let mut clip = Clip::new(
        Id::new(IdKind::Clip, 1),
        Flicks::ZERO,
        frames(60),
        ClipContent::Video {
            asset: Id::new(IdKind::Asset, 2),
            source_in: frames(30),
        },
    );
    assert_eq!(clip.source_at(frames(10)), Some(frames(40)));
    assert_eq!(clip.offset_of_source(frames(40)), Some(frames(10)));
    assert_eq!(
        clip.offset_of_source(frames(10)),
        None,
        "before the clip's source"
    );
    clip.speed = Some(Speed::constant(2.0));
    assert_eq!(clip.source_length(), frames(120));
    assert_eq!(clip.source_at(frames(10)), Some(frames(50)));
    assert_eq!(clip.offset_of_source(frames(50)), Some(frames(10)));
    let mut right = clip.clone();
    right.retime_part(frames(20), frames(40));
    assert_eq!(right.content.source_in(), Some(frames(70)));
    clip.speed.as_mut().unwrap().reverse = true;
    assert_eq!(
        clip.source_at(Flicks::ZERO),
        Some(frames(150)),
        "starts at the end"
    );
    let mut back = clip.clone();
    back.retime_part(frames(20), frames(40));
    assert_eq!(
        back.content.source_in(),
        Some(frames(30)),
        "the start of the source"
    );
    let mut plain = Clip::new(
        Id::new(IdKind::Clip, 3),
        Flicks::ZERO,
        frames(60),
        ClipContent::Video {
            asset: Id::new(IdKind::Asset, 2),
            source_in: Flicks::ZERO,
        },
    );
    plain.retime_part(frames(-5), frames(70));
    assert_eq!(plain.content.source_in(), Some(frames(-5)));
    assert!(plain.speed.is_none());
    let mut text = Clip::new(
        Id::new(IdKind::Clip, 4),
        Flicks::ZERO,
        frames(60),
        ClipContent::Text(Box::new(TextContent {
            text: "Hi".into(),
            style: TextStyle::default(),
            ..Default::default()
        })),
    );
    text.retime_part(frames(5), frames(10));
    assert_eq!(text.source_at(Flicks::ZERO), None);
}

#[test]
fn markers_groups_and_detached_sound_round_trip_and_are_checked() {
    let mut project = round_one_project();
    let timeline = project.manifest.primary_timeline.clone();
    let group = Id::new(IdKind::Group, 90);
    {
        let doc = primary(&mut project);
        doc.markers.push(Marker {
            id: Id::new(IdKind::Marker, 91),
            at: frames(15),
            name: Some("drop".into()),
            color: Some(Color::parse("#ff8800").unwrap()),
        });
        for clip in doc.tracks.iter_mut().flat_map(|t| &mut t.clips).take(2) {
            clip.group = Some(group.clone());
        }
        doc.tracks[0].clips[0].audio.detached = true;
    }
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    assert_eq!(Project::load(&store).unwrap(), project);
    let json = serde_json::to_value(&project.timelines[&timeline]).unwrap();
    assert_eq!(json["markers"][0]["color"], "#FF8800");
    assert_eq!(json["tracks"][0]["clips"][0]["audio"]["detached"], true);
    let plain = round_one_project();
    let id = plain.manifest.primary_timeline.clone();
    let plain = serde_json::to_value(&plain.timelines[&id]).unwrap();
    assert!(plain.get("markers").is_none(), "no markers, no key");

    type Breakage = fn(&mut Project);
    let cases: [(&str, &str, Breakage); 5] = [
        ("a group of one", "fosscut.model.group", |p| {
            primary(p).tracks[0].clips[0].group = None;
        }),
        ("a marker before zero", "fosscut.model.marker", |p| {
            primary(p).markers[0].at = frames(-1);
        }),
        ("a marker with a clip id", "fosscut.model.id_kind", |p| {
            primary(p).markers[0].id = Id::new(IdKind::Clip, 92);
        }),
        ("a group with a mask id", "fosscut.model.id_kind", |p| {
            for clip in primary(p).tracks.iter_mut().flat_map(|t| &mut t.clips) {
                if clip.group.is_some() {
                    clip.group = Some(Id::new(IdKind::Mask, 93));
                }
            }
        }),
        (
            "detached sound on a text clip",
            "fosscut.model.detached",
            |p| {
                let doc = primary(p);
                let text = doc
                    .tracks
                    .iter_mut()
                    .flat_map(|t| &mut t.clips)
                    .find(|c| matches!(c.content, ClipContent::Text(_)))
                    .unwrap();
                text.audio.detached = true;
            },
        ),
    ];
    for (case, code, breakage) in cases {
        let mut broken = project.clone();
        breakage(&mut broken);
        let codes: Vec<String> = validate(&broken).into_iter().map(|i| i.code).collect();
        assert!(codes.contains(&code.to_string()), "{case}: {codes:?}");
    }
}

#[test]
fn media_folders_round_trip_and_are_checked() {
    let mut project = round_one_project();
    project.media.folders = vec!["B-roll".into(), "B-roll/Day 1".into()];
    project.media.assets[0].folder = Some("B-roll/Day 1".into());
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    assert_eq!(Project::load(&store).unwrap(), project);
    let json = serde_json::to_value(&project.media).unwrap();
    assert_eq!(json["assets"][0]["folder"], "B-roll/Day 1");
    assert!(
        serde_json::to_value(&round_one_project().media)
            .unwrap()
            .get("folders")
            .is_none()
    );
    assert_eq!(
        folder_and_parents("a/b/c"),
        vec!["a".to_string(), "a/b".into(), "a/b/c".into()]
    );
    for bad in ["", "a//b", " a", "a/", "/a"] {
        assert!(!valid_folder(bad), "{bad:?}");
    }
    type Breakage = fn(&mut Project);
    let cases: [(&str, Breakage); 3] = [
        ("a missing parent", |p| {
            p.media.folders = vec!["B-roll/Day 1".into()]
        }),
        ("out of order", |p| {
            p.media.folders.reverse();
        }),
        ("an unlisted folder", |p| {
            p.media.assets[0].folder = Some("Elsewhere".into());
        }),
    ];
    for (case, breakage) in cases {
        let mut broken = project.clone();
        breakage(&mut broken);
        let codes: Vec<String> = validate(&broken).into_iter().map(|i| i.code).collect();
        assert!(
            codes.contains(&"fosscut.model.folder".to_string()),
            "{case}: {codes:?}"
        );
    }
}

#[test]
fn removed_timelines_lose_their_documents() {
    let mut project = round_one_project();
    let extra = Id::new(IdKind::Timeline, 77);
    let mut copy = project.primary_timeline().unwrap().clone();
    copy.id = extra.clone();
    project.manifest.timelines.push(TimelineEntry {
        role: Default::default(),
        id: extra.clone(),
        name: "Copy".into(),
    });
    project.timelines.insert(extra.clone(), copy);
    let root = std::env::temp_dir().join(format!("fosscut-model-removal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut store = DirStore::new(root.join("Removal.fosscut"));
    project.save_all(&mut store).unwrap();
    let path = DocumentKey::Timeline(extra.clone()).path();
    assert!(store.exists(&path));
    project.timelines.remove(&extra);
    project.manifest.timelines.retain(|t| t.id != extra);
    project
        .save_documents(
            &mut store,
            &[DocumentKey::Timeline(extra.clone()), DocumentKey::Manifest],
        )
        .unwrap();
    assert!(!store.exists(&path), "the document went with the timeline");
    assert_eq!(Project::load(&store).unwrap(), project);
    store.remove("timelines/never.json").unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let mut memory = MemoryStore::new();
    memory.write_atomic("a.json", b"{}").unwrap();
    memory.remove("a.json").unwrap();
    assert!(!memory.exists("a.json"));
}

/// A project whose primary timeline shows a compound timeline holding a copy of its clips.
fn nested_project() -> (Project, Id, Id) {
    let mut project = round_one_project();
    let outer = project.manifest.primary_timeline.clone();
    let inner = Id::new(IdKind::Timeline, 60);
    let mut doc = project.timelines[&outer].clone();
    doc.id = inner.clone();
    for (n, clip) in doc.tracks.iter_mut().flat_map(|t| &mut t.clips).enumerate() {
        clip.id = Id::new(IdKind::Clip, 200 + n as u64);
    }
    for (n, track) in doc.tracks.iter_mut().enumerate() {
        track.id = Id::new(IdKind::Track, 300 + n as u64);
    }
    project.manifest.timelines.push(TimelineEntry {
        id: inner.clone(),
        name: "Inside".into(),
        role: TimelineRole::Compound,
    });
    project.timelines.insert(inner.clone(), doc);
    let shown = project.timelines[&inner].duration();
    let compound = Clip::new(
        Id::new(IdKind::Clip, 400),
        frames(0),
        shown,
        ClipContent::Timeline {
            timeline: inner.clone(),
            source_in: Flicks::ZERO,
        },
    );
    let track = Id::new(IdKind::Track, 401);
    project
        .timelines
        .get_mut(&outer)
        .unwrap()
        .tracks
        .push(Track {
            id: track,
            kind: TrackKind::Video,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![compound],
        });
    (project, outer, inner)
}

#[test]
fn compound_clips_round_trip_and_nesting_is_checked() {
    let (project, outer, inner) = nested_project();
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    assert_eq!(Project::load(&store).unwrap(), project);
    let manifest = serde_json::to_value(&project.manifest).unwrap();
    assert_eq!(manifest["timelines"][1]["role"], "compound");
    assert!(
        manifest["timelines"][0].get("role").is_none(),
        "sequences are the default"
    );
    let codes = |p: &Project| -> Vec<String> { validate(p).into_iter().map(|i| i.code).collect() };
    fn compound_clip<'a>(p: &'a mut Project, outer: &Id) -> &'a mut Clip {
        p.timelines
            .get_mut(outer)
            .unwrap()
            .tracks
            .last_mut()
            .unwrap()
            .clips
            .last_mut()
            .unwrap()
    }
    let mut cycle = project.clone();
    let back = Clip::new(
        Id::new(IdKind::Clip, 500),
        frames(0),
        frames(10),
        ClipContent::Timeline {
            timeline: outer.clone(),
            source_in: Flicks::ZERO,
        },
    );
    cycle.manifest.timelines[0].role = TimelineRole::Compound;
    cycle.timelines.get_mut(&inner).unwrap().tracks.push(Track {
        id: Id::new(IdKind::Track, 501),
        kind: TrackKind::Video,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        caption_style: None,
        clips: vec![back],
    });
    assert!(
        codes(&cycle).contains(&"fosscut.model.nesting".to_string()),
        "{:?}",
        codes(&cycle)
    );
    let mut past = project.clone();
    let longer = compound_clip(&mut past, &outer).duration + frames(1);
    compound_clip(&mut past, &outer).duration = longer;
    assert!(codes(&past).contains(&"fosscut.model.source_range".to_string()));
    let mut plain = project.clone();
    plain.manifest.timelines[1].role = TimelineRole::Sequence;
    assert!(
        codes(&plain).contains(&"fosscut.model.nesting".to_string()),
        "not a compound timeline"
    );
    let mut faster = project.clone();
    faster.timelines.get_mut(&inner).unwrap().format.rate = Rate::new(60, 1).unwrap();
    assert!(
        codes(&faster).contains(&"fosscut.model.nesting".to_string()),
        "another rate"
    );
    let mut gone = project.clone();
    gone.timelines.remove(&inner);
    gone.manifest.timelines.truncate(1);
    assert!(codes(&gone).contains(&"fosscut.model.nesting".to_string()));
}

#[test]
fn styled_text_fonts_presets_and_sound_effects_are_checked() {
    let mut project = round_one_project();
    let font = project.allocate_id(IdKind::Asset);
    project.media.assets.push(Asset {
        folder: None,
        id: font.clone(),
        kind: AssetKind::Font,
        name: "Fixture Sans.ttf".into(),
        path: PathHints {
            relative: Some("fonts/Fixture Sans.ttf".into()),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            font: Some(FontInfo {
                families: vec!["Fixture Sans".into()],
                faces: 1,
            }),
            ..Default::default()
        },
    });
    assert_eq!(
        project.media.font_families(),
        vec!["Fixture Sans".to_string(), "Inter".into()]
    );
    {
        let text = first_text(&mut project);
        text.text = "Breaking news".into();
        text.style.font = Some("Fixture Sans".into());
        text.style.case = TextCase::Upper;
        text.style.letter_spacing = 0.08;
        text.style.stroke.enabled = true;
        text.style.glow.enabled = true;
        text.spans = vec![TextSpan {
            start: 9,
            end: 13,
            color: Color::parse("#FFCC00").ok(),
            bold: Some(true),
            italic: None,
            underline: Some(true),
            font: None,
            size: Some(0.1),
        }];
        text.reveal = Some(TextReveal {
            unit: RevealUnit::Letter,
            motion: RevealMotion::Appear,
            duration: frames(30),
            exit: true,
        });
    }
    project.manifest.text_presets = vec![TextPreset {
        name: "Lower third".into(),
        style: TextStyle {
            background: TextBox {
                enabled: true,
                ..TextBox::default()
            },
            ..TextStyle::default()
        },
    }];
    assert!(validate(&project).is_empty(), "{:?}", validate(&project));
    let mut store = MemoryStore::new();
    project.save_all(&mut store).unwrap();
    assert_eq!(Project::load(&store).unwrap(), project);

    type Breakage = fn(&mut Project);
    let cases: [(&str, &str, Breakage); 9] = [
        ("an unknown font", "fosscut.model.unknown_font", |p| {
            p.media.assets.pop();
        }),
        ("a span past the end", "fosscut.model.text_span", |p| {
            first_text(p).spans[0].end = 40;
        }),
        ("an empty span", "fosscut.model.text_span", |p| {
            first_text(p).spans[0].start = 13;
        }),
        ("wide spacing", "fosscut.model.text_style", |p| {
            first_text(p).style.letter_spacing = 3.0;
        }),
        ("tight lines", "fosscut.model.text_style", |p| {
            first_text(p).style.line_height = 0.1;
        }),
        ("a far shadow", "fosscut.model.text_style", |p| {
            first_text(p).style.shadow.offset = [2.0, 0.0];
        }),
        ("an instant reveal", "fosscut.model.text_reveal", |p| {
            first_text(p).reveal.as_mut().unwrap().duration = Flicks::ZERO;
        }),
        ("unsorted presets", "fosscut.model.text_preset", |p| {
            let mut second = p.manifest.text_presets[0].clone();
            second.name = "Alpha".into();
            p.manifest.text_presets.push(second);
        }),
        (
            "a sound effect on a title",
            "fosscut.model.effect_target",
            |p| {
                let id = p.allocate_id(IdKind::Effect);
                first_text_clip(p).effects.push(Effect {
                    id,
                    effect_type: "audio.denoise".into(),
                    version: 1,
                    enabled: true,
                    params: Default::default(),
                    keyframes: Default::default(),
                });
            },
        ),
    ];
    for (case, code, breakage) in cases {
        let mut broken = project.clone();
        breakage(&mut broken);
        let codes: Vec<String> = validate(&broken).into_iter().map(|i| i.code).collect();
        assert!(codes.contains(&code.to_string()), "{case}: {codes:?}");
    }
    // A picture effect on an audio-only clip is refused the same way.
    let mut sound = project.clone();
    let timeline = sound.manifest.primary_timeline.clone();
    let audio = sound.allocate_id(IdKind::Asset);
    let mut asset = sound.media.assets[0].clone();
    asset.id = audio.clone();
    asset.kind = AssetKind::Audio;
    asset.info.video = None;
    asset.info.audio = Some(AudioInfo {
        sample_rate: 48_000,
        channels: 2,
        codec: Some("aac".into()),
    });
    sound.media.assets.push(asset);
    let clip_id = sound.allocate_id(IdKind::Clip);
    let effect_id = sound.allocate_id(IdKind::Effect);
    let mut clip = Clip::new(
        clip_id,
        Flicks::ZERO,
        frames(30),
        ClipContent::Audio {
            asset: audio,
            source_in: Flicks::ZERO,
        },
    );
    clip.effects.push(Effect {
        id: effect_id,
        effect_type: "color.adjust".into(),
        version: 1,
        enabled: true,
        params: Default::default(),
        keyframes: Default::default(),
    });
    let doc = sound.timelines.get_mut(&timeline).unwrap();
    doc.tracks.push(Track {
        id: Id::new(IdKind::Track, 900),
        kind: TrackKind::Audio,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        caption_style: None,
        clips: vec![clip.clone()],
    });
    let codes: Vec<String> = validate(&sound).into_iter().map(|i| i.code).collect();
    assert!(
        codes.contains(&"fosscut.model.effect_target".to_string()),
        "{codes:?}"
    );
    clip.effects[0].effect_type = "audio.vocals".into();
    sound
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .last_mut()
        .unwrap()
        .clips = vec![clip];
    assert!(validate(&sound).is_empty(), "{:?}", validate(&sound));
}

fn first_text_clip(p: &mut Project) -> &mut Clip {
    let timeline = p.manifest.primary_timeline.clone();
    p.timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .iter_mut()
        .flat_map(|t| t.clips.iter_mut())
        .find(|c| matches!(c.content, ClipContent::Text(_)))
        .unwrap()
}

fn first_text(p: &mut Project) -> &mut TextContent {
    match &mut first_text_clip(p).content {
        ClipContent::Text(text) => text,
        _ => unreachable!(),
    }
}
