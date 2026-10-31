//! Subject framing: a crop of another aspect, or a zoomed crop of the same one, that follows
//! a subject through each shot of a clip, for automatic reframing (vertical versions of wide
//! footage) and for camera tracking.
//!
//! **Subjects** come from cached analyses, per frame: a person's landmarks ([`from_pose`]:
//! the face is the point to keep central, and face, shoulders and hips with room for the
//! head are what must stay in the crop), a matte ([`from_alpha`]: the alpha's centroid and
//! the box holding all but 2% of its weight on each side), or a tracked region
//! ([`from_region`]). Frames without one take the subject from the nearest frames that have
//! it, linearly in between.
//!
//! **Each shot is framed on its own**, so no move crosses a cut:
//!
//! 1. **Zoom.** A given zoom, or the largest within `min_zoom` and `max_zoom` that still holds
//!    nine in ten of the shot's subjects with `padding` around them and `headroom` above:
//!    the crop is the largest of the target aspect inside the picture, divided by the zoom,
//!    constant through the shot.
//! 2. **A corridor per axis.** The crop's centre may be anywhere from the subject's focus
//!    less a dead zone (`dead_zone` of the crop's size) to the focus plus it, narrowed to the
//!    centres that keep the subject's box inside the crop, and to those that keep the crop
//!    inside the picture. Vertically, the focus puts the top of the subject `headroom` of the
//!    crop's height below the crop's top. The focus is first median-filtered over 0.3 s (a
//!    window that narrows at the ends of the shot), so a single wrong detection does not move
//!    the camera.
//! 3. **The taut string** through the corridor: the shortest path that stays inside it,
//!    which holds still wherever it can and pans at a steady speed where it must. With free
//!    ends it is also the path of least total variation, and the minimizer of any convex cost
//!    of its steps inside the corridor (Davies and Kovac, "Local extremes, runs, strings and
//!    multiresolution", Annals of Statistics 2001; Grasmair, "The equivalence of the taut
//!    string algorithm and BV-regularization", JMIV 2007). It is found greedily: from each
//!    bend, the range of slopes that pass every later gate narrows until it empties, and the
//!    string bends at the gate that last narrowed the side it ran out on (the funnel of Lee and
//!    Preparata, "Euclidean shortest paths in the presence of rectilinear barriers", Networks
//!    1984, for a corridor monotone in time).
//! 4. **Speed.** Steps longer than `max_speed` crop widths (heights, vertically) per second
//!    are shortened by clamping forward and backward in time and averaging the two, which
//!    keeps the limit without lagging behind the subject.
//! 5. **Ease in and out.** Two passes of a moving average over a quarter of `ease_seconds`
//!    each round every start and stop of a pan into a gradual one over about `ease_seconds`;
//!    an average never speeds a path up, so the limit holds. At the ends of a shot the path
//!    continues at its slope, so a pan under way at a cut keeps its place. The crop is
//!    finally kept inside the picture.
//!
//! Coordinates are fractions of the displayed picture: x from its left edge, y from its top.

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::pose::{LANDMARKS, Landmark, MIN_PRESENCE};
use crate::track::Region;

/// Options of a framing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct FramingOptions {
    /// How far the subject may move from the crop's centre before the camera follows, as a
    /// fraction of the crop's width (or height, vertically), 0 to 0.4 (default 0.1).
    pub dead_zone: f64,
    /// The fastest pan, in crop widths (or heights) per second, 0.05 to 10 (default 1).
    pub max_speed: f64,
    /// How long a pan takes to start and to stop, in seconds, 0 to 3 (default 0.5).
    pub ease_seconds: f64,
    /// Room above the subject, as a fraction of the crop's height, 0 to 0.4 (default 0.08).
    pub headroom: f64,
    /// Room around the subject when choosing the zoom, as a fraction of its size, 0 to 1
    /// (default 0.15).
    pub padding: f64,
    /// The least zoom, 1 or more (default 1: the largest crop of the target aspect).
    pub min_zoom: f64,
    /// The most zoom chosen automatically, from `min_zoom` to 4 (default 1, so the crop
    /// only pans).
    pub max_zoom: f64,
    /// A fixed zoom from 1 to 4, instead of choosing one per shot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f64>,
}

impl Default for FramingOptions {
    fn default() -> FramingOptions {
        FramingOptions {
            dead_zone: 0.1,
            max_speed: 1.0,
            ease_seconds: 0.5,
            headroom: 0.08,
            padding: 0.15,
            min_zoom: 1.0,
            max_zoom: 1.0,
            zoom: None,
        }
    }
}

