//! Helpers for talking-head reels on generated media: music ducked under a tone that stands
//! in for speech (and under transcribed speech from the fixtures), a separate recording
//! synced under the camera's clips from a delayed copy of its sound, punch-ins centred on a
//! face from a pose written into the cache, and captions with emphasised keywords. Every
//! plan is applied through the command framework, and the result is checked: the music's
//! level as mixed, the recording's place and the groups, the zoom frame by frame.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use fosscut_analysis::pose::{LANDMARKS, Landmark, PoseFrame, PoseTrack};
use fosscut_analysis::vision::{DataFile, pack};
use fosscut_analysis::{Analyzer, Options, cache};
use fosscut_core::{Request, Session};
use fosscut_engine::duck::{DuckContext, DuckOptions, SpeechSource, duck};
use fosscut_engine::preview::Sound;
use fosscut_engine::punch::{Moment, PunchRequest, punch_in};
use fosscut_engine::sync::{SyncRequest, sync_audio};
use fosscut_model::{
    Asset, AssetKind, AudioInfo, Clip, ClipContent, ContentIdentity, Id, IdKind, MediaInfo,
    MemoryStore, PathHints, Project, TextContent, TimelineFormat, Track, TrackKind, VideoInfo,
};
use fosscut_time::{Flicks, Rate};
use serde_json::json;

fn required(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty() && v != "0")
}

fn ffmpeg_available() -> bool {
    let found = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        found || !required("FOSSCUT_REQUIRE_FFMPEG"),
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    found
}

fn workspace() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-reel-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Reel.fosscut")).unwrap();
        dir
    })
}

fn bundle() -> PathBuf {
    workspace().join("Reel.fosscut")
}

fn ffmpeg(args: &[&str]) {
    let status = Command::new("ffmpeg")
        .current_dir(workspace())
        .args(["-y", "-v", "error"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
}

/// The generated media, made once: a tone heard from 1 s to 2 s and 4 s to 4.5 s (speech),
/// a steady tone (music), a camera with a room's sound, the same sound recorded 1.5 s later
/// on its own, and an unrelated noise.
fn media() -> &'static Path {
    static MADE: OnceLock<()> = OnceLock::new();
    MADE.get_or_init(|| {
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=300:sample_rate=48000:duration=6",
            "-af",
            "volume='between(t,1,2)+between(t,4,4.5)':eval=frame",
            "-c:a",
            "pcm_s16le",
            "speech.wav",
        ]);
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=220:sample_rate=48000:duration=6",
            "-c:a",
            "pcm_s16le",
            "music.wav",
        ]);
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=duration=8:color=pink:seed=7:sample_rate=48000",
            "-af",
            "volume='0.15+0.85*gt(sin(2*PI*1.3*t)*sin(2*PI*0.37*t+0.5),0.3)':eval=frame",
            "-c:a",
            "pcm_s16le",
            "room.wav",
        ]);
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=8",
            "-i",
            "room.wav",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-shortest",
            "camera.mp4",
        ]);
        ffmpeg(&[
            "-ss",
            "1.5",
            "-i",
            "room.wav",
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=duration=8:color=white:seed=3:amplitude=0.02:sample_rate=48000",
            "-filter_complex",
            "[0]volume=0.5[a];[a][1]amix=inputs=2:duration=first:normalize=0",
            "-c:a",
            "pcm_s16le",
            "recorder.wav",
        ]);
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=duration=8:color=brown:seed=41:sample_rate=48000",
            "-af",
            "volume='0.3+0.7*gt(sin(2*PI*0.21*t+2),0.8)':eval=frame",
            "-c:a",
            "pcm_s16le",
            "other.wav",
        ]);
    });
    workspace()
}

fn audio_asset(n: u64, file: &str, seconds: f64) -> Asset {
    Asset {
        folder: None,
        id: Id::new(IdKind::Asset, n),
        kind: AssetKind::Audio,
        name: file.into(),
        path: PathHints {
            relative: Some(format!("../{file}")),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            duration: Some(Flicks::from_millis((seconds * 1000.0) as i64)),
            audio: Some(AudioInfo {
                sample_rate: 48_000,
                channels: 1,
                codec: None,
            }),
            ..MediaInfo::default()
        },
    }
}

