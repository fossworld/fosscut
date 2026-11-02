//! Vector paths on shape clips (ADR-0036): every drawable shape's outline as a Bezier path,
//! its length, the parts trims keep, points along it, SVG path data, and clips that follow
//! a shape's path.
//!
//! Outlines are measured in canvas pixels of the shape's box, centred on its middle with y
//! growing downwards, before the clip's own transform: the space the shape is drawn in, so
//! trims and follow progress mean the same fraction of the drawn line.

use fosscut_time::Flicks;
use kurbo::{BezPath, ParamCurve, ParamCurveArclen, ParamCurveDeriv, PathEl, PathSeg, Point, Vec2};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::is_default;
use crate::shape::{Contour, ShapeContent};
use crate::{Clip, ClipContent, Id, KeyValue, MaskShape, PathPoint, Project, Timeline, animate};

/// How exactly arc lengths are measured, in canvas pixels.
const ACCURACY: f64 = 1e-3;

/// The magic number of quarter circles drawn as cubic Beziers: 4/3 (sqrt(2) - 1).
const KAPPA: f64 = 0.552_284_749_830_793_4;

/// A clip that moves along the path of a shape clip (ADR-0036).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Follow {
    /// The shape clip whose outline it follows (`clip_…`), on the same timeline.
    pub clip: Id,
    /// How far along the path, as a fraction of its whole length: 0 at its start, 1 at its
    /// end. On a closed path values past 1 go round again; on an open path they hold at
    /// the ends. Animate `follow.progress`.
    #[serde(default, skip_serializing_if = "is_default")]
    pub progress: f64,
    /// Turns the clip with the path's direction, added to its own rotation: 0 degrees
    /// runs left to right.
    #[serde(default, skip_serializing_if = "is_default")]
    pub orient: bool,
    /// Added to the point on the path, as fractions of the canvas width and height.
    #[serde(default, skip_serializing_if = "is_default")]
    pub offset: [f64; 2],
}

/// Where a following clip is at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowPlacement {
    /// Its centre's offset from the canvas centre, as fractions of the canvas width and
    /// height: the point on the path plus the follow's offset.
    pub position: [f64; 2],
    /// Degrees clockwise to add to its rotation: the path's direction when it orients,
    /// otherwise 0.
    pub rotation: f64,
}

impl ShapeContent {
    /// True when the shape uses a stroke, trims, an open path, more contours or the
    /// nonzero rule: what the vector renderer draws (ADR-0036). Other shapes draw as
    /// before.
    pub fn is_vector(&self) -> bool {
        self.stroke.is_some()
            || self.trimmed()
            || !self.closed
            || !self.contours.is_empty()
            || self.fill_rule != crate::FillRule::EvenOdd
    }

    /// True when trims hide part of the outline.
    pub fn trimmed(&self) -> bool {
        self.trim_start != 0.0 || self.trim_end != 1.0
    }

    /// The outline of the shape for a box `size` canvas pixels large, centred on (0, 0)
    /// with y growing downwards: one subpath per contour, closed ones closed. Rectangles
    /// start at the top-left end of their top edge, ellipses at the top, stars at their
    /// top point and hearts at the dip between their lobes, all running clockwise.
    pub fn outline(&self, size: [f64; 2]) -> BezPath {
        let (hx, hy) = (size[0] / 2.0, size[1] / 2.0);
        let mut path = BezPath::new();
        match self.shape {
            MaskShape::Rectangle => rectangle(&mut path, hx, hy, self.roundness),
            MaskShape::Ellipse => ellipse(&mut path, hx, hy),
            MaskShape::Star => polygon(&mut path, &star(), hx, hy),
            MaskShape::Heart => polygon(&mut path, &heart(), hx, hy),
            MaskShape::Path => {
                points(&mut path, &self.points, self.closed, hx, hy);
                for contour in &self.contours {
                    points(&mut path, &contour.points, contour.closed, hx, hy);
                }
            }
            _ => {}
        }
        path
    }
}

fn rectangle(path: &mut BezPath, hx: f64, hy: f64, roundness: f64) {
    let r = roundness.clamp(0.0, 1.0) * hx.min(hy);
    let k = r * KAPPA;
    let round = r > 1e-9;
    let line = |path: &mut BezPath, to: (f64, f64)| {
        let last = path.elements().last().and_then(|e| e.end_point());
        if last.is_none_or(|p| p.distance(Point::new(to.0, to.1)) > 1e-9) {
            path.line_to(to);
        }
    };
    path.move_to((-hx + r, -hy));
    line(path, (hx - r, -hy));
    if round {
        path.curve_to((hx - r + k, -hy), (hx, -hy + r - k), (hx, -hy + r));
    }
    line(path, (hx, hy - r));
    if round {
        path.curve_to((hx, hy - r + k), (hx - r + k, hy), (hx - r, hy));
    }
    line(path, (-hx + r, hy));
    if round {
        path.curve_to((-hx + r - k, hy), (-hx, hy - r + k), (-hx, hy - r));
    }
    line(path, (-hx, -hy + r));
    if round {
        path.curve_to((-hx, -hy + r - k), (-hx + r - k, -hy), (-hx + r, -hy));
    }
    path.close_path();
}

fn ellipse(path: &mut BezPath, hx: f64, hy: f64) {
    let (kx, ky) = (hx * KAPPA, hy * KAPPA);
    path.move_to((0.0, -hy));
    path.curve_to((kx, -hy), (hx, -ky), (hx, 0.0));
    path.curve_to((hx, ky), (kx, hy), (0.0, hy));
    path.curve_to((-kx, hy), (-hx, ky), (-hx, 0.0));
    path.curve_to((-hx, -ky), (-kx, -hy), (0.0, -hy));
    path.close_path();
}