impl FramingOptions {
    /// The options as they are used, every value inside its range.
    pub fn clamped(&self) -> FramingOptions {
        let d = FramingOptions::default();
        let finite = |v: f64, default: f64| if v.is_finite() { v } else { default };
        let min_zoom = finite(self.min_zoom, d.min_zoom).clamp(1.0, 4.0);
        FramingOptions {
            dead_zone: finite(self.dead_zone, d.dead_zone).clamp(0.0, 0.4),
            max_speed: finite(self.max_speed, d.max_speed).clamp(0.05, 10.0),
            ease_seconds: finite(self.ease_seconds, d.ease_seconds).clamp(0.0, 3.0),
            headroom: finite(self.headroom, d.headroom).clamp(0.0, 0.4),
            padding: finite(self.padding, d.padding).clamp(0.0, 1.0),
            min_zoom,
            max_zoom: finite(self.max_zoom, d.max_zoom).clamp(min_zoom, 4.0),
            zoom: self
                .zoom
                .filter(|z| z.is_finite())
                .map(|z| z.clamp(1.0, 4.0)),
        }
    }
}

/// A subject in one frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Subject {
    /// The point to keep central: x from the left edge.
    pub x: f64,
    /// The point to keep central: y from the top edge.
    pub y: f64,
    /// What must stay inside the crop: left, top, right and bottom edges.
    pub bounds: [f64; 4],
}

impl Subject {
    /// The subject of a stored frame as seen in the picture displayed turned `rotation`
    /// degrees clockwise (0, 90, 180 or 270).
    pub fn turned(self, rotation: u16) -> Subject {
        let point = |x: f64, y: f64| match rotation {
            90 => (1.0 - y, x),
            180 => (1.0 - x, 1.0 - y),
            270 => (y, 1.0 - x),
            _ => (x, y),
        };
        let (x, y) = point(self.x, self.y);
        let [l, t, r, b] = self.bounds;
        let (ax, ay) = point(l, t);
        let (bx, by) = point(r, b);
        Subject {
            x,
            y,
            bounds: [ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)],
        }
    }

    fn width(&self) -> f64 {
        self.bounds[2] - self.bounds[0]
    }

    fn height(&self) -> f64 {
        self.bounds[3] - self.bounds[1]
    }

    fn mix(&self, other: &Subject, t: f64) -> Subject {
        let lerp = |a: f64, b: f64| a + (b - a) * t;
        Subject {
            x: lerp(self.x, other.x),
            y: lerp(self.y, other.y),
            bounds: [0, 1, 2, 3].map(|i| lerp(self.bounds[i], other.bounds[i])),
        }
    }
}

/// Landmarks that make up the face (MediaPipe's 0 to 10: nose, eyes, ears and mouth).
const FACE: std::ops::RangeInclusive<usize> = 0..=10;
/// Shoulders and hips.
const TORSO: [usize; 4] = [11, 12, 23, 24];

/// A person from pose landmarks (fractions of the frame), when `presence` is high enough:
/// the face's centre (or the torso's, or all visible landmarks', when the face is hidden) is
/// the focus; face, shoulders and hips are the bounds, raised for the top of the head by
/// 0.6 of the distance from the nose to the shoulders (or a tenth of the bounds' height).
pub fn from_pose(presence: f32, landmarks: &[Landmark; LANDMARKS]) -> Option<Subject> {
    if presence < MIN_PRESENCE {
        return None;
    }
    let seen = |i: usize| {
        let l = landmarks[i];
        (l.visibility >= 0.5 && l.presence >= 0.5).then_some((f64::from(l.x), f64::from(l.y)))
    };
    let face: Vec<(f64, f64)> = FACE.filter_map(seen).collect();
    let torso: Vec<(f64, f64)> = TORSO.iter().filter_map(|&i| seen(i)).collect();
    let all: Vec<(f64, f64)> = (0..LANDMARKS).filter_map(seen).collect();
    let core: Vec<(f64, f64)> = face.iter().chain(&torso).copied().collect();
    let core = if core.is_empty() { all.clone() } else { core };
    if core.is_empty() {
        return None;
    }
    let centre = |points: &[(f64, f64)]| {
        let n = points.len() as f64;
        (
            points.iter().map(|p| p.0).sum::<f64>() / n,
            points.iter().map(|p| p.1).sum::<f64>() / n,
        )
    };
    let focus = if !face.is_empty() {
        centre(&face)
    } else if !torso.is_empty() {
        centre(&torso)
    } else {
        centre(&all)
    };
    let fold = |f: fn(f64, f64) -> f64, pick: fn(&(f64, f64)) -> f64, start: f64| {
        core.iter().map(pick).fold(start, f)
    };
    let mut bounds = [
        fold(f64::min, |p| p.0, f64::MAX),
        fold(f64::min, |p| p.1, f64::MAX),
        fold(f64::max, |p| p.0, f64::MIN),
        fold(f64::max, |p| p.1, f64::MIN),
    ];
    let head = match (seen(0), seen(11), seen(12)) {
        (Some(nose), Some(a), Some(b)) => 0.6 * ((a.1 + b.1) / 2.0 - nose.1).max(0.0),
        _ => 0.1 * (bounds[3] - bounds[1]),
    };
    bounds[1] -= head;
    Some(Subject {
        x: focus.0,
        y: focus.1,
        bounds: bounds.map(|v| v.clamp(0.0, 1.0)),
    })
}

