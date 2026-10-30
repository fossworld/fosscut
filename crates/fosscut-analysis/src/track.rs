//! Following a region of the picture from frame to frame.
//!
//! [`Tracker`] keeps a template of the region, sampled from a copy of each frame about 320
//! pixels wide, and finds it in the next frame by normalised cross-correlation (Lewis, "Fast
//! normalized cross-correlation", Vision Interface 1995). The search covers a window around
//! where the region is expected (its last position plus its recent velocity), which grows
//! with its speed, from coarse to fine over an image pyramid: the best few matches of the
//! coarsest level are refined level by level, and the best final match is refined below a
//! pixel with a parabola through its neighbours' correlations.
//!
//! The template adapts slowly to gradual changes of appearance: while the match is
//! confident, a tenth of it is blended into the template each frame. The region's size
//! follows the subject's by matching the first template at 0.95, 1 and 1.05 times the size
//! and fitting a parabola; comparing with the first template rather than the adapted one
//! keeps the size from drifting, as in the drift correction of Matthews, Ishikawa and Baker
//! ("The template update problem", PAMI 2004).
//!
//! The confidence is the squared correlation of the match: the share of the window's
//! variation that the template explains. While it stays below 0.3 the region stays where it
//! was last seen; after five such frames the tracker reports the region lost, and looks for
//! it in a wider window until a match explains at least half of the variation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::gray::{Gray, parabola_minimum, pyramid};
use crate::scenes::Luma;

/// The smallest side of a region, in pixels at the analysis size.
const MIN_SIZE: f64 = 6.0;
/// Pyramid levels: the analysis copy and two halvings.
const LEVELS: usize = 3;
/// A pyramid level is searched only while the template is at least this many pixels on
/// each side there.
const LEVEL_SIZE: usize = 8;
/// Search reach around the expected position, in pixels at the analysis size, at rest.
const BASE_REACH: f64 = 8.0;
/// Extra reach per pixel per frame of recent speed.
const SPEED_REACH: f64 = 2.0;
/// Extra reach per frame the region has not been seen.
const UNSEEN_REACH: f64 = 4.0;
/// Reach while the region is lost.
const LOST_REACH: f64 = 40.0;
/// The largest reach.
const MAX_REACH: f64 = 96.0;
/// Matches at the coarsest level refined further.
const CANDIDATES: usize = 3;
/// Below this confidence the region is not seen.
const LOW: f64 = 0.3;
/// Frames in a row not seen before the region is lost.
const LOST_AFTER: u32 = 5;
/// Confidence that finds a lost region again.
const RECOVER: f64 = 0.5;
/// Confidence from which the match is blended into the template.
const UPDATE: f64 = 0.7;
/// The share of the match blended into the template.
const UPDATE_RATE: f32 = 0.1;
/// Confidence from which the size is measured.
const SCALE_CONFIDENCE: f64 = 0.5;
/// The first template must correlate at least this well for the size to change.
const SCALE_CORRELATION: f64 = 0.5;
/// Relative sizes tried on either side of the current one.
const SCALE_STEP: f64 = 0.05;

/// A region of the frame, as fractions of its width and height: centre x, centre y, width,
/// height.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Region {
    /// The centre's distance from the left edge, as a fraction of the frame's width.
    pub x: f64,
    /// The centre's distance from the top edge, as a fraction of the frame's height.
    pub y: f64,
    /// The width, as a fraction of the frame's width.
    pub width: f64,
    /// The height, as a fraction of the frame's height.
    pub height: f64,
}

/// Where the region is in one frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrackPoint {
    /// The region: where it was found, or where it was last seen when not found.
    pub region: Region,
    /// How sure the match is, from 0 to 1: its squared normalised correlation with the
    /// template.
    pub confidence: f64,
    /// Whether the region has not been seen for several frames.
    pub lost: bool,
}

