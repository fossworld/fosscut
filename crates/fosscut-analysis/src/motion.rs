//! Camera motion and stabilization.
//!
//! [`MotionEstimator`] measures how the picture moves from one frame to the next, on a copy
//! of each frame about 320 pixels wide. A grid of 8 by 6 patches is matched between
//! consecutive frames by block matching, with the zero-mean sum of squared differences as
//! the cost, over a three-level image pyramid from coarse to fine (hierarchical motion
//! estimation: Bergen, Anandan, Hanna and Hingorani, "Hierarchical model-based motion
//! estimation", ECCV 1992). Each match is refined below a pixel by fitting a parabola through
//! the costs next to the best one, along each axis. Patches without texture in two
//! directions are left out: the smaller eigenvalue of their structure tensor is too low
//! (Shi and Tomasi, "Good features to track", CVPR 1994).
//!
//! A similarity transform (shift, rotation and scale about the frame's centre) is fitted to
//! the matches robustly. Every pair of matches far enough apart proposes a transform, as in
//! RANSAC (Fischler and Bolles, "Random sample consensus", CACM 1981), but all pairs are
//! tried, so no random numbers are needed; each is scored with the truncated quadratic cost
//! of MSAC (Torr and Zisserman, "MLESAC", CVIU 2000). The best proposal is refitted by least
//! squares on the matches that agree with it (Umeyama, "Least-squares estimation of
//! transformation parameters between two point patterns", PAMI 1991) until they stop
//! changing.
//!
//! [`stabilize`] smooths the accumulated camera path with a Gaussian (Matsushita, Ofek, Ge,
//! Tang and Shum, "Full-frame video stabilization with motion inpainting", PAMI 2006), as a
//! Gaussian-weighted local linear regression so that a steady pan stays steady up to the
//! ends. Each frame's correction moves it from the actual path onto the smoothed one, and
//! one constant zoom hides the moving edges; where that would need more than 1.5, the
//! corrections are eased off, bounding them by the crop as Grundmann, Kwatra and Essa do
//! ("Auto-directed video stabilization with robust L1 optimal camera paths", CVPR 2011).

use fosscut_time::Flicks;
use rustfft::num_complex::Complex64;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::gray::{Gray, parabola_minimum, pyramid};
use crate::scenes::Luma;

/// Patches across the longer side of the frame, and along the shorter.
const GRID: (usize, usize) = (8, 6);
/// Pyramid levels: the analysis copy and two halvings.
const LEVELS: usize = 3;
/// Patches are `2 * RADIUS + 1` pixels square at every level.
const RADIUS: isize = 8;
/// Search reach at the coarsest level, in its pixels: four times as many at full size.
const COARSE_REACH: isize = 8;
/// Search reach at the finer levels, around the doubled coarser match.
const FINE_REACH: isize = 2;
/// The smallest eigenvalue of a patch's structure tensor, in squared grey levels per pixel,
/// below which it has too little texture in some direction to match.
const MIN_TEXTURE: f64 = 2.0;
/// How far, in pixels, a match may sit from the fitted transform and still agree with it.
const INLIER_DISTANCE: f64 = 1.0;
/// Pairs proposing a transform must be at least this share of the frame's longer side apart.
const MIN_SEPARATION: f64 = 0.2;
/// Fewest agreeing matches for a fit.
const MIN_INLIERS: usize = 4;
/// Agreeing matches for full confidence.
const FULL_INLIERS: f64 = 16.0;
/// Steps fitted with less confidence count as no motion in the path.
const MIN_CONFIDENCE: f64 = 0.1;
/// Gaussian sigma, in frames, at full stabilization strength.
const FULL_SIGMA: f64 = 30.0;
/// The largest zoom stabilization uses.
const MAX_ZOOM: f64 = 1.5;

/// How the picture moved from one frame to the next: a similarity transform in pixels of the
/// frame at the analysis size, plus how sure the fit is.
///
/// A point `p` of the previous frame, measured from the frame's centre, is at
/// `scale * R(rotation) * p + (dx, dy)` in this one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameMotion {
    /// The frame's presentation time.
    pub pts: Flicks,
    /// How far the picture moved right, in pixels at the analysis size.
    pub dx: f64,
    /// How far the picture moved down, in pixels at the analysis size.
    pub dy: f64,
    /// How far the picture turned clockwise about the frame's centre, in radians.
    pub rotation: f64,
    /// How much the picture grew about the frame's centre (1 is no change).
    pub scale: f64,
    /// How sure the fit is, from 0 to 1: the share of the textured patches that agree with
    /// it, reduced when fewer than 16 agree. The first frame has 1; a frame that could not
    /// be compared (no texture, a cut, a size change) has 0 and no motion.
    pub confidence: f64,
}

/// One frame of a [`CameraPath`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PathFrame {
    /// The frame's presentation time.
    pub pts: Flicks,
    /// Where the centre of this frame's view is in the first frame's picture, right of its
    /// centre, as a fraction of the frame's width.
    pub x: f64,
    /// Where the centre of this frame's view is in the first frame's picture, below its
    /// centre, as a fraction of the frame's height.
    pub y: f64,
    /// How far the camera has rolled clockwise since the first frame, in degrees (the
    /// picture turns the other way).
    pub rotation: f64,
    /// How far the camera has zoomed in since the first frame (1 is no change; 2 shows the
    /// picture twice as large).
    pub scale: f64,
    /// The confidence of the step from the previous frame (see [`FrameMotion`]); steps below
    /// 0.1 count as no motion.
    pub confidence: f64,
}