/// The subject of an alpha matte (`size` bytes, rows top to bottom, 255 subject): its
/// weighted centroid, and the box that leaves 2% of the alpha's weight outside on each
/// side; `None` when the subject covers less than 0.2% of the frame.
pub fn from_alpha(alpha: &[u8], size: (u32, u32)) -> Option<Subject> {
    let (w, h) = (size.0 as usize, size.1 as usize);
    if w == 0 || h == 0 || alpha.len() < w * h {
        return None;
    }
    let mut columns = vec![0f64; w];
    let mut rows = vec![0f64; h];
    for (y, row) in alpha.chunks_exact(w).take(h).enumerate() {
        for (x, &a) in row.iter().enumerate() {
            let a = f64::from(a) / 255.0;
            columns[x] += a;
            rows[y] += a;
        }
    }
    let total: f64 = columns.iter().sum();
    if total < 0.002 * (w * h) as f64 {
        return None;
    }
    let quantile = |mass: &[f64], q: f64| {
        let target = q * total;
        let mut sum = 0.0;
        for (i, m) in mass.iter().enumerate() {
            sum += m;
            if sum >= target {
                return i;
            }
        }
        mass.len() - 1
    };
    let mean = |mass: &[f64]| {
        mass.iter()
            .enumerate()
            .map(|(i, m)| (i as f64 + 0.5) * m)
            .sum::<f64>()
            / total
    };
    Some(Subject {
        x: mean(&columns) / w as f64,
        y: mean(&rows) / h as f64,
        bounds: [
            quantile(&columns, 0.02) as f64 / w as f64,
            quantile(&rows, 0.02) as f64 / h as f64,
            (quantile(&columns, 0.98) + 1) as f64 / w as f64,
            (quantile(&rows, 0.98) + 1) as f64 / h as f64,
        ],
    })
}

/// A tracked region as a subject: its centre is the focus and its box the bounds.
pub fn from_region(region: &Region) -> Subject {
    let half = (region.width / 2.0, region.height / 2.0);
    Subject {
        x: region.x,
        y: region.y,
        bounds: [
            (region.x - half.0).clamp(0.0, 1.0),
            (region.y - half.1).clamp(0.0, 1.0),
            (region.x + half.0).clamp(0.0, 1.0),
            (region.y + half.1).clamp(0.0, 1.0),
        ],
    }
}

/// The crop shown in one frame: its centre and size, as fractions of the picture.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Crop {
    /// Centre x.
    pub x: f64,
    /// Centre y.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// How one shot was framed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShotFraming {
    /// Its first frame.
    pub start: usize,
    /// The frame after its last.
    pub end: usize,
    /// The zoom used.
    pub zoom: f64,
    /// Frames with a subject found in them.
    pub subject_frames: usize,
    /// Of those, frames whose subject's bounds stay inside the crop.
    pub inside_frames: usize,
}

/// A crop per frame and how each shot was framed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Framing {
    /// One crop per frame.
    pub crops: Vec<Crop>,
    /// The shots, in order.
    pub shots: Vec<ShotFraming>,
}

