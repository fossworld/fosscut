//! Long video to shorts (CC-017) from a hand-built transcript, levels, scene cuts and pose
//! written straight into the cache: candidates are whole sentences inside the clip, mapped
//! to its timeline, the livelier part ranks first, and a short built from the best one is a
//! 1080x1920 timeline that plays exactly that range, follows the speaker, and carries its
//! captions and a title.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use fosscut_analysis::highlights::HighlightOptions;
use fosscut_analysis::pose::{LANDMARKS, Landmark, PoseFrame, PoseTrack};
use fosscut_analysis::scenes::Cut;
use fosscut_analysis::speech::{Segment, Transcript, Word};
use fosscut_analysis::vision::{DataFile, pack};
use fosscut_analysis::{Analyzer, Levels, Moment, Options, Scenes, cache};
use fosscut_core::{Request, Session};
use fosscut_engine::reframe::SubjectSource;
use fosscut_engine::shorts::{ShortRequest, ShortSource, ShortsContext, candidates, make_short};
use fosscut_model::{
    Asset, AssetKind, AudioInfo, Clip, ClipContent, ContentIdentity, Id, IdKind, MediaInfo,
    MemoryStore, PathHints, Project, TimelineFormat, Track, TrackKind, VideoInfo,
};
use fosscut_time::{FLICKS_PER_SECOND, Flicks, Rate};
use serde_json::json;

fn seconds(s: f64) -> Flicks {
    Flicks::new((s * FLICKS_PER_SECOND as f64).round() as i64)
}

fn identity() -> ContentIdentity {
    ContentIdentity {
        size: 7777,
        sample_blake3: "7a".repeat(32),
    }
}