fn polygon(path: &mut BezPath, corners: &[[f64; 2]], hx: f64, hy: f64) {
    for (i, p) in corners.iter().enumerate() {
        let point = (p[0] * hx, p[1] * hy);
        if i == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    path.close_path();
}

/// A five-pointed star, point up, in the unit box: the corners the mask geometry uses
/// (ADR-0011 item 4), so a stroke follows the fill's edge.
fn star() -> Vec<[f64; 2]> {
    let inner = (18f64.to_radians().sin()) / (126f64.to_radians().sin());
    (0..10)
        .map(|i| {
            let radius = if i % 2 == 0 { 1.0 } else { inner };
            let angle = (-90.0 + 36.0 * f64::from(i)).to_radians();
            [radius * angle.cos(), radius * angle.sin()]
        })
        .collect()
}

/// A heart, point down, in the unit box: the parametric heart curve the mask geometry
/// traces.
fn heart() -> Vec<[f64; 2]> {
    let raw: Vec<[f64; 2]> = (0..96)
        .map(|i| {
            let t = f64::from(i) / 96.0 * std::f64::consts::TAU;
            let x = 16.0 * t.sin().powi(3);
            let y =
                13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            [x, -y]
        })
        .collect();
    let (min_y, max_y) = raw
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p[1]), hi.max(p[1]))
        });
    let middle = (min_y + max_y) / 2.0;
    let half = (max_y - min_y) / 2.0;
    raw.into_iter()
        .map(|[x, y]| [x / 16.0, (y - middle) / half])
        .collect()
}

/// Adds a contour of path points (in the -1 to 1 box) scaled by the half sizes: a line
/// between points without handles, otherwise a cubic Bezier through their handles.
fn points(path: &mut BezPath, contour: &[PathPoint], closed: bool, hx: f64, hy: f64) {
    let Some(first) = contour.first() else {
        return;
    };
    let at = |p: [f64; 2]| Point::new(p[0] * hx, p[1] * hy);
    let handle = |p: &PathPoint, h: Option<[f64; 2]>| {
        h.map_or(at(p.at), |d| at([p.at[0] + d[0], p.at[1] + d[1]]))
    };
    path.move_to(at(first.at));
    let count = contour.len();
    let segments = if closed { count } else { count - 1 };
    for i in 0..segments {
        let (a, b) = (&contour[i], &contour[(i + 1) % count]);
        if a.out_handle.is_none() && b.in_handle.is_none() {
            path.line_to(at(b.at));
        } else {
            path.curve_to(handle(a, a.out_handle), handle(b, b.in_handle), at(b.at));
        }
    }
    if closed {
        path.close_path();
    }
}

/// One contour of a path, measured.
struct Measured {
    segments: Vec<(PathSeg, f64)>,
    length: f64,
    closed: bool,
}

/// The contours of `path` with the length of every segment; a closed contour's closing
/// edge is a segment of its own.
fn measure(path: &BezPath) -> Vec<Measured> {
    let mut contours: Vec<Measured> = Vec::new();
    let mut start = Point::ZERO;
    let mut last = Point::ZERO;
    let push = |contours: &mut Vec<Measured>, segment: PathSeg| {
        let length = segment.arclen(ACCURACY);
        if let Some(contour) = contours.last_mut() {
            contour.segments.push((segment, length));
            contour.length += length;
        }
    };
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                contours.push(Measured {
                    segments: Vec::new(),
                    length: 0.0,
                    closed: false,
                });
                start = p;
                last = p;
            }
            PathEl::LineTo(p) => {
                push(&mut contours, PathSeg::Line(kurbo::Line::new(last, p)));
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                push(
                    &mut contours,
                    PathSeg::Quad(kurbo::QuadBez::new(last, c, p)),
                );
                last = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                push(
                    &mut contours,
                    PathSeg::Cubic(kurbo::CubicBez::new(last, c1, c2, p)),
                );
                last = p;
            }
            PathEl::ClosePath => {
                if last.distance(start) > 1e-9 {
                    push(&mut contours, PathSeg::Line(kurbo::Line::new(last, start)));
                }
                if let Some(contour) = contours.last_mut() {
                    contour.closed = true;
                }
                last = start;
            }
        }
    }
    contours
}

/// The whole length of `path` in its own units.
pub fn path_length(path: &BezPath) -> f64 {
    measure(path).iter().map(|c| c.length).sum()
}

/// Appends the part of `contour` from `from` to `to` (lengths along it) to `out`, starting
/// a new subpath unless `join` continues the last one.
fn append_part(out: &mut BezPath, contour: &Measured, from: f64, to: f64, join: bool) {
    let mut base = 0.0;
    let mut started = join;
    for (segment, length) in &contour.segments {
        let (s0, s1) = (base, base + length);
        base = s1;
        let (a, b) = (from.max(s0), to.min(s1));
        if b < a || (b == a && *length > 0.0) {
            continue;
        }
        let (t0, t1) = if *length > 0.0 {
            (
                segment.inv_arclen(a - s0, ACCURACY),
                segment.inv_arclen(b - s0, ACCURACY),
            )
        } else {
            (0.0, 1.0)
        };
        let part = segment.subsegment(t0..t1);
        if !started {
            out.move_to(part.start());
            started = true;
        }
        match part {
            PathSeg::Line(l) => out.line_to(l.p1),
            PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
            PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
        }
    }
}

/// The part of `path` a trim keeps (ADR-0036): from fraction `start` to fraction `end` of
/// its whole length (either order), moved along by `offset` and wrapping past the end.
/// Contours count one after another. Without a trim the path comes back unchanged, closed
/// contours still closed; a trimmed part is open.
pub fn trim_path(path: &BezPath, start: f64, end: f64, offset: f64) -> BezPath {
    let (low, high) = (
        start.min(end).clamp(0.0, 1.0),
        start.max(end).clamp(0.0, 1.0),
    );
    let span = high - low;
    if span >= 1.0 - 1e-12 {
        return path.clone();
    }
    let contours = measure(path);
    let total: f64 = contours.iter().map(|c| c.length).sum();
    let mut out = BezPath::new();
    if span <= 1e-12 || total <= 0.0 || !offset.is_finite() {
        return out;
    }
    let first = (low + offset).rem_euclid(1.0);
    let mut ranges = vec![(first, (first + span).min(1.0))];
    if first + span > 1.0 {
        ranges.push((0.0, first + span - 1.0));
    }
    // A single closed contour wraps through its start without a break.
    let wraps = ranges.len() == 2 && contours.len() == 1 && contours[0].closed;
    for (index, (from, to)) in ranges.into_iter().enumerate() {
        let (from, to) = (from * total, to * total);
        let mut base = 0.0;
        for contour in &contours {
            let (c0, c1) = (base, base + contour.length);
            base = c1;
            let (a, b) = (from.max(c0), to.min(c1));
            if b > a {
                append_part(&mut out, contour, a - c0, b - c0, wraps && index == 1);
            }
        }
    }
    out
}

