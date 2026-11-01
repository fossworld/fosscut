//! Golden frames of vector paths (ADR-0036), a module of `goldens.rs`: an open curve with
//! round caps, dashes, a gradient stroke, a trim from 30% to 70%, a clip following a curve
//! while the curve draws on, and an SVG drawing rendered sharp at two output sizes.

use std::collections::HashMap;
use std::sync::Arc;

use fosscut_media::VideoFrame;
use fosscut_model::{
    Clip, ClipContent, Color, Follow, Gradient, Id, IdKind, Interpolation, KeyValue, Keyframe,
    LineCap, LineJoin, MaskShape, PathPoint, Project, ShapeContent, ShapeStroke, TimelineFormat,
    Track, TrackKind, Transform, parse_path_data,
};
use fosscut_render::plan::{Layer, RenderPlan, SourceRef, plan_frame};
use fosscut_render::vector::VectorImage;
use fosscut_render::{FrameProvider, RenderError};
use fosscut_time::{Flicks, Rate};

use super::check_at;
use super::support::*;

const CANVAS: (u32, u32) = (1920, 1080);
const SIZE: (u32, u32) = (640, 360);

fn color(hex: &str) -> Color {
    Color::parse(hex).unwrap()
}

/// A dark backdrop covering the canvas.
fn backdrop() -> Layer {
    Layer::shape(ShapeContent::new(
        MaskShape::Rectangle,
        [1.0, 1.0],
        color("#14161C"),
    ))
}

/// A path shape drawn by SVG path data in canvas pixels, without fill, placed where the
/// data says.
fn drawn(data: &str, closed_fill: Option<&str>, stroke: ShapeStroke) -> Layer {
    let fitted = parse_path_data(data, 1.0).unwrap();
    let [x, y, w, h] = fitted.bounds;
    let (cw, ch) = (f64::from(CANVAS.0), f64::from(CANVAS.1));
    let mut shape = ShapeContent::new(MaskShape::Path, [w / cw, h / ch], Color::default());
    shape.points = fitted.contours[0].points.clone();
    shape.closed = fitted.contours[0].closed;
    shape.contours = fitted.contours[1..].to_vec();
    shape.fill = closed_fill.map(|c| fosscut_model::Paint {
        color: color(c),
        gradient: None,
    });
    shape.stroke = Some(stroke);
    Layer {
        transform: Transform {
            position: [(x + w / 2.0) / cw - 0.5, (y + h / 2.0) / ch - 0.5],
            ..Transform::default()
        },
        ..Layer::shape(shape)
    }
}

fn plan(layers: Vec<Layer>) -> RenderPlan {
    let mut all = vec![backdrop()];
    all.extend(layers);
    RenderPlan {
        canvas: CANVAS,
        layers: all,
        skipped: Vec::new(),
    }
}

const SWOOSH: &str = "M 240 760 C 560 160 960 160 1120 520 S 1560 900 1700 300";

/// An open S curve with round caps, and the same curve with butt caps and a bevelled zigzag
/// below it.
#[test]
fn stroke_round_caps() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let round = ShapeStroke {
        cap: LineCap::Round,
        ..ShapeStroke::new(color("#FFC845"), 0.04)
    };
    let zigzag = ShapeStroke {
        join: LineJoin::Bevel,
        ..ShapeStroke::new(color("#4FD1C5"), 0.025)
    };
    let plan = plan(vec![
        drawn(SWOOSH, None, round),
        drawn(
            "M 300 960 L 600 860 L 900 960 L 1200 860 L 1500 960",
            None,
            zigzag,
        ),
    ]);
    check_at(
        "vector-stroke-round-caps",
        &mut renderer,
        &plan,
        &mut Frames::default(),
        SIZE,
    );
}

