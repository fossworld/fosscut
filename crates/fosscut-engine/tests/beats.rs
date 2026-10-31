//! Editing to the beat (CC-199) on generated music: a click track at 120 bpm with every
//! fourth click accented is measured through FFmpeg, cached, and marked on the timeline
//! through its clip's trim; the cuts of a video track move onto its beats without changing
//! the order, overlapping or reading past a clip's media; silence has no beats.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use fosscut_analysis::BeatOptions;
use fosscut_core::{Request, Session};
use fosscut_engine::beats::{BeatContext, BeatSource, CutOptions, cut_to_beats, markers};
use fosscut_model::{
    Asset, AssetKind, AudioInfo, Clip, ClipContent, Id, IdKind, MediaInfo, MemoryStore, PathHints,
    Project, TimelineFormat, Track, TrackKind, VideoInfo,
};
use fosscut_time::{FLICKS_PER_SECOND, Flicks, Rate};
use serde_json::json;

fn required(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Whether FFmpeg is here to decode the music (the analysis links its libraries).
fn available() -> bool {
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

fn seconds(s: f64) -> Flicks {
    Flicks::new((s * FLICKS_PER_SECOND as f64).round() as i64)
}

fn frames(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

/// Writes 16-bit mono 48 kHz `samples` as a WAV file.
fn write_wav(path: &Path, samples: &[f32]) {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32_767.0).round() as i16).to_le_bytes())
        .collect();
    let mut file = Vec::with_capacity(44 + data.len());
    file.extend(b"RIFF");
    file.extend((36 + data.len() as u32).to_le_bytes());
    file.extend(b"WAVEfmt ");
    file.extend(16u32.to_le_bytes());
    file.extend(1u16.to_le_bytes());
    file.extend(1u16.to_le_bytes());
    file.extend(48_000u32.to_le_bytes());
    file.extend((48_000u32 * 2).to_le_bytes());
    file.extend(2u16.to_le_bytes());
    file.extend(16u16.to_le_bytes());
    file.extend(b"data");
    file.extend((data.len() as u32).to_le_bytes());
    file.extend(data);
    std::fs::write(path, file).unwrap();
}

/// The working folder with `clicks.wav` (12 s, 120 bpm from 0.2 s, every fourth click
/// twice as loud) and `silence.wav`.
fn workspace() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-beats-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Show.fosscut")).unwrap();
        let rate = 48_000.0;
        let mut music = vec![0.0f32; 12 * 48_000];
        for k in 0..24 {
            let at = ((0.2 + 0.5 * f64::from(k)) * rate) as usize;
            let amplitude = if k % 4 == 0 { 0.8 } else { 0.4 };
            for i in 0..240 {
                let t = i as f32 / 48_000.0;
                music[at + i] += amplitude
                    * (2.0 * std::f32::consts::PI * 2000.0 * t).sin()
                    * (-t / 0.0015).exp();
            }
        }
        write_wav(&dir.join("clicks.wav"), &music);
        write_wav(&dir.join("silence.wav"), &vec![0.0; 6 * 48_000]);
        dir
    })
}

fn audio_asset(n: u64, file: &str, length: f64) -> Asset {
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
            duration: Some(seconds(length)),
            audio: Some(AudioInfo {
                sample_rate: 48_000,
                channels: 1,
                codec: None,
            }),
            ..MediaInfo::default()
        },
    }
}