fn fresh_cache(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-shorts-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn store(cache: &Path, analyzer: Analyzer, result: serde_json::Value, data: Option<&[u8]>) {
    let key = Options::default().for_analyzer(analyzer);
    let path = cache::entry_path(cache, &identity(), analyzer, &key);
    if let Some(data) = data {
        cache::write_data(&path, data).unwrap();
    }
    cache::write(&path, analyzer, &key, &result).unwrap();
}

/// Two minutes of talk: slow sentences of 10 words over 6 s for the first 60 s, then
/// quick ones of 24 words over 7.2 s, each followed by 0.8 s of silence.
fn transcript() -> Transcript {
    let mut words = Vec::new();
    let mut segments = Vec::new();
    let mut t = 0.5;
    let mut n = 0;
    while t < 118.0 {
        let (count, per_word) = if t < 60.0 { (10, 0.6) } else { (24, 0.3) };
        let first = t;
        for k in 0..count {
            let text = if k + 1 == count {
                format!("word{n}.")
            } else {
                format!("word{n}")
            };
            words.push(Word {
                text,
                start: Moment::new(seconds(t)),
                end: Moment::new(seconds(t + per_word * 0.8)),
                probability: 0.9,
            });
            t += per_word;
            n += 1;
        }
        segments.push(Segment {
            text: String::new(),
            start: Moment::new(seconds(first)),
            end: Moment::new(seconds(t)),
            no_speech_probability: 0.0,
        });
        t += 0.8;
    }
    Transcript {
        model: "whisper-tiny.en".into(),
        language: "en".into(),
        segments,
        words,
        fillers: Vec::new(),
        pauses: Vec::new(),
        min_pause_ms: 300,
    }
}

fn cache_all(cache: &Path, with_pose: bool) {
    store(
        cache,
        Analyzer::Transcript,
        serde_json::to_value(transcript()).unwrap(),
        None,
    );
    let levels = Levels {
        window_ms: 1000,
        lufs: (0..120)
            .map(|s| Some(if s >= 60 && s % 4 == 0 { -13.0 } else { -23.0 }))
            .collect(),
    };
    store(
        cache,
        Analyzer::Levels,
        serde_json::to_value(levels).unwrap(),
        None,
    );
    let scenes = Scenes {
        threshold: 0.35,
        min_shot_ms: 500,
        frames: 3600,
        cuts: (0..20)
            .map(|k| Cut {
                frame: 0,
                time: Moment::new(seconds(61.0 + 2.5 * f64::from(k))),
                score: 1.0,
            })
            .collect(),
    };
    store(
        cache,
        Analyzer::Scenes,
        serde_json::to_value(scenes).unwrap(),
        None,
    );
    if with_pose {
        // A speaker sitting right of centre, at 30 fps for the whole file.
        let mask = pack(&[0u8; 16]);
        let mut landmarks = [Landmark::default(); LANDMARKS];
        for (n, x, y) in [
            (0, 0.62, 0.3),
            (11, 0.56, 0.45),
            (12, 0.68, 0.45),
            (23, 0.58, 0.8),
            (24, 0.66, 0.8),
        ] {
            landmarks[n] = Landmark {
                x,
                y,
                visibility: 0.9,
                presence: 0.9,
            };
        }
        let track = PoseTrack {
            mask_size: (4, 4),
            frames: (0..3600)
                .map(|i| PoseFrame {
                    time: Rate::FPS_30.frames_to_flicks(i),
                    presence: 0.9,
                    landmarks,
                    mask: mask.clone(),
                })
                .collect(),
        };
        let data = track.encode();
        let summary = track.summary(DataFile::of(&data));
        store(
            cache,
            Analyzer::Pose,
            serde_json::to_value(summary).unwrap(),
            Some(&data),
        );
    }
}

/// A 1920x1080 timeline whose clip plays 10 s to 110 s of the talk from 5 s on.
fn project() -> Project {
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Talk", format);
    project.media.assets.push(Asset {
        folder: None,
        id: Id::new(IdKind::Asset, 1),
        kind: AssetKind::Video,
        name: "talk.mp4".into(),
        path: PathHints {
            relative: Some("../talk.mp4".into()),
            absolute: None,
        },
        identity: Some(identity()),
        info: MediaInfo {
            duration: Some(seconds(120.0)),
            video: Some(VideoInfo {
                width: 1920,
                height: 1080,
                rate: Some(Rate::FPS_30),
                codec: None,
                rotation: 0,
            }),
            audio: Some(AudioInfo {
                sample_rate: 48_000,
                channels: 2,
                codec: None,
            }),
            ..MediaInfo::default()
        },
    });
    let timeline = project.manifest.primary_timeline.clone();
    project.timelines.get_mut(&timeline).unwrap().tracks = vec![Track {
        id: Id::new(IdKind::Track, 10),
        kind: TrackKind::Video,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        caption_style: None,
        clips: vec![Clip::new(
            Id::new(IdKind::Clip, 20),
            seconds(5.0),
            Rate::FPS_30.frames_to_flicks(3000),
            ClipContent::Video {
                asset: Id::new(IdKind::Asset, 1),
                source_in: seconds(10.0),
            },
        )],
    }];
    project.manifest.next_id = 100;
    project
}

fn context(cache: &Path) -> ShortsContext {
    ShortsContext {
        bundle: std::env::temp_dir().join("Talk.fosscut"),
        cache: cache.to_path_buf(),
    }
}

fn apply(project: Project, commands: &[serde_json::Value]) -> Project {
    let mut session = Session::create(MemoryStore::new(), project).unwrap();
    let request: Request = serde_json::from_value(json!({ "commands": commands })).unwrap();
    session.execute(request).unwrap();
    session.project().clone()
}

#[test]
fn candidates_are_sentences_inside_the_clip_and_the_lively_part_ranks_first() {
    let cache = fresh_cache("candidates");
    cache_all(&cache, false);
    let project = project();
    let clip = Id::new(IdKind::Clip, 20);
    let found = candidates(
        &project,
        &context(&cache),
        &clip,
        &HighlightOptions::default(),
    )
    .unwrap();
    assert_eq!(found.measured, ["transcript", "levels", "scenes"]);
    assert!(found.missing.is_empty());
    assert!(found.candidates.len() >= 3, "{}", found.candidates.len());
    let spoken = transcript();
    for candidate in &found.candidates {
        let h = &candidate.highlight;
        let (start, end) = (h.span.start.flicks, h.span.end.flicks);
        assert!(
            start >= seconds(10.0) && end <= seconds(110.0),
            "inside the clip"
        );
        assert!(h.seconds >= 15.0 && h.seconds <= 60.0);
        for word in &spoken.words {
            let inside = |t: Flicks| t > word.start.flicks && t < word.end.flicks;
            assert!(!inside(start) && !inside(end), "never inside a word");
        }
        // The clip plays its source from 10 s at 5 s on the timeline.
        assert_eq!(candidate.timeline_start, start - seconds(5.0));
        assert_eq!(candidate.timeline_end, end - seconds(5.0));
    }
    let best = &found.candidates[0].highlight;
    assert!(
        best.span.start.flicks >= seconds(59.0),
        "the quick, loud, cut-up part: {best:?}"
    );
    assert!(best.reasons.loudness_peak_lu.unwrap() > 9.0);
    assert!(best.reasons.scene_cuts.unwrap() > 0);
    let json = serde_json::to_value(&found).unwrap();
    assert!(json["candidates"][0]["reasons"]["scores"]["density"].is_number());
    assert!(json["candidates"][0]["timeline_start"].is_number());

    let tight = HighlightOptions {
        min_seconds: 200.0,
        max_seconds: 300.0,
        ..HighlightOptions::default()
    };
    let none = candidates(&project, &context(&cache), &clip, &tight).unwrap_err();
    assert_eq!(none.code(), "fosscut.shorts.no_candidates");
    assert_eq!(none.details()["clip"], clip.to_string());
    let empty = fresh_cache("candidates-empty");
    let missing = candidates(
        &project,
        &context(&empty),
        &clip,
        &HighlightOptions::default(),
    )
    .unwrap_err();
    assert_eq!(missing.code(), "fosscut.analysis.missing");
    assert_eq!(missing.details()["analyzer"], "transcript");
}

#[test]
fn a_short_is_a_vertical_timeline_of_the_range_with_captions_and_a_title() {
    let cache = fresh_cache("make");
    cache_all(&cache, true);
    let project = project();
    let clip = Id::new(IdKind::Clip, 20);
    let found = candidates(
        &project,
        &context(&cache),
        &clip,
        &HighlightOptions::default(),
    )
    .unwrap();
    let best = &found.candidates[0].highlight;
    let (start, end) = (best.span.start.flicks, best.span.end.flicks);
    let mut request = ShortRequest::new(ShortSource::Clip(clip.clone()), start, end);
    request.captions = true;
    request.caption_preset = Some("bold-pop".into());
    request.title = Some("The quick part".into());
    request.name = Some("Short 1".into());
    let plan = make_short(&project, &context(&cache), &request).unwrap();
    let names: Vec<&str> = plan
        .commands
        .iter()
        .map(|c| c["command"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "timeline.add",
            "clip.add",
            "clip.set",
            "keyframe.set_all",
            "keyframe.set_all",
            "captions.add",
            "text.add"
        ]
    );
    assert!(plan.centred.is_none());
    assert!(plan.framing.as_ref().unwrap().commands.is_empty());
    let built = apply(project.clone(), &plan.commands);
    let short = &built.timelines[&plan.timeline];
    assert_eq!(short.name, "Short 1");
    assert_eq!((short.format.width, short.format.height), (1080, 1920));
    assert_eq!(short.format.rate, Rate::FPS_30);
    let video = short
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap();
    let placed = &video.clips[0];
    assert_eq!(placed.id, plan.clip);
    assert_eq!(plan.start, Rate::FPS_30.snap(start));
    assert_eq!(placed.content.source_in(), Some(plan.start));
    assert_eq!(placed.duration, plan.duration);
    assert!(plan.start + plan.duration <= end);
    assert!(end - plan.start - plan.duration < Rate::FPS_30.frame_duration());
    // The speaker at 0.62 of the picture stays near the middle of the tall canvas.
    let fit = (1080.0 / 1920.0) / (1920.0 / 1080.0);
    let t = placed.transform_at(Flicks::ZERO);
    assert!((t.scale * fit - 1.0).abs() < 1e-6);
    let x = t.position[0] + (0.62 - 0.5) * t.scale;
    assert!(x.abs() < 0.12, "{x}");
    let captions = short
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Caption)
        .unwrap();
    assert!(!captions.clips.is_empty());
    assert_eq!(plan.caption_words, best.words as usize);
    assert!(captions.clips.iter().all(|c| c.end() <= plan.duration));
    let text = short
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Text)
        .unwrap();
    assert!(matches!(&text.clips[0].content, ClipContent::Text(t) if t.text == "The quick part"));
    // The original is untouched; the new timeline is not primary.
    assert_eq!(
        built.manifest.primary_timeline,
        project.manifest.primary_timeline
    );
    assert_eq!(
        built.timelines[&project.manifest.primary_timeline],
        project.timelines[&project.manifest.primary_timeline]
    );

    // Centred, without captions: the picture fills the canvas.
    let mut plain = ShortRequest::new(ShortSource::Asset(Id::new(IdKind::Asset, 1)), start, end);
    plain.follow = None;
    let simple = make_short(&project, &context(&cache), &plain).unwrap();
    assert_eq!(simple.commands.len(), 3);
    assert!(simple.framing.is_none() && simple.caption_words == 0);

    // Following someone needs their pose.
    let no_pose = fresh_cache("make-no-pose");
    cache_all(&no_pose, false);
    let mut follow = ShortRequest::new(ShortSource::Clip(clip.clone()), start, end);
    follow.follow = Some(SubjectSource::Person);
    let missing = make_short(&project, &context(&no_pose), &follow).unwrap_err();
    assert_eq!(missing.code(), "fosscut.analysis.missing");
    assert_eq!(missing.details()["analyzer"], "pose");
    assert_eq!(missing.details()["effect"], "shorts");
    let backwards = ShortRequest::new(ShortSource::Clip(clip), end, start);
    let refused = make_short(&project, &context(&cache), &backwards).unwrap_err();
    assert_eq!(refused.code(), "fosscut.request.invalid_arguments");
}
