//! Framing with the real pose model (ADR-0033): a drawn person-like figure (the one the
//! vision tests use) walks across a wide frame, MediaPipe Pose Landmarker finds it in every
//! frame, and the vertical crop that [`fosscut_analysis::framing`] computes from its
//! landmarks keeps the face inside, following it within the speed limit.
//!
//! It needs the `onnxruntime` and `mediapipe-pose-full` models in `FOSSCUT_MODELS_DIR` and
//! skips without them; `FOSSCUT_REQUIRE_MODELS` turns a missing one into a failure.

#![cfg(feature = "onnx")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use fosscut_analysis::framing::{FramingOptions, frame, from_pose};
use fosscut_analysis::models;
use fosscut_analysis::pose::{PoseAnalyzer, PoseOptions};
use fosscut_analysis::vision::Rgb;
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

fn at(n: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(n)
}

#[test]
fn a_vertical_crop_follows_a_walking_figure_found_by_the_pose_model() {
    let Some(dir) = installed(&["mediapipe-pose-full"]) else {
        return;
    };
    let mut analyzer = PoseAnalyzer::new(&PoseOptions::default(), &dir).unwrap();
    // From 150 pixels left of the middle to 150 right, 10 pixels a frame (0.47 of the
    // frame's width a second at 30 fps).
    let shifts: Vec<f32> = (0..31).map(|i| -150.0 + 10.0 * i as f32).collect();
    for (i, dx) in shifts.iter().enumerate() {
        analyzer
            .push_picture(at(i as i64), &figure(*dx).picture(), (W, H))
            .unwrap();
    }
    let track = analyzer.finish();
    let subjects: Vec<_> = track
        .frames
        .iter()
        .map(|f| from_pose(f.presence, &f.landmarks))
        .collect();
    assert!(
        subjects.iter().all(Option::is_some),
        "the figure is found in every frame"
    );
    for (subject, dx) in subjects.iter().zip(&shifts) {
        let subject = subject.unwrap();
        let want = 0.5 + f64::from(*dx) / f64::from(W);
        assert!(
            (subject.x - want).abs() < 0.03,
            "face at {} for {want}",
            subject.x
        );
    }
    // A 9:16 crop of the 16:9 frame.
    let aspect = (9.0 / 16.0) / (16.0 / 9.0);
    let options = FramingOptions {
        max_speed: 2.0,
        ..FramingOptions::default()
    };
    let framing = frame(&subjects, 30.0, &[], aspect, &options);
    let limit = options.max_speed * aspect / 30.0;
    let mut worst = 0.0f64;
    for (i, (crop, dx)) in framing.crops.iter().zip(&shifts).enumerate() {
        let face = 0.5 + f64::from(*dx) / f64::from(W);
        let off = (face - crop.x).abs() / crop.width;
        worst = worst.max(off);
        assert!(
            off < 0.35,
            "frame {i}: the face is {off} of the crop's width from its centre"
        );
    }
    eprintln!("pose framing: the face at most {worst:.3} of the crop's width off centre");
    let xs: Vec<f64> = framing.crops.iter().map(|c| c.x).collect();
    assert!(xs.windows(2).all(|w| (w[1] - w[0]).abs() <= limit + 1e-9));
    assert!(
        xs[30] - xs[0] > 0.3,
        "the crop follows: {} to {}",
        xs[0],
        xs[30]
    );
}