/// Follows one region through frames; see the [module documentation](self).
#[derive(Clone, Debug)]
pub struct Tracker {
    /// The adapted template, `template_size` pixels, rows top to bottom.
    template: Vec<f32>,
    /// The template from the first frame, for measuring the size.
    first: Template,
    /// The templates' width and height.
    template_size: (usize, usize),
    /// The region's centre, in pixels at the analysis size.
    centre: (f64, f64),
    /// The region's width and height, in pixels at the analysis size.
    size: (f64, f64),
    /// Recent motion, in pixels per frame.
    velocity: (f64, f64),
    /// The analysis copy's width and height.
    frame: (usize, usize),
    /// How many of its pixels the whole frame spans, across and down.
    extent: (f64, f64),
    /// Frames in a row the region has not been seen.
    unseen: u32,
    /// Whether it is lost.
    lost: bool,
}

impl Tracker {
    /// Starts following `region` in `first`.
    pub fn new(first: Luma<'_>, region: Region) -> Tracker {
        let image = Gray::from_luma(first);
        let extent = image.extent;
        let side = |fraction: f64, span: f64| {
            let pixels = if fraction.is_nan() {
                0.0
            } else {
                fraction * span
            };
            pixels.clamp(MIN_SIZE.min(span), span.max(0.0))
        };
        let size = (side(region.width, extent.0), side(region.height, extent.1));
        let position = |fraction: f64, span: f64| {
            let fraction = if fraction.is_finite() { fraction } else { 0.5 };
            fraction.clamp(0.0, 1.0) * span - 0.5
        };
        let centre = (position(region.x, extent.0), position(region.y, extent.1));
        let template_size = (
            (size.0.round() as usize).max(1),
            (size.1.round() as usize).max(1),
        );
        let template = sample(&image, centre, size, template_size);
        Tracker {
            first: Template::new(template_size.0, template_size.1, &template),
            template,
            template_size,
            centre,
            size,
            velocity: (0.0, 0.0),
            frame: (image.width, image.height),
            extent,
            unseen: 0,
            lost: false,
        }
    }

    /// Where the region is in the next frame, with confidence from 0 to 1. Once confidence
    /// stays below 0.3 for several frames the tracker reports the region lost: it keeps the
    /// last position, with `lost: true`, until the region is found again.
    pub fn push(&mut self, frame: Luma<'_>) -> TrackPoint {
        let image = Gray::from_luma(frame);
        let usable = !image.is_empty() && (image.width, image.height) == self.frame;
        let found = if usable { self.search(&image) } else { None };
        let (centre, correlation) = found.unwrap_or((self.centre, 0.0));
        let confidence = correlation.max(0.0).powi(2);
        let seen = if self.lost {
            confidence >= RECOVER
        } else {
            confidence >= LOW
        };
        if seen {
            let frames = f64::from(self.unseen + 1);
            let moved = (
                (centre.0 - self.centre.0) / frames,
                (centre.1 - self.centre.1) / frames,
            );
            self.velocity = if self.lost {
                (0.0, 0.0)
            } else {
                (
                    0.5 * (self.velocity.0 + moved.0),
                    0.5 * (self.velocity.1 + moved.1),
                )
            };
            self.centre = centre;
            self.unseen = 0;
            self.lost = false;
            if confidence >= SCALE_CONFIDENCE {
                self.rescale(&image);
            }
            if confidence >= UPDATE {
                let current = sample(&image, self.centre, self.size, self.template_size);
                for (kept, new) in self.template.iter_mut().zip(current) {
                    *kept += (new - *kept) * UPDATE_RATE;
                }
            }
        } else {
            self.unseen = self.unseen.saturating_add(1);
            if self.unseen >= LOST_AFTER {
                self.lost = true;
                self.velocity = (0.0, 0.0);
            }
        }
        TrackPoint {
            region: self.region(),
            confidence,
            lost: self.lost,
        }
    }

    /// The region as fractions of the frame.
    fn region(&self) -> Region {
        let (width, height) = (self.extent.0.max(1.0), self.extent.1.max(1.0));
        Region {
            x: (self.centre.0 + 0.5) / width,
            y: (self.centre.1 + 0.5) / height,
            width: self.size.0 / width,
            height: self.size.1 / height,
        }
    }

