//! Reframing, vertical copies and camera tracking (CC-084, CC-175, CC-087) from analyses
//! written straight into the cache, so no model runs: a person still for two seconds, then a
//! cut, then walking right; a matte of a square moving across; and a picture with nobody in
//! it. The returned calls are applied through the command framework and the clips' keyframes
//! are checked frame by frame: the subject stays near the middle of the canvas and inside it,
//! the picture always covers the canvas, and the framing jumps only at the cut. A region
//! is also followed on generated media (with FFmpeg): a textured patch moving across.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use fosscut_analysis::matte::{MatteFrame, MatteModel, MatteTrack};
use fosscut_analysis::pose::{LANDMARKS, Landmark, PoseFrame, PoseTrack};
use fosscut_analysis::scenes::Cut;
use fosscut_analysis::vision::{DataFile, pack};
use fosscut_analysis::{Analyzer, Moment, Options, Scenes, cache};
use fosscut_core::{Request, Session};
use fosscut_engine::reframe::{
    ReframeRequest, SubjectSource, VerticalRequest, camera_track, reframe, vertical,
};
use fosscut_model::{
    Asset, AssetKind, Clip, ClipContent, ContentIdentity, Id, IdKind, Interpolation, MediaInfo,
    MemoryStore, PathHints, Project, TimelineFormat, Track, TrackKind, VideoInfo,
};
use fosscut_time::{Flicks, Rate};
use serde_json::json;

