//! Measured motion on generated media (ADR-0030) and multicam sync (ADR-0029): a clip cut
//! from a still picture with a known shake is steadied by keyframes that cancel the shake, a
//! patch moving on a known path is followed by a mask and by a title, and two recordings of
//! the same sound are lined up.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use common::{frames, required, workspace};
use fosscut_analysis::Region;
use fosscut_core::{Request, Session};
use fosscut_engine::motion::{TrackTarget, stabilize, sync_offsets, track};
use fosscut_model::{
    Asset, AssetKind, AudioInfo, Clip, ClipContent, Id, IdKind, KeyValue, Mask, MaskShape,
    MediaInfo, MemoryStore, PathHints, Project, TextContent, TimelineFormat, Track, TrackKind,
    VideoInfo,
};
use fosscut_time::{Flicks, Rate};
use serde_json::json;

fn ffmpeg(args: &[&str]) {
    let status = Command::new("ffmpeg")
        .current_dir(workspace())
        .args(["-y", "-v", "error"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
}

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

/// The generated files, made once: a still texture (seeded noise at two scales, like the
/// detail of a real scene), a shaky view of it, a patch moving over
/// a flat background, and a sound with a copy that started 1.5 s later.
fn media() -> &'static Path {
    static MADE: OnceLock<PathBuf> = OnceLock::new();
    MADE.get_or_init(|| {
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:size=60x34:rate=30:duration=1,noise=alls=100:allf=0:all_seed=5,scale=480:270:flags=bicubic,eq=contrast=2[coarse];color=c=gray:size=480x270:rate=30:duration=1,noise=alls=100:allf=0:all_seed=9,gblur=sigma=1.2[fine];[coarse][fine]blend=all_mode=average",
            "-frames:v",
            "1",
            "texture.png",
        ]);
        ffmpeg(&[
            "-loop",
            "1",
            "-framerate",
            "30",
            "-i",
            "texture.png",
            "-vf",
            "crop=w=320:h=180:x='80+24*sin(2*PI*2.5*t)':y='45+12*sin(2*PI*1.7*t+1)':exact=1,format=yuv420p",
            "-r",
            "30",
            "-t",
            "2",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "shaky.mp4",
        ]);
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "color=c=0x404040:size=320x180:rate=30:duration=2",
            "-loop",
            "1",
            "-i",
            "texture.png",
            "-filter_complex",
            "[1]crop=40:40:200:100[patch];[0][patch]overlay=x='40+100*t':y='70+20*sin(2*PI*t)':shortest=1:format=yuv444,format=yuv420p",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "moving.mp4",
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
            "later.wav",
        ]);
        workspace().to_path_buf()
    })
}

fn bundle() -> PathBuf {
    workspace().join("Show.fosscut")
}