    /// The best match's centre in `image` and its correlation, if any window fits.
    fn search(&self, image: &Gray) -> Option<((f64, f64), f64)> {
        let width = (self.size.0.round() as usize).clamp(1, image.width);
        let height = (self.size.1.round() as usize).clamp(1, image.height);
        let adapted = Gray::new(
            self.template_size.0,
            self.template_size.1,
            self.template.clone(),
        );
        let mut shrunk = adapted.resized(width, height);
        let frames = pyramid(image.clone(), LEVELS, LEVEL_SIZE);
        let mut templates = vec![Template::of(&shrunk)];
        while templates.len() < frames.len() {
            shrunk = shrunk.half();
            if shrunk.width < LEVEL_SIZE || shrunk.height < LEVEL_SIZE {
                break;
            }
            templates.push(Template::of(&shrunk));
        }
        let top = templates.len() - 1;
        // Where the window's top-left corner is expected, at full size.
        let ahead = if self.lost {
            0.0
        } else {
            f64::from(self.unseen + 1)
        };
        let expected = (
            self.centre.0 + self.velocity.0 * ahead - (width as f64 - 1.0) / 2.0,
            self.centre.1 + self.velocity.1 * ahead - (height as f64 - 1.0) / 2.0,
        );
        let reach = if self.lost {
            LOST_REACH
        } else {
            let speed = self.velocity.0.hypot(self.velocity.1);
            BASE_REACH + SPEED_REACH * speed + UNSEEN_REACH * f64::from(self.unseen)
        }
        .min(MAX_REACH);

        // Every position at the coarsest level, keeping the best few apart from each other.
        let scale = f64::from(1u32 << top);
        let (cx, cy) = (
            (expected.0 / scale).round() as isize,
            (expected.1 / scale).round() as isize,
        );
        let r = (reach / scale).ceil() as isize + 1;
        let mut scored = Vec::new();
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                if let Some(score) = templates[top].correlate(&frames[top], x, y) {
                    scored.push((score, x, y));
                }
            }
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut starts: Vec<(f64, isize, isize)> = Vec::new();
        for &(score, x, y) in &scored {
            if starts
                .iter()
                .all(|&(_, sx, sy)| (sx - x).abs().max((sy - y).abs()) >= 2)
            {
                starts.push((score, x, y));
                if starts.len() == CANDIDATES {
                    break;
                }
            }
        }

        // Refine each down to full size and keep the best.
        let mut best: Option<(f64, isize, isize)> = None;
        for (mut score, mut x, mut y) in starts {
            for level in (0..top).rev() {
                let (around_x, around_y) = (2 * x, 2 * y);
                let mut found = None;
                for sy in around_y - 2..=around_y + 2 {
                    for sx in around_x - 2..=around_x + 2 {
                        if let Some(s) = templates[level].correlate(&frames[level], sx, sy)
                            && found.is_none_or(|(f, _, _)| s > f)
                        {
                            found = Some((s, sx, sy));
                        }
                    }
                }
                let Some(refined) = found else {
                    break;
                };
                (score, x, y) = refined;
            }
            if best.is_none_or(|(b, _, _)| score > b) {
                best = Some((score, x, y));
            }
        }
        let (score, x, y) = best?;
        let around = |dx: isize, dy: isize| templates[0].correlate(&frames[0], x + dx, y + dy);
        let offset = |before: Option<f64>, after: Option<f64>| match (before, after) {
            (Some(before), Some(after)) => parabola_minimum(-before, -score, -after),
            _ => 0.0,
        };
        let fx = offset(around(-1, 0), around(1, 0));
        let fy = offset(around(0, -1), around(0, 1));
        Some((
            (
                x as f64 + fx + (width as f64 - 1.0) / 2.0,
                y as f64 + fy + (height as f64 - 1.0) / 2.0,
            ),
            score,
        ))
    }

    /// Follows a change of the subject's size, measured against the first template.
    fn rescale(&mut self, image: &Gray) {
        let score = |factor: f64| {
            let size = (self.size.0 * factor, self.size.1 * factor);
            self.first
                .correlation(&sample(image, self.centre, size, self.template_size))
        };
        let (smaller, same, larger) =
            (score(1.0 - SCALE_STEP), score(1.0), score(1.0 + SCALE_STEP));
        if same.max(smaller).max(larger) < SCALE_CORRELATION {
            return;
        }
        let factor = 1.0 + SCALE_STEP * parabola_minimum(-smaller, -same, -larger);
        let limit = (self.extent.0.max(MIN_SIZE), self.extent.1.max(MIN_SIZE));
        self.size = (
            (self.size.0 * factor).clamp(MIN_SIZE.min(limit.0), limit.0),
            (self.size.1 * factor).clamp(MIN_SIZE.min(limit.1), limit.1),
        );
    }
}