/// A camera file of a minute (never decoded: only its length matters to the trims).
fn video_asset(n: u64) -> Asset {
    Asset {
        folder: None,
        id: Id::new(IdKind::Asset, n),
        kind: AssetKind::Video,
        name: format!("camera{n}.mp4"),
        path: PathHints {
            relative: Some(format!("../camera{n}.mp4")),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            duration: Some(seconds(60.0)),
            video: Some(VideoInfo {
                width: 320,
                height: 180,
                rate: Some(Rate::FPS_30),
                codec: None,
                rotation: 0,
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

/// A video clip of camera `asset` from `start` to `end` seconds, reading from `source_in`.
fn shot(id: u64, asset: u64, start: f64, end: f64, source_in: f64) -> Clip {
    Clip::new(
        Id::new(IdKind::Clip, id),
        seconds(start),
        seconds(end) - seconds(start),
        ClipContent::Video {
            asset: Id::new(IdKind::Asset, asset),
            source_in: seconds(source_in),
        },
    )
}

/// The music plays from 0.5 s of its file at 1 s on the timeline for 10 s, so its beats
/// fall at 1.2 s, 1.7 s and on, every 15 frames, with downbeats at 2.7 s, 4.7 s and on;
/// shots on a video track are cut off the beat.
fn project() -> Project {
    let format = TimelineFormat {
        width: 320,
        height: 180,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Show", format);
    project.media.assets.extend([
        audio_asset(1, "clicks.wav", 12.0),
        audio_asset(2, "silence.wav", 6.0),
        video_asset(3),
        video_asset(4),
    ]);
    let music = Clip::new(
        Id::new(IdKind::Clip, 50),
        seconds(1.0),
        seconds(10.0),
        ClipContent::Audio {
            asset: Id::new(IdKind::Asset, 1),
            source_in: seconds(0.5),
        },
    );
    let quiet = Clip::new(
        Id::new(IdKind::Clip, 51),
        seconds(12.0),
        seconds(5.0),
        ClipContent::Audio {
            asset: Id::new(IdKind::Asset, 2),
            source_in: Flicks::ZERO,
        },
    );
    let timeline = project.manifest.primary_timeline.clone();
    project.timelines.get_mut(&timeline).unwrap().tracks = vec![
        track(
            10,
            TrackKind::Video,
            vec![
                shot(20, 3, 0.0, 3.0, 5.0),
                shot(21, 3, 3.0, 5.4, 10.0),
                shot(22, 3, 5.4, 7.4, 20.0),
                // No media before its first frame: its start cannot move earlier.
                shot(23, 4, 7.4, 8.4, 0.0),
                // After a gap, with nothing after it.
                shot(24, 4, 9.0, 10.0, 30.0),
            ],
        ),
        track(11, TrackKind::Audio, vec![music, quiet]),
    ];
    project.manifest.next_id = 100;
    project
}

fn context(cache: &Path) -> BeatContext {
    BeatContext {
        bundle: workspace().join("Show.fosscut"),
        cache: cache.to_path_buf(),
        models: workspace().join("models"),
        options: BeatOptions::default(),
    }
}

fn fresh_cache(name: &str) -> PathBuf {
    let dir = workspace().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn apply(project: Project, commands: &[serde_json::Value]) -> Project {
    let mut session = Session::create(MemoryStore::new(), project).unwrap();
    let request: Request = serde_json::from_value(json!({ "commands": commands })).unwrap();
    session.execute(request).unwrap();
    session.project().clone()
}

fn clip_of(project: &Project, id: u64) -> &Clip {
    let id = Id::new(IdKind::Clip, id);
    project
        .timelines
        .values()
        .flat_map(|t| &t.tracks)
        .flat_map(|t| &t.clips)
        .find(|c| c.id == id)
        .unwrap()
}

#[test]
fn beats_become_markers_through_the_music_clip() {
    if !available() {
        return;
    }
    let cache = fresh_cache("cache-markers");
    let project = project();
    let music = BeatSource::Clip(Id::new(IdKind::Clip, 50));
    let plan = markers(&project, &context(&cache), &music, 1, &mut |_| true).unwrap();
    let tempo = plan.tempo_bpm.unwrap();
    assert!((tempo - 120.0).abs() < 1.0, "tempo {tempo}");
    assert_eq!(plan.beats_per_bar, Some(4));
    let expected: Vec<Flicks> = (0..20).map(|k| frames(36 + 15 * k)).collect();
    assert_eq!(
        plan.at, expected,
        "every beat, on its frame, inside the clip"
    );
    assert_eq!(plan.commands.len(), 20);
    assert_eq!(plan.commands[0]["command"], "marker.add");
    let marked = apply(project.clone(), &plan.commands);
    let timeline = &marked.timelines[&marked.manifest.primary_timeline];
    assert_eq!(timeline.markers.len(), 20);
    let downbeats: Vec<Flicks> = timeline
        .markers
        .iter()
        .filter(|m| m.name.as_deref() == Some("downbeat"))
        .map(|m| m.at)
        .collect();
    assert_eq!(
        downbeats,
        (0..5).map(|k| frames(81 + 60 * k)).collect::<Vec<_>>()
    );

    // Every fourth beat is a bar; frames already marked are left alone, and the cached
    // beats answer at once.
    let bars = markers(&marked, &context(&cache), &music, 4, &mut |_| true).unwrap();
    assert!(bars.commands.is_empty());
    assert_eq!(bars.existing, 5);
    let by_asset = BeatSource::Asset {
        asset: Id::new(IdKind::Asset, 1),
        timeline: None,
    };
    let bars = markers(&project, &context(&cache), &by_asset, 4, &mut |_| true).unwrap();
    assert_eq!(bars.at, downbeats);
    assert!(
        bars.commands
            .iter()
            .all(|c| c["args"]["name"] == "downbeat")
    );
    let refused = markers(&project, &context(&cache), &music, 0, &mut |_| true).unwrap_err();
    assert_eq!(refused.code(), "fosscut.request.invalid_arguments");
}

#[test]
fn cuts_move_onto_beats_in_order_without_overlap() {
    if !available() {
        return;
    }
    let cache = fresh_cache("cache-cuts");
    let project = project();
    let options = CutOptions {
        tolerance: Some(seconds(0.35)),
        ..CutOptions::default()
    };
    let plan = cut_to_beats(
        &project,
        &context(&cache),
        &Id::new(IdKind::Track, 10),
        &Id::new(IdKind::Clip, 50),
        &options,
        &mut |_| true,
    )
    .unwrap();
    assert_eq!(plan.beats, 20);
    assert!(plan.unsnapped.is_empty(), "{:?}", plan.unsnapped);
    let moved: Vec<(Flicks, Flicks)> = plan.snapped.iter().map(|s| (s.from, s.to)).collect();
    assert_eq!(
        moved,
        [
            (seconds(3.0), frames(96)),
            (seconds(5.4), frames(156)),
            // The nearer beat, 7.2 s, needs media before shot 23's first frame.
            (seconds(7.4), frames(231)),
            (seconds(9.0), frames(276)),
        ]
    );
    let cut = apply(project.clone(), &plan.commands);
    let spans: Vec<(Flicks, Flicks)> = (20..25)
        .map(|id| {
            let c = clip_of(&cut, id);
            (c.start, c.end())
        })
        .collect();
    assert_eq!(
        spans,
        [
            (Flicks::ZERO, frames(96)),
            (frames(96), frames(156)),
            (frames(156), frames(231)),
            (frames(231), seconds(8.4)),
            (frames(276), frames(276) + seconds(1.0)),
        ]
    );
    // What plays under the rest of each shot stays put: shot 21 starts 0.2 s later in its
    // media, and shot 24 keeps its media.
    assert_eq!(clip_of(&cut, 21).content.source_in(), Some(seconds(10.2)));
    assert_eq!(clip_of(&cut, 24).content.source_in(), Some(seconds(30.0)));

    // With the default tolerance (half a beat apart) the cut at 7.4 s cannot reach 7.7 s.
    let near = cut_to_beats(
        &project,
        &context(&cache),
        &Id::new(IdKind::Track, 10),
        &Id::new(IdKind::Clip, 50),
        &CutOptions::default(),
        &mut |_| true,
    )
    .unwrap();
    assert_eq!(near.tolerance, Flicks::new(frames(15).get() / 2));
    assert_eq!(near.unsnapped.len(), 1);
    assert_eq!(near.unsnapped[0].at, seconds(7.4));
    assert_eq!(near.unsnapped[0].reason, "fosscut.clip.source_range");
    // Cutting every second beat: cuts land on the downbeat-counted beats only.
    let halves = cut_to_beats(
        &project,
        &context(&cache),
        &Id::new(IdKind::Track, 10),
        &Id::new(IdKind::Clip, 50),
        &CutOptions {
            every: 2,
            tolerance: Some(seconds(0.6)),
            ..CutOptions::default()
        },
        &mut |_| true,
    )
    .unwrap();
    for snap in &halves.snapped {
        assert_eq!((snap.to - frames(81)).get().rem_euclid(frames(30).get()), 0);
    }
    let refused = cut_to_beats(
        &project,
        &context(&cache),
        &Id::new(IdKind::Track, 11),
        &Id::new(IdKind::Clip, 50),
        &CutOptions::default(),
        &mut |_| true,
    )
    .unwrap_err();
    assert_eq!(refused.code(), "fosscut.request.invalid_arguments");
}

#[test]
fn silence_has_no_beats() {
    if !available() {
        return;
    }
    let cache = fresh_cache("cache-silence");
    let project = project();
    let quiet = BeatSource::Clip(Id::new(IdKind::Clip, 51));
    let error = markers(&project, &context(&cache), &quiet, 1, &mut |_| true).unwrap_err();
    assert_eq!(error.code(), "fosscut.beats.none");
    assert_eq!(error.details(), json!({ "asset": "asset_0002" }));
}