/// The point at `fraction` of the whole length of `path` (0 its start, 1 its end), with
/// the direction it runs there; `None` for a path without length.
pub fn point_along(path: &BezPath, fraction: f64) -> Option<(Point, Vec2)> {
    let contours = measure(path);
    let total: f64 = contours.iter().map(|c| c.length).sum();
    if total <= 0.0 || !fraction.is_finite() {
        return None;
    }
    let target = fraction.clamp(0.0, 1.0) * total;
    let mut base = 0.0;
    let mut last = None;
    for contour in &contours {
        for (segment, length) in &contour.segments {
            if *length <= 0.0 {
                continue;
            }
            last = Some((*segment, 1.0));
            if target <= base + length {
                let t = segment.inv_arclen(target - base, ACCURACY);
                return Some((segment.eval(t), direction(segment, t)));
            }
            base += length;
        }
    }
    last.map(|(segment, t)| (segment.eval(t), direction(&segment, t)))
}

/// The direction a segment runs at `t`, looking a little further along where the
/// derivative vanishes (a handle on its point).
fn direction(segment: &PathSeg, t: f64) -> Vec2 {
    let derivative = |t: f64| match segment {
        PathSeg::Line(l) => l.p1 - l.p0,
        PathSeg::Quad(q) => q.deriv().eval(t).to_vec2(),
        PathSeg::Cubic(c) => c.deriv().eval(t).to_vec2(),
    };
    let d = derivative(t);
    if d.hypot() > 1e-9 {
        return d;
    }
    let nearby = if t > 0.5 { t - 1e-3 } else { t + 1e-3 };
    let d = derivative(nearby);
    if d.hypot() > 1e-9 {
        d
    } else {
        segment.end() - segment.start()
    }
}

/// Path points for the subpaths of `path`, in its own units: a point per vertex with
/// handles towards its control points, quadratic curves raised to cubic ones, and a
/// closed contour's last point merged into its first when they meet.
fn contours_of(path: &BezPath) -> Vec<Contour> {
    let mut contours: Vec<Contour> = Vec::new();
    let handle = |control: Point, anchor: Point| {
        let d = control - anchor;
        (d.hypot() > 1e-12).then_some([d.x, d.y])
    };
    let mut last = Point::ZERO;
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                contours.push(Contour {
                    points: vec![PathPoint {
                        at: [p.x, p.y],
                        in_handle: None,
                        out_handle: None,
                    }],
                    closed: false,
                });
                last = p;
                continue;
            }
            PathEl::ClosePath => {
                if let Some(contour) = contours.last_mut() {
                    contour.closed = true;
                    let n = contour.points.len();
                    if n > 1 {
                        let (a, b) = (contour.points[0].at, contour.points[n - 1].at);
                        if (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9 {
                            let end = contour.points.pop();
                            if let Some(end) = end {
                                contour.points[0].in_handle = end.in_handle;
                            }
                        }
                    }
                    if let Some(first) = contour.points.first() {
                        last = Point::new(first.at[0], first.at[1]);
                    }
                }
                continue;
            }
            _ => {}
        }
        let Some(contour) = contours.last_mut() else {
            continue;
        };
        let (c1, c2, p) = match *element {
            PathEl::LineTo(p) => (last, p, p),
            PathEl::QuadTo(c, p) => (
                last + (c - last) * (2.0 / 3.0),
                p + (c - p) * (2.0 / 3.0),
                p,
            ),
            PathEl::CurveTo(c1, c2, p) => (c1, c2, p),
            _ => continue,
        };
        if let Some(previous) = contour.points.last_mut() {
            previous.out_handle = handle(c1, last);
        }
        contour.points.push(PathPoint {
            at: [p.x, p.y],
            in_handle: handle(c2, p),
            out_handle: None,
        });
        last = p;
    }
    contours.retain(|c| !c.points.is_empty());
    contours
}

/// `path` with every segment of a closed subpath of fewer than three segments split in
/// half, the same curve, so its contour keeps at least three points (a leaf drawn with two
/// curves, for example).
fn with_three_points(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut subpath: Vec<PathEl> = Vec::new();
    let flush = |out: &mut BezPath, subpath: &mut Vec<PathEl>| {
        let closed = subpath.last() == Some(&PathEl::ClosePath);
        let mut single = BezPath::from_vec(std::mem::take(subpath));
        let segments: Vec<PathSeg> = single.segments().collect();
        let distinct = segments
            .iter()
            .filter(|s| s.arclen(ACCURACY) > 1e-9)
            .count();
        if closed && (1..3).contains(&distinct) {
            single = BezPath::new();
            for (i, segment) in segments.iter().enumerate() {
                for half in [segment.subsegment(0.0..0.5), segment.subsegment(0.5..1.0)] {
                    if i == 0 && single.elements().is_empty() {
                        single.move_to(half.start());
                    }
                    match half {
                        PathSeg::Line(l) => single.line_to(l.p1),
                        PathSeg::Quad(q) => single.quad_to(q.p1, q.p2),
                        PathSeg::Cubic(c) => single.curve_to(c.p1, c.p2, c.p3),
                    }
                }
            }
            single.close_path();
        }
        for element in single.elements() {
            out.push(*element);
        }
    };
    for element in path.elements() {
        if matches!(element, PathEl::MoveTo(_)) && !subpath.is_empty() {
            flush(&mut out, &mut subpath);
        }
        subpath.push(*element);
    }
    if !subpath.is_empty() {
        flush(&mut out, &mut subpath);
    }
    out
}

/// A path fitted into the box of its bounds: its contours with points from -1 to 1 across
/// the box, and the box in the path's own units.
#[derive(Clone, Debug, PartialEq)]
pub struct FittedPath {
    /// Its contours, the first one first.
    pub contours: Vec<Contour>,
    /// The box: its left, top, width and height. A side shorter than the minimum the
    /// caller gave is widened to it around its middle, so a straight line still has a box.
    pub bounds: [f64; 4],
}