/// `image` sampled on a `grid` of points spread over the region at `centre` of `size`.
fn sample(image: &Gray, centre: (f64, f64), size: (f64, f64), grid: (usize, usize)) -> Vec<f32> {
    let step = (size.0 / grid.0 as f64, size.1 / grid.1 as f64);
    let mut values = Vec::with_capacity(grid.0 * grid.1);
    for j in 0..grid.1 {
        let y = centre.1 + (j as f64 - (grid.1 as f64 - 1.0) / 2.0) * step.1;
        values.extend((0..grid.0).map(|i| {
            let x = centre.0 + (i as f64 - (grid.0 as f64 - 1.0) / 2.0) * step.0;
            image.bilinear(x, y)
        }));
    }
    values
}

/// A template ready for correlation: its values less their mean, and their norm.
#[derive(Clone, Debug)]
struct Template {
    width: usize,
    height: usize,
    values: Vec<f32>,
    norm: f64,
}

impl Template {
    /// The template of `values`, `width` by `height`.
    fn new(width: usize, height: usize, values: &[f32]) -> Template {
        let mean = values.iter().map(|&v| f64::from(v)).sum::<f64>() / values.len().max(1) as f64;
        let values: Vec<f32> = values
            .iter()
            .map(|&v| (f64::from(v) - mean) as f32)
            .collect();
        let norm = values
            .iter()
            .map(|&v| f64::from(v).powi(2))
            .sum::<f64>()
            .sqrt();
        Template {
            width,
            height,
            values,
            norm,
        }
    }

    /// The template of a whole image.
    fn of(image: &Gray) -> Template {
        Template::new(image.width, image.height, &image.data)
    }

    /// The normalised cross-correlation with the window of `frame` whose top-left pixel is
    /// `(x, y)`; `None` when the window does not fit in the frame, 0 when either is flat.
    fn correlate(&self, frame: &Gray, x: isize, y: isize) -> Option<f64> {
        if x < 0
            || y < 0
            || x as usize + self.width > frame.width
            || y as usize + self.height > frame.height
        {
            return None;
        }
        let (x, y) = (x as usize, y as usize);
        let (mut cross, mut sum, mut squares) = (0.0f64, 0.0f64, 0.0f64);
        for row in 0..self.height {
            let window = &frame.data[(y + row) * frame.width + x..][..self.width];
            let template = &self.values[row * self.width..][..self.width];
            let (mut c, mut s, mut q) = (0.0f32, 0.0f32, 0.0f32);
            for (&p, &t) in window.iter().zip(template) {
                c += p * t;
                s += p;
                q += p * p;
            }
            cross += f64::from(c);
            sum += f64::from(s);
            squares += f64::from(q);
        }
        Some(self.normalise(cross, sum, squares))
    }

    /// The normalised cross-correlation with `values`, sampled on the template's grid.
    fn correlation(&self, values: &[f32]) -> f64 {
        let (mut cross, mut sum, mut squares) = (0.0f64, 0.0f64, 0.0f64);
        for (&p, &t) in values.iter().zip(&self.values) {
            let p = f64::from(p);
            cross += p * f64::from(t);
            sum += p;
            squares += p * p;
        }
        self.normalise(cross, sum, squares)
    }

    /// The correlation from the window's sums: of its products with the template, of its
    /// values and of their squares.
    fn normalise(&self, cross: f64, sum: f64, squares: f64) -> f64 {
        let n = self.values.len() as f64;
        let variation = squares - sum * sum / n.max(1.0);
        if self.norm < 1e-3 || variation <= 1e-6 * n {
            return 0.0;
        }
        (cross / (self.norm * variation.sqrt())).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::gray::testing::{noise, plane, texture};

    /// A frame of 640 by 360 (320 by 180 at the analysis size) of a calm textured background
    /// with a bright square of `side` analysis pixels at `centre`, under `cover` when given:
    /// the top-left corner and side of a block with a fine texture.
    fn frame(centre: (f64, f64), side: f64, cover: Option<((f64, f64), f64)>) -> Vec<u8> {
        plane(640, 360, |x, y| {
            let (u, v) = ((x + 0.5) / 2.0 - 0.5, (y + 0.5) / 2.0 - 0.5);
            if let Some(((left, top), size)) = cover
                && (left..left + size).contains(&u)
                && (top..top + size).contains(&v)
            {
                return 128.0
                    + 60.0 * (noise(u, v, 4.0, 42) - 0.5)
                    + 40.0 * (noise(u, v, 8.0, 43) - 0.5);
            }
            let background = 128.0 + 0.5 * (texture(u + 500.0, v + 300.0, 3) - 128.0);
            let inside = |d: f64| (side / 2.0 - d.abs() + 0.5).clamp(0.0, 1.0);
            let cover = inside(u - centre.0) * inside(v - centre.1);
            background + (235.0 - background) * cover
        })
    }

    fn luma(data: &[u8]) -> Luma<'_> {
        Luma {
            width: 640,
            height: 360,
            stride: 640,
            data,
        }
    }