fn video_asset(n: u64, file: &str) -> Asset {
    Asset {
        folder: None,
        id: Id::new(IdKind::Asset, n),
        kind: AssetKind::Video,
        name: file.into(),
        path: PathHints {
            relative: Some(format!("../{file}")),
            absolute: None,
        },
        identity: None,
        info: MediaInfo {
            duration: Some(frames(60)),
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

fn audio_asset(n: u64, file: &str, seconds: i64) -> Asset {
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
            duration: Some(Flicks::from_seconds(seconds).unwrap()),
            audio: Some(AudioInfo {
                sample_rate: 48_000,
                channels: 1,
                codec: None,
            }),
            ..MediaInfo::default()
        },
    }
}

fn track_of(n: u64, kind: TrackKind, clips: Vec<Clip>) -> Track {
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

/// A 2 s timeline at 320x180: the shaky clip, the moving patch with a mask over it, and a
/// title above.
fn project() -> Project {
    let format = TimelineFormat {
        width: 320,
        height: 180,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Show", format);
    project.media.assets.extend([
        video_asset(1, "shaky.mp4"),
        video_asset(2, "moving.mp4"),
        audio_asset(3, "room.wav", 8),
        audio_asset(4, "later.wav", 6),
    ]);
    let video = |n: u64, asset: u64| {
        Clip::new(
            Id::new(IdKind::Clip, n),
            Flicks::ZERO,
            frames(60),
            ClipContent::Video {
                asset: Id::new(IdKind::Asset, asset),
                source_in: Flicks::ZERO,
            },
        )
    };
    let mut patch = video(21, 2);
    patch
        .masks
        .push(Mask::new(Id::new(IdKind::Mask, 22), MaskShape::Ellipse));
    let mut title = Clip::new(
        Id::new(IdKind::Clip, 31),
        frames(15),
        frames(45),
        ClipContent::Text(Box::new(TextContent {
            text: "Follow".into(),
            ..Default::default()
        })),
    );
    title.transform.position = [0.1, -0.2];
    let timeline = project.manifest.primary_timeline.clone();
    let doc = project.timelines.get_mut(&timeline).unwrap();
    doc.tracks = vec![
        track_of(10, TrackKind::Video, vec![video(11, 1)]),
        track_of(20, TrackKind::Video, vec![patch]),
        track_of(30, TrackKind::Text, vec![title]),
    ];
    project.manifest.next_id = 40;
    project
}

/// Applies `commands` through the command framework and returns the project after them.
fn apply(project: Project, commands: &[serde_json::Value]) -> Project {
    let mut session = Session::create(MemoryStore::new(), project).unwrap();
    let request: Request = serde_json::from_value(json!({ "commands": commands })).unwrap();
    session.execute(request).unwrap();
    session.project().clone()
}

fn clip(project: &Project, id: u64) -> &Clip {
    let id = Id::new(IdKind::Clip, id);
    project
        .timelines
        .values()
        .flat_map(|t| &t.tracks)
        .flat_map(|t| &t.clips)
        .find(|c| c.id == id)
        .unwrap()
}

fn pairs(keys: &[fosscut_model::Keyframe]) -> Vec<(Flicks, [f64; 2])> {
    keys.iter()
        .map(|k| match k.value {
            KeyValue::Pair(pair) => (k.time, pair),
            ref other => panic!("not a pair: {other:?}"),
        })
        .collect()
}

fn seconds(time: Flicks) -> f64 {
    time.get() as f64 / 705_600_000.0
}

#[test]
fn a_shaky_clip_is_steadied_by_keyframes_that_cancel_the_shake() {
    if !available() {
        return;
    }
    media();
    let project = project();
    let shaky = Id::new(IdKind::Clip, 11);
    let plan = stabilize(&project, &bundle(), &shaky, 1.0, &mut |_| true).unwrap();
    assert_eq!(plan.clip, shaky);
    assert_eq!(plan.frames, 60);
    let zoom = plan.zoom.unwrap();
    assert!(zoom > 1.0 && zoom <= 1.5, "zoom {zoom}");
    assert_eq!(plan.commands.len(), 3);
    let steadied = apply(project, &plan.commands);
    let keys = &clip(&steadied, 11).keyframes;
    let position = pairs(&keys["transform.position"]);
    assert_eq!(position.len(), 60, "one key per frame");
    // The view moved right by the shake, so the picture moves right by it to cancel it,
    // scaled by the zoom: compare with the shake after removing both means.
    let shake = |t: f64| {
        (
            24.0 * (2.0 * std::f64::consts::PI * 2.5 * t).sin() / 320.0,
            12.0 * (2.0 * std::f64::consts::PI * 1.7 * t + 1.0).sin() / 180.0,
        )
    };
    // What is left after cancelling the shake (scaled by the zoom), in pixels, over the
    // middle frames: the smoothed camera path may drift slowly, as it keeps slow motion,
    // but nothing of the shake may remain.
    let middle = &position[10..50];
    for (axis, size) in [(0, 320.0), (1, 180.0)] {
        let left: Vec<(f64, f64)> = middle
            .iter()
            .map(|(time, p)| {
                let t = seconds(*time);
                let wanted = [shake(t).0, shake(t).1][axis];
                (t, (p[axis] - zoom * wanted) * size)
            })
            .collect();
        let n = left.len() as f64;
        let (mt, mr) = (
            left.iter().map(|l| l.0).sum::<f64>() / n,
            left.iter().map(|l| l.1).sum::<f64>() / n,
        );
        let slope = left.iter().map(|l| (l.0 - mt) * (l.1 - mr)).sum::<f64>()
            / left.iter().map(|l| (l.0 - mt).powi(2)).sum::<f64>();
        let worst = left
            .iter()
            .map(|l| (l.1 - mr - slope * (l.0 - mt)).abs())
            .fold(0.0, f64::max);
        assert!(worst < 1.5, "axis {axis}: worst residual {worst} px");
        assert!(slope.abs() < 10.0, "axis {axis}: drift {slope} px/s");
    }
    let scale = &keys["transform.scale"];
    assert!(
        scale
            .iter()
            .all(|k| (k.value.number().unwrap() - zoom).abs() < 0.01),
        "the zoom holds"
    );
    assert!(keys.contains_key("transform.rotation"));

    // Strength 0 changes nothing but still replaces the keys with the clip's placement.
    let still = stabilize(&steadied, &bundle(), &shaky, 0.0, &mut |_| true).unwrap();
    assert_eq!(still.zoom, Some(1.0));
    let steady = apply(steadied, &still.commands);
    let position = pairs(&clip(&steady, 11).keyframes["transform.position"]);
    assert!(
        position
            .iter()
            .all(|(_, p)| p[0].abs() < 1e-9 && p[1].abs() < 1e-9)
    );

    let refused = stabilize(&steady, &bundle(), &shaky, 4.0, &mut |_| true).unwrap_err();
    assert_eq!(refused.code(), "fosscut.request.invalid_arguments");
    let title = Id::new(IdKind::Clip, 31);
    let refused = stabilize(&steady, &bundle(), &title, 1.0, &mut |_| true).unwrap_err();
    assert_eq!(refused.code(), "fosscut.request.invalid_arguments");
    let cancelled = stabilize(&steady, &bundle(), &shaky, 1.0, &mut |_| false).unwrap_err();
    assert_eq!(cancelled.code(), "fosscut.analysis.cancelled");
}

/// Where the patch's centre is at `t`, as fractions of the picture: `overlay` places it
/// at whole pixels, truncating.
fn patch_centre(t: f64) -> (f64, f64) {
    let x = (40.0 + 100.0 * t + 1e-9).floor();
    let y = (70.0 + 20.0 * (2.0 * std::f64::consts::PI * t).sin() + 1e-9).floor();
    ((x + 20.0) / 320.0, (y + 20.0) / 180.0)
}

#[test]
fn a_mask_and_a_title_follow_a_moving_patch() {
    if !available() {
        return;
    }
    media();
    let project = project();
    let patch = Id::new(IdKind::Clip, 21);
    let (x, y) = patch_centre(0.0);
    let region = Region {
        x,
        y,
        width: 40.0 / 320.0,
        height: 40.0 / 180.0,
    };
    let mask = Id::new(IdKind::Mask, 22);
    let plan = track(
        &project,
        &bundle(),
        &patch,
        region,
        Flicks::ZERO,
        &TrackTarget::Mask(mask.clone()),
        &mut |_| true,
    )
    .unwrap();
    assert_eq!(plan.clip, patch);
    assert!(plan.lost.is_empty(), "{:?}", plan.lost);
    assert_eq!(
        plan.commands[0]["args"]["property"],
        format!("masks.{mask}.center")
    );
    let masked = apply(project.clone(), &plan.commands);
    let centres = pairs(&clip(&masked, 21).masks[0].keyframes["center"]);
    assert_eq!(centres.len(), 60);
    for (time, [cx, cy]) in &centres {
        let (ex, ey) = patch_centre(seconds(*time));
        assert!(
            ((cx - ex) * 320.0).abs() < 1.5 && ((cy - ey) * 180.0).abs() < 1.5,
            "at {}: {cx}, {cy} for {ex}, {ey}",
            seconds(*time)
        );
    }

    // The title starts at 0.5 s; tracking from there moves it as the patch moves on the
    // canvas, from where it is.
    let title = Id::new(IdKind::Clip, 31);
    let at = frames(15);
    let (x, y) = patch_centre(0.5);
    let plan = track(
        &project,
        &bundle(),
        &patch,
        Region { x, y, ..region },
        at,
        &TrackTarget::Clip(title.clone()),
        &mut |_| true,
    )
    .unwrap();
    assert_eq!(plan.clip, title);
    let followed = apply(project.clone(), &plan.commands);
    let positions = pairs(&clip(&followed, 31).keyframes["transform.position"]);
    assert_eq!(positions.len(), 45, "the title's frames from 0.5 s");
    assert_eq!(positions[0].0, Flicks::ZERO, "keys from the title's start");
    for (offset, [px, py]) in &positions {
        let t = seconds(at + *offset);
        let (ex, ey) = patch_centre(t);
        let (x0, y0) = patch_centre(0.5);
        let (wx, wy) = (0.1 + ex - x0, -0.2 + ey - y0);
        assert!(
            ((px - wx) * 320.0).abs() < 1.5 && ((py - wy) * 180.0).abs() < 1.5,
            "at {t}: {px}, {py} for {wx}, {wy}"
        );
    }

    let errors = [
        track(
            &project,
            &bundle(),
            &patch,
            Region {
                width: 0.0,
                ..region
            },
            Flicks::ZERO,
            &TrackTarget::Mask(mask.clone()),
            &mut |_| true,
        ),
        track(
            &project,
            &bundle(),
            &patch,
            region,
            frames(90),
            &TrackTarget::Mask(mask.clone()),
            &mut |_| true,
        ),
        track(
            &project,
            &bundle(),
            &patch,
            region,
            Flicks::ZERO,
            &TrackTarget::Mask(Id::new(IdKind::Mask, 99)),
            &mut |_| true,
        ),
        track(
            &project,
            &bundle(),
            &patch,
            region,
            Flicks::ZERO,
            &TrackTarget::Clip(patch.clone()),
            &mut |_| true,
        ),
    ];
    for error in errors {
        assert_eq!(
            error.unwrap_err().code(),
            "fosscut.request.invalid_arguments"
        );
    }
}

#[test]
fn recordings_of_the_same_sound_are_lined_up() {
    if !available() {
        return;
    }
    media();
    let project = project();
    let angles = [Id::new(IdKind::Asset, 3), Id::new(IdKind::Asset, 4)];
    let found = sync_offsets(&project, &bundle(), &angles, 0.0).unwrap();
    assert_eq!(found.offsets[0], Flicks::ZERO);
    let offset = seconds(found.offsets[1]);
    assert!((offset - 1.5).abs() < 0.002, "offset {offset}");
    assert!(found.confidence[1] > 0.5, "{:?}", found.confidence);

    // The other way round, the first angle started later.
    let reversed = [angles[1].clone(), angles[0].clone()];
    let found = sync_offsets(&project, &bundle(), &reversed, 3.0).unwrap();
    assert!((seconds(found.offsets[1]) + 1.5).abs() < 0.002);

    let one = sync_offsets(&project, &bundle(), &angles[..1], 0.0).unwrap_err();
    assert_eq!(one.code(), "fosscut.request.invalid_arguments");
    let silent = [angles[0].clone(), Id::new(IdKind::Asset, 1)];
    let silent = sync_offsets(&project, &bundle(), &silent, 0.0).unwrap_err();
    assert_eq!(silent.code(), "fosscut.analysis.no_audio");
}