/// Dashes: a rounded rectangle outlined in dashes with a fill, and a dotted line of round
/// dots (zero-length dashes with round caps).
#[test]
fn stroke_dashes() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut card = ShapeContent::new(MaskShape::Rectangle, [0.5, 0.45], color("#2B3A67"));
    card.roundness = 0.3;
    card.stroke = Some(ShapeStroke {
        dashes: vec![0.04, 0.02],
        join: LineJoin::Round,
        ..ShapeStroke::new(color("#FFFFFF"), 0.012)
    });
    let dots = ShapeStroke {
        cap: LineCap::Round,
        dashes: vec![0.0, 0.045],
        ..ShapeStroke::new(color("#FF5C8A"), 0.03)
    };
    let plan = plan(vec![
        Layer {
            transform: Transform {
                position: [0.0, -0.1],
                ..Transform::default()
            },
            ..Layer::shape(card)
        },
        drawn("M 260 900 L 1660 900", None, dots),
    ]);
    check_at(
        "vector-dashes",
        &mut renderer,
        &plan,
        &mut Frames::default(),
        SIZE,
    );
}

/// A ring drawn by a thick gradient stroke without fill, and a line with a gradient across
/// its box.
#[test]
fn stroke_gradient() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut ring = ShapeContent::new(MaskShape::Ellipse, [0.35, 0.62], Color::default());
    ring.fill = None;
    ring.stroke = Some(ShapeStroke {
        gradient: Some(Gradient {
            to: color("#7C3AED"),
            angle: 45.0,
            radial: false,
        }),
        ..ShapeStroke::new(color("#22D3EE"), 0.06)
    });
    let line = ShapeStroke {
        cap: LineCap::Round,
        gradient: Some(Gradient {
            to: color("#F97316"),
            angle: 0.0,
            radial: false,
        }),
        ..ShapeStroke::new(color("#FDE047"), 0.03)
    };
    let plan = plan(vec![
        Layer {
            transform: Transform {
                position: [-0.2, 0.0],
                ..Transform::default()
            },
            ..Layer::shape(ring)
        },
        drawn("M 1150 800 C 1300 200 1600 200 1750 800", None, line),
    ]);
    check_at(
        "vector-gradient-stroke",
        &mut renderer,
        &plan,
        &mut Frames::default(),
        SIZE,
    );
}

/// The S curve trimmed to its part from 30% to 70% over the whole curve drawn thin, and a
/// circle trimmed to the same fractions, which start at its top and run clockwise.
#[test]
fn stroke_trimmed() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let guide = ShapeStroke::new(color("#3B4252"), 0.008);
    let mut trimmed = drawn(
        SWOOSH,
        None,
        ShapeStroke {
            cap: LineCap::Round,
            ..ShapeStroke::new(color("#FFC845"), 0.04)
        },
    );
    if let fosscut_render::plan::LayerSource::Shape(shape) = &mut trimmed.source {
        shape.trim_start = 0.3;
        shape.trim_end = 0.7;
    }
    let mut circle = ShapeContent::new(MaskShape::Ellipse, [0.2, 0.3555], Color::default());
    circle.fill = None;
    circle.stroke = Some(ShapeStroke {
        cap: LineCap::Round,
        ..ShapeStroke::new(color("#4FD1C5"), 0.03)
    });
    circle.trim_start = 0.3;
    circle.trim_end = 0.7;
    let plan = plan(vec![
        drawn(SWOOSH, None, guide),
        trimmed,
        Layer {
            transform: Transform {
                position: [0.3, 0.25],
                ..Transform::default()
            },
            ..Layer::shape(circle)
        },
    ]);
    check_at(
        "vector-trim",
        &mut renderer,
        &plan,
        &mut Frames::default(),
        SIZE,
    );
}

fn keys(path: &str, from: f64, to: f64, seconds: i64) -> (String, Vec<Keyframe>) {
    (
        path.to_string(),
        vec![
            Keyframe {
                time: Flicks::ZERO,
                value: KeyValue::Number(from),
                interpolation: Interpolation::Linear,
            },
            Keyframe {
                time: Flicks::from_seconds(seconds).unwrap(),
                value: KeyValue::Number(to),
                interpolation: Interpolation::Linear,
            },
        ],
    )
}