/// Fits `path` into the box of its exact bounds, each side at least `min_side`: what a
/// shape clip stores (ADR-0036). `None` when the path has no points.
pub fn fit_path(path: &BezPath, min_side: f64) -> Option<FittedPath> {
    let contours = contours_of(&with_three_points(path));
    if contours.is_empty() {
        return None;
    }
    let bounds = kurbo::Shape::bounding_box(path);
    let widen = |low: f64, high: f64| {
        let middle = (low + high) / 2.0;
        let half = ((high - low) / 2.0).max(min_side / 2.0).max(1e-12);
        (middle, half)
    };
    let (cx, hx) = widen(bounds.x0, bounds.x1);
    let (cy, hy) = widen(bounds.y0, bounds.y1);
    let fit = |p: [f64; 2]| [(p[0] - cx) / hx, (p[1] - cy) / hy];
    let scale = |d: [f64; 2]| [d[0] / hx, d[1] / hy];
    let contours = contours
        .into_iter()
        .map(|c| Contour {
            points: c
                .points
                .into_iter()
                .map(|p| PathPoint {
                    at: fit(p.at),
                    in_handle: p.in_handle.map(scale),
                    out_handle: p.out_handle.map(scale),
                })
                .collect(),
            closed: c.closed,
        })
        .collect();
    Some(FittedPath {
        contours,
        bounds: [cx - hx, cy - hy, 2.0 * hx, 2.0 * hy],
    })
}

/// Parses SVG path data (`M 0 0 C 10 0 20 10 20 20`, any of the SVG path commands) and
/// fits it into the box of its bounds, as [`fit_path`].
pub fn parse_path_data(data: &str, min_side: f64) -> Result<FittedPath, String> {
    let path = BezPath::from_svg(data).map_err(|e| format!("invalid SVG path data: {e}"))?;
    if path
        .elements()
        .iter()
        .flat_map(|e| e.end_point())
        .any(|p| !p.is_finite())
    {
        return Err("SVG path data must hold finite numbers".into());
    }
    fit_path(&path, min_side).ok_or_else(|| "the SVG path data draws nothing".into())
}

impl Clip {
    /// The shape of a shape clip at `offset` from its start: its trims and stroke width with
    /// keyframes applied and kept in range, and the draw animations (`draw_on`,
    /// `draw_off`) narrowing what its trims show. `None` for other clips.
    pub fn shape_at(&self, offset: Flicks) -> Option<ShapeContent> {
        let ClipContent::Shape(shape) = &self.content else {
            return None;
        };
        let mut shape = (**shape).clone();
        let number = |path: &str, value: f64| {
            self.keyed(path, offset)
                .and_then(KeyValue::number)
                .unwrap_or(value)
        };
        shape.trim_start = number("shape.trim_start", shape.trim_start).clamp(0.0, 1.0);
        shape.trim_end = number("shape.trim_end", shape.trim_end).clamp(0.0, 1.0);
        shape.trim_offset = number("shape.trim_offset", shape.trim_offset);
        if let Some(stroke) = &mut shape.stroke {
            stroke.width = number("shape.stroke_width", stroke.width).clamp(0.0, 0.5);
        }
        let [from, to] = animate(&self.animations, offset, self.duration).draw;
        if from != 0.0 || to != 1.0 {
            let (start, end) = (shape.trim_start, shape.trim_end);
            shape.trim_start = start + (end - start) * from;
            shape.trim_end = start + (end - start) * to;
        }
        Some(shape)
    }

    /// The follow progress at `offset` from the clip's start, with keyframes applied;
    /// `None` when the clip follows no path.
    pub fn follow_progress_at(&self, offset: Flicks) -> Option<f64> {
        let follow = self.follow.as_ref()?;
        Some(
            self.keyed("follow.progress", offset)
                .and_then(KeyValue::number)
                .unwrap_or(follow.progress),
        )
    }
}

/// The timeline holding clip `id`, with the clip.
fn find_clip<'a>(project: &'a Project, id: &Id) -> Option<(&'a Timeline, &'a Clip)> {
    project
        .timelines
        .values()
        .find_map(|t| t.find_clip(id).map(|(_, clip)| (t, clip)))
}

/// Where `clip` is at `offset` from its start when it follows a shape clip's path
/// (ADR-0036): the point at its progress along the shape's whole outline, placed as the
/// shape clip is placed then (its keyframes and animations, held at its edges outside its
/// time), plus the follow's offset, and the path's direction when it orients. `None` when
/// the clip follows nothing, or the path is missing or has no length.
pub fn follow_placement(project: &Project, clip: &Clip, offset: Flicks) -> Option<FollowPlacement> {
    let follow = clip.follow.as_ref()?;
    let (timeline, path_clip) = find_clip(project, &follow.clip)?;
    let ClipContent::Shape(shape) = &path_clip.content else {
        return None;
    };
    let (width, height) = (
        f64::from(timeline.format.width),
        f64::from(timeline.format.height),
    );
    let at = (clip.start + offset - path_clip.start)
        .max(Flicks::ZERO)
        .min(path_clip.duration);
    let motion = animate(&path_clip.animations, at, path_clip.duration);
    let mut transform = path_clip.transform_at(at);
    transform.position[0] += motion.offset[0];
    transform.position[1] += motion.offset[1];
    transform.scale *= motion.scale;
    transform.rotation += motion.rotation;
    let outline = shape.outline([shape.size[0] * width, shape.size[1] * height]);
    let progress = clip.follow_progress_at(offset)?;
    let closed = !outline.elements().is_empty() && measure(&outline).iter().all(|c| c.closed);
    let fraction = if closed && progress > 1.0 {
        let wrapped = progress.rem_euclid(1.0);
        if wrapped == 0.0 { 1.0 } else { wrapped }
    } else if closed && progress < 0.0 {
        progress.rem_euclid(1.0)
    } else {
        progress.clamp(0.0, 1.0)
    };
    let (point, tangent) = point_along(&outline, fraction)?;
    // The shape's box as placed in the plane: mirrored within its place, scaled and
    // turned clockwise around its anchor, and moved. Turns in depth are not followed.
    let [fx, fy] = transform.size_factors();
    let mirror = |v: Vec2| {
        Vec2::new(
            if transform.flip_horizontal { -v.x } else { v.x },
            if transform.flip_vertical { -v.y } else { v.y },
        )
    };
    let scale = |v: Vec2| Vec2::new(v.x * fx, v.y * fy);
    let (sin, cos) = transform.rotation.to_radians().sin_cos();
    let turn = |v: Vec2| Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos);
    let pivot = Vec2::new(
        transform.anchor[0] * shape.size[0] * width,
        transform.anchor[1] * shape.size[1] * height,
    );
    let placed = pivot + turn(scale(mirror(point.to_vec2()) - pivot));
    let direction = turn(scale(mirror(tangent)));
    Some(FollowPlacement {
        position: [
            transform.position[0] + placed.x / width + follow.offset[0],
            transform.position[1] + placed.y / height + follow.offset[1],
        ],
        rotation: if follow.orient {
            direction.y.atan2(direction.x).to_degrees()
        } else {
            0.0
        },
    })
}