fn frames(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

fn identity(n: u64) -> ContentIdentity {
    ContentIdentity {
        size: 9000 + n,
        sample_blake3: format!("{:02x}", 0x30 + n).repeat(32),
    }
}

/// An empty cache folder of its own.
fn fresh_cache(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-reframe-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn store(
    cache: &Path,
    asset: u64,
    analyzer: Analyzer,
    result: serde_json::Value,
    data: Option<&[u8]>,
) {
    let key = Options::default().for_analyzer(analyzer);
    let path = cache::entry_path(cache, &identity(asset), analyzer, &key);
    if let Some(data) = data {
        cache::write_data(&path, data).unwrap();
    }
    cache::write(&path, analyzer, &key, &result).unwrap();
}

/// Where the person's nose is at frame `i`: still at 0.3 for two seconds, then (after the
/// cut) walking from 0.55 to 0.75.
fn nose_x(i: usize) -> f32 {
    if i < 60 {
        0.3
    } else {
        0.55 + 0.2 * (i - 60) as f32 / 59.0
    }
}

/// A pose track of 120 frames at 30 fps: a person whose nose is at `x(i)`, or nobody.
fn pose_track(x: impl Fn(usize) -> Option<f32>) -> (PoseTrack, Vec<u8>) {
    let mask = pack(&[0u8; 16]);
    let frames = (0..120)
        .map(|i| {
            let mut landmarks = [Landmark::default(); LANDMARKS];
            let presence = match x(i) {
                Some(x) => {
                    let mut put = |n: usize, px: f32, py: f32| {
                        landmarks[n] = Landmark {
                            x: px,
                            y: py,
                            visibility: 0.95,
                            presence: 0.95,
                        };
                    };
                    put(0, x, 0.3);
                    put(2, x - 0.01, 0.28);
                    put(5, x + 0.01, 0.28);
                    put(11, x - 0.06, 0.45);
                    put(12, x + 0.06, 0.45);
                    put(23, x - 0.04, 0.8);
                    put(24, x + 0.04, 0.8);
                    0.95
                }
                None => 0.0,
            };
            PoseFrame {
                time: frames(i as i64),
                presence,
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
    (track, data)
}

fn cache_pose(cache: &Path, asset: u64, x: impl Fn(usize) -> Option<f32>) {
    let (track, data) = pose_track(x);
    let summary = track.summary(DataFile::of(&data));
    store(
        cache,
        asset,
        Analyzer::Pose,
        serde_json::to_value(&summary).unwrap(),
        Some(&data),
    );
}

fn cache_scenes(cache: &Path, asset: u64, cuts: &[i64]) {
    let scenes = Scenes {
        threshold: 0.35,
        min_shot_ms: 500,
        frames: 120,
        cuts: cuts
            .iter()
            .map(|&f| Cut {
                frame: f as u64,
                time: Moment::new(frames(f)),
                score: 1.0,
            })
            .collect(),
    };
    store(
        cache,
        asset,
        Analyzer::Scenes,
        serde_json::to_value(&scenes).unwrap(),
        None,
    );
}

/// A matte track: a square of a fifth of the height moving from 0.2 to 0.8 across.
fn cache_square_matte(cache: &Path, asset: u64) {
    let (w, h) = (128u32, 72u32);
    let frames = (0..120)
        .map(|i| {
            let cx = (0.2 + 0.6 * i as f64 / 119.0) * f64::from(w);
            let alpha: Vec<u8> = (0..w * h)
                .map(|p| {
                    let (x, y) = (f64::from(p % w) + 0.5, f64::from(p / w) + 0.5);
                    if (x - cx).abs() < 7.2 && (y - 36.0).abs() < 7.2 {
                        255
                    } else {
                        0
                    }
                })
                .collect();
            MatteFrame {
                time: frames(i),
                key: true,
                alpha: pack(&alpha),
            }
        })
        .collect();
    let track = MatteTrack {
        size: (w, h),
        frames,
    };
    let data = track.encode();
    let summary = track.summary(MatteModel::Person, DataFile::of(&data));
    store(
        cache,
        asset,
        Analyzer::Matte,
        serde_json::to_value(&summary).unwrap(),
        Some(&data),
    );
}

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
        identity: Some(identity(n)),
        info: MediaInfo {
            duration: Some(frames(120)),
            video: Some(VideoInfo {
                width: 1920,
                height: 1080,
                rate: Some(Rate::FPS_30),
                codec: None,
                rotation: 0,
            }),
            ..MediaInfo::default()
        },
    }
}

fn video_clip(id: u64, asset: u64, start: i64, length: i64) -> Clip {
    Clip::new(
        Id::new(IdKind::Clip, id),
        frames(start),
        frames(length),
        ClipContent::Video {
            asset: Id::new(IdKind::Asset, asset),
            source_in: Flicks::ZERO,
        },
    )
}

/// A project on a `width` by `height` canvas whose video track holds `clips`.
fn project(width: u32, height: u32, clips: Vec<Clip>) -> Project {
    let format = TimelineFormat {
        width,
        height,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Show", format);
    project
        .media
        .assets
        .extend([video_asset(1), video_asset(2), video_asset(3)]);
    let timeline = project.manifest.primary_timeline.clone();
    project.timelines.get_mut(&timeline).unwrap().tracks = vec![Track {
        id: Id::new(IdKind::Track, 10),
        kind: TrackKind::Video,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        caption_style: None,
        clips,
    }];
    project.manifest.next_id = 100;
    project
}

fn request(cache: &Path, source: SubjectSource) -> ReframeRequest {
    ReframeRequest {
        bundle: std::env::temp_dir().join("Show.fosscut"),
        cache: cache.to_path_buf(),
        source,
        framing: Default::default(),
        shots: true,
    }
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

/// Where picture point `u` (fraction of the width) lands on a canvas whose fitted picture
/// is `fit` canvas widths wide, at frame `i` of `clip`: fractions of the canvas from its
/// centre, with the scale.
fn on_canvas(clip: &Clip, i: i64, u: f64, fit: (f64, f64)) -> (f64, [f64; 2], f64) {
    let t = clip.transform_at(frames(i));
    (
        t.position[0] + (u - 0.5) * fit.0 * t.scale,
        t.position,
        t.scale,
    )
}

#[test]
fn a_clip_follows_its_person_on_a_vertical_canvas_and_cuts_jump() {
    let cache = fresh_cache("person");
    cache_pose(&cache, 1, |i| Some(nose_x(i)));
    cache_scenes(&cache, 1, &[60]);
    let project = project(1080, 1920, vec![video_clip(20, 1, 0, 120)]);
    let clip = Id::new(IdKind::Clip, 20);
    let plan = reframe(
        &project,
        &clip,
        &request(&cache, SubjectSource::Person),
        &mut |_| true,
    )
    .unwrap();
    assert_eq!(plan.frames, 120);
    assert_eq!(plan.subject_frames, 120);
    assert_eq!(plan.shots.len(), 2);
    assert_eq!(
        (plan.shots[0].start, plan.shots[0].end),
        (Flicks::ZERO, frames(60))
    );
    assert!(
        plan.shots
            .iter()
            .all(|s| s.inside_frames == s.subject_frames && s.zoom == 1.0)
    );
    assert_eq!(plan.commands.len(), 2);
    let framed = apply(project, &plan.commands);
    let clip = clip_of(&framed, &clip);
    // 16:9 fitted into 9:16 is one canvas wide and 0.316 high.
    let fit = (1.0, (1080.0 / 1920.0) / (1920.0 / 1080.0));
    for i in 0..120 {
        let (x, position, scale) = on_canvas(clip, i, f64::from(nose_x(i as usize)), fit);
        assert!(
            (scale * fit.1 - 1.0).abs() < 1e-6,
            "frame {i}: the height fills: {scale}"
        );
        assert!(
            position[0].abs() <= (scale * fit.0 - 1.0) / 2.0 + 1e-6,
            "frame {i}: the picture covers the canvas"
        );
        assert!(x.abs() < 0.15, "frame {i}: the face is {x} from the centre");
        // The shoulders stay on the canvas.
        for side in [-0.06, 0.06] {
            let (s, _, _) = on_canvas(clip, i, f64::from(nose_x(i as usize)) + side, fit);
            assert!(
                s.abs() <= 0.5,
                "frame {i}: a shoulder is off the canvas at {s}"
            );
        }
    }
    // Still in the first shot, then a jump at the cut, held exactly to it.
    let x = |i: i64| clip.transform_at(frames(i)).position[0];
    assert!((0..60).all(|i| (x(i) - x(0)).abs() < 1e-9));
    assert!((x(60) - x(59)).abs() > 0.3);
    let keys = &clip.keyframes["transform.position"];
    let hold = keys.iter().find(|k| k.time == frames(59)).unwrap();
    assert_eq!(hold.interpolation, Interpolation::Hold);
    assert!(
        keys.len() < 60,
        "keys only where the path bends: {}",
        keys.len()
    );
}

#[test]
fn camera_tracking_zooms_in_place_and_follows() {
    let cache = fresh_cache("camera");
    cache_pose(&cache, 1, |i| Some(0.3 + 0.4 * i as f32 / 119.0));
    let project = project(1920, 1080, vec![video_clip(20, 1, 0, 120)]);
    let clip = Id::new(IdKind::Clip, 20);
    let mut one_shot = request(&cache, SubjectSource::Person);
    one_shot.shots = false;
    let plan = camera_track(&project, &clip, 1.5, &one_shot, &mut |_| true).unwrap();
    assert_eq!(plan.shots.len(), 1);
    assert_eq!(plan.shots[0].zoom, 1.5);
    let tracked = apply(project.clone(), &plan.commands);
    let clip_doc = clip_of(&tracked, &clip);
    for i in (0..120).step_by(7) {
        let u = 0.3 + 0.4 * i as f64 / 119.0;
        let (x, position, scale) = on_canvas(clip_doc, i, u, (1.0, 1.0));
        assert!((scale - 1.5).abs() < 1e-6);
        assert!(position[0].abs() <= 0.25 + 1e-6, "covers the canvas");
        assert!(x.abs() < 0.2, "frame {i}: {x}");
    }
    let refused = camera_track(&project, &clip, 0.5, &one_shot, &mut |_| true).unwrap_err();
    assert_eq!(refused.code(), "fosscut.request.invalid_arguments");
}

#[test]
fn a_matte_is_followed_and_missing_analyses_are_named() {
    let cache = fresh_cache("matte");
    let scene = project(1080, 1920, vec![video_clip(20, 1, 0, 120)]);
    let clip = Id::new(IdKind::Clip, 20);
    let person = request(&cache, SubjectSource::Person);
    let missing = reframe(&scene, &clip, &person, &mut |_| true).unwrap_err();
    assert_eq!(missing.code(), "fosscut.analysis.missing");
    assert_eq!(missing.details()["analyzer"], "pose");
    assert_eq!(missing.details()["effect"], "reframe");
    let subject = request(&cache, SubjectSource::Subject);
    let missing = reframe(&scene, &clip, &subject, &mut |_| true).unwrap_err();
    assert_eq!(missing.details()["analyzer"], "matte");
    assert_eq!(
        missing.details()["options"],
        json!({ "matte": { "model": "subject" } })
    );
    cache_square_matte(&cache, 1);
    let missing = reframe(&scene, &clip, &subject, &mut |_| true).unwrap_err();
    assert_eq!(missing.details()["analyzer"], "scenes", "{missing}");
    cache_scenes(&cache, 1, &[]);
    let plan = reframe(&scene, &clip, &subject, &mut |_| true).unwrap();
    assert_eq!(plan.shots.len(), 1);
    let framed = apply(scene.clone(), &plan.commands);
    let clip_doc = clip_of(&framed, &clip);
    let fit = (1.0, (1080.0 / 1920.0) / (1920.0 / 1080.0));
    for i in 0..120 {
        let u = 0.2 + 0.6 * i as f64 / 119.0;
        let (x, _, _) = on_canvas(clip_doc, i, u, fit);
        assert!(
            x.abs() < 0.18,
            "frame {i}: the square is {x} from the centre"
        );
    }
    // Nobody in the picture.
    cache_pose(&cache, 2, |_| None);
    cache_scenes(&cache, 2, &[]);
    let empty = project(1080, 1920, vec![video_clip(21, 2, 0, 120)]);
    let none = reframe(&empty, &Id::new(IdKind::Clip, 21), &person, &mut |_| true).unwrap_err();
    assert_eq!(none.code(), "fosscut.reframe.no_subject");
    let clip = Id::new(IdKind::Clip, 21).to_string();
    assert_eq!(none.details(), json!({ "clip": clip, "subject": "person" }));
}

#[test]
fn a_vertical_copy_reframes_every_clip_and_keeps_the_original() {
    let cache = fresh_cache("vertical");
    cache_pose(&cache, 1, |i| Some(nose_x(i)));
    cache_scenes(&cache, 1, &[60]);
    cache_pose(&cache, 2, |_| None);
    cache_scenes(&cache, 2, &[]);
    let wide = project(
        1920,
        1080,
        vec![video_clip(20, 1, 0, 120), video_clip(21, 2, 120, 60)],
    );
    let original = wide.timelines[&wide.manifest.primary_timeline].clone();
    let job = VerticalRequest {
        reframe: request(&cache, SubjectSource::Person),
        width: 1080,
        height: 1920,
        name: None,
    };
    let plan = vertical(&wide, None, &job, &mut |_| true).unwrap();
    assert_eq!(plan.commands[0]["command"], "timeline.duplicate");
    assert_eq!(plan.commands[0]["args"]["fit"], "fill");
    assert_eq!(plan.clips.len(), 1);
    assert!(
        plan.clips[0].commands.is_empty(),
        "the calls are in commands"
    );
    assert_eq!(plan.centred.len(), 1);
    assert_eq!(plan.centred[0].reason, "fosscut.reframe.no_subject");
    assert_eq!(plan.commands.len(), 3);
    let done = apply(wide.clone(), &plan.commands);
    let copy = &done.timelines[&plan.timeline];
    assert_eq!((copy.format.width, copy.format.height), (1080, 1920));
    assert_eq!(copy.name, format!("{} vertical", original.name));
    assert_eq!(done.timelines[&wide.manifest.primary_timeline], original);
    let followed = clip_of(&done, &plan.clips[0].clip);
    assert!(followed.keyframes.contains_key("transform.position"));
    let centred = clip_of(&done, &plan.centred[0].clip);
    assert!(centred.keyframes.is_empty());
    assert!(
        (centred.transform.scale - 3.160_493_827).abs() < 1e-6,
        "filled by the copy"
    );
    // A region belongs to one clip.
    let region = VerticalRequest {
        reframe: request(
            &cache,
            SubjectSource::Region {
                region: fosscut_analysis::Region {
                    x: 0.5,
                    y: 0.5,
                    width: 0.1,
                    height: 0.1,
                },
                at: None,
            },
        ),
        ..job.clone()
    };
    let refused = vertical(&wide, None, &region, &mut |_| true).unwrap_err();
    assert_eq!(refused.code(), "fosscut.request.invalid_arguments");
    // Without the scenes of the second clip's media, the plan names them.
    let bare = fresh_cache("vertical-bare");
    cache_pose(&bare, 1, |i| Some(nose_x(i)));
    cache_scenes(&bare, 1, &[60]);
    let job = VerticalRequest {
        reframe: request(&bare, SubjectSource::Person),
        ..job
    };
    let missing = vertical(&wide, None, &job, &mut |_| true).unwrap_err();
    assert_eq!(missing.code(), "fosscut.analysis.missing");
    assert_eq!(missing.details()["asset"], "asset_0002");
}

fn required(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty() && v != "0")
}

fn ffmpeg(dir: &Path, args: &[&str]) {
    let status = Command::new("ffmpeg")
        .current_dir(dir)
        .args(["-y", "-v", "error"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
}

#[test]
fn a_tracked_region_is_followed_on_a_tall_canvas() {
    let found = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        found || !required("FOSSCUT_REQUIRE_FFMPEG"),
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    if !found {
        return;
    }
    let dir = fresh_cache("tracking-media");
    std::fs::create_dir_all(dir.join("Show.fosscut")).unwrap();
    ffmpeg(
        &dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:size=60x34:rate=30:duration=1,noise=alls=100:allf=0:all_seed=5,scale=480:270:flags=bicubic,eq=contrast=2",
            "-frames:v",
            "1",
            "texture.png",
        ],
    );
    // A 40-pixel patch of texture moving right by 100 pixels a second over a flat grey.
    ffmpeg(
        &dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=0x404040:size=320x180:rate=30:duration=2",
            "-loop",
            "1",
            "-i",
            "texture.png",
            "-filter_complex",
            "[1]crop=40:40:200:100[patch];[0][patch]overlay=x='40+100*t':y=70:shortest=1:format=yuv444,format=yuv420p",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "moving.mp4",
        ],
    );
    let mut scene = project(180, 320, vec![video_clip(20, 1, 0, 60)]);
    let asset = &mut scene.media.assets[0];
    asset.path.relative = Some("../moving.mp4".into());
    asset.identity = None;
    let info = asset.info.video.as_mut().unwrap();
    (info.width, info.height) = (320, 180);
    asset.info.duration = Some(frames(60));
    let centre = |i: i64| (40.0 + 100.0 * i as f64 / 30.0 + 20.0) / 320.0;
    let mut job = ReframeRequest {
        bundle: dir.join("Show.fosscut"),
        ..request(
            &dir,
            SubjectSource::Region {
                region: fosscut_analysis::Region {
                    x: centre(0),
                    y: 90.0 / 180.0,
                    width: 40.0 / 320.0,
                    height: 40.0 / 180.0,
                },
                at: None,
            },
        )
    };
    job.shots = false;
    job.framing.max_speed = 2.0;
    let clip = Id::new(IdKind::Clip, 20);
    let plan = reframe(&scene, &clip, &job, &mut |_| true).unwrap();
    assert_eq!(plan.subject_frames, 60, "found in every frame");
    let framed = apply(scene, &plan.commands);
    let clip_doc = clip_of(&framed, &clip);
    let fit = (1.0, (180.0 / 320.0) / (320.0 / 180.0));
    for i in 0..60 {
        let (x, _, _) = on_canvas(clip_doc, i, centre(i), fit);
        assert!(x.abs() < 0.2, "frame {i}: the patch is {x} from the centre");
    }
}