/// A route that draws itself on with an arrow following its end, turned along it, and a
/// rotated, enlarged copy of the route carrying a second arrow: planned from a project, so
/// the trims, follow progress and placements come from keyframes and transforms.
#[test]
fn follower_on_a_curve() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let format = TimelineFormat {
        width: CANVAS.0,
        height: CANVAS.1,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "follow", format);
    let timeline = project.manifest.primary_timeline.clone();
    let seconds = Flicks::from_seconds(4).unwrap();
    let route = |n: u64, transform: Transform| {
        let fitted =
            parse_path_data("M 0 300 C 200 0 500 0 700 300 S 1100 600 1300 200", 1.0).unwrap();
        let mut shape = ShapeContent::new(MaskShape::Path, [0.6, 0.35], Color::default());
        shape.points = fitted.contours[0].points.clone();
        shape.closed = false;
        shape.fill = None;
        shape.stroke = Some(ShapeStroke {
            cap: LineCap::Round,
            ..ShapeStroke::new(color("#FFC845"), 0.015)
        });
        let mut clip = Clip::new(
            Id::new(IdKind::Clip, n),
            Flicks::ZERO,
            seconds,
            ClipContent::Shape(Box::new(shape)),
        );
        clip.transform = transform;
        clip.keyframes.extend([keys("shape.trim_end", 0.0, 1.0, 4)]);
        clip
    };
    // An arrow pointing right, so turning it shows the path's direction.
    let arrow = |n: u64, path: u64, fill: &str| {
        let mut shape = ShapeContent::new(MaskShape::Path, [0.05, 0.06], color(fill));
        shape.points = [[-1.0, -1.0], [1.0, 0.0], [-1.0, 1.0], [-0.4, 0.0]]
            .into_iter()
            .map(|at| PathPoint {
                at,
                in_handle: None,
                out_handle: None,
            })
            .collect();
        let mut clip = Clip::new(
            Id::new(IdKind::Clip, n),
            Flicks::ZERO,
            seconds,
            ClipContent::Shape(Box::new(shape)),
        );
        clip.follow = Some(Follow {
            clip: Id::new(IdKind::Clip, path),
            progress: 0.0,
            orient: true,
            offset: [0.0, 0.0],
        });
        clip.keyframes
            .extend([keys("follow.progress", 0.0, 1.0, 4)]);
        clip
    };
    let clips = vec![
        Clip::new(
            Id::new(IdKind::Clip, 1),
            Flicks::ZERO,
            seconds,
            ClipContent::Shape(Box::new(ShapeContent::new(
                MaskShape::Rectangle,
                [1.0, 1.0],
                color("#14161C"),
            ))),
        ),
        route(
            2,
            Transform {
                position: [0.0, -0.18],
                ..Transform::default()
            },
        ),
        route(
            3,
            Transform {
                position: [0.05, 0.22],
                scale: 0.8,
                rotation: -12.0,
                flip_vertical: true,
                ..Transform::default()
            },
        ),
        arrow(4, 2, "#FF5C8A"),
        arrow(5, 3, "#4FD1C5"),
    ];
    let doc = project.timelines.get_mut(&timeline).unwrap();
    for (n, clip) in clips.into_iter().enumerate() {
        doc.tracks.push(Track {
            id: Id::new(IdKind::Track, 10 + n as u64),
            kind: TrackKind::Video,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![clip],
        });
    }
    assert!(fosscut_model::validate(&project).is_empty());
    let at = Flicks::from_seconds_f64(2.4).unwrap();
    let plan = plan_frame(&project, &timeline, at).unwrap();
    check_at(
        "vector-follow",
        &mut renderer,
        &plan,
        &mut Frames::default(),
        SIZE,
    );
}