/// Frames a clip: `subjects` has one entry per frame at `fps` frames a second, `shots`
/// the first frame of every shot after the first (unsorted and out-of-range entries are
/// ignored), and `aspect` the crop's width over its height measured in fractions of the
/// picture (the target's pixel aspect over the picture's).
pub fn frame(
    subjects: &[Option<Subject>],
    fps: f64,
    shots: &[usize],
    aspect: f64,
    options: &FramingOptions,
) -> Framing {
    let options = options.clamped();
    let n = subjects.len();
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    };
    let mut starts: Vec<usize> = shots.iter().copied().filter(|&s| s > 0 && s < n).collect();
    starts.sort_unstable();
    starts.dedup();
    starts.insert(0, 0);
    let mut crops = Vec::with_capacity(n);
    let mut framed = Vec::with_capacity(starts.len());
    for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(n);
        if start >= end {
            continue;
        }
        let (shot_crops, report) = frame_shot(&subjects[start..end], fps, aspect, &options, start);
        crops.extend(shot_crops);
        framed.push(report);
    }
    Framing {
        crops,
        shots: framed,
    }
}

/// The largest crop of `aspect` inside the picture: width and height.
fn largest(aspect: f64) -> (f64, f64) {
    if aspect <= 1.0 {
        (aspect, 1.0)
    } else {
        (1.0, 1.0 / aspect)
    }
}

/// The value at `q` (0 to 1) of `values`, which must not be empty.
fn quantile(values: &mut [f64], q: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * q).round() as usize]
}

fn frame_shot(
    subjects: &[Option<Subject>],
    fps: f64,
    aspect: f64,
    options: &FramingOptions,
    offset: usize,
) -> (Vec<Crop>, ShotFraming) {
    let n = subjects.len();
    let (max_w, max_h) = largest(aspect);
    let found: Vec<&Subject> = subjects.iter().flatten().collect();
    let zoom = match options.zoom {
        Some(zoom) => zoom,
        None if found.is_empty() => options.min_zoom,
        None => {
            let mut widths: Vec<f64> = found.iter().map(|s| s.width()).collect();
            let mut heights: Vec<f64> = found.iter().map(|s| s.height()).collect();
            let need_w = quantile(&mut widths, 0.9) * (1.0 + 2.0 * options.padding);
            let need_h =
                quantile(&mut heights, 0.9) * (1.0 + options.padding) / (1.0 - options.headroom);
            let fit = (max_w / need_w.max(1e-6)).min(max_h / need_h.max(1e-6));
            fit.clamp(options.min_zoom, options.max_zoom)
        }
    };
    let (width, height) = (max_w / zoom, max_h / zoom);
    let report = |crops: &[Crop]| ShotFraming {
        start: offset,
        end: offset + n,
        zoom: (zoom * 1000.0).round() / 1000.0,
        subject_frames: found.len(),
        inside_frames: subjects
            .iter()
            .zip(crops)
            .filter(|(s, c)| s.is_some_and(|s| inside(&s, c)))
            .count(),
    };
    if found.is_empty() {
        let crops = vec![
            Crop {
                x: 0.5,
                y: 0.5,
                width,
                height,
            };
            n
        ];
        let framing = report(&crops);
        return (crops, framing);
    }
    let filled = fill(subjects);
    let window = ((0.15 * fps).round() as usize).max(1);
    let focus_x = median_filter(&filled.iter().map(|s| s.x).collect::<Vec<_>>(), window);
    let tops = median_filter(
        &filled.iter().map(|s| s.bounds[1]).collect::<Vec<_>>(),
        window,
    );
    let horizontal = Axis {
        size: width,
        focus: focus_x,
        low: filled.iter().map(|s| s.bounds[0]).collect(),
        high: filled.iter().map(|s| s.bounds[2]).collect(),
    };
    // Vertically the focus is where the crop's centre puts the subject's top `headroom`
    // below the crop's top.
    let vertical = Axis {
        size: height,
        focus: tops
            .iter()
            .map(|top| top - options.headroom * height + height / 2.0)
            .collect(),
        low: filled.iter().map(|s| s.bounds[1]).collect(),
        high: filled.iter().map(|s| s.bounds[3]).collect(),
    };
    let xs = horizontal.path(fps, options);
    let ys = vertical.path(fps, options);
    let crops: Vec<Crop> = xs
        .iter()
        .zip(&ys)
        .map(|(&x, &y)| Crop {
            x,
            y,
            width,
            height,
        })
        .collect();
    let framing = report(&crops);
    (crops, framing)
}