fn track(n: u64, kind: TrackKind, clips: Vec<Clip>) -> Track {
    Track {
        id: Id::new(IdKind::Track, n),
        kind,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        caption_style: None,
        clips,
    }
}

fn project_with(assets: Vec<Asset>, tracks: Vec<Track>) -> Project {
    let format = TimelineFormat {
        width: 1080,
        height: 1920,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 7), "Reel", format);
    project.media.assets = assets;
    let timeline = project.manifest.primary_timeline.clone();
    project.timelines.get_mut(&timeline).unwrap().tracks = tracks;
    project.manifest.next_id = 100;
    project
}

fn apply(project: Project, commands: &[serde_json::Value]) -> Project {
    let mut session = Session::create(MemoryStore::new(), project).unwrap();
    let request: Request = serde_json::from_value(json!({ "commands": commands })).unwrap();
    session.execute(request).unwrap();
    session.project().clone()
}

fn clip_of<'a>(project: &'a Project, id: &Id) -> &'a Clip {
    project
        .timelines
        .values()
        .flat_map(|t| &t.tracks)
        .flat_map(|t| &t.clips)
        .find(|c| &c.id == id)
        .unwrap()
}

fn ms(n: i64) -> Flicks {
    Flicks::from_millis(n)
}

/// An empty analysis cache of its own.
fn fresh_cache(name: &str) -> PathBuf {
    let dir = workspace().join(format!("cache-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The RMS level, in dB, of the timeline's sound from `at` for 200 ms.
fn level_db(project: &Project, at: i64) -> f64 {
    let timeline = project.manifest.primary_timeline.clone();
    let mixed = Sound::new()
        .mix(project, &bundle(), &timeline, ms(at), ms(200), 48_000, 1)
        .unwrap();
    let power = mixed
        .samples
        .iter()
        .map(|s| f64::from(*s).powi(2))
        .sum::<f64>()
        / mixed.samples.len().max(1) as f64;
    10.0 * power.max(1e-12).log10()
}

#[test]
fn music_ducks_under_speech_found_between_silences() {
    if !ffmpeg_available() {
        return;
    }
    media();
    let speech = Clip::new(
        Id::new(IdKind::Clip, 1),
        Flicks::ZERO,
        ms(5500),
        ClipContent::Audio {
            asset: Id::new(IdKind::Asset, 1),
            source_in: ms(500),
        },
    );
    let music = Clip::new(
        Id::new(IdKind::Clip, 2),
        Flicks::ZERO,
        ms(6000),
        ClipContent::Audio {
            asset: Id::new(IdKind::Asset, 2),
            source_in: Flicks::ZERO,
        },
    );
    let project = project_with(
        vec![
            audio_asset(1, "speech.wav", 6.0),
            audio_asset(2, "music.wav", 6.0),
        ],
        vec![
            track(10, TrackKind::Audio, vec![speech]),
            track(11, TrackKind::Audio, vec![music]),
        ],
    );
    let context = DuckContext {
        bundle: bundle(),
        cache: fresh_cache("duck"),
        models: workspace().join("no-models"),
    };
    let music_id = Id::new(IdKind::Clip, 2);
    // No speech clips named: every other clip with sound is listened to.
    let plan = duck(
        &project,
        &context,
        std::slice::from_ref(&music_id),
        &[],
        &DuckOptions::default(),
        &mut |_| true,
    )
    .unwrap();
    assert_eq!(plan.speech_clips[0].source, "silences");
    // The tone sounds at 1-2 s and 4-4.5 s of its file; the clip starts 0.5 s into it.
    // Silences are measured in 50 ms windows and the edges fall on frames.
    assert_eq!(plan.speech.len(), 2, "{:?}", plan.speech);
    for (range, (a, b)) in plan.speech.iter().zip([(500, 1500), (3500, 4000)]) {
        assert!(
            (range.start - ms(a)).get().abs() <= ms(70).get(),
            "{range:?}"
        );
        assert!((range.end - ms(b)).get().abs() <= ms(70).get(), "{range:?}");
    }
    assert_eq!(plan.commands.len(), 1);
    let ducked = &plan.clips[0];
    assert!((ducked.ducked - 10f64.powf(-12.0 / 20.0)).abs() < 1e-3);

    // As mixed, with the speech muted: 12 dB down under speech, back to full after.
    let timeline = project.manifest.primary_timeline.clone();
    let music_only = |mut p: Project| {
        p.timelines.get_mut(&timeline).unwrap().tracks[0].muted = true;
        p
    };
    let applied = music_only(apply(project.clone(), &plan.commands));
    let full = level_db(&applied, 2800);
    let under = level_db(&applied, 900);
    assert!((under - full - -12.0).abs() < 0.5, "{under} vs {full}");
    assert!((level_db(&applied, 5300) - full).abs() < 0.2);
    let before = level_db(&music_only(project.clone()), 900);
    assert!(
        (before - full).abs() < 0.2,
        "the original is left as it was"
    );

    // Deeper, quicker, with a gap so long the two stretches join.
    let joined = duck(
        &project,
        &context,
        std::slice::from_ref(&music_id),
        &[Id::new(IdKind::Clip, 1)],
        &DuckOptions {
            depth_db: -20.0,
            attack: ms(50),
            release: ms(100),
            gap: ms(2500),
            source: SpeechSource::Silences,
        },
        &mut |_| true,
    )
    .unwrap();
    assert_eq!(joined.speech.len(), 1);
    let applied = apply(project.clone(), &joined.commands);
    let quiet = music_only(applied.clone());
    let deep = level_db(&quiet, 2800);
    assert!(
        (deep - full - -20.0).abs() < 0.5,
        "{deep} vs {full}: {:?} {:?}",
        joined.speech,
        joined.commands
    );

    // Normalizing ducked music keeps its shape: the keys scale together.
    let leveled = fosscut_engine::loudness::normalize(
        &applied,
        &bundle(),
        std::slice::from_ref(&music_id),
        -30.0,
        -1.0,
        false,
    )
    .unwrap();
    assert_eq!(leveled.commands[0]["command"], "keyframe.set_all");
    let leveled = apply(applied, &leveled.commands);
    let keys = &clip_of(&leveled, &music_id).keyframes["volume"];
    let values: Vec<f64> = keys.iter().filter_map(|k| k.value.number()).collect();
    let loudest = values.iter().copied().fold(0.0, f64::max);
    let quietest = values.iter().copied().fold(f64::MAX, f64::min);
    assert!(((quietest / loudest).log10() * 20.0 - -20.0).abs() < 0.05);

    // Errors: no depth that does nothing, music must have sound and share the timeline.
    let wrong = |music: &[Id], speech: &[Id], options: DuckOptions| {
        duck(&project, &context, music, speech, &options, &mut |_| true)
            .unwrap_err()
            .code()
            .to_string()
    };
    let invalid = "fosscut.request.invalid_arguments";
    let shallow = DuckOptions {
        depth_db: 0.0,
        ..DuckOptions::default()
    };
    assert_eq!(
        wrong(std::slice::from_ref(&music_id), &[], shallow),
        invalid
    );
    assert_eq!(wrong(&[], &[], DuckOptions::default()), invalid);
    assert_eq!(
        wrong(
            std::slice::from_ref(&music_id),
            std::slice::from_ref(&music_id),
            DuckOptions::default()
        ),
        invalid
    );
    assert_eq!(
        wrong(&[Id::new(IdKind::Clip, 77)], &[], DuckOptions::default()),
        invalid
    );
    let transcript_only = DuckOptions {
        source: SpeechSource::Transcript,
        ..DuckOptions::default()
    };
    assert_eq!(
        wrong(std::slice::from_ref(&music_id), &[], transcript_only),
        "fosscut.analysis.missing"
    );
}

fn camera_asset() -> Asset {
    Asset {
        folder: None,
        id: Id::new(IdKind::Asset, 3),
        kind: AssetKind::Video,
        name: "camera.mp4".into(),
        path: PathHints {
            relative: Some("../camera.mp4".into()),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            duration: Some(ms(8000)),
            video: Some(VideoInfo {
                width: 320,
                height: 180,
                rate: Some(Rate::FPS_30),
                codec: None,
                rotation: 0,
            }),
            audio: Some(AudioInfo {
                sample_rate: 48_000,
                channels: 1,
                codec: None,
            }),
            ..MediaInfo::default()
        },
    }
}

fn camera_clip(id: u64, start: i64, source_in: i64, length: i64) -> Clip {
    Clip::new(
        Id::new(IdKind::Clip, id),
        ms(start),
        ms(length),
        ClipContent::Video {
            asset: Id::new(IdKind::Asset, 3),
            source_in: ms(source_in),
        },
    )
}

#[test]
fn a_separate_recording_is_synced_under_every_camera_clip() {
    if !ffmpeg_available() {
        return;
    }
    media();
    // Two camera clips: the first from 2 s of the camera, grouped with a title; the second
    // from 0.5 s, before the recorder started.
    let mut first = camera_clip(1, 0, 2000, 2000);
    let mut title = Clip::new(
        Id::new(IdKind::Clip, 5),
        Flicks::ZERO,
        ms(1000),
        ClipContent::Text(Box::new(TextContent::plain("Hi"))),
    );
    let group = Id::new(IdKind::Group, 50);
    first.group = Some(group.clone());
    title.group = Some(group.clone());
    let second = camera_clip(2, 2000, 500, 2000);
    let project = project_with(
        vec![
            camera_asset(),
            audio_asset(4, "recorder.wav", 6.5),
            audio_asset(6, "other.wav", 8.0),
        ],
        vec![
            track(10, TrackKind::Video, vec![first, second]),
            track(11, TrackKind::Text, vec![title]),
        ],
    );
    let request = SyncRequest {
        camera: Id::new(IdKind::Asset, 3),
        recording: Id::new(IdKind::Asset, 4),
        timeline: None,
        track: None,
        max_offset_seconds: 0.0,
    };
    let plan = sync_audio(&project, &bundle(), &request).unwrap();
    assert!((plan.offset_seconds - 1.5).abs() < 0.003, "{plan:?}");
    assert!(plan.confidence > 0.5, "{}", plan.confidence);
    assert!(plan.new_track && plan.skipped.is_empty());
    assert_eq!(plan.pairs.len(), 2);

    let synced = apply(project.clone(), &plan.commands);
    let timeline = &synced.timelines[&synced.manifest.primary_timeline];
    let added = timeline.track(&plan.track).unwrap();
    assert_eq!(added.kind, TrackKind::Audio);
    // The first clip's recording starts with it, from 0.5 s of the recording.
    let one = clip_of(&synced, &plan.pairs[0].recording_clip);
    assert_eq!(one.start, Flicks::ZERO);
    assert_eq!(one.duration, ms(2000));
    assert_eq!(
        one.content.source_in().unwrap(),
        ms(500),
        "on the nearest frame"
    );
    let exact = ms(2000) - plan.offset;
    assert!(
        ((ms(500) - exact).as_seconds_f64() - plan.residual_seconds).abs() < 1e-6,
        "{plan:?}"
    );
    assert!(plan.residual_seconds.abs() <= 1.0 / 60.0);
    // The second clip starts before the recorder did: its recording starts a second in, on
    // a frame, and plays what the camera heard then.
    let two = clip_of(&synced, &plan.pairs[1].recording_clip);
    let frame = Rate::FPS_30.frame_duration();
    assert!(
        two.start >= ms(3000) - ms(3) && two.start <= ms(3000) + frame,
        "{two:?}"
    );
    assert_eq!(Rate::FPS_30.snap(two.start), two.start, "on a frame");
    assert_eq!(two.end(), ms(4000));
    let heard = ms(500) + (two.start - ms(2000)) - plan.offset;
    let placed = two.content.source_in().unwrap();
    assert!(((placed - heard).as_seconds_f64() - plan.residual_seconds).abs() < 1e-6);
    // The cameras are silent and grouped with their recordings; the title stays with the
    // first.
    for pair in &plan.pairs {
        let camera = clip_of(&synced, &pair.camera_clip);
        assert_eq!(camera.volume, 0.0);
        assert_eq!(camera.group.as_ref(), Some(&pair.group));
        assert_eq!(
            clip_of(&synced, &pair.recording_clip).group.as_ref(),
            Some(&pair.group)
        );
    }
    assert_eq!(
        clip_of(&synced, &Id::new(IdKind::Clip, 5)).group.as_ref(),
        Some(&plan.pairs[0].group)
    );

    // A recording of something else is refused with its measurement, and nothing changes.
    let unrelated = SyncRequest {
        recording: Id::new(IdKind::Asset, 6),
        ..request.clone()
    };
    let error = sync_audio(&project, &bundle(), &unrelated).unwrap_err();
    assert_eq!(error.code(), "fosscut.audio.sync_uncertain", "{error}");
    assert!(error.details()["confidence"].as_f64().unwrap() < 0.3);
    // Wrong arguments.
    for wrong in [
        SyncRequest {
            recording: Id::new(IdKind::Asset, 3),
            ..request.clone()
        },
        SyncRequest {
            camera: Id::new(IdKind::Asset, 4),
            ..request.clone()
        },
        SyncRequest {
            track: Some(Id::new(IdKind::Track, 10)),
            ..request.clone()
        },
    ] {
        assert_eq!(
            sync_audio(&project, &bundle(), &wrong).unwrap_err().code(),
            "fosscut.request.invalid_arguments"
        );
    }
}

fn identity() -> ContentIdentity {
    ContentIdentity {
        size: 4242,
        sample_blake3: "ab".repeat(32),
    }
}

/// A pose of 150 frames at 30 fps: a face whose nose is at (0.6, 0.3), turned slightly.
fn cache_face(cache: &Path) {
    let mask = pack(&[0u8; 16]);
    let frames = (0..150)
        .map(|i| {
            let mut landmarks = [Landmark::default(); LANDMARKS];
            let mut put = |n: usize, x: f32, y: f32| {
                landmarks[n] = Landmark {
                    x,
                    y,
                    visibility: 0.95,
                    presence: 0.95,
                };
            };
            put(0, 0.6, 0.3);
            put(2, 0.59, 0.28);
            put(5, 0.61, 0.28);
            put(11, 0.52, 0.45);
            put(12, 0.68, 0.45);
            PoseFrame {
                time: Rate::FPS_30.frames_to_flicks(i),
                presence: 0.95,
                landmarks,
                mask: mask.clone(),
            }
        })
        .collect();
    let track = PoseTrack {
        mask_size: (4, 4),
        frames,
    };
    let data = track.encode();
    let summary = track.summary(DataFile::of(&data));
    let key = Options::default().for_analyzer(Analyzer::Pose);
    let path = cache::entry_path(cache, &identity(), Analyzer::Pose, &key);
    cache::write_data(&path, &data).unwrap();
    cache::write(
        &path,
        Analyzer::Pose,
        &key,
        &serde_json::to_value(&summary).unwrap(),
    )
    .unwrap();
}

#[test]
fn punch_ins_zoom_on_the_face_and_back() {
    let cache = fresh_cache("punch");
    cache_face(&cache);
    let mut camera = camera_asset();
    camera.identity = Some(identity());
    let clip = camera_clip(1, 0, 0, 5000);
    let project = project_with(vec![camera], vec![track(10, TrackKind::Video, vec![clip])]);
    let id = Id::new(IdKind::Clip, 1);
    let moment = |start: i64, end: i64, scale: f64| Moment {
        start: ms(start),
        end: ms(end),
        scale,
        ease: ms(250),
    };
    let request = PunchRequest {
        bundle: bundle(),
        cache: cache.clone(),
        clip: id.clone(),
        moments: vec![moment(3000, 3500, 1.15), moment(1000, 2000, 1.2)],
        focus: None,
    };
    let plan = punch_in(&project, &request).unwrap();
    assert_eq!(plan.anchor, "pose");
    assert_eq!(plan.moments[0].start, ms(1000), "moments are put in order");
    // The face's centre: the nose and eyes.
    let [x, y] = plan.moments[0].focus;
    assert!(
        (x - 0.6).abs() < 1e-3 && (y - 0.2867).abs() < 1e-3,
        "{x} {y}"
    );
    let applied = apply(project.clone(), &plan.commands);
    let clip = clip_of(&applied, &id);
    let scale = |t: i64| clip.transform_at(ms(t)).scale;
    assert_eq!(scale(500), 1.0);
    assert_eq!(scale(1000), 1.0);
    assert!((scale(1500) - 1.2).abs() < 1e-9, "held");
    assert!(scale(1100) > 1.0 && scale(1100) < 1.2, "easing in");
    assert_eq!(scale(2000), 1.0);
    assert_eq!(scale(2500), 1.0);
    let peak = (90..=105)
        .map(|f| clip.transform_at(Rate::FPS_30.frames_to_flicks(f)).scale)
        .fold(0.0, f64::max);
    assert!(
        (peak - 1.15).abs() < 1e-9,
        "a short moment peaks halfway: {peak}"
    );
    let keys = &clip.keyframes["transform.scale"];
    assert!(
        keys.iter().all(|k| Rate::FPS_30.snap(k.time) == k.time),
        "on frames"
    );
    assert_eq!(scale(4000), 1.0);
    let anchor = clip.transform_at(ms(1500)).anchor;
    assert!((anchor[0] - 0.1).abs() < 1e-3 && (anchor[1] + 0.2133).abs() < 1e-3);
    // Only scale and anchor change.
    let mut keyed: Vec<&String> = clip.keyframes.keys().collect();
    keyed.sort();
    assert_eq!(keyed, ["transform.anchor", "transform.scale"]);
    assert_eq!(clip.transform.position, [0.0, 0.0]);

    // Without a pose: the upper third's centre; with a point given, that point.
    let empty = fresh_cache("punch-empty");
    let bare = punch_in(
        &project,
        &PunchRequest {
            cache: empty.clone(),
            ..request.clone()
        },
    )
    .unwrap();
    assert_eq!(bare.anchor, "default");
    assert_eq!(bare.moments[0].focus, [0.5, 0.333333]);
    let given = punch_in(
        &project,
        &PunchRequest {
            focus: Some([0.25, 0.75]),
            ..request.clone()
        },
    )
    .unwrap();
    assert_eq!(given.anchor, "focus");
    assert_eq!(given.moments[1].focus, [0.25, 0.75]);

    // A clip already scaled keeps its anchor (moving it would move the picture) and zooms
    // from its own scale.
    let mut scaled = project.clone();
    let timeline = scaled.manifest.primary_timeline.clone();
    scaled.timelines.get_mut(&timeline).unwrap().tracks[0].clips[0]
        .transform
        .scale = 1.5;
    let kept = punch_in(&scaled, &request).unwrap();
    assert_eq!(kept.anchor, "kept");
    assert_eq!(kept.commands.len(), 1);
    let applied = apply(scaled, &kept.commands);
    let clip = clip_of(&applied, &id);
    assert!((clip.transform_at(ms(1500)).scale - 1.8).abs() < 1e-9);
    assert_eq!(clip.transform_at(ms(500)).scale, 1.5);

    // Moments outside the clip, overlapping, or with no zoom are refused.
    for moments in [
        vec![moment(4500, 5500, 1.2)],
        vec![moment(1000, 2000, 1.2), moment(1500, 2500, 1.2)],
        vec![moment(1000, 2000, 1.0)],
        vec![moment(2000, 1000, 1.2)],
        Vec::new(),
    ] {
        let error = punch_in(
            &project,
            &PunchRequest {
                moments,
                ..request.clone()
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "fosscut.request.invalid_arguments");
    }
}

/// A models folder holding only `whisper-tiny.en` from `FOSSCUT_MODELS_DIR`, when FFmpeg
/// and the model are there; CI sets `FOSSCUT_REQUIRE_SPEECH`, which makes them required.
fn only_tiny() -> Option<PathBuf> {
    let source = std::env::var_os("FOSSCUT_MODELS_DIR")
        .map(PathBuf::from)
        .map(|dir| dir.join("ggml-tiny.en.bin"))
        .filter(|file| file.is_file());
    assert!(
        source.is_some() || !required("FOSSCUT_REQUIRE_SPEECH"),
        "FOSSCUT_REQUIRE_SPEECH is set but whisper-tiny.en is not in FOSSCUT_MODELS_DIR"
    );
    let source = source?;
    if !ffmpeg_available() {
        return None;
    }
    let dir = workspace().join("models-tiny");
    let target = dir.join("ggml-tiny.en.bin");
    if !target.exists() {
        std::fs::create_dir_all(&dir).unwrap();
        if std::fs::hard_link(&source, &target).is_err() {
            std::fs::copy(&source, &target).unwrap();
        }
    }
    Some(dir)
}

#[test]
fn transcribed_speech_ducks_music_and_captions_its_keywords() {
    let Some(models) = only_tiny() else {
        return;
    };
    media();
    let speech_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/speech/librispeech-5105-28233-0005/audio.flac");
    let mut speech_asset = audio_asset(1, "speech.flac", 11.39);
    speech_asset.path = PathHints {
        relative: None,
        absolute: Some(speech_file.display().to_string()),
    };
    speech_asset.info.audio.as_mut().unwrap().sample_rate = 16_000;
    let speech = Clip::new(
        Id::new(IdKind::Clip, 1),
        Flicks::ZERO,
        ms(10_000),
        ClipContent::Audio {
            asset: Id::new(IdKind::Asset, 1),
            source_in: Flicks::ZERO,
        },
    );
    let music = Clip::new(
        Id::new(IdKind::Clip, 2),
        Flicks::ZERO,
        ms(6000),
        ClipContent::Audio {
            asset: Id::new(IdKind::Asset, 2),
            source_in: Flicks::ZERO,
        },
    );
    let project = project_with(
        vec![speech_asset, audio_asset(2, "music.wav", 6.0)],
        vec![
            track(10, TrackKind::Audio, vec![speech]),
            track(11, TrackKind::Audio, vec![music]),
        ],
    );
    let cache = fresh_cache("transcript");
    let report = fosscut_engine::analysis::analyze(
        &project,
        &fosscut_engine::analysis::AnalysisRequest {
            bundle: bundle(),
            asset: Id::new(IdKind::Asset, 1),
            analyzers: vec![Analyzer::Transcript],
            options: Options::default(),
            refresh: false,
            cache: cache.clone(),
            models: models.clone(),
        },
        &mut |_| true,
    )
    .unwrap();
    let transcript = report.results.transcript.unwrap();
    let plan = duck(
        &project,
        &DuckContext {
            bundle: bundle(),
            cache: cache.clone(),
            models: models.clone(),
        },
        &[Id::new(IdKind::Clip, 2)],
        &[Id::new(IdKind::Clip, 1)],
        &DuckOptions::default(),
        &mut |_| true,
    )
    .unwrap();
    assert_eq!(plan.speech_clips[0].source, "transcript");
    // Every word inside the music's six seconds is under ducked music.
    let first = transcript.words[0].start.flicks;
    assert!(plan.speech[0].start <= first + ms(1));
    for word in transcript.words.iter().filter(|w| w.end.flicks < ms(6000)) {
        assert!(
            plan.speech
                .iter()
                .any(|r| r.start <= word.start.flicks && word.end.flicks <= r.end),
            "{word:?}"
        );
    }
    assert_eq!(plan.clips.len(), 1);

    // Captions from the same transcript, with a keyword said once and stressed.
    let keyword = transcript
        .words
        .iter()
        .find(|w| w.text.trim_matches(|c: char| !c.is_alphanumeric()).len() > 4)
        .unwrap();
    let generated = fosscut_engine::captions::generate(
        &project,
        &fosscut_engine::captions::GenerateRequest {
            bundle: bundle(),
            timeline: None,
            source_track: None,
            transcript: Default::default(),
            cache,
            models,
            skip_fillers: false,
            lyrics: false,
            emphasis: vec![fosscut_engine::captions::EmphasisWord {
                text: keyword.text.to_uppercase(),
                at: Some(keyword.start.flicks),
            }],
        },
    )
    .unwrap();
    let marked: Vec<&serde_json::Value> = generated
        .words
        .iter()
        .filter(|w| w["emphasis"] == true)
        .collect();
    assert_eq!(marked.len(), 1, "{marked:?}");
    assert_eq!(
        marked[0]["text"].as_str().unwrap().trim(),
        keyword.text.trim()
    );
    let mut session = Session::create(MemoryStore::new(), project).unwrap();
    let added = session
        .execute(Request::single(
            "captions.add",
            json!({ "words": generated.words, "preset": "reel" }),
        ))
        .unwrap();
    let pages = added.results[0]["pages"].as_array().unwrap();
    assert_eq!(pages.len(), generated.words.len(), "one word a page");
}