/// The camera's path: per frame its accumulated position, rotation and scale, from the first
/// frame, in fractions of the frame width and height (so it is independent of the analysis
/// size).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraPath {
    /// The frames' width divided by their height, which relates the fractions of width and
    /// height when the path turns.
    pub aspect: f64,
    /// Every frame pushed, in order.
    pub frames: Vec<PathFrame>,
}

/// How to steady one frame.
///
/// Scale the picture by `scale` and turn it clockwise by `rotation` about its centre, then
/// move it by `dx` and `dy`; finally scale the result by the [`Stabilization`]'s `zoom` about
/// the frame's centre. Together: scale by `scale * zoom`, turn by `rotation` and move by
/// `zoom * dx` and `zoom * dy`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Correction {
    /// The frame's presentation time.
    pub pts: Flicks,
    /// How far to move the picture right, as a fraction of the frame's width.
    pub dx: f64,
    /// How far to move the picture down, as a fraction of the frame's height.
    pub dy: f64,
    /// How far to turn the picture clockwise about its centre, in degrees.
    pub rotation: f64,
    /// How much to scale the picture about its centre (1 is no change).
    pub scale: f64,
}

/// Corrections that steady a [`CameraPath`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stabilization {
    /// One correction per frame of the path.
    pub frames: Vec<Correction>,
    /// One constant zoom, from 1 to 1.5, that keeps the corrected picture covering the
    /// frame in every frame.
    pub zoom: f64,
}

/// Measures camera motion frame by frame; see the [module documentation](self).
#[derive(Clone, Debug, Default)]
pub struct MotionEstimator {
    /// The previous frame's pyramid.
    previous: Vec<Gray>,
    /// The first frame's extent at the analysis size.
    extent: Option<(f64, f64)>,
    /// One per frame pushed.
    motions: Vec<FrameMotion>,
}

impl MotionEstimator {
    /// An estimator that has seen no frames.
    pub fn new() -> MotionEstimator {
        MotionEstimator::default()
    }

    /// Adds the next frame, shown at `pts`. Frames can be any size: the estimator works on a
    /// copy averaged down to about 320 pixels wide.
    pub fn push(&mut self, pts: Flicks, frame: Luma<'_>) {
        let current = pyramid(Gray::from_luma(frame), LEVELS, RADIUS as usize);
        let same_size = matches!(
            (self.previous.first(), current.first()),
            (Some(a), Some(b)) if (a.width, a.height) == (b.width, b.height)
        );
        let (step, confidence) = if self.motions.is_empty() {
            (Similarity::IDENTITY, 1.0)
        } else if same_size {
            estimate(&self.previous, &current)
        } else {
            (Similarity::IDENTITY, 0.0)
        };
        if self.extent.is_none() {
            self.extent = current.first().map(|base| base.extent);
        }
        self.motions.push(FrameMotion {
            pts,
            dx: step.t.re,
            dy: step.t.im,
            rotation: step.z.arg(),
            scale: step.z.norm(),
            confidence,
        });
        self.previous = current;
    }

    /// How the picture moved into each frame pushed so far.
    pub fn motions(&self) -> &[FrameMotion] {
        &self.motions
    }

    /// The camera path through every frame pushed.
    pub fn finish(self) -> CameraPath {
        let (width, height) = self.extent.unwrap_or((1.0, 1.0));
        let (width, height) = (width.max(1.0), height.max(1.0));
        // How the first frame's picture maps into the current one.
        let mut content = Similarity::IDENTITY;
        let frames = self
            .motions
            .iter()
            .map(|motion| {
                if motion.confidence >= MIN_CONFIDENCE {
                    content = content.then(Similarity::of(motion));
                }
                let camera = content.inverse();
                PathFrame {
                    pts: motion.pts,
                    x: camera.t.re / width,
                    y: camera.t.im / height,
                    rotation: camera.z.arg().to_degrees(),
                    scale: 1.0 / camera.z.norm(),
                    confidence: motion.confidence,
                }
            })
            .collect();
        CameraPath {
            aspect: width / height,
            frames,
        }
    }
}