    /// A region of `side` analysis pixels at `centre`.
    fn region(centre: (f64, f64), side: f64) -> Region {
        Region {
            x: (centre.0 + 0.5) / 320.0,
            y: (centre.1 + 0.5) / 180.0,
            width: side / 320.0,
            height: side / 180.0,
        }
    }

    /// The region's centre in analysis pixels.
    fn centre(region: Region) -> (f64, f64) {
        (region.x * 320.0 - 0.5, region.y * 180.0 - 0.5)
    }

    #[test]
    fn a_square_is_followed_along_a_curve() {
        let path = |k: usize| {
            let t = k as f64;
            (
                60.0 + 3.2 * t,
                90.0 + 45.0 * (t * std::f64::consts::TAU / 60.0).sin(),
            )
        };
        let mut tracker = Tracker::new(luma(&frame(path(0), 24.0, None)), region(path(0), 36.0));
        let mut worst = 0.0f64;
        let mut least = 1.0f64;
        for k in 1..=60 {
            let point = tracker.push(luma(&frame(path(k), 24.0, None)));
            let (x, y) = centre(point.region);
            let truth = path(k);
            worst = worst.max((x - truth.0).hypot(y - truth.1));
            least = least.min(point.confidence);
            assert!(!point.lost, "frame {k}: {point:?}");
            assert!((point.region.width * 320.0 - 36.0).abs() < 2.0, "{point:?}");
        }
        println!("worst error {worst:.3} px, lowest confidence {least:.3}");
        assert!(worst < 1.5, "{worst}");
        assert!(least > 0.5, "{least}");
    }

    #[test]
    fn a_covered_square_is_lost_where_it_was_last_seen() {
        let path = |k: usize| (80.0 + 2.0 * k as f64, 90.0 - k as f64);
        let mut tracker = Tracker::new(luma(&frame(path(0), 24.0, None)), region(path(0), 36.0));
        let cover = Some(((path(19).0 - 70.0, path(19).1 - 70.0), 140.0));
        let mut lost_at = None;
        // The highest confidence while covered, before and after the region is lost.
        let (mut before_lost, mut while_lost) = (0.0f64, 0.0f64);
        for k in 1..=40 {
            let covered = if k >= 20 { cover } else { None };
            let point = tracker.push(luma(&frame(path(k), 24.0, covered)));
            if k < 20 {
                assert!(
                    !point.lost && point.confidence > 0.5,
                    "frame {k}: {point:?}"
                );
            } else {
                let (x, y) = centre(point.region);
                assert!((x - path(19).0).hypot(y - path(19).1) < 1.5, "{point:?}");
                if point.lost {
                    while_lost = while_lost.max(point.confidence);
                } else {
                    before_lost = before_lost.max(point.confidence);
                }
            }
            if point.lost && lost_at.is_none() {
                lost_at = Some(k);
            }
            if lost_at.is_some() {
                assert!(point.lost, "frame {k}: {point:?}");
            }
        }
        println!(
            "lost at frame {lost_at:?}; highest confidence while covered {before_lost:.3}, \
             while lost (searching wider) {while_lost:.3}"
        );
        assert_eq!(lost_at, Some(24));
        assert!(before_lost < LOW && while_lost < RECOVER);
    }