/// Whether `subject`'s bounds are inside `crop`, give or take a thousandth.
fn inside(subject: &Subject, crop: &Crop) -> bool {
    let e = 1e-3;
    let [l, t, r, b] = subject.bounds;
    l >= crop.x - crop.width / 2.0 - e
        && r <= crop.x + crop.width / 2.0 + e
        && t >= crop.y - crop.height / 2.0 - e
        && b <= crop.y + crop.height / 2.0 + e
}

/// Every frame's subject, taken from the nearest frames with one (linearly between two).
fn fill(subjects: &[Option<Subject>]) -> Vec<Subject> {
    let known: Vec<(usize, Subject)> = subjects
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.map(|s| (i, s)))
        .collect();
    let mut next = 0;
    (0..subjects.len())
        .map(|i| {
            while next < known.len() && known[next].0 < i {
                next += 1;
            }
            match (next.checked_sub(1).map(|p| known[p]), known.get(next)) {
                (_, Some(&(j, s))) if j == i => s,
                (Some((a, sa)), Some(&(b, sb))) => sa.mix(&sb, (i - a) as f64 / (b - a) as f64),
                (Some((_, s)), None) | (None, Some(&(_, s))) => s,
                (None, None) => Subject {
                    x: 0.5,
                    y: 0.5,
                    bounds: [0.5, 0.5, 0.5, 0.5],
                },
            }
        })
        .collect()
}

/// The median of each value with its `reach` neighbours on either side; near the ends the
/// window narrows to stay centred, so a steady motion is not pulled inwards there.
fn median_filter(values: &[f64], reach: usize) -> Vec<f64> {
    let n = values.len();
    (0..n)
        .map(|i| {
            let reach = reach.min(i).min(n - 1 - i);
            let (lo, hi) = (i - reach, i + reach + 1);
            let mut window = values[lo..hi].to_vec();
            window.sort_by(f64::total_cmp);
            window[window.len() / 2]
        })
        .collect()
}

/// One axis of a shot: the crop's size along it, the focus per frame, and the subject's
/// bounds per frame.
struct Axis {
    size: f64,
    focus: Vec<f64>,
    low: Vec<f64>,
    high: Vec<f64>,
}

impl Axis {
    /// The crop's centre per frame along this axis.
    fn path(&self, fps: f64, options: &FramingOptions) -> Vec<f64> {
        let half = self.size / 2.0;
        let (min, max) = (half, (1.0 - half).max(half));
        let dead = options.dead_zone * self.size;
        let mut lower = Vec::with_capacity(self.focus.len());
        let mut upper = Vec::with_capacity(self.focus.len());
        for ((&focus, &low), &high) in self.focus.iter().zip(&self.low).zip(&self.high) {
            let (mut lo, mut hi) = (focus - dead, focus + dead);
            // Centres that keep the subject's box inside the crop, when it fits.
            let (keep_lo, keep_hi) = (high - half, low + half);
            if keep_lo <= keep_hi {
                if lo.max(keep_lo) <= hi.min(keep_hi) {
                    lo = lo.max(keep_lo);
                    hi = hi.min(keep_hi);
                } else {
                    (lo, hi) = (keep_lo, keep_hi);
                }
            }
            // And the crop inside the picture.
            let (lo, hi) = (lo.clamp(min, max), hi.clamp(min, max));
            lower.push(lo.min(hi));
            upper.push(hi.max(lo));
        }
        let string = taut_string(&lower, &upper);
        let limited = limit_speed(&string, options.max_speed * self.size / fps);
        let reach = ((options.ease_seconds * fps / 4.0).round() as usize).min(limited.len());
        let eased = moving_average(&moving_average(&limited, reach), reach);
        eased.into_iter().map(|v| v.clamp(min, max)).collect()
    }
}

