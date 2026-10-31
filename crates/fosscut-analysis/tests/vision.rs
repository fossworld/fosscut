//! The pose and matte analyzers with the real models (ADR-0033, validation) on drawn
//! scenes: a person-like figure on a plain background, and a vase. The figure is simple
//! shapes in skin, shirt and trouser colours, which MediaPipe's detector takes for a person;
//! its silhouette is known exactly, so the masks and landmarks can be checked against it.
//!
//! They need the `onnxruntime`, `mediapipe-pose-full` and `birefnet-lite` models in
//! `FOSSCUT_MODELS_DIR` and skip without them; `FOSSCUT_REQUIRE_MODELS` turns a missing one
//! into a failure. The end-to-end test also needs FFmpeg (`FOSSCUT_REQUIRE_FFMPEG`).
//! Timings print with `--nocapture`.

#![cfg(feature = "onnx")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use fosscut_analysis::decode::{Media, analyze};
use fosscut_analysis::matte::{MatteAnalyzer, MatteModel, MatteOptions, MatteTrack};
use fosscut_analysis::models;
use fosscut_analysis::pose::{MIN_PRESENCE, PoseAnalyzer, PoseOptions, PoseTrack};
use fosscut_analysis::vision::Rgb;
use fosscut_analysis::{AnalysisError, Analyzer, Options};
use fosscut_media::{
    ColorInfo, Matrix, PixelFormat, Plane, Primaries, Range, Transfer, VideoFrame,
};
use fosscut_time::{Flicks, Rate};

const W: u32 = 640;
const H: u32 = 360;

fn required(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty() && v != "0")
}

/// The models folder when the runtime and every model in `names` are installed.
fn installed(names: &[&str]) -> Option<PathBuf> {
    let dir = std::env::var_os("FOSSCUT_MODELS_DIR").map(PathBuf::from);
    let ready = dir.as_ref().is_some_and(|d| {
        fosscut_analysis::onnx::runtime_path(d).is_some()
            && names
                .iter()
                .all(|n| models::is_installed(d, models::manifest(n).unwrap()))
    });
    assert!(
        ready || !required("FOSSCUT_REQUIRE_MODELS"),
        "FOSSCUT_REQUIRE_MODELS is set but ONNX Runtime or {names:?} is not installed in FOSSCUT_MODELS_DIR"
    );
    ready.then(|| dir.unwrap())
}

/// A filled shape of a drawn scene.
enum Shape {
    Ellipse {
        c: (f32, f32),
        r: (f32, f32),
    },
    /// A thick line with round ends.
    Capsule {
        a: (f32, f32),
        b: (f32, f32),
        r: f32,
    },
    Polygon(Vec<(f32, f32)>),
}

impl Shape {
    fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            Shape::Ellipse { c, r } => {
                let (dx, dy) = ((x - c.0) / r.0, (y - c.1) / r.1);
                dx * dx + dy * dy <= 1.0
            }
            Shape::Capsule { a, b, r } => {
                let (vx, vy) = (b.0 - a.0, b.1 - a.1);
                let t = (((x - a.0) * vx + (y - a.1) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
                let (px, py) = (a.0 + t * vx - x, a.1 + t * vy - y);
                px * px + py * py <= r * r
            }
            Shape::Polygon(points) => {
                let mut inside = false;
                let mut j = points.len() - 1;
                for i in 0..points.len() {
                    let (pi, pj) = (points[i], points[j]);
                    if (pi.1 > y) != (pj.1 > y)
                        && x < (pj.0 - pi.0) * (y - pi.1) / (pj.1 - pi.1) + pi.0
                    {
                        inside = !inside;
                    }
                    j = i;
                }
                inside
            }
        }
    }
}

/// A scene: a background and shapes drawn in order, with which belong to the subject.
struct Scene {
    background: [u8; 3],
    shapes: Vec<(Shape, [u8; 3])>,
}

impl Scene {
    fn at(&self, x: u32, y: u32) -> ([u8; 3], bool) {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut colour = self.background;
        let mut subject = false;
        for (shape, c) in &self.shapes {
            if shape.contains(fx, fy) {
                colour = *c;
                subject = true;
            }
        }
        (colour, subject)
    }

    fn picture(&self) -> Rgb {
        Rgb::from_fn(W, H, |x, y| self.at(x, y).0)
    }