    #[test]
    fn a_lost_square_is_found_again() {
        let mut tracker = Tracker::new(
            luma(&frame((100.0, 90.0), 24.0, None)),
            region((100.0, 90.0), 36.0),
        );
        let cover = Some(((50.0, 40.0), 100.0));
        for _ in 0..8 {
            tracker.push(luma(&frame((100.0, 90.0), 24.0, cover)));
        }
        // It comes back out a little to the right.
        let point = tracker.push(luma(&frame((112.0, 92.0), 24.0, None)));
        assert!(!point.lost && point.confidence > 0.5, "{point:?}");
        let (x, y) = centre(point.region);
        assert!(
            (x - 112.0).abs() < 1.5 && (y - 92.0).abs() < 1.5,
            "{point:?}"
        );
    }

    #[test]
    fn a_growing_square_is_followed_with_its_size() {
        let side = |k: usize| 20.0 * 1.6f64.powf(k as f64 / 60.0);
        let path = |k: usize| (120.0 + 0.5 * k as f64, 90.0 + 0.25 * k as f64);
        let mut tracker = Tracker::new(
            luma(&frame(path(0), side(0), None)),
            region(path(0), 1.5 * side(0)),
        );
        let mut worst = 0.0f64;
        let mut worst_position = 0.0f64;
        for k in 1..=60 {
            let point = tracker.push(luma(&frame(path(k), side(k), None)));
            let (x, y) = centre(point.region);
            worst_position = worst_position.max((x - path(k).0).hypot(y - path(k).1));
            let ratio = point.region.width * 320.0 / (1.5 * side(k));
            worst = worst.max((ratio - 1.0).abs());
            assert!(!point.lost, "frame {k}: {point:?}");
        }
        println!(
            "worst size error {:.1}%, position {worst_position:.3} px",
            worst * 100.0
        );
        assert!(worst < 0.1, "{worst}");
        assert!(worst_position < 1.5, "{worst_position}");
    }

    #[test]
    fn odd_input_does_not_panic() {
        let flat = vec![100u8; 640 * 360];
        let mut tracker = Tracker::new(
            luma(&flat),
            Region {
                x: f64::NAN,
                y: 2.0,
                width: -1.0,
                height: f64::INFINITY,
            },
        );
        let point = tracker.push(luma(&flat));
        assert_eq!(point.confidence, 0.0);
        let r = point.region;
        assert!(
            r.x == 0.5 && r.y == 1.0 && r.width > 0.0 && r.height == 1.0,
            "{r:?}"
        );
        // A frame of another size is not searched.
        let small = vec![100u8; 64 * 36];
        let point = tracker.push(Luma {
            width: 64,
            height: 36,
            stride: 64,
            data: &small,
        });
        assert_eq!(point.confidence, 0.0);
        let json = serde_json::to_value(&point).unwrap();
        assert_eq!(serde_json::from_value::<TrackPoint>(json).unwrap(), point);
    }

    #[test]
    #[ignore = "timing means something only in an optimized build: run with --release -- --ignored --nocapture"]
    fn tracking_time_per_1080p_frame() {
        let (width, height) = (2048u32, 1200u32);
        let square = |x: f64, y: f64| {
            let background = 128.0 + 0.5 * (texture(x / 6.0, y / 6.0, 3) - 128.0);
            let inside = |d: f64| (72.0 - d.abs()).clamp(0.0, 1.0);
            background + (235.0 - background) * inside(x - 1000.0) * inside(y - 600.0)
        };
        let picture = plane(width, height, square);
        let window = |k: u32| {
            let offset = ((k % 100) * width + 2 * k) as usize;
            Luma {
                width: 1920,
                height: 1080,
                stride: width as usize,
                data: &picture[offset..],
            }
        };
        let start_region = Region {
            x: 1000.0 / 1920.0,
            y: 600.0 / 1080.0,
            width: 216.0 / 1920.0,
            height: 216.0 / 1080.0,
        };
        let mut tracker = Tracker::new(window(0), start_region);
        let frames = 60;
        let start = Instant::now();
        for k in 1..=frames {
            let point = tracker.push(window(k));
            assert!(point.confidence > 0.5, "{point:?}");
        }
        let each = start.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        println!("tracking: {each:.2} ms per 1080p frame (a region a ninth of the height)");
        if !cfg!(debug_assertions) {
            assert!(each < 30.0, "{each} ms");
        }
    }
}