/// A frame provider with SVG drawings beside the test frames.
struct Drawings {
    frames: Frames,
    images: HashMap<Id, Arc<VectorImage>>,
}

impl FrameProvider for Drawings {
    fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError> {
        self.frames.frame(source)
    }

    fn vector(&self, asset: &Id) -> Option<Arc<VectorImage>> {
        self.images.get(asset).cloned()
    }
}

/// A small logo written for this test: a ring, a swoosh with round caps and an even-odd
/// star with a hole, 48 by 32 units.
const LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="32" viewBox="0 0 48 32">
  <defs>
    <linearGradient id="sky" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#22D3EE"/><stop offset="1" stop-color="#7C3AED"/>
    </linearGradient>
  </defs>
  <circle cx="16" cy="16" r="12" fill="none" stroke="url(#sky)" stroke-width="3"/>
  <path d="M 6 22 C 14 12 26 26 42 8" fill="none" stroke="#FFC845" stroke-width="2.5" stroke-linecap="round"/>
  <path fill="#FF5C8A" fill-rule="evenodd" d="M 38 14 L 41 22 L 47 22 L 42 26 L 44 32 L 38 28 L 32 32 L 34 26 L 29 22 L 35 22 Z M 36 24 h 4 v 3 h -4 z"/>
</svg>"##;

/// The logo shown as an image clip at three quarters of the canvas height, over the test
/// pattern, at two output sizes: drawn at each size, its edges stay one pixel wide.
#[test]
fn svg_drawing_sharp_at_two_sizes() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let logo = Id::new(IdKind::Asset, 40);
    let mut drawings = Drawings {
        frames: Frames::default().with(1, pattern(640, 360, 0)),
        images: HashMap::from([(
            logo.clone(),
            Arc::new(VectorImage::parse(LOGO.as_bytes()).unwrap()),
        )]),
    };
    let plan = RenderPlan {
        canvas: CANVAS,
        layers: vec![
            Layer::new(source(1)),
            Layer {
                transform: Transform {
                    scale: 0.75,
                    ..Transform::default()
                },
                ..Layer::new(SourceRef {
                    asset: logo,
                    time: Flicks::ZERO,
                })
            },
        ],
        skipped: Vec::new(),
    };
    for (name, size) in [
        ("vector-svg-small", (320, 180)),
        ("vector-svg-large", (1280, 720)),
    ] {
        check_at(name, &mut renderer, &plan, &mut drawings, size);
    }
    // Over black, the star's left edge goes from background to pink within a pixel or two
    // at the large size, where a drawing rasterized once and enlarged would ramp over many.
    let mut sharp = plan.clone();
    sharp.layers[0] = Layer::shape(ShapeContent::new(
        MaskShape::Rectangle,
        [1.0, 1.0],
        color("#000000"),
    ));
    let image = renderer
        .render_image(
            &sharp,
            &mut drawings,
            (1280, 720),
            fosscut_render::color::TransferFunction::Bt709,
        )
        .unwrap();
    // The logo is 810 x 540 output pixels from (235, 90); 25 of its 32 units down crosses
    // the star from about 32.75 units across.
    let row: u32 = 90 + (25.0 * 16.875) as u32;
    let away = |x: u32| {
        let p = &image.rgba[((row * 1280 + x) * 4) as usize..][..4];
        (i32::from(p[0]) - 255).abs() + (i32::from(p[1]) - 92).abs() + (i32::from(p[2]) - 138).abs()
    };
    let edge = (700..900)
        .find(|x| away(*x) < 60)
        .expect("the star is drawn");
    assert!((780..800).contains(&edge), "the star's edge at {edge}");
    assert!(
        away(edge - 3) > 300,
        "background before the edge: {}",
        away(edge - 3)
    );
    assert!(
        away(edge + 3) < 30,
        "pink after the edge: {}",
        away(edge + 3)
    );
}