    /// Whether the pixel at fractions `(u, v)` of the frame is the subject.
    fn subject(&self, u: f32, v: f32) -> bool {
        let x = ((u * W as f32) as u32).min(W - 1);
        let y = ((v * H as f32) as u32).min(H - 1);
        self.at(x, y).1
    }
}

/// A person standing in the middle of the frame, moved `dx` pixels right.
fn figure(dx: f32) -> Scene {
    let skin = [224, 172, 140];
    let shirt = [200, 60, 50];
    let trousers = [40, 50, 90];
    let x = |v: f32| 320.0 + dx + v;
    let mut shapes = vec![
        (
            Shape::Ellipse {
                c: (x(0.0), 58.0),
                r: (23.0, 22.0),
            },
            [50, 30, 20],
        ),
        (
            Shape::Ellipse {
                c: (x(0.0), 70.0),
                r: (21.0, 22.0),
            },
            skin,
        ),
        (
            Shape::Ellipse {
                c: (x(-9.0), 66.0),
                r: (3.0, 3.0),
            },
            [30, 30, 30],
        ),
        (
            Shape::Ellipse {
                c: (x(9.0), 66.0),
                r: (3.0, 3.0),
            },
            [30, 30, 30],
        ),
        (
            Shape::Capsule {
                a: (x(0.0), 90.0),
                b: (x(0.0), 104.0),
                r: 8.0,
            },
            skin,
        ),
        (
            Shape::Polygon(vec![
                (x(-45.0), 104.0),
                (x(45.0), 104.0),
                (x(36.0), 200.0),
                (x(-36.0), 200.0),
            ]),
            shirt,
        ),
        (
            Shape::Polygon(vec![
                (x(-36.0), 195.0),
                (x(36.0), 195.0),
                (x(38.0), 215.0),
                (x(-38.0), 215.0),
            ]),
            trousers,
        ),
    ];
    for side in [-1.0f32, 1.0] {
        let s = |v: f32| x(side * v);
        shapes.extend([
            (
                Shape::Capsule {
                    a: (s(45.0), 112.0),
                    b: (s(70.0), 165.0),
                    r: 9.0,
                },
                shirt,
            ),
            (
                Shape::Capsule {
                    a: (s(70.0), 165.0),
                    b: (s(80.0), 215.0),
                    r: 7.5,
                },
                skin,
            ),
            (
                Shape::Ellipse {
                    c: (s(80.0), 221.0),
                    r: (10.0, 11.0),
                },
                skin,
            ),
            (
                Shape::Capsule {
                    a: (s(20.0), 210.0),
                    b: (s(26.0), 275.0),
                    r: 13.0,
                },
                trousers,
            ),
            (
                Shape::Capsule {
                    a: (s(26.0), 275.0),
                    b: (s(30.0), 335.0),
                    r: 11.0,
                },
                trousers,
            ),
            (
                Shape::Ellipse {
                    c: (s(33.0), 337.0),
                    r: (15.0, 9.0),
                },
                [20, 20, 20],
            ),
        ]);
    }
    Scene {
        background: [70, 110, 160],
        shapes,
    }
}

/// A green vase on a beige wall: a subject that is not a person.
fn vase() -> Scene {
    let green = [30, 110, 60];
    Scene {
        background: [220, 215, 200],
        shapes: vec![
            (
                Shape::Ellipse {
                    c: (320.0, 210.0),
                    r: (60.0, 90.0),
                },
                green,
            ),
            (
                Shape::Polygon(vec![
                    (295.0, 60.0),
                    (345.0, 60.0),
                    (345.0, 140.0),
                    (295.0, 140.0),
                ]),
                green,
            ),
            (
                Shape::Ellipse {
                    c: (320.0, 300.0),
                    r: (40.0, 10.0),
                },
                [20, 60, 30],
            ),
        ],
    }
}

/// Agreement of a stored matte (bytes, `size`) with the scene's subject: intersection over
/// union, and the largest value found well outside the subject.
fn agreement(scene: &Scene, alpha: &[u8], size: (u32, u32)) -> (f64, u8) {
    let (mut inter, mut union, mut stray) = (0u32, 0u32, 0u8);
    for j in 0..size.1 {
        for i in 0..size.0 {
            let (u, v) = (
                (i as f32 + 0.5) / size.0 as f32,
                (j as f32 + 0.5) / size.1 as f32,
            );
            let truth = scene.subject(u, v);
            let found = alpha[(j * size.0 + i) as usize] >= 128;
            inter += u32::from(truth && found);
            union += u32::from(truth || found);
            // Ten pixels of the frame away from the subject in every direction.
            let far = (-1..=1).all(|a: i32| {
                (-1..=1).all(|b: i32| {
                    !scene.subject(
                        u + a as f32 * 10.0 / W as f32,
                        v + b as f32 * 10.0 / H as f32,
                    )
                })
            });
            if far {
                stray = stray.max(alpha[(j * size.0 + i) as usize]);
            }
        }
    }
    (f64::from(inter) / f64::from(union.max(1)), stray)
}