/// Problems with the vector parts of a shape (ADR-0036), as messages.
pub(crate) fn shape_problems(shape: &ShapeContent) -> Vec<String> {
    let within = |v: f64, low: f64, high: f64| v.is_finite() && v >= low && v <= high;
    let mut problems = Vec::new();
    let mut bad = |message: &str| problems.push(message.to_string());
    let path = shape.shape == MaskShape::Path;
    if !path && (!shape.closed || !shape.contours.is_empty()) {
        bad("only path shapes can be open or have more contours");
    }
    if path && !shape.closed && shape.points.len() < 2 {
        bad("an open path needs at least two points");
    }
    for contour in &shape.contours {
        if contour.points.len() < if contour.closed { 3 } else { 2 } {
            bad("a contour needs at least two points, or three when closed");
        }
    }
    let finite = |p: &PathPoint| {
        p.at.iter()
            .chain(p.in_handle.iter().flatten())
            .chain(p.out_handle.iter().flatten())
            .all(|v| v.is_finite())
    };
    if !shape
        .points
        .iter()
        .chain(shape.contours.iter().flat_map(|c| &c.points))
        .all(finite)
    {
        bad("path points and handles must be finite numbers");
    }
    if let Some(gradient) = shape.fill.as_ref().and_then(|f| f.gradient.as_ref())
        && !gradient.angle.is_finite()
    {
        bad("a gradient's angle must be a number of degrees");
    }
    if let Some(stroke) = &shape.stroke {
        if !within(stroke.width, 0.0, 0.5) {
            bad("a stroke's width must be from 0 to 0.5 of the canvas height");
        }
        if !(stroke.miter_limit.is_finite() && stroke.miter_limit >= 1.0) {
            bad("a stroke's miter_limit must be at least 1");
        }
        if stroke.dashes.len() > 32
            || !stroke.dashes.iter().all(|d| within(*d, 0.0, 4.0))
            || (!stroke.dashes.is_empty() && stroke.dashes.iter().sum::<f64>() <= 0.0)
        {
            bad("dashes are up to 32 lengths from 0 to 4 of the canvas height, not all 0");
        }
        if !stroke.dash_offset.is_finite() {
            bad("a stroke's dash_offset must be a finite number");
        }
        if let Some(gradient) = &stroke.gradient
            && !gradient.angle.is_finite()
        {
            bad("a gradient's angle must be a number of degrees");
        }
    }
    if !within(shape.trim_start, 0.0, 1.0) || !within(shape.trim_end, 0.0, 1.0) {
        bad("trim_start and trim_end must be from 0 to 1");
    }
    if !shape.trim_offset.is_finite() {
        bad("trim_offset must be a finite number");
    }
    problems
}