/// The shortest path through gates `lower[i]..=upper[i]` at evenly spaced times, with free
/// ends; see the [module documentation](self).
pub fn taut_string(lower: &[f64], upper: &[f64]) -> Vec<f64> {
    let n = lower.len().min(upper.len());
    let mut path = vec![0.0; n];
    if n == 0 {
        return path;
    }
    // The free start: one level as long as it fits every gate.
    let (mut lo, mut hi) = (lower[0], upper[0]);
    let (mut lo_at, mut hi_at) = (0, 0);
    let mut anchor = None;
    for i in 1..n {
        if lower[i] > hi {
            anchor = Some((hi_at, hi));
            break;
        }
        if upper[i] < lo {
            anchor = Some((lo_at, lo));
            break;
        }
        if lower[i] > lo {
            lo = lower[i];
            lo_at = i;
        }
        if upper[i] < hi {
            hi = upper[i];
            hi_at = i;
        }
    }
    let Some((mut t0, mut y0)) = anchor else {
        path.fill((lo + hi) / 2.0);
        return path;
    };
    path[..=t0].fill(y0);
    while t0 + 1 < n {
        let (mut smin, mut smax) = (f64::NEG_INFINITY, f64::INFINITY);
        let (mut jmin, mut jmax) = (t0, t0);
        let mut bend = None;
        for i in t0 + 1..n {
            let d = (i - t0) as f64;
            let (a, b) = ((lower[i] - y0) / d, (upper[i] - y0) / d);
            if a > smax {
                bend = Some((jmax, y0 + smax * (jmax - t0) as f64));
                break;
            }
            if b < smin {
                bend = Some((jmin, y0 + smin * (jmin - t0) as f64));
                break;
            }
            if a > smin {
                smin = a;
                jmin = i;
            }
            if b < smax {
                smax = b;
                jmax = i;
            }
        }
        match bend {
            Some((j, y)) => {
                for (k, value) in path.iter_mut().enumerate().take(j + 1).skip(t0 + 1) {
                    *value = y0 + (y - y0) * (k - t0) as f64 / (j - t0) as f64;
                }
                t0 = j;
                y0 = y;
            }
            None => {
                // The free end: as level as the gates allow.
                let slope = if smin > 0.0 {
                    smin
                } else if smax < 0.0 {
                    smax
                } else {
                    0.0
                };
                for (k, value) in path.iter_mut().enumerate().skip(t0 + 1) {
                    *value = y0 + slope * (k - t0) as f64;
                }
                break;
            }
        }
    }
    path
}

/// `path` with no step longer than `step`: the mean of clamping it forward and backward.
fn limit_speed(path: &[f64], step: f64) -> Vec<f64> {
    let n = path.len();
    if n == 0 {
        return Vec::new();
    }
    let mut forward = path.to_vec();
    for i in 1..n {
        forward[i] = forward[i].clamp(forward[i - 1] - step, forward[i - 1] + step);
    }
    let mut backward = path.to_vec();
    for i in (0..n - 1).rev() {
        backward[i] = backward[i].clamp(backward[i + 1] - step, backward[i + 1] + step);
    }
    forward
        .iter()
        .zip(&backward)
        .map(|(a, b)| (a + b) / 2.0)
        .collect()
}

/// The mean of each value with its `reach` neighbours on either side; beyond the ends the
/// values continue at the slope of their first and last steps, so a pan already under way
/// at a cut is not slowed or shifted there.
fn moving_average(values: &[f64], reach: usize) -> Vec<f64> {
    let n = values.len();
    if reach == 0 || n < 2 {
        return values.to_vec();
    }
    let (first, last) = (values[1] - values[0], values[n - 1] - values[n - 2]);
    let at = |i: i64| {
        if i < 0 {
            values[0] + first * i as f64
        } else if i >= n as i64 {
            values[n - 1] + last * (i - n as i64 + 1) as f64
        } else {
            values[i as usize]
        }
    };
    let width = (2 * reach + 1) as f64;
    (0..n as i64)
        .map(|i| (i - reach as i64..=i + reach as i64).map(at).sum::<f64>() / width)
        .collect()
}