/// Corrections that steady `path`, with `strength` from 0 (none) to 1 (a Gaussian of about
/// 30 frames): for each frame, the offset, rotation and scale to apply to the picture, and the
/// zoom that hides the moving edges.
///
/// The correction moves each frame from the path onto its smoothed version. The zoom is the
/// smallest that keeps the picture covering the frame for every correction, at most 1.5;
/// where a frame would need more, its correction is reduced, easing in and out over about a
/// sigma of frames so that the reduction does not jump.
pub fn stabilize(path: &CameraPath, strength: f64) -> Stabilization {
    let strength = if strength.is_nan() {
        0.0
    } else {
        strength.clamp(0.0, 1.0)
    };
    let sigma = FULL_SIGMA * strength;
    let still = || Stabilization {
        frames: path
            .frames
            .iter()
            .map(|frame| Correction {
                pts: frame.pts,
                dx: 0.0,
                dy: 0.0,
                rotation: 0.0,
                scale: 1.0,
            })
            .collect(),
        zoom: 1.0,
    };
    if sigma < 1e-6 || path.frames.is_empty() {
        return still();
    }
    let aspect = if path.aspect.is_finite() && path.aspect > 0.0 {
        path.aspect
    } else {
        1.0
    };
    // The path in units of the frame's height, so that turns keep distances.
    let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
    let xs: Vec<f64> = path.frames.iter().map(|f| finite(f.x) * aspect).collect();
    let ys: Vec<f64> = path.frames.iter().map(|f| finite(f.y)).collect();
    let angles: Vec<f64> = path
        .frames
        .iter()
        .map(|f| finite(f.rotation).to_radians())
        .collect();
    let zooms: Vec<f64> = path
        .frames
        .iter()
        .map(|f| {
            if f.scale.is_finite() && f.scale > 0.0 {
                f.scale.ln()
            } else {
                0.0
            }
        })
        .collect();
    let (smooth_x, smooth_y) = (smooth(&xs, sigma), smooth(&ys, sigma));
    let (smooth_angle, smooth_zoom) = (smooth(&angles, sigma), smooth(&zooms, sigma));
    let corrections: Vec<Similarity> = (0..path.frames.len())
        .map(|k| {
            let actual = Similarity::camera(xs[k], ys[k], angles[k], zooms[k]);
            let steady =
                Similarity::camera(smooth_x[k], smooth_y[k], smooth_angle[k], smooth_zoom[k]);
            // Picture point -> where the actual camera saw it -> where the steady one does.
            actual.then(steady.inverse())
        })
        .collect();

    // Ease corrections off where they would need more than the largest zoom, changing the
    // share kept by at most 1 / sigma per frame.
    let mut kept: Vec<f64> = corrections
        .iter()
        .map(|&c| largest_share(c, aspect, 1.0))
        .collect();
    let ramp = 1.0 / sigma.max(1.0);
    for k in 1..kept.len() {
        kept[k] = kept[k].min(kept[k - 1] + ramp);
    }
    for k in (0..kept.len().saturating_sub(1)).rev() {
        kept[k] = kept[k].min(kept[k + 1] + ramp);
    }
    let limited: Vec<Similarity> = corrections
        .iter()
        .zip(&kept)
        .map(|(&c, &share)| partial(c, largest_share(c, aspect, share)))
        .collect();
    let zoom = limited
        .iter()
        .map(|&c| needed_zoom(c, aspect))
        .fold(1.0, f64::max)
        .min(MAX_ZOOM);
    Stabilization {
        frames: path
            .frames
            .iter()
            .zip(&limited)
            .map(|(frame, c)| Correction {
                pts: frame.pts,
                dx: c.t.re / aspect,
                dy: c.t.im,
                rotation: c.z.arg().to_degrees(),
                scale: c.z.norm(),
            })
            .collect(),
        zoom,
    }
}

/// A similarity transform of the plane, as complex numbers: `p -> z * p + t`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Similarity {
    /// Scale and rotation.
    z: Complex64,
    /// Translation.
    t: Complex64,
}

impl Similarity {
    /// No change.
    const IDENTITY: Similarity = Similarity {
        z: Complex64::new(1.0, 0.0),
        t: Complex64::new(0.0, 0.0),
    };

    /// The transform `motion` describes.
    fn of(motion: &FrameMotion) -> Similarity {
        Similarity {
            z: Complex64::from_polar(motion.scale, motion.rotation),
            t: Complex64::new(motion.dx, motion.dy),
        }
    }

    /// The camera at `(x, y)`, rolled by `angle` radians and zoomed in by `e^log_zoom`, as
    /// the map from its frame's points to the first frame's.
    fn camera(x: f64, y: f64, angle: f64, log_zoom: f64) -> Similarity {
        Similarity {
            z: Complex64::from_polar((-log_zoom).exp(), angle),
            t: Complex64::new(x, y),
        }
    }

    /// Where it takes `p`.
    fn apply(self, p: Complex64) -> Complex64 {
        self.z * p + self.t
    }

    /// This transform followed by `next`.
    fn then(self, next: Similarity) -> Similarity {
        Similarity {
            z: next.z * self.z,
            t: next.z * self.t + next.t,
        }
    }

    /// The transform that undoes this one.
    fn inverse(self) -> Similarity {
        let z = self.z.inv();
        Similarity { z, t: -self.t * z }
    }
}

/// `correction` with only `share` of its shift, turn and scale.
fn partial(correction: Similarity, share: f64) -> Similarity {
    Similarity {
        z: Complex64::from_polar(correction.z.norm().powf(share), correction.z.arg() * share),
        t: correction.t * share,
    }
}

/// The smallest zoom about the centre that keeps the picture, moved by `correction`, covering
/// the whole frame (in units of the frame's height, `aspect` wide); infinite when no zoom can.
fn needed_zoom(correction: Similarity, aspect: f64) -> f64 {
    let (half_width, half_height) = (aspect / 2.0, 0.5);
    let back = correction.inverse();
    // With zoom `1 / u`, the frame's corner `c` shows the picture's point `back(u * c)`: a
    // straight line from the point the frame's centre shows (u = 0) as u grows.
    let origin = back.t;
    if !(origin.re.abs() < half_width && origin.im.abs() < half_height) {
        return f64::INFINITY;
    }
    let mut reach = 1.0f64;
    for (cx, cy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        let direction = back.z * Complex64::new(cx * half_width, cy * half_height);
        for (start, step, limit) in [
            (origin.re, direction.re, half_width),
            (origin.im, direction.im, half_height),
        ] {
            if step > 0.0 {
                reach = reach.min((limit - start) / step);
            } else if step < 0.0 {
                reach = reach.min((-limit - start) / step);
            }
        }
    }
    1.0 / reach
}