/// Problems with a clip's vector parts on `timeline` (ADR-0036): what it follows, and
/// keyframes of shape and follow properties on clips without them. Codes with messages.
pub(crate) fn clip_problems(timeline: &Timeline, clip: &Clip) -> Vec<(&'static str, String)> {
    let mut problems = Vec::new();
    let shape = matches!(clip.content, ClipContent::Shape(_));
    for path in clip.keyframes.keys() {
        if path.starts_with("shape.") && !shape {
            problems.push((
                "fosscut.model.keyframe_path",
                format!("{path} is a property of shape clips only"),
            ));
        }
        if path.starts_with("follow.") && clip.follow.is_none() {
            problems.push((
                "fosscut.model.keyframe_path",
                format!("{path} needs the clip to follow a path"),
            ));
        }
    }
    if let ClipContent::Shape(content) = &clip.content
        && content.stroke.is_none()
        && clip.keyframes.contains_key("shape.stroke_width")
    {
        problems.push((
            "fosscut.model.keyframe_path",
            "shape.stroke_width needs a stroke".to_string(),
        ));
    }
    let Some(follow) = &clip.follow else {
        return problems;
    };
    let mut bad = |message: String| problems.push(("fosscut.model.follow", message));
    if matches!(
        clip.content,
        ClipContent::Audio { .. } | ClipContent::Adjustment
    ) {
        bad(format!(
            "{} clips have no picture to move along a path",
            clip.content.kind_name()
        ));
    }
    if !(follow.progress.is_finite() && follow.offset.iter().all(|v| v.is_finite())) {
        bad("follow progress and offset must be finite numbers".into());
    }
    if follow.clip == clip.id {
        bad("a clip cannot follow itself".into());
        return problems;
    }
    match timeline.find_clip(&follow.clip) {
        None => bad(format!(
            "{} follows {}, which is not on its timeline",
            clip.id, follow.clip
        )),
        Some((_, target)) => {
            if !matches!(target.content, ClipContent::Shape(_)) {
                bad(format!(
                    "{} follows {}, a {} clip; only shape clips have a path",
                    clip.id,
                    follow.clip,
                    target.content.kind_name()
                ));
            }
            if target.follow.is_some() {
                bad(format!(
                    "{} follows {}, which follows a path itself",
                    clip.id, follow.clip
                ));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Color, IdKind, Interpolation, Keyframe, LineCap, ShapeStroke, TimelineFormat, Track,
        TrackKind,
    };
    use fosscut_time::Rate;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn outlines_measure_like_their_shapes() {
        let circle = ShapeContent::new(MaskShape::Ellipse, [0.1, 0.1], Color::default());
        let length = path_length(&circle.outline([200.0, 200.0]));
        assert!(
            close(length, std::f64::consts::TAU * 100.0, 0.1),
            "{length}"
        );
        let square = ShapeContent::new(MaskShape::Rectangle, [0.1, 0.1], Color::default());
        assert!(close(
            path_length(&square.outline([100.0, 50.0])),
            300.0,
            1e-6
        ));
        let mut round = square.clone();
        round.roundness = 1.0;
        // A 100 x 50 box with fully round ends: two straight 50 px edges and a circle of
        // radius 25.
        let length = path_length(&round.outline([100.0, 50.0]));
        assert!(
            close(length, 100.0 + std::f64::consts::TAU * 25.0, 0.05),
            "{length}"
        );
        let star = ShapeContent::new(MaskShape::Star, [0.1, 0.1], Color::default());
        let start = point_along(&star.outline([100.0, 100.0]), 0.0).unwrap().0;
        assert!(
            close(start.x, 0.0, 1e-9) && close(start.y, -50.0, 1e-9),
            "top point"
        );
        let heart = ShapeContent::new(MaskShape::Heart, [0.1, 0.1], Color::default());
        assert!(path_length(&heart.outline([100.0, 100.0])) > 250.0);
    }

    #[test]
    fn trims_keep_the_part_between_fractions_and_wrap() {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((100.0, 0.0));
        let part = trim_path(&line, 0.3, 0.7, 0.0);
        assert!(close(path_length(&part), 40.0, 1e-6));
        assert_eq!(part.elements()[0], PathEl::MoveTo(Point::new(30.0, 0.0)));
        // Either order, and nothing when empty.
        assert!(close(
            path_length(&trim_path(&line, 0.7, 0.3, 0.0)),
            40.0,
            1e-6
        ));
        assert!(trim_path(&line, 0.5, 0.5, 0.0).elements().is_empty());
        assert_eq!(trim_path(&line, 0.0, 1.0, 0.25), line, "untrimmed");
        // An offset past the end of an open path splits it into two parts.
        let wrapped = trim_path(&line, 0.0, 0.5, 0.75);
        let moves = wrapped
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::MoveTo(_)))
            .count();
        assert_eq!(moves, 2);
        assert!(close(path_length(&wrapped), 50.0, 1e-6));
        // On a closed square it continues through the start without a break.
        let square = ShapeContent::new(MaskShape::Rectangle, [0.1, 0.1], Color::default())
            .outline([100.0, 100.0]);
        let corner = trim_path(&square, 0.0, 0.5, 0.75);
        let moves = corner
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::MoveTo(_)))
            .count();
        assert_eq!(moves, 1);
        assert!(close(path_length(&corner), 200.0, 1e-6));
        // A curve keeps its shape: a quarter of a circle is a quarter of its length.
        let circle = ShapeContent::new(MaskShape::Ellipse, [0.1, 0.1], Color::default())
            .outline([200.0, 200.0]);
        let quarter = trim_path(&circle, 0.0, 0.25, 0.0);
        assert!(close(
            path_length(&quarter),
            std::f64::consts::TAU * 25.0,
            0.05
        ));
        let end = point_along(&quarter, 1.0).unwrap().0;
        assert!(
            close(end.x, 100.0, 0.01) && close(end.y, 0.0, 0.01),
            "{end:?}"
        );
    }

    #[test]
    fn points_along_a_path_carry_its_direction() {
        let fitted = parse_path_data("M 0 0 L 100 0 L 100 100", 1.0).unwrap();
        assert_eq!(fitted.bounds, [0.0, 0.0, 100.0, 100.0]);
        let mut shape = ShapeContent::new(MaskShape::Path, [0.1, 0.1], Color::default());
        shape.points = fitted.contours[0].points.clone();
        shape.closed = false;
        let path = shape.outline([100.0, 100.0]);
        let (point, direction) = point_along(&path, 0.25).unwrap();
        assert!(
            close(point.x, 0.0, 1e-6) && close(point.y, -50.0, 1e-6),
            "{point:?}"
        );
        assert!(direction.x > 0.0 && close(direction.y, 0.0, 1e-9));
        let (point, direction) = point_along(&path, 0.75).unwrap();
        assert!(
            close(point.x, 50.0, 1e-6) && close(point.y, 0.0, 1e-6),
            "{point:?}"
        );
        assert!(
            direction.y > 0.0 && close(direction.x, 0.0, 1e-9),
            "downwards"
        );
        assert!(point_along(&BezPath::new(), 0.5).is_none());
    }

    #[test]
    fn svg_path_data_fits_its_box_with_handles_and_contours() {
        let fitted = parse_path_data(
            "M 10 10 C 40 10 60 30 60 60 Q 30 90 10 60 Z M 20 20 h 10 v 10 h -10 z",
            1.0,
        )
        .unwrap();
        assert_eq!(fitted.contours.len(), 2);
        let first = &fitted.contours[0];
        assert!(first.closed);
        assert_eq!(first.points.len(), 3);
        assert_eq!(first.points[0].at, [-1.0, -1.0]);
        assert!(first.points[0].out_handle.is_some() && first.points[1].in_handle.is_some());
        let square = &fitted.contours[1];
        assert_eq!(
            square.points.len(),
            4,
            "the closing point merges with the first"
        );
        assert!(
            square
                .points
                .iter()
                .all(|p| p.in_handle.is_none() && p.out_handle.is_none())
        );
        // A straight horizontal line still has a box, as tall as the minimum side.
        let line = parse_path_data("M 100 500 L 1800 500", 1.0).unwrap();
        assert_eq!(line.bounds, [100.0, 499.5, 1700.0, 1.0]);
        assert!(!line.contours[0].closed);
        assert_eq!(line.contours[0].points[1].at, [1.0, 0.0]);
        assert!(parse_path_data("M 0 0 X 1", 1.0).is_err());
        // A leaf of two curves keeps three points or more, and its shape.
        let leaf = "M 0 50 C 30 0 70 0 100 50 C 70 100 30 100 0 50 Z";
        let fitted = parse_path_data(leaf, 1.0).unwrap();
        assert_eq!(fitted.contours[0].points.len(), 4);
        let mut shape = ShapeContent::new(MaskShape::Path, [0.1, 0.1], Color::default());
        shape.points = fitted.contours[0].points.clone();
        let [_, _, w, h] = fitted.bounds;
        let length = path_length(&shape.outline([w, h]));
        let original = path_length(&BezPath::from_svg(leaf).unwrap());
        assert!(close(length, original, 1e-3), "{length} {original}");
        assert!(parse_path_data("", 1.0).is_err());
        // Fitting and drawing the fitted points gives the curve back.
        let original = BezPath::from_svg("M 0 0 C 50 -40 100 40 150 0").unwrap();
        let fitted = fit_path(&original, 1.0).unwrap();
        let mut shape = ShapeContent::new(MaskShape::Path, [0.1, 0.1], Color::default());
        shape.points = fitted.contours[0].points.clone();
        shape.closed = false;
        let [_, _, w, h] = fitted.bounds;
        let redrawn = shape.outline([w, h]);
        assert!(close(path_length(&redrawn), path_length(&original), 1e-6));
    }

    fn project_with(clips: Vec<Clip>) -> Project {
        let format = TimelineFormat {
            width: 1000,
            height: 500,
            rate: Rate::FPS_30,
            sample_rate: 48_000,
        };
        let mut project = Project::new(Id::new(IdKind::Project, 1), "p", format);
        let timeline = project.manifest.primary_timeline.clone();
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
        project
    }

    #[test]
    fn followers_sit_on_the_path_as_its_clip_places_it() {
        let seconds = |s: i64| Flicks::from_seconds(s).unwrap();
        // A line across the middle half of the canvas: 500 px wide.
        let mut line = ShapeContent::new(MaskShape::Path, [0.5, 0.1], Color::default());
        let fitted = parse_path_data("M 0 0 L 100 0", 1.0).unwrap();
        line.points = fitted.contours[0].points.clone();
        line.closed = false;
        line.fill = None;
        let path_id = Id::new(IdKind::Clip, 1);
        let mut path = Clip::new(
            path_id.clone(),
            Flicks::ZERO,
            seconds(4),
            ClipContent::Shape(Box::new(line)),
        );
        let dot = ShapeContent::new(MaskShape::Ellipse, [0.02, 0.04], Color::default());
        let mut follower = Clip::new(
            Id::new(IdKind::Clip, 2),
            seconds(1),
            seconds(2),
            ClipContent::Shape(Box::new(dot)),
        );
        follower.follow = Some(Follow {
            clip: path_id.clone(),
            progress: 0.5,
            orient: true,
            offset: [0.0, -0.1],
        });
        let project = project_with(vec![path.clone(), follower.clone()]);
        let placed = follow_placement(&project, &follower, Flicks::ZERO).unwrap();
        assert!(close(placed.position[0], 0.0, 1e-9) && close(placed.position[1], -0.1, 1e-9));
        assert!(close(placed.rotation, 0.0, 1e-9));
        // Progress keyframes move it; the path clip's own placement carries it along.
        follower.keyframes.insert(
            "follow.progress".into(),
            vec![
                Keyframe {
                    time: Flicks::ZERO,
                    value: KeyValue::Number(0.0),
                    interpolation: Interpolation::Linear,
                },
                Keyframe {
                    time: seconds(2),
                    value: KeyValue::Number(1.0),
                    interpolation: Interpolation::Linear,
                },
            ],
        );
        path.transform.rotation = 90.0;
        path.transform.position = [0.1, 0.0];
        let project = project_with(vec![path, follower.clone()]);
        let start = follow_placement(&project, &follower, Flicks::ZERO).unwrap();
        // Turned a quarter clockwise, the line's start (250 px left) is 250 px up.
        assert!(close(start.position[0], 0.1, 1e-9), "{start:?}");
        assert!(close(start.position[1], -0.5 - 0.1, 1e-9), "{start:?}");
        assert!(close(start.rotation, 90.0, 1e-9), "runs downwards");
        let end = follow_placement(&project, &follower, seconds(2)).unwrap();
        assert!(close(end.position[1], 0.5 - 0.1, 1e-9), "{end:?}");
        // Turned around an anchor at the line's start instead, the start stays put.
        let mut anchored = project.clone();
        let timeline = anchored.manifest.primary_timeline.clone();
        let doc = anchored.timelines.get_mut(&timeline).unwrap();
        let line = &mut doc.tracks[0].clips[0];
        line.transform.anchor = [-0.5, 0.0];
        let start = follow_placement(&anchored, &follower, Flicks::ZERO).unwrap();
        assert!(close(start.position[0], 0.1 - 0.25, 1e-9), "{start:?}");
        assert!(close(start.position[1], -0.1, 1e-9), "{start:?}");
        let end = follow_placement(&anchored, &follower, seconds(2)).unwrap();
        assert!(close(end.position[0], 0.1 - 0.25, 1e-9), "{end:?}");
        assert!(close(end.position[1], 1.0 - 0.1, 1e-9), "{end:?}");
        let mut lost = follower.clone();
        lost.follow.as_mut().unwrap().clip = Id::new(IdKind::Clip, 99);
        assert!(follow_placement(&project, &lost, Flicks::ZERO).is_none());
    }

    #[test]
    fn closed_paths_wrap_progress_and_shapes_animate_their_trims() {
        let ring = ShapeContent::new(MaskShape::Ellipse, [0.2, 0.4], Color::default());
        let path_id = Id::new(IdKind::Clip, 1);
        let seconds = |s: i64| Flicks::from_seconds(s).unwrap();
        let mut path = Clip::new(
            path_id.clone(),
            Flicks::ZERO,
            seconds(4),
            ClipContent::Shape(Box::new(ring)),
        );
        let mut follower = Clip::new(
            Id::new(IdKind::Clip, 2),
            Flicks::ZERO,
            seconds(4),
            ClipContent::Shape(Box::new(ShapeContent::new(
                MaskShape::Star,
                [0.05, 0.05],
                Color::default(),
            ))),
        );
        follower.follow = Some(Follow {
            clip: path_id,
            progress: 1.25,
            orient: false,
            offset: [0.0, 0.0],
        });
        let project = project_with(vec![path.clone(), follower.clone()]);
        let placed = follow_placement(&project, &follower, Flicks::ZERO).unwrap();
        // A quarter of the way round from the top, clockwise: the right edge.
        assert!(close(placed.position[0], 0.1, 1e-3), "{placed:?}");
        assert!(close(placed.position[1], 0.0, 1e-3), "{placed:?}");

        path.keyframes.insert(
            "shape.trim_end".into(),
            vec![
                Keyframe {
                    time: Flicks::ZERO,
                    value: KeyValue::Number(0.0),
                    interpolation: Interpolation::Linear,
                },
                Keyframe {
                    time: seconds(2),
                    value: KeyValue::Number(1.0),
                    interpolation: Interpolation::Linear,
                },
            ],
        );
        assert_eq!(path.shape_at(seconds(1)).unwrap().trim_end, 0.5);
        path.animations.exit = Some(
            serde_json::from_value(serde_json::json!({
                "preset": "draw_off",
                "duration": seconds(1),
            }))
            .unwrap(),
        );
        let leaving = path
            .shape_at(Flicks::from_seconds_f64(3.5).unwrap())
            .unwrap();
        assert!(
            leaving.trim_start > 0.0 && leaving.trim_start < 1.0,
            "{leaving:?}"
        );
        assert_eq!(leaving.trim_end, 1.0);
        assert!(follower.shape_at(Flicks::ZERO).unwrap().trim_end == 1.0);
        assert!(
            Clip::new(
                Id::new(IdKind::Clip, 3),
                Flicks::ZERO,
                seconds(1),
                ClipContent::Adjustment
            )
            .shape_at(Flicks::ZERO)
            .is_none()
        );
    }

    #[test]
    fn strokes_open_paths_and_follows_round_trip_sparsely() {
        let mut line = ShapeContent::new(MaskShape::Path, [0.4, 0.1], Color::default());
        line.points = parse_path_data("M 0 0 C 10 -10 20 10 30 0", 1.0)
            .unwrap()
            .contours
            .remove(0)
            .points;
        line.closed = false;
        line.fill = None;
        line.stroke = Some(ShapeStroke {
            cap: LineCap::Round,
            dashes: vec![0.02, 0.01],
            ..ShapeStroke::new(Color::parse("#FF3366").unwrap(), 0.02)
        });
        line.trim_end = 0.5;
        let json = serde_json::to_value(&line).unwrap();
        assert_eq!(json["closed"], false);
        assert!(json["fill"].is_null(), "no fill is stored as null");
        assert_eq!(
            json["stroke"],
            serde_json::json!({"color": "#FF3366", "width": 0.02, "cap": "round", "dashes": [0.02, 0.01]})
        );
        assert_eq!(json["trim_end"], 0.5);
        assert!(json.get("trim_start").is_none() && json.get("contours").is_none());
        let back: ShapeContent = serde_json::from_value(json).unwrap();
        assert_eq!(back, line);
        // Older documents without the new fields read as before: closed, filled white.
        let old: ShapeContent =
            serde_json::from_str(r#"{"shape": "ellipse", "size": [0.2, 0.2]}"#).unwrap();
        assert!(old.closed && old.fill.is_some() && old.stroke.is_none());
        assert_eq!((old.trim_start, old.trim_end), (0.0, 1.0));
        assert!(!old.is_vector() && line.is_vector());

        let mut clip = Clip::new(
            Id::new(IdKind::Clip, 2),
            Flicks::ZERO,
            Flicks::from_seconds(1).unwrap(),
            ClipContent::Shape(Box::new(line)),
        );
        clip.follow = Some(Follow {
            clip: Id::new(IdKind::Clip, 1),
            progress: 0.25,
            orient: true,
            offset: [0.0, 0.0],
        });
        let json = serde_json::to_value(&clip).unwrap();
        assert_eq!(
            json["follow"],
            serde_json::json!({"clip": "clip_0001", "progress": 0.25, "orient": true})
        );
        assert_eq!(serde_json::from_value::<Clip>(json).unwrap(), clip);
    }

    #[test]
    fn validation_refuses_broken_vectors_and_follows() {
        let codes = |project: &Project| {
            let mut found: Vec<String> = crate::validate(project)
                .into_iter()
                .map(|i| format!("{}: {}", i.code, i.message))
                .collect();
            found.sort();
            found
        };
        let mut stroked = ShapeContent::new(MaskShape::Ellipse, [0.2, 0.2], Color::default());
        stroked.stroke = Some(ShapeStroke::new(Color::default(), 0.02));
        let path_id = Id::new(IdKind::Clip, 1);
        let seconds = Flicks::from_seconds(2).unwrap();
        let path = Clip::new(
            path_id.clone(),
            Flicks::ZERO,
            seconds,
            ClipContent::Shape(Box::new(stroked.clone())),
        );
        let mut follower = Clip::new(
            Id::new(IdKind::Clip, 2),
            Flicks::ZERO,
            seconds,
            ClipContent::Shape(Box::new(ShapeContent::new(
                MaskShape::Star,
                [0.05, 0.05],
                Color::default(),
            ))),
        );
        follower.follow = Some(Follow {
            clip: path_id.clone(),
            progress: 0.0,
            orient: false,
            offset: [0.0, 0.0],
        });
        let good = project_with(vec![path.clone(), follower.clone()]);
        assert!(codes(&good).is_empty(), "{:?}", codes(&good));

        // Bad strokes, trims and open shapes that are not paths.
        let mut broken = stroked.clone();
        broken.stroke.as_mut().unwrap().width = 0.9;
        broken.stroke.as_mut().unwrap().dashes = vec![0.0, 0.0];
        broken.trim_end = 1.5;
        broken.closed = false;
        let bad = project_with(vec![Clip::new(
            path_id.clone(),
            Flicks::ZERO,
            seconds,
            ClipContent::Shape(Box::new(broken)),
        )]);
        let found = codes(&bad);
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(found.iter().all(|c| c.starts_with("fosscut.model.shape")));

        // Followers need a shape clip on their timeline that follows nothing itself.
        let mut lost = follower.clone();
        lost.follow.as_mut().unwrap().clip = Id::new(IdKind::Clip, 9);
        assert!(
            codes(&project_with(vec![path.clone(), lost]))[0].contains("is not on its timeline")
        );
        let mut chained = path.clone();
        chained.follow = Some(Follow {
            clip: Id::new(IdKind::Clip, 3),
            progress: 0.0,
            orient: false,
            offset: [0.0, 0.0],
        });
        let third = Clip::new(
            Id::new(IdKind::Clip, 3),
            Flicks::ZERO,
            seconds,
            ClipContent::Shape(Box::new(stroked.clone())),
        );
        let found = codes(&project_with(vec![chained, follower.clone(), third]));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("follows a path itself"));
        let mut not_shape = follower.clone();
        not_shape.follow.as_mut().unwrap().clip = Id::new(IdKind::Clip, 3);
        let title = Clip::new(
            Id::new(IdKind::Clip, 3),
            Flicks::ZERO,
            seconds,
            ClipContent::Adjustment,
        );
        assert!(
            codes(&project_with(vec![path.clone(), not_shape, title]))
                .iter()
                .any(|c| c.contains("only shape clips have a path"))
        );

        // Shape and follow keyframes belong to shape clips and followers.
        let mut keyed = Clip::new(
            Id::new(IdKind::Clip, 4),
            Flicks::ZERO,
            seconds,
            ClipContent::Adjustment,
        );
        keyed.keyframes.insert(
            "shape.trim_end".into(),
            vec![Keyframe {
                time: Flicks::ZERO,
                value: KeyValue::Number(0.5),
                interpolation: Interpolation::Linear,
            }],
        );
        keyed.keyframes.insert(
            "follow.progress".into(),
            vec![Keyframe {
                time: Flicks::ZERO,
                value: KeyValue::Number(0.5),
                interpolation: Interpolation::Linear,
            }],
        );
        let found = codes(&project_with(vec![keyed]));
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found
                .iter()
                .all(|c| c.starts_with("fosscut.model.keyframe_path"))
        );
    }
}