/// The time of frame `index` at `fps`, for reports.
pub fn frame_time(index: usize, fps: f64) -> Flicks {
    Flicks::new((index as f64 / fps * fosscut_time::FLICKS_PER_SECOND as f64).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(x: f64) -> Option<Subject> {
        Some(Subject {
            x,
            y: 0.35,
            bounds: [x - 0.08, 0.2, x + 0.08, 0.9],
        })
    }

    /// The largest step of `values`.
    fn fastest(values: &[f64]) -> f64 {
        values
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f64::max)
    }

    const VERTICAL: f64 = (1080.0 / 1920.0) / (1920.0 / 1080.0);

    #[test]
    fn a_still_subject_holds_the_crop_still_on_it() {
        let subjects: Vec<Option<Subject>> = (0..90)
            .map(|i| person(0.7 + 0.01 * ((i as f64) * 1.7).sin()))
            .collect();
        let framing = frame(&subjects, 30.0, &[], VERTICAL, &FramingOptions::default());
        assert_eq!(framing.crops.len(), 90);
        let xs: Vec<f64> = framing.crops.iter().map(|c| c.x).collect();
        assert!(
            fastest(&xs) < 1e-9,
            "jitter inside the dead zone is ignored"
        );
        assert!((xs[0] - 0.7).abs() < 0.04, "{}", xs[0]);
        let crop = framing.crops[0];
        assert!((crop.width - VERTICAL).abs() < 1e-12 && (crop.height - 1.0).abs() < 1e-12);
        assert!(
            (crop.y - 0.5).abs() < 1e-12,
            "a full-height crop cannot move up or down"
        );
        assert_eq!(framing.shots[0].inside_frames, 90);
    }

    #[test]
    fn a_walk_is_followed_smoothly_within_the_speed_limit() {
        // Still, then a walk from 0.25 to 0.75 over 2 s, then still.
        let x = |i: usize| match i {
            0..30 => 0.25,
            30..90 => 0.25 + 0.5 * (i - 30) as f64 / 60.0,
            _ => 0.75,
        };
        let subjects: Vec<Option<Subject>> = (0..150).map(|i| person(x(i))).collect();
        let options = FramingOptions::default();
        let framing = frame(&subjects, 30.0, &[], VERTICAL, &options);
        let xs: Vec<f64> = framing.crops.iter().map(|c| c.x).collect();
        let limit = options.max_speed * VERTICAL / 30.0;
        assert!(fastest(&xs) <= limit + 1e-9, "{} > {limit}", fastest(&xs));
        assert_eq!(
            framing.shots[0].inside_frames, 150,
            "the subject stays in frame"
        );
        // Ease: the first steps of the pan are small.
        let steps: Vec<f64> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        let first = steps.iter().position(|s| s.abs() > 1e-9).unwrap();
        assert!(steps[first].abs() < 0.3 * steps.iter().copied().fold(0.0, f64::max));
        assert!(xs[..10].windows(2).all(|w| (w[1] - w[0]).abs() < 1e-9));
        assert!(xs[140..].windows(2).all(|w| (w[1] - w[0]).abs() < 1e-9));
    }

    #[test]
    fn each_shot_is_framed_on_its_own_and_empty_shots_are_centred() {
        let mut subjects: Vec<Option<Subject>> = (0..60).map(|_| person(0.2)).collect();
        subjects.extend((0..60).map(|_| person(0.8)));
        subjects.extend((0..30).map(|_| None));
        let framing = frame(
            &subjects,
            30.0,
            &[60, 120, 400],
            VERTICAL,
            &FramingOptions::default(),
        );
        assert_eq!(framing.shots.len(), 3);
        let xs: Vec<f64> = framing.crops.iter().map(|c| c.x).collect();
        assert!(xs[..60].iter().all(|x| (x - xs[0]).abs() < 1e-12));
        assert!(xs[60..120].iter().all(|x| (x - xs[60]).abs() < 1e-12));
        assert!((xs[59] - 0.2).abs() < 0.03 && (xs[60] - 0.8).abs() < 0.03);
        assert!(xs[120..].iter().all(|x| (x - 0.5).abs() < 1e-12));
        assert_eq!(framing.shots[2].subject_frames, 0);
        assert_eq!((framing.shots[1].start, framing.shots[1].end), (60, 120));
    }

    #[test]
    fn zoom_follows_the_limits_and_keeps_the_crop_inside_the_picture() {
        // A small subject near the left edge on a same-aspect crop.
        let small = Some(Subject {
            x: 0.1,
            y: 0.5,
            bounds: [0.05, 0.4, 0.15, 0.6],
        });
        let subjects = vec![small; 30];
        let zoomed = FramingOptions {
            max_zoom: 2.0,
            ..FramingOptions::default()
        };
        let framing = frame(&subjects, 30.0, &[], 1.0, &zoomed);
        assert_eq!(framing.shots[0].zoom, 2.0);
        let crop = framing.crops[0];
        assert!((crop.width - 0.5).abs() < 1e-12);
        assert!(
            (crop.x - 0.25).abs() < 1e-12,
            "kept inside the picture: {}",
            crop.x
        );
        let fixed = FramingOptions {
            zoom: Some(1.5),
            ..FramingOptions::default()
        };
        let framing = frame(&subjects, 30.0, &[], 1.0, &fixed);
        assert_eq!(framing.shots[0].zoom, 1.5);
        let none = frame(&subjects, 30.0, &[], 1.0, &FramingOptions::default());
        assert_eq!(none.shots[0].zoom, 1.0);
        let wild = FramingOptions {
            dead_zone: 9.0,
            max_speed: f64::NAN,
            min_zoom: 3.0,
            max_zoom: 2.0,
            zoom: Some(f64::NAN),
            ..FramingOptions::default()
        }
        .clamped();
        assert_eq!(
            (wild.dead_zone, wild.max_speed, wild.max_zoom, wild.zoom),
            (0.4, 1.0, 3.0, None)
        );
    }

    #[test]
    fn the_taut_string_holds_still_and_pans_straight() {
        // A corridor that steps up: flat, a straight ramp, flat.
        let lower: Vec<f64> = (0..30)
            .map(|i| {
                if i < 10 {
                    0.0
                } else if i < 20 {
                    0.1 * (i - 10) as f64
                } else {
                    1.0
                }
            })
            .collect();
        let upper: Vec<f64> = lower.iter().map(|l| l + 0.2).collect();
        let path = taut_string(&lower, &upper);
        for (i, p) in path.iter().enumerate() {
            assert!(*p >= lower[i] - 1e-12 && *p <= upper[i] + 1e-12, "{i}: {p}");
        }
        assert!(path[..10].windows(2).all(|w| (w[0] - w[1]).abs() < 1e-12));
        assert!(path[20..].windows(2).all(|w| (w[0] - w[1]).abs() < 1e-12));
        assert!(taut_string(&[], &[]).is_empty());
        let level = taut_string(&[0.2, 0.3], &[0.6, 0.5]);
        assert!(level.iter().all(|v| (v - 0.4).abs() < 1e-12), "{level:?}");
        assert_eq!(frame_time(30, 30.0), Flicks::from_seconds(1).unwrap());
    }

    #[test]
    fn subjects_come_from_landmarks_mattes_and_regions() {
        let mut landmarks = [Landmark::default(); LANDMARKS];
        let mut put = |i: usize, x: f32, y: f32| {
            landmarks[i] = Landmark {
                x,
                y,
                visibility: 0.9,
                presence: 0.9,
            };
        };
        put(0, 0.5, 0.3);
        put(11, 0.4, 0.5);
        put(12, 0.6, 0.5);
        put(23, 0.45, 0.8);
        put(24, 0.55, 0.8);
        put(15, 0.95, 0.4); // a wrist held out is not part of the bounds
        let subject = from_pose(0.9, &landmarks).unwrap();
        assert!((subject.x - 0.5).abs() < 1e-6 && (subject.y - 0.3).abs() < 1e-6);
        let [l, t, r, b] = subject.bounds;
        assert!((l - 0.4).abs() < 1e-6 && (r - 0.6).abs() < 1e-6 && (b - 0.8).abs() < 1e-6);
        assert!((t - 0.18).abs() < 1e-6, "head room above the nose: {t}");
        assert!(from_pose(0.2, &landmarks).is_none());

        let (w, h) = (100u32, 50u32);
        let mut alpha = vec![0u8; (w * h) as usize];
        for y in 10..30 {
            for x in 60..80 {
                alpha[y * 100 + x] = 255;
            }
        }
        let matte = from_alpha(&alpha, (w, h)).unwrap();
        assert!((matte.x - 0.7).abs() < 1e-9 && (matte.y - 0.4).abs() < 1e-9);
        assert_eq!(matte.bounds, [0.6, 0.2, 0.8, 0.6]);
        assert!(from_alpha(&vec![0u8; 5000], (w, h)).is_none());
        assert!(from_alpha(&alpha, (0, 0)).is_none());

        let region = from_region(&Region {
            x: 0.5,
            y: 0.5,
            width: 0.2,
            height: 0.4,
        });
        let near = |a: [f64; 4], b: [f64; 4]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-12);
        assert!(
            near(region.bounds, [0.4, 0.3, 0.6, 0.7]),
            "{:?}",
            region.bounds
        );
        let turned = region.turned(90);
        assert_eq!((turned.x, turned.y), (0.5, 0.5));
        assert!(
            near(turned.bounds, [0.3, 0.4, 0.7, 0.6]),
            "{:?}",
            turned.bounds
        );
        assert_eq!(subject.turned(0), subject);
    }
}