/// The largest share of `correction`, up to `upper`, that needs a zoom of at most 1.5.
fn largest_share(correction: Similarity, aspect: f64, upper: f64) -> f64 {
    if needed_zoom(partial(correction, upper), aspect) <= MAX_ZOOM {
        return upper;
    }
    let (mut low, mut high) = (0.0, upper);
    for _ in 0..40 {
        let middle = (low + high) / 2.0;
        if needed_zoom(partial(correction, middle), aspect) <= MAX_ZOOM {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

/// `values` smoothed by a local linear regression with Gaussian weights of `sigma` samples:
/// in the middle this is a Gaussian blur, and a straight line stays straight up to the ends.
fn smooth(values: &[f64], sigma: f64) -> Vec<f64> {
    let reach = (3.0 * sigma).ceil() as usize;
    let weights: Vec<f64> = (0..=reach)
        .map(|d| (-0.5 * (d as f64 / sigma).powi(2)).exp())
        .collect();
    (0..values.len())
        .map(|k| {
            let (mut s0, mut s1, mut s2, mut t0, mut t1) = (0.0, 0.0, 0.0, 0.0, 0.0);
            let last = (k + reach).min(values.len() - 1);
            for (j, &value) in values
                .iter()
                .enumerate()
                .take(last + 1)
                .skip(k.saturating_sub(reach))
            {
                let d = j as f64 - k as f64;
                let w = weights[j.abs_diff(k)];
                s0 += w;
                s1 += w * d;
                s2 += w * d * d;
                t0 += w * value;
                t1 += w * d * value;
            }
            let determinant = s0 * s2 - s1 * s1;
            if determinant > 1e-9 * s0 * s0 {
                (s2 * t0 - s1 * t1) / determinant
            } else {
                t0 / s0
            }
        })
        .collect()
}

/// One patch's centre in the previous frame and where it matched in the current one, from
/// the frame's centre.
#[derive(Clone, Copy, Debug)]
struct Match {
    from: Complex64,
    to: Complex64,
}

/// How the picture moved from `previous` to `current` (pyramids of the same size), and how
/// sure the fit is.
fn estimate(previous: &[Gray], current: &[Gray]) -> (Similarity, f64) {
    let none = (Similarity::IDENTITY, 0.0);
    let (Some(base), Some(_)) = (previous.first(), current.first()) else {
        return none;
    };
    if base.is_empty() {
        return none;
    }
    let levels = previous.len().min(current.len());
    let (columns, rows) = if base.width >= base.height {
        GRID
    } else {
        (GRID.1, GRID.0)
    };
    let centre = Complex64::new(base.extent.0 / 2.0 - 0.5, base.extent.1 / 2.0 - 0.5);
    let mut matches = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        for column in 0..columns {
            let x = ((column as f64 + 0.5) * base.width as f64 / columns as f64 - 0.5).round();
            let y = ((row as f64 + 0.5) * base.height as f64 / rows as f64 - 0.5).round();
            let (x, y) = (x as isize, y as isize);
            if texture(base, x, y) < MIN_TEXTURE {
                continue;
            }
            let shift = match_patch(&previous[..levels], &current[..levels], x, y);
            let from = Complex64::new(x as f64, y as f64) - centre;
            matches.push(Match {
                from,
                to: from + shift,
            });
        }
    }
    let separation = MIN_SEPARATION * base.width.max(base.height) as f64;
    let Some((fit, inliers)) = fit_similarity(&matches, separation) else {
        return none;
    };
    let agree = inliers as f64;
    let confidence = agree / matches.len() as f64 * (agree / FULL_INLIERS).min(1.0);
    (fit, confidence)
}

/// The smaller eigenvalue of the structure tensor of the patch at `(x, y)`, per pixel.
fn texture(image: &Gray, x: isize, y: isize) -> f64 {
    let (mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0);
    for py in y - RADIUS..=y + RADIUS {
        for px in x - RADIUS..=x + RADIUS {
            let gx = f64::from(image.clamped(px + 1, py) - image.clamped(px - 1, py)) / 2.0;
            let gy = f64::from(image.clamped(px, py + 1) - image.clamped(px, py - 1)) / 2.0;
            xx += gx * gx;
            yy += gy * gy;
            xy += gx * gy;
        }
    }
    let n = ((2 * RADIUS + 1) * (2 * RADIUS + 1)) as f64;
    let (xx, yy, xy) = (xx / n, yy / n, xy / n);
    (xx + yy) / 2.0 - (((xx - yy) / 2.0).powi(2) + xy * xy).sqrt()
}

/// Where the patch at `(x, y)` of `previous` moved to in `current`, searched coarse to fine
/// and refined below a pixel.
fn match_patch(previous: &[Gray], current: &[Gray], x: isize, y: isize) -> Complex64 {
    let top = previous.len().saturating_sub(1);
    let mut shift = (0isize, 0isize);
    for level in (0..=top).rev() {
        let (a, b) = (&previous[level], &current[level]);
        let (cx, cy) = (at_level(x, level), at_level(y, level));
        let (around, reach) = if level == top {
            ((0, 0), COARSE_REACH)
        } else {
            ((2 * shift.0, 2 * shift.1), FINE_REACH)
        };
        let mut best = f64::INFINITY;
        for sy in around.1 - reach..=around.1 + reach {
            for sx in around.0 - reach..=around.0 + reach {
                let cost = zssd(a, cx, cy, b, cx + sx, cy + sy);
                if cost < best {
                    best = cost;
                    shift = (sx, sy);
                }
            }
        }
    }
    let (a, b) = (&previous[0], &current[0]);
    let cost = |dx: isize, dy: isize| zssd(a, x, y, b, x + shift.0 + dx, y + shift.1 + dy);
    let at = cost(0, 0);
    let fx = parabola_minimum(cost(-1, 0), at, cost(1, 0));
    let fy = parabola_minimum(cost(0, -1), at, cost(0, 1));
    Complex64::new(shift.0 as f64 + fx, shift.1 as f64 + fy)
}

/// A full-size pixel coordinate at pyramid `level`.
fn at_level(coordinate: isize, level: usize) -> isize {
    ((coordinate as f64 + 0.5) / f64::from(1u32 << level) - 0.5).round() as isize
}

/// The zero-mean sum of squared differences between the patch of `a` centred at `(ax, ay)`
/// and the patch of `b` centred at `(bx, by)`, repeating edge pixels outside.
fn zssd(a: &Gray, ax: isize, ay: isize, b: &Gray, bx: isize, by: isize) -> f64 {
    let side = (2 * RADIUS + 1) as usize;
    let inside = |image: &Gray, x: isize, y: isize| {
        x >= RADIUS
            && y >= RADIUS
            && x + RADIUS < image.width as isize
            && y + RADIUS < image.height as isize
    };
    let (mut sum, mut squares) = (0.0f64, 0.0f64);
    if inside(a, ax, ay) && inside(b, bx, by) {
        for dy in -RADIUS..=RADIUS {
            let row_a = &a.data[(ay + dy) as usize * a.width + (ax - RADIUS) as usize..][..side];
            let row_b = &b.data[(by + dy) as usize * b.width + (bx - RADIUS) as usize..][..side];
            let (mut s, mut q) = (0.0f32, 0.0f32);
            for (&p, &r) in row_a.iter().zip(row_b) {
                let d = p - r;
                s += d;
                q += d * d;
            }
            sum += f64::from(s);
            squares += f64::from(q);
        }
    } else {
        for dy in -RADIUS..=RADIUS {
            for dx in -RADIUS..=RADIUS {
                let d = f64::from(a.clamped(ax + dx, ay + dy) - b.clamped(bx + dx, by + dy));
                sum += d;
                squares += d * d;
            }
        }
    }
    squares - sum * sum / (side * side) as f64
}

/// The similarity that most matches agree with, refitted on them, and how many agree.
fn fit_similarity(matches: &[Match], min_separation: f64) -> Option<(Similarity, usize)> {
    if matches.len() < MIN_INLIERS {
        return None;
    }
    let limit = INLIER_DISTANCE * INLIER_DISTANCE;
    let error = |fit: Similarity, m: &Match| (fit.apply(m.from) - m.to).norm_sqr();
    let mut best: Option<(f64, Similarity)> = None;
    for (i, a) in matches.iter().enumerate() {
        for b in &matches[i + 1..] {
            let span = a.from - b.from;
            if span.norm() < min_separation {
                continue;
            }
            let z = (a.to - b.to) / span;
            // Frame to frame, the picture neither halves nor turns over.
            if !(0.8..=1.25).contains(&z.norm()) || z.arg().abs() > 0.35 {
                continue;
            }
            let fit = Similarity {
                z,
                t: a.to - z * a.from,
            };
            let cost: f64 = matches.iter().map(|m| error(fit, m).min(limit)).sum();
            if best.is_none_or(|(lowest, _)| cost < lowest) {
                best = Some((cost, fit));
            }
        }
    }
    let (_, mut fit) = best?;
    let mut inliers: Vec<bool> = Vec::new();
    for _ in 0..8 {
        let agree: Vec<bool> = matches.iter().map(|m| error(fit, m) < limit).collect();
        if agree == inliers {
            break;
        }
        if agree.iter().filter(|&&a| a).count() < MIN_INLIERS {
            return None;
        }
        fit = least_squares(matches, &agree)?;
        inliers = agree;
    }
    let count = matches.iter().filter(|m| error(fit, m) < limit).count();
    (count >= MIN_INLIERS).then_some((fit, count))
}

/// The least-squares similarity from the chosen matches' `from` to their `to`.
fn least_squares(matches: &[Match], chosen: &[bool]) -> Option<Similarity> {
    let picked: Vec<&Match> = matches
        .iter()
        .zip(chosen)
        .filter_map(|(m, &c)| c.then_some(m))
        .collect();
    if picked.len() < 2 {
        return None;
    }
    let n = picked.len() as f64;
    let from_mean = picked.iter().map(|m| m.from).sum::<Complex64>() / n;
    let to_mean = picked.iter().map(|m| m.to).sum::<Complex64>() / n;
    let (mut cross, mut spread) = (Complex64::new(0.0, 0.0), 0.0);
    for m in &picked {
        let (a, b) = (m.from - from_mean, m.to - to_mean);
        cross += b * a.conj();
        spread += a.norm_sqr();
    }
    if spread <= f64::EPSILON {
        return None;
    }
    let z = cross / spread;
    Some(Similarity {
        z,
        t: to_mean - z * from_mean,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use fosscut_time::Rate;

    use super::*;
    use crate::gray::testing::{Random, plane, texture};

    /// Frame `k` of a camera that moved by `steps` (one per frame after the first, in
    /// analysis pixels), over `world`, rendered `factor` times the analysis size of 320 by
    /// 180.
    fn render(
        steps: &[Similarity],
        factor: u32,
        world: impl Fn(f64, f64, usize) -> f64,
    ) -> Vec<Vec<u8>> {
        let mut content = Similarity::IDENTITY;
        let mut frames = Vec::new();
        for k in 0..=steps.len() {
            if k > 0 {
                content = content.then(steps[k - 1]);
            }
            let camera = content.inverse();
            let f = f64::from(factor);
            frames.push(plane(320 * factor, 180 * factor, |x, y| {
                let q = Complex64::new((x + 0.5) / f - 0.5 - 159.5, (y + 0.5) / f - 0.5 - 89.5);
                let w = camera.apply(q);
                world(w.re, w.im, k)
            }));
        }
        frames
    }

    fn luma(data: &[u8], factor: u32) -> Luma<'_> {
        Luma {
            width: 320 * factor,
            height: 180 * factor,
            stride: (320 * factor) as usize,
            data,
        }
    }

    fn estimate_all(frames: &[Vec<u8>], factor: u32) -> MotionEstimator {
        let mut estimator = MotionEstimator::new();
        for (k, frame) in frames.iter().enumerate() {
            estimator.push(Rate::FPS_30.frames_to_flicks(k as i64), luma(frame, factor));
        }
        estimator
    }

    fn step(dx: f64, dy: f64, degrees: f64, scale: f64) -> Similarity {
        Similarity {
            z: Complex64::from_polar(scale, degrees.to_radians()),
            t: Complex64::new(dx, dy),
        }
    }

    fn scenery(u: f64, v: f64, _: usize) -> f64 {
        texture(u + 1000.0, v + 700.0, 7)
    }

    #[test]
    fn shifts_and_turns_are_recovered_per_frame() {
        let steps: Vec<Similarity> = (0..12)
            .map(|k| {
                let k = f64::from(k);
                step(
                    6.0 * (1.3 * k).sin(),
                    4.0 * (0.9 * k).cos(),
                    0.6 * (2.1 * k).sin(),
                    1.0,
                )
            })
            .collect();
        // Rendered at 640 by 360, so the estimator also averages down by 2.
        let estimator = estimate_all(&render(&steps, 2, scenery), 2);
        let motions = estimator.motions();
        assert_eq!(motions.len(), 13);
        assert_eq!(motions[0].confidence, 1.0);
        let (mut worst_shift, mut worst_turn) = (0.0f64, 0.0f64);
        for (motion, truth) in motions[1..].iter().zip(&steps) {
            worst_shift = worst_shift
                .max((motion.dx - truth.t.re).abs())
                .max((motion.dy - truth.t.im).abs());
            worst_turn = worst_turn.max((motion.rotation - truth.z.arg()).to_degrees().abs());
            assert!((motion.scale - 1.0).abs() < 0.003, "{motion:?}");
            assert!(motion.confidence > 0.9, "{motion:?}");
        }
        println!("worst shift error {worst_shift:.3} px, turn error {worst_turn:.4} degrees");
        assert!(worst_shift < 0.3, "{worst_shift}");
        assert!(worst_turn < 0.2, "{worst_turn}");
    }

    #[test]
    fn zooms_and_large_shifts_are_recovered() {
        let steps = [
            step(20.0, -12.0, 0.0, 1.0),
            step(-25.0, 8.0, 1.5, 1.0),
            step(1.0, 2.0, 0.0, 1.03),
            step(0.0, 0.0, -1.0, 0.97),
        ];
        let estimator = estimate_all(&render(&steps, 1, scenery), 1);
        for (motion, truth) in estimator.motions()[1..].iter().zip(&steps) {
            assert!((motion.dx - truth.t.re).abs() < 0.3, "{motion:?}");
            assert!((motion.dy - truth.t.im).abs() < 0.3, "{motion:?}");
            assert!((motion.rotation - truth.z.arg()).to_degrees().abs() < 0.2);
            assert!((motion.scale - truth.z.norm()).abs() < 0.003, "{motion:?}");
        }
    }

    #[test]
    fn a_moving_object_does_not_pull_the_fit() {
        // The camera drifts by (2, -1) per frame while a textured box covering a fifth of the
        // frame slides 9 pixels right per frame.
        let steps = vec![step(2.0, -1.0, 0.0, 1.0); 6];
        let frames = render(&steps, 1, |u, v, k| {
            let (left, top) = (-70.0 + 9.0 * k as f64 - 2.0 * k as f64, -40.0 + k as f64);
            if (left..left + 140.0).contains(&u) && (top..top + 80.0).contains(&v) {
                texture(u - left + 50.0, v - top + 50.0, 99)
            } else {
                scenery(u, v, k)
            }
        });
        let estimator = estimate_all(&frames, 1);
        let confidences: Vec<String> = estimator.motions()[1..]
            .iter()
            .map(|m| format!("{:.2}", m.confidence))
            .collect();
        println!("confidence with a moving object: {}", confidences.join(" "));
        for motion in &estimator.motions()[1..] {
            assert!((motion.dx - 2.0).abs() < 0.3, "{motion:?}");
            assert!((motion.dy + 1.0).abs() < 0.3, "{motion:?}");
            assert!(
                motion.confidence > 0.5 && motion.confidence < 0.95,
                "{motion:?}"
            );
        }
    }

    #[test]
    fn flat_frames_and_size_changes_have_no_confidence() {
        let flat = vec![128u8; 320 * 180];
        let mut estimator = MotionEstimator::new();
        for k in 0..3 {
            estimator.push(Flicks::new(k), luma(&flat, 1));
        }
        let other = vec![128u8; 640 * 180];
        estimator.push(
            Flicks::new(3),
            Luma {
                width: 640,
                height: 180,
                stride: 640,
                data: &other,
            },
        );
        let confidences: Vec<f64> = estimator.motions().iter().map(|m| m.confidence).collect();
        assert_eq!(confidences, [1.0, 0.0, 0.0, 0.0]);
        let path = estimator.finish();
        assert!(
            path.frames
                .iter()
                .all(|f| f.x == 0.0 && f.y == 0.0 && f.scale == 1.0)
        );
        assert!((path.aspect - 16.0 / 9.0).abs() < 1e-12);
        assert!(MotionEstimator::new().finish().frames.is_empty());
    }

    #[test]
    fn the_path_accumulates_the_camera_motion() {
        // A pan right and down with shake: the picture moves the other way.
        let mut random = Random(5);
        let steps: Vec<Similarity> = (0..30)
            .map(|_| {
                step(
                    -1.5 + 2.0 * random.signed(),
                    -0.8 + 2.0 * random.signed(),
                    0.3 * random.signed(),
                    1.0,
                )
            })
            .collect();
        let path = estimate_all(&render(&steps, 1, scenery), 1).finish();
        let mut content = Similarity::IDENTITY;
        let mut worst = (0.0f64, 0.0f64);
        for (frame, truth) in path.frames[1..].iter().zip(&steps) {
            content = content.then(*truth);
            let camera = content.inverse();
            let error = (frame.x * 320.0 - camera.t.re).hypot(frame.y * 180.0 - camera.t.im);
            let turn = (frame.rotation - camera.z.arg().to_degrees()).abs();
            worst = (worst.0.max(error), worst.1.max(turn));
        }
        println!("after 30 frames: {:.3} px, {:.4} degrees", worst.0, worst.1);
        assert!(worst.0 < 1.0 && worst.1 < 0.3, "{worst:?}");
        let last = &path.frames[30];
        assert!(
            last.x > 0.1 && last.y > 0.03,
            "the camera panned right and down: {last:?}"
        );
    }

    /// A path of `frames` frames: a pan of `pan` (fractions of width and height per frame)
    /// with a sideways sway of `sway` (fractions, every 60 frames) and uniform shake of
    /// `shake` (fractions) and up to `roll` degrees.
    fn shaky_path(frames: usize, pan: (f64, f64), sway: f64, shake: f64, roll: f64) -> CameraPath {
        let mut random = Random(11);
        CameraPath {
            aspect: 16.0 / 9.0,
            frames: (0..frames)
                .map(|k| PathFrame {
                    pts: Rate::FPS_30.frames_to_flicks(k as i64),
                    x: pan.0 * k as f64
                        + sway * (k as f64 * std::f64::consts::TAU / 60.0).sin()
                        + shake * random.signed(),
                    y: pan.1 * k as f64 + shake * random.signed(),
                    rotation: roll * random.signed(),
                    scale: 1.0 + 0.01 * random.signed(),
                    confidence: 1.0,
                })
                .collect(),
        }
    }

    /// The steadied camera of each frame: the actual camera with its correction undone.
    fn steadied(path: &CameraPath, stabilization: &Stabilization) -> Vec<Similarity> {
        let aspect = path.aspect;
        path.frames
            .iter()
            .zip(&stabilization.frames)
            .map(|(f, c)| {
                let actual =
                    Similarity::camera(f.x * aspect, f.y, f.rotation.to_radians(), f.scale.ln());
                let correction = Similarity {
                    z: Complex64::from_polar(c.scale, c.rotation.to_radians()),
                    t: Complex64::new(c.dx * aspect, c.dy),
                };
                correction.inverse().then(actual)
            })
            .collect()
    }

    /// Whether every frame's corrected picture, zoomed by `zoom`, covers the frame.
    fn covered(path: &CameraPath, stabilization: &Stabilization, zoom: f64) -> bool {
        let (w, h) = (path.aspect / 2.0, 0.5);
        stabilization.frames.iter().all(|c| {
            let z = Complex64::from_polar(c.scale * zoom, c.rotation.to_radians());
            let t = Complex64::new(c.dx * path.aspect, c.dy) * zoom;
            [(w, h), (-w, h), (w, -h), (-w, -h)].iter().all(|&(x, y)| {
                let source = (Complex64::new(x, y) - t) / z;
                source.re.abs() <= w + 1e-9 && source.im.abs() <= h + 1e-9
            })
        })
    }

    #[test]
    fn stabilizing_removes_shake_and_keeps_the_pan() {
        let pan = (0.002, -0.001);
        let path = shaky_path(300, pan, 0.0, 0.01, 0.5);
        let stabilization = stabilize(&path, 1.0);
        assert_eq!(stabilization.frames.len(), 300);
        let cameras = steadied(&path, &stabilization);
        let jitter = |positions: &[(f64, f64)]| {
            let sum: f64 = positions
                .iter()
                .enumerate()
                .map(|(k, &(x, y))| {
                    let (px, py) = (pan.0 * k as f64, pan.1 * k as f64);
                    (x - px).powi(2) + (y - py).powi(2)
                })
                .sum();
            (sum / positions.len() as f64).sqrt()
        };
        let before: Vec<(f64, f64)> = path.frames.iter().map(|f| (f.x, f.y)).collect();
        let after: Vec<(f64, f64)> = cameras
            .iter()
            .map(|c| (c.t.re / path.aspect, c.t.im))
            .collect();
        let (shaky, steady) = (jitter(&before), jitter(&after));
        println!(
            "jitter {shaky:.5} -> {steady:.5} ({:.1}% left), zoom {:.4}",
            100.0 * steady / shaky,
            stabilization.zoom
        );
        assert!(steady < 0.2 * shaky, "{shaky} -> {steady}");
        // The pan survives: the steadied path ends where the pan does.
        let end = after[299];
        assert!((end.0 - pan.0 * 299.0).abs() < 0.005, "{end:?}");
        assert!((end.1 - pan.1 * 299.0).abs() < 0.005, "{end:?}");
        // Roll and zoom shake drop too.
        let roll_before = path.frames.iter().map(|f| f.rotation.abs()).sum::<f64>();
        let roll_after = cameras
            .iter()
            .map(|c| c.z.arg().to_degrees().abs())
            .sum::<f64>();
        assert!(
            roll_after < 0.2 * roll_before,
            "{roll_before} -> {roll_after}"
        );
        // The zoom covers every correction, and no smaller zoom would.
        assert!(stabilization.zoom > 1.0 && stabilization.zoom < 1.5);
        assert!(covered(&path, &stabilization, stabilization.zoom));
        assert!(!covered(
            &path,
            &stabilization,
            stabilization.zoom * (1.0 - 1e-6)
        ));
    }

    #[test]
    fn strength_scales_the_smoothing() {
        let path = shaky_path(120, (0.001, 0.0), 0.02, 0.005, 0.3);
        let still = stabilize(&path, 0.0);
        assert_eq!(still.zoom, 1.0);
        assert!(
            still
                .frames
                .iter()
                .all(|c| { c.dx == 0.0 && c.dy == 0.0 && c.rotation == 0.0 && c.scale == 1.0 })
        );
        assert_eq!(stabilize(&path, f64::NAN), still);
        let size = |s: &Stabilization| s.frames.iter().map(|c| c.dx.abs()).sum::<f64>();
        let (light, full) = (stabilize(&path, 0.2), stabilize(&path, 1.0));
        assert!(size(&light) > 0.0 && size(&light) < size(&full));
        assert!(light.zoom <= full.zoom);
        assert_eq!(stabilize(&path, 7.0), full);
        let empty = CameraPath {
            aspect: 1.0,
            frames: Vec::new(),
        };
        assert!(stabilize(&empty, 1.0).frames.is_empty());
    }

    #[test]
    fn corrections_beyond_the_largest_zoom_are_reduced() {
        // Shake of up to 15% of the frame would need more than 1.5.
        let path = shaky_path(200, (0.0, 0.0), 0.0, 0.15, 1.0);
        let stabilization = stabilize(&path, 1.0);
        assert!(stabilization.zoom <= 1.5 && stabilization.zoom > 1.4);
        assert!(covered(&path, &stabilization, stabilization.zoom));
        // The corrections still steady the picture, if not completely.
        let cameras = steadied(&path, &stabilization);
        let spread = |xs: Vec<f64>| xs.iter().map(|x| x * x).sum::<f64>().sqrt();
        let before = spread(path.frames.iter().map(|f| f.x).collect());
        let after = spread(cameras.iter().map(|c| c.t.re / path.aspect).collect());
        println!(
            "zoom {:.4}, sideways shake {:.1}% left",
            stabilization.zoom,
            100.0 * after / before
        );
        assert!(after < 0.8 * before, "{before} -> {after}");
    }

    #[test]
    fn the_path_round_trips_as_json() {
        let path = shaky_path(3, (0.0, 0.0), 0.0, 0.01, 0.1);
        let json = serde_json::to_value(&path).unwrap();
        assert!(json["frames"][0]["rotation"].is_number());
        assert_eq!(serde_json::from_value::<CameraPath>(json).unwrap(), path);
        let stabilization = stabilize(&path, 0.5);
        let json = serde_json::to_string(&stabilization).unwrap();
        assert_eq!(
            serde_json::from_str::<Stabilization>(&json).unwrap(),
            stabilization
        );
    }

    #[test]
    #[ignore = "timing means something only in an optimized build: run with --release -- --ignored --nocapture"]
    fn motion_estimation_time_per_1080p_frame() {
        // One large picture; each frame is a 1920 by 1080 window of it, shifted.
        let (width, height) = (2048u32, 1200u32);
        let picture = plane(width, height, |x, y| texture(x / 6.0, y / 6.0, 3));
        let mut estimator = MotionEstimator::new();
        let frames = 60;
        let start = Instant::now();
        for k in 0..frames {
            let (x, y) = (k * 2, (k * 3) % 100);
            let offset = (y * width + x) as usize;
            estimator.push(
                Flicks::new(i64::from(k)),
                Luma {
                    width: 1920,
                    height: 1080,
                    stride: width as usize,
                    data: &picture[offset..],
                },
            );
        }
        let each = start.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        println!("motion estimation: {each:.2} ms per 1080p frame");
        let confident = estimator.motions().iter().all(|m| m.confidence > 0.9);
        assert!(confident);
        if !cfg!(debug_assertions) {
            assert!(each < 30.0, "{each} ms");
        }
    }
}