/// `picture` as a limited-range BT.709 4:2:0 frame at `time`.
fn frame(picture: &Rgb, time: Flicks) -> VideoFrame {
    let (w, h) = (picture.width as usize, picture.height as usize);
    let rgb = |x: usize, y: usize| {
        let i = (y * w + x) * 3;
        [0, 1, 2].map(|c| f32::from(picture.data[i + c]) / 255.0)
    };
    let (kr, kb) = (0.2126, 0.0722);
    let luma = |[r, g, b]: [f32; 3]| kr * r + (1.0 - kr - kb) * g + kb * b;
    let y_plane: Vec<u8> = (0..w * h)
        .map(|i| (16.0 + 219.0 * luma(rgb(i % w, i / w))).round() as u8)
        .collect();
    let (cw, ch) = (w / 2, h / 2);
    let mut cb = Vec::with_capacity(cw * ch);
    let mut cr = Vec::with_capacity(cw * ch);
    for j in 0..ch {
        for i in 0..cw {
            let c = rgb(i * 2, j * 2);
            let y = luma(c);
            cb.push((128.0 + 224.0 * (c[2] - y) / (2.0 * (1.0 - kb))).round() as u8);
            cr.push((128.0 + 224.0 * (c[0] - y) / (2.0 * (1.0 - kr))).round() as u8);
        }
    }
    let plane = |data: Vec<u8>, stride: usize, rows: usize| Plane {
        stride,
        rows,
        data: data.into(),
    };
    VideoFrame {
        pts: time,
        duration: Rate::FPS_30.frame_duration(),
        width: picture.width,
        height: picture.height,
        format: PixelFormat::Yuv420p,
        planes: vec![plane(y_plane, w, h), plane(cb, cw, ch), plane(cr, cw, ch)],
        color: ColorInfo {
            matrix: Matrix::Bt709,
            range: Range::Limited,
            primaries: Primaries::Bt709,
            transfer: Transfer::Bt709,
        },
    }
}

fn at(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

#[test]
fn pose_finds_the_figure_its_landmarks_and_its_mask() {
    let Some(dir) = installed(&["mediapipe-pose-full"]) else {
        return;
    };
    let mut analyzer = PoseAnalyzer::new(&PoseOptions::default(), &dir).unwrap();
    let scenes: Vec<Scene> = (0..6).map(|i| figure(i as f32 * 4.0)).collect();
    let started = Instant::now();
    for (i, scene) in scenes.iter().enumerate() {
        analyzer
            .push_picture(at(i as i64), &scene.picture(), (W, H))
            .unwrap();
    }
    eprintln!(
        "pose: {} frames of {W}x{H} in {} ms",
        scenes.len(),
        started.elapsed().as_millis()
    );
    let track = analyzer.finish();
    assert_eq!(track.frames.len(), scenes.len());
    assert_eq!(track.mask_size, (256, 144));
    for (i, (scene, frame)) in scenes.iter().zip(&track.frames).enumerate() {
        assert!(
            frame.presence >= MIN_PRESENCE,
            "frame {i}: presence {}",
            frame.presence
        );
        let dx = i as f32 * 4.0 / W as f32;
        let l = &frame.landmarks;
        // Where the figure's joints were drawn, as fractions of the frame: the nose on the
        // face, shoulders at the top corners of the shirt, hips at the waist and ankles above
        // the shoes. Left and right are the model's guess, so pairs are compared as pairs.
        let near = |got: (f32, f32), want: (f32, f32), tolerance: f32| {
            (got.0 - want.0).abs() <= tolerance && (got.1 - want.1).abs() <= tolerance
        };
        let pair = |a: usize, b: usize, want: (f32, f32), tolerance: f32| {
            let (xa, xb) = (l[a].x.min(l[b].x), l[a].x.max(l[b].x));
            let y = (l[a].y + l[b].y) / 2.0;
            near((xa, y), (0.5 + dx - want.0, want.1), tolerance)
                && near((xb, y), (0.5 + dx + want.0, want.1), tolerance)
        };
        assert!(
            near((l[0].x, l[0].y), (0.5 + dx, 72.0 / 360.0), 0.03),
            "frame {i}: nose {:?}",
            l[0]
        );
        assert!(
            pair(11, 12, (42.0 / 640.0, 110.0 / 360.0), 0.04),
            "frame {i}: shoulders {:?} {:?}",
            l[11],
            l[12]
        );
        assert!(
            pair(23, 24, (22.0 / 640.0, 205.0 / 360.0), 0.05),
            "frame {i}: hips {:?} {:?}",
            l[23],
            l[24]
        );
        assert!(
            pair(27, 28, (29.0 / 640.0, 328.0 / 360.0), 0.05),
            "frame {i}: ankles {:?} {:?}",
            l[27],
            l[28]
        );
        // The mask covers the figure and not the background, and the body's landmarks fall
        // inside it.
        let mask = track.mask(i).unwrap();
        let (iou, stray) = agreement(scene, &mask, track.mask_size);
        eprintln!("pose frame {i}: mask IoU {iou:.3}, background up to {stray}");
        assert!(
            iou > 0.75,
            "frame {i}: the mask overlaps the figure by {iou:.2}"
        );
        assert!(stray < 64, "frame {i}: the background reaches {stray}");
        let (mw, mh) = track.mask_size;
        for n in [0, 11, 12, 23, 24, 25, 26] {
            let (x, y) = ((l[n].x * mw as f32) as u32, (l[n].y * mh as f32) as u32);
            assert!(
                mask[(y.min(mh - 1) * mw + x.min(mw - 1)) as usize] >= 128,
                "frame {i}: landmark {n} is outside the mask"
            );
        }
    }
    let data = track.encode();
    assert_eq!(PoseTrack::decode(&data).unwrap(), track);
    eprintln!(
        "pose: {} bytes of data for {} frames",
        data.len(),
        track.frames.len()
    );
}

#[test]
fn the_person_matte_separates_the_figure_and_the_empty_wall_has_none() {
    let Some(dir) = installed(&["mediapipe-pose-full"]) else {
        return;
    };
    let mut analyzer = MatteAnalyzer::new(&MatteOptions::default(), &dir).unwrap();
    let wall = Scene {
        background: [70, 110, 160],
        shapes: Vec::new(),
    };
    let scenes = [figure(0.0), figure(2.0), figure(4.0), wall];
    for (i, scene) in scenes.iter().enumerate() {
        assert!(
            analyzer
                .push(&frame(&scene.picture(), at(i as i64)))
                .unwrap()
        );
    }
    let track = analyzer.finish();
    assert_eq!(track.size, (512, 288));
    for (i, scene) in scenes.iter().take(3).enumerate() {
        let (iou, stray) = agreement(scene, &track.alpha(i).unwrap(), track.size);
        eprintln!("person matte frame {i}: IoU {iou:.3}, background up to {stray}");
        assert!(iou > 0.75, "frame {i}: {iou:.2}");
        assert!(stray < 64, "frame {i}: {stray}");
    }
    // No person: an empty matte (the smoothing only borrows from frames that agree).
    let empty = track.alpha(3).unwrap();
    assert!(empty.iter().all(|&v| v < 32), "the wall has no subject");
}

#[test]
fn birefnet_mattes_a_vase_and_frames_between_keys_follow_it() {
    let Some(dir) = installed(&["birefnet-lite"]) else {
        return;
    };
    let options = MatteOptions {
        model: MatteModel::Subject,
        every: 3,
        size: 256,
        smoothing: 0.0,
        ..MatteOptions::default()
    };
    let mut analyzer = MatteAnalyzer::new(&options, &dir).unwrap();
    let scene = vase();
    let started = Instant::now();
    let mut keys = 0;
    for i in 0..2 {
        keys += u32::from(analyzer.push(&frame(&scene.picture(), at(i))).unwrap());
    }
    eprintln!(
        "birefnet-lite: {keys} key frame of {W}x{H} (plus one between) in {} ms",
        started.elapsed().as_millis()
    );
    assert_eq!(keys, 1, "frame 0 is a key; frame 1 follows it");
    let track = analyzer.finish();
    assert_eq!(track.size, (256, 144));
    assert_eq!(track.frames.len(), 2);
    assert!(track.frames[0].key && !track.frames[1].key);
    for i in 0..2 {
        let (iou, stray) = agreement(&scene, &track.alpha(i).unwrap(), track.size);
        eprintln!("birefnet frame {i}: matte IoU {iou:.3}, background up to {stray}");
        assert!(
            iou > 0.85,
            "frame {i}: the matte overlaps the vase by {iou:.2}"
        );
        assert!(stray < 64, "frame {i}: the wall reaches {stray}");
    }
    let data = track.encode();
    assert_eq!(MatteTrack::decode(&data).unwrap(), track);
}

/// Encodes `scenes` as a 30 fps MPEG-4 file in `dir`, or `None` without FFmpeg.
fn encode(dir: &Path, name: &str, scenes: &[Scene]) -> Option<PathBuf> {
    let present = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        present || !required("FOSSCUT_REQUIRE_FFMPEG"),
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    if !present {
        return None;
    }
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    let mut child = Command::new("ffmpeg")
        .args([
            "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", "640x360", "-r",
            "30", "-i", "-",
        ])
        .args(["-c:v", "mpeg4", "-q:v", "2", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write as _;
        let stdin = child.stdin.as_mut().unwrap();
        for scene in scenes {
            stdin.write_all(&scene.picture().data).unwrap();
        }
    }
    assert!(child.wait().unwrap().success());
    Some(path)
}

#[test]
fn pose_and_mattes_run_over_a_file_with_progress_and_cancellation() {
    let Some(dir) = installed(&["mediapipe-pose-full"]) else {
        return;
    };
    let work = std::env::temp_dir().join(format!("fosscut-vision-{}", std::process::id()));
    let scenes: Vec<Scene> = (0..8).map(|i| figure(i as f32 * 3.0)).collect();
    let Some(path) = encode(&work, "figure.mp4", &scenes) else {
        return;
    };
    let media = Media {
        path: &path,
        duration: at(8),
        audio_channels: None,
        video_rate: Some(Rate::FPS_30),
    };
    let mut seen = Vec::new();
    let started = Instant::now();
    let results = analyze(
        media,
        &[Analyzer::Pose, Analyzer::Matte],
        &Options::default(),
        &dir,
        &mut |p| {
            seen.push(p);
            true
        },
    )
    .unwrap();
    eprintln!(
        "pose and person matte over 8 frames: {} ms",
        started.elapsed().as_millis()
    );
    assert_eq!(seen.last().map(|p| (p.done, p.total)), Some((8, 8)));
    let pose = results.pose.as_ref().unwrap();
    assert_eq!((pose.frames, pose.frames_with_person), (8, 8));
    let data = &results.data[&Analyzer::Pose];
    assert!(pose.data.matches(data));
    let track = PoseTrack::decode(data).unwrap();
    let (iou, _) = agreement(&scenes[4], &track.mask(4).unwrap(), track.mask_size);
    assert!(iou > 0.7, "the decoded file's mask still fits: {iou:.2}");
    let matte = results.matte.as_ref().unwrap();
    assert_eq!(
        (matte.frames, matte.keyframes, matte.kind),
        (8, 8, MatteModel::Person)
    );
    assert!(
        matte.coverage > 0.05 && matte.coverage < 0.3,
        "{}",
        matte.coverage
    );
    assert!(matte.data.matches(&results.data[&Analyzer::Matte]));
    // The reply names the data by hash and never carries it.
    let reply = serde_json::to_value(&results).unwrap();
    assert!(reply.get("data").is_none());
    assert_eq!(reply["pose"]["data"]["blake3"], pose.data.blake3);
    assert_eq!(reply["pose"]["samples"].as_array().unwrap().len(), 1);

    // Cancelling stops at the first frame.
    let cancelled = analyze(
        media,
        &[Analyzer::Pose],
        &Options::default(),
        &dir,
        &mut |_| false,
    );
    assert!(matches!(cancelled, Err(AnalysisError::Cancelled)));
    // A model that is not installed is named, with the runtime when that is missing too.
    let empty = work.join("no-models");
    let missing = analyze(
        media,
        &[Analyzer::Matte],
        &Options::default(),
        &empty,
        &mut |_| true,
    );
    match missing {
        Err(AnalysisError::Model(models::ModelError::NotInstalled { what })) => {
            assert!(what.starts_with("mediapipe-pose-full"), "{what}");
        }
        other => panic!("expected a missing model, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&work);
}
