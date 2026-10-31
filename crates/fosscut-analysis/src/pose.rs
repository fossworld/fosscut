//! Body pose and person masks (ADR-0033, item 4) with MediaPipe Pose Landmarker full
//! (BlazePose GHUM), converted to ONNX: model `mediapipe-pose-full`, files `detector.onnx`
//! and `landmarks.onnx`.
//!
//! The pipeline follows MediaPipe's published pose solution:
//!
//! 1. **Detection** (only while no person is followed): the frame is letterboxed into the
//!    detector's 224 by 224 input, scaled to -1..1. Its 2254 outputs are relative to SSD
//!    anchors (Liu and others, "SSD: Single shot multibox detector", ECCV 2016) on feature
//!    maps of strides 8, 16 and 32 (28 by 28 and 14 by 14 with 2 anchors per cell, 7 by 7
//!    with 6), centred in their cells with a fixed unit size. Each holds a box and four
//!    keypoints in input pixels; scores are logits. Detections above 0.5 are merged by
//!    weighted non-maximum suppression and the best is kept.
//! 2. **Region:** keypoint 0 is the middle of the hips and keypoint 1 sets a circle around
//!    the body. The region is a square centred on the hips, twice the circle's radius wide and
//!    enlarged 1.25 times, turned so the body stands upright in it: its rotation is 90
//!    degrees minus the angle of the hips-to-circle vector.
//! 3. **Landmarks:** the region is sampled into the landmark model's 256 by 256 input (0..1).
//!    It gives 39 points of x, y, z, visibility and presence in input pixels (logits for the
//!    last two), of which the first 33 are the body's landmarks and points 33 and 34 set the
//!    next frame's region as the detector's keypoints do; a presence probability; and the
//!    person mask as 256 by 256 logits. Points and mask are mapped back to the frame through
//!    the region.
//! 4. **Following:** while presence stays at 0.5 or above, the next frame's region comes
//!    from these landmarks and the detector does not run.
//!
//! Masks are stored at 256 pixels on the frame's longer side, smoothed in time by
//! [`TemporalSmoother`]; landmark positions are smoothed the same way, a bilateral filter in
//! time that leaves fast moves alone. A person seen in one frame only, with nobody in the
//! frames on either side, is taken for a false detection and dropped, and a person lost for
//! one frame between two that have them is filled in from them ([`debounce`]), so body
//! effects never flash for a single frame. Everything is per frame of the source, at most
//! `max_fps` frames per second.

use std::path::Path;

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::vision::{
    Affine, DataError, DataFile, Reader, Rgb, TemporalSmoother, Writer, frame_at, pack,
    sample_gray, sigmoid, stored_size, to_bytes, unpack,
};
use crate::{AnalysisError, Moment, Span};

/// The model the pose analyzer runs.
pub const MODEL: &str = "mediapipe-pose-full";
/// Body landmarks per frame, in MediaPipe's order (0 nose, 11 and 12 shoulders, 13 and 14
/// elbows, 15 and 16 wrists, 23 and 24 hips, 25 and 26 knees, 27 and 28 ankles, and so on).
pub const LANDMARKS: usize = 33;
/// The longer side of a stored person mask, in pixels.
pub const MASK_SIDE: u32 = 256;
/// The detector's input side.
const DETECTOR: usize = 224;
/// The landmark model's input side.
const CROP: usize = 256;
/// Points the landmark model gives (33 body landmarks and 6 auxiliary ones).
const POINTS: usize = 39;
/// Most samples in a [`Pose`] summary.
pub const MAX_SAMPLES: i64 = 60;
/// Anchors of the detector.
pub const ANCHORS: usize = 2254;
/// Lowest detection score kept.
const MIN_SCORE: f32 = 0.5;
/// Lowest presence that counts as a person.
pub const MIN_PRESENCE: f32 = 0.5;
/// Detections overlapping the best by more than this are merged into it.
const NMS_OVERLAP: f32 = 0.3;
/// How much larger than the body's circle the region is.
const REGION_SCALE: f64 = 1.25;
/// The longest side, in pixels, of the copy of a frame the pose model reads from (a frame
/// up to twice as large is used as it is): enough detail for its 256-pixel crops.
pub const WORKING_SIDE: u32 = 768;

/// Options of the `pose` analyzer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PoseOptions {
    /// Most frames analyzed per second, 1 to 60 (default 30); a faster source is analyzed
    /// at every other frame or sparser, and the body effects hold each result until the next.
    pub max_fps: u32,
    /// How much landmarks and masks are smoothed in time, from 0 (not at all) to 1 (a
    /// Gaussian of two frames); fast moves are left alone. Default 0.5.
    pub smoothing: f64,
}

impl Default for PoseOptions {
    fn default() -> PoseOptions {
        PoseOptions {
            max_fps: 30,
            smoothing: 0.5,
        }
    }
}

/// One body landmark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Landmark {
    /// Distance from the frame's left edge, as a fraction of its width (outside 0..1 when
    /// the point is beyond the frame).
    pub x: f32,
    /// Distance from the top edge, as a fraction of the height.
    pub y: f32,
    /// How likely the point is visible (not hidden by the body or something else), 0 to 1.
    pub visibility: f32,
    /// How likely the point is inside the frame, 0 to 1.
    pub presence: f32,
}

/// The pose in one analyzed frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseFrame {
    /// The frame's time from the start of the file.
    pub time: Flicks,
    /// How likely a person is in the frame, 0 to 1; landmarks mean nothing below 0.5.
    pub presence: f32,
    /// The 33 body landmarks.
    pub landmarks: [Landmark; LANDMARKS],
    /// The person mask, packed with [`crate::vision::pack`]: `mask_size` bytes from 0 (no
    /// person) to 255.
    pub mask: Vec<u8>,
}

/// Every analyzed frame's pose and person mask: the data file of a `pose` result.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseTrack {
    /// The stored masks' width and height.
    pub mask_size: (u32, u32),
    /// The frames in time order.
    pub frames: Vec<PoseFrame>,
}

/// A sample of the pose for agents: about one per second, at most [`MAX_SAMPLES`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PoseSample {
    /// When.
    pub at: Moment,
    /// How likely a person is in the frame.
    pub presence: f32,
    /// The landmarks as `[x, y, visibility]`, positions as fractions of the frame; empty
    /// when no person is present.
    pub landmarks: Vec<[f32; 3]>,
}

/// What the `pose` analyzer reports: a summary, samples, and the data file with every
/// frame's landmarks and mask, which body effects read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pose {
    /// The model.
    pub model: String,
    /// Frames analyzed.
    pub frames: u32,
    /// Frames with a person.
    pub frames_with_person: u32,
    /// The time from the first to the last analyzed frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    /// Where the person was over all frames: left, top, right and bottom edges of the
    /// landmarks as fractions of the frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f32; 4]>,
    /// The stored masks' width and height in pixels.
    pub mask_size: [u32; 2],
    /// About one frame per second, at most 60 spread over the clip.
    pub samples: Vec<PoseSample>,
    /// The data file beside the cache entry.
    pub data: DataFile,
}

/// An observation of a person in one frame, in the frame's pixel coordinates of the picture
/// the model read.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    /// How likely a person is there.
    pub presence: f32,
    /// The 33 landmarks as fractions of the picture.
    pub landmarks: [Landmark; LANDMARKS],
    /// The region the landmark model read.
    pub region: Region,
    /// The region the next frame should read, from the auxiliary points.
    pub next: Region,
    /// The person mask over the region, `256 * 256` probabilities.
    pub mask: Vec<f32>,
}

/// A square region of the picture, turned by `rotation` radians clockwise about its centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    /// Centre x, in pixels.
    pub x: f64,
    /// Centre y, in pixels.
    pub y: f64,
    /// Side, in pixels.
    pub side: f64,
    /// Clockwise rotation in radians.
    pub rotation: f64,
}

impl Region {
    /// The region around a body whose hips are at `centre` and whose circle passes through
    /// `edge` (both in pixels), as MediaPipe builds it.
    pub fn around(centre: (f64, f64), edge: (f64, f64)) -> Region {
        let (dx, dy) = (edge.0 - centre.0, edge.1 - centre.1);
        let angle = std::f64::consts::FRAC_PI_2 - (-dy).atan2(dx);
        Region {
            x: centre.0,
            y: centre.1,
            side: 2.0 * dx.hypot(dy) * REGION_SCALE,
            rotation: normalize_radians(angle),
        }
    }

    /// The map from a pixel of a `size` by `size` model input to the picture.
    pub fn map(&self, size: usize) -> Affine {
        let (c, s) = (self.rotation.cos(), self.rotation.sin());
        let k = self.side / size as f64;
        // u, v in input pixels; (u / size - 0.5) * side turned by the rotation.
        Affine {
            m: [
                c * k,
                -s * k,
                self.x - 0.5 * self.side * (c - s),
                s * k,
                c * k,
                self.y - 0.5 * self.side * (s + c),
            ],
        }
    }
}

/// `angle` brought into -pi..pi.
fn normalize_radians(angle: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    angle - tau * ((angle + std::f64::consts::PI) / tau).floor()
}

/// The centres of the detector's anchors, as fractions of its input, in output order.
pub fn anchors() -> Vec<[f32; 2]> {
    // Layers of strides 8, 16, 32, 32, 32; consecutive layers of one stride share a
    // feature map, each layer adding two anchors per cell (its scale and an interpolated
    // one), all of the same fixed size.
    let strides = [8usize, 16, 32, 32, 32];
    let mut out = Vec::with_capacity(ANCHORS);
    let mut layer = 0;
    while layer < strides.len() {
        let mut last = layer;
        while last < strides.len() && strides[last] == strides[layer] {
            last += 1;
        }
        let per_cell = 2 * (last - layer);
        let cells = DETECTOR.div_ceil(strides[layer]);
        for y in 0..cells {
            for x in 0..cells {
                for _ in 0..per_cell {
                    out.push([
                        (x as f32 + 0.5) / cells as f32,
                        (y as f32 + 0.5) / cells as f32,
                    ]);
                }
            }
        }
        layer = last;
    }
    out
}

/// How a picture sits in the detector's square input: scaled by `scale` and moved by `pad`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Letterbox {
    /// Input pixels per picture pixel.
    pub scale: f64,
    /// Where the picture's top-left corner is in the input, in pixels.
    pub pad: (f64, f64),
}

impl Letterbox {
    /// The letterbox of a `width` by `height` picture: it fits the input with black above
    /// and below, or at the sides.
    pub fn of(width: u32, height: u32) -> Letterbox {
        let side = DETECTOR as f64;
        let scale = side / f64::from(width.max(height).max(1));
        Letterbox {
            scale,
            pad: (
                (side - f64::from(width) * scale) / 2.0,
                (side - f64::from(height) * scale) / 2.0,
            ),
        }
    }

    /// The map from an input pixel to the picture.
    pub fn map(&self) -> Affine {
        Affine {
            m: [
                1.0 / self.scale,
                0.0,
                -self.pad.0 / self.scale,
                0.0,
                1.0 / self.scale,
                -self.pad.1 / self.scale,
            ],
        }
    }

    /// A point of the input, in pixels, in the picture.
    fn to_picture(self, x: f64, y: f64) -> (f64, f64) {
        self.map().apply(x, y)
    }
}

/// A person the detector found, in picture pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    /// Its score, 0 to 1.
    pub score: f32,
    /// The box's centre and size: x, y, width, height.
    pub bounds: [f64; 4],
    /// The four keypoints: hips, the body circle's edge, and two for the upper body.
    pub keypoints: [(f64, f64); 4],
}

/// The detections in the detector's outputs: `boxes` holds 12 values per anchor (box
/// centre, size, then four keypoints, in input pixels relative to the anchor) and `scores`
/// one logit each. Only those scoring 0.5 or more are returned, in picture pixels.
pub fn detections(boxes: &[f32], scores: &[f32], letterbox: Letterbox) -> Vec<Detection> {
    let side = DETECTOR as f64;
    anchors()
        .iter()
        .enumerate()
        .filter_map(|(i, anchor)| {
            let score = sigmoid(*scores.get(i)?);
            if score < MIN_SCORE {
                return None;
            }
            let raw = boxes.get(i * 12..i * 12 + 12)?;
            let at = |k: usize| {
                let x = (f64::from(raw[k]) / side + f64::from(anchor[0])) * side;
                let y = (f64::from(raw[k + 1]) / side + f64::from(anchor[1])) * side;
                letterbox.to_picture(x, y)
            };
            let (cx, cy) = at(0);
            let size = (
                f64::from(raw[2]) / letterbox.scale,
                f64::from(raw[3]) / letterbox.scale,
            );
            Some(Detection {
                score,
                bounds: [cx, cy, size.0, size.1],
                keypoints: [at(4), at(6), at(8), at(10)],
            })
        })
        .collect()
}

/// The overlap of two boxes, as intersection over union.
fn overlap(a: &[f64; 4], b: &[f64; 4]) -> f32 {
    let edges = |r: &[f64; 4]| {
        (
            r[0] - r[2] / 2.0,
            r[1] - r[3] / 2.0,
            r[0] + r[2] / 2.0,
            r[1] + r[3] / 2.0,
        )
    };
    let (a0, a1, a2, a3) = edges(a);
    let (b0, b1, b2, b3) = edges(b);
    let w = (a2.min(b2) - a0.max(b0)).max(0.0);
    let h = (a3.min(b3) - a1.max(b1)).max(0.0);
    let inter = w * h;
    let union = a[2] * a[3] + b[2] * b[3] - inter;
    if union <= 0.0 {
        0.0
    } else {
        (inter / union) as f32
    }
}

/// The best detection, merged with those overlapping it by score-weighted averaging
/// (weighted non-maximum suppression, as MediaPipe's detectors use).
pub fn best(detections: &[Detection]) -> Option<Detection> {
    let top = detections
        .iter()
        .max_by(|a, b| a.score.total_cmp(&b.score))?;
    let group: Vec<&Detection> = detections
        .iter()
        .filter(|d| overlap(&d.bounds, &top.bounds) > NMS_OVERLAP)
        .collect();
    let total: f64 = group.iter().map(|d| f64::from(d.score)).sum();
    if total <= 0.0 {
        return Some(top.clone());
    }
    let mut bounds = [0.0; 4];
    let mut keypoints = [(0.0, 0.0); 4];
    for d in &group {
        let w = f64::from(d.score) / total;
        for (b, v) in bounds.iter_mut().zip(d.bounds) {
            *b += w * v;
        }
        for (k, v) in keypoints.iter_mut().zip(d.keypoints) {
            k.0 += w * v.0;
            k.1 += w * v.1;
        }
    }
    Some(Detection {
        score: top.score,
        bounds,
        keypoints,
    })
}

/// Reads the landmark model's outputs for `region` of a `width` by `height` picture: the 39
/// points (`points`, 5 values each in input pixels, visibility and presence as logits), the
/// presence probability and the mask logits.
pub fn observation(
    region: Region,
    picture: (u32, u32),
    points: &[f32],
    presence: f32,
    mask_logits: &[f32],
) -> Option<Observation> {
    if points.len() < POINTS * 5 || mask_logits.len() < CROP * CROP {
        return None;
    }
    let map = region.map(CROP);
    let (w, h) = (f64::from(picture.0.max(1)), f64::from(picture.1.max(1)));
    let point = |i: usize| {
        let p = &points[i * 5..i * 5 + 5];
        let (x, y) = map.apply(f64::from(p[0]), f64::from(p[1]));
        (x, y, sigmoid(p[3]), sigmoid(p[4]))
    };
    let mut landmarks = [Landmark::default(); LANDMARKS];
    for (i, landmark) in landmarks.iter_mut().enumerate() {
        let (x, y, visibility, presence) = point(i);
        *landmark = Landmark {
            x: (x / w) as f32,
            y: (y / h) as f32,
            visibility,
            presence,
        };
    }
    let (cx, cy, _, _) = point(33);
    let (ex, ey, _, _) = point(34);
    Some(Observation {
        presence,
        landmarks,
        region,
        next: Region::around((cx, cy), (ex, ey)),
        mask: mask_logits[..CROP * CROP]
            .iter()
            .map(|&l| sigmoid(l))
            .collect(),
    })
}

/// The person mask of `observation` over a whole `picture`-sized frame, `size` pixels:
/// probabilities 0 to 1, zero outside the region the model read.
pub fn frame_mask(observation: &Observation, picture: (u32, u32), size: (u32, u32)) -> Vec<f32> {
    let (sw, sh) = (size.0 as usize, size.1 as usize);
    let Some(back) = observation.region.map(CROP).inverse() else {
        return vec![0.0; sw * sh];
    };
    let (kx, ky) = (
        f64::from(picture.0) / sw as f64,
        f64::from(picture.1) / sh as f64,
    );
    let mut out = Vec::with_capacity(sw * sh);
    for j in 0..sh {
        for i in 0..sw {
            let (u, v) = back.apply((i as f64 + 0.5) * kx, (j as f64 + 0.5) * ky);
            out.push(sample_gray(&observation.mask, CROP, CROP, u, v, 0.0));
        }
    }
    out
}

/// The detector's input for `picture`: letterboxed to 224 by 224, channels last, -1..1.
pub fn detector_input(picture: &Rgb) -> (Letterbox, Vec<f32>) {
    let letterbox = Letterbox::of(picture.width, picture.height);
    let pixels = crate::vision::warp(picture, &letterbox.map(), DETECTOR, DETECTOR);
    let values = pixels
        .iter()
        .flat_map(|rgb| rgb.map(|c| c * 2.0 - 1.0))
        .collect();
    (letterbox, values)
}

/// The landmark model's input for `region` of `picture`: 256 by 256, channels last, 0..1.
pub fn landmark_input(picture: &Rgb, region: &Region) -> Vec<f32> {
    crate::vision::warp(picture, &region.map(CROP), CROP, CROP)
        .into_iter()
        .flatten()
        .collect()
}

/// Gathers observations frame by frame into a [`PoseTrack`]: masks over the frame,
/// smoothed in time, and landmarks smoothed once every frame is in.
pub struct PoseBuilder {
    mask_size: (u32, u32),
    smoothing: f64,
    smoother: TemporalSmoother,
    /// Frames whose mask is still being smoothed: time, presence, landmarks.
    pending: std::collections::VecDeque<(Flicks, f32, [Landmark; LANDMARKS])>,
    frames: Vec<PoseFrame>,
}

impl PoseBuilder {
    /// A builder for frames `width` by `height`, smoothing by `smoothing` (0 to 1).
    pub fn new(width: u32, height: u32, smoothing: f64) -> PoseBuilder {
        PoseBuilder {
            mask_size: stored_size(width, height, MASK_SIDE),
            smoothing,
            smoother: TemporalSmoother::new(smoothing),
            pending: std::collections::VecDeque::new(),
            frames: Vec::new(),
        }
    }

    /// The stored masks' size.
    pub fn mask_size(&self) -> (u32, u32) {
        self.mask_size
    }

    /// Adds the frame at `time` of a `picture`-sized picture, with what was seen in it.
    pub fn push(&mut self, time: Flicks, picture: (u32, u32), seen: Option<&Observation>) {
        let (w, h) = self.mask_size;
        let (mask, presence, landmarks) = match seen {
            Some(o) => (
                frame_mask(o, picture, self.mask_size),
                o.presence,
                o.landmarks,
            ),
            None => (
                vec![0.0; w as usize * h as usize],
                0.0,
                [Landmark::default(); LANDMARKS],
            ),
        };
        self.pending.push_back((time, presence, landmarks));
        if let Some(done) = self.smoother.push(time, mask) {
            self.complete(done);
        }
    }

    fn complete(&mut self, (time, mask): (Flicks, Vec<f32>)) {
        if let Some((pending_time, presence, landmarks)) = self.pending.pop_front() {
            debug_assert_eq!(pending_time, time);
            self.frames.push(PoseFrame {
                time,
                presence,
                landmarks,
                mask: pack(&to_bytes(&mask)),
            });
        }
    }

    /// The finished track, with landmarks smoothed in time.
    pub fn finish(mut self) -> PoseTrack {
        let smoother = std::mem::replace(&mut self.smoother, TemporalSmoother::new(0.0));
        for done in smoother.finish() {
            self.complete(done);
        }
        let present: Vec<bool> = self
            .frames
            .iter()
            .map(|f| f.presence >= MIN_PRESENCE)
            .collect();
        let empty = pack(&vec![
            0u8;
            self.mask_size.0 as usize * self.mask_size.1 as usize
        ]);
        for (i, fix) in debounce(&present).into_iter().enumerate() {
            match fix {
                Fix::Keep => {}
                Fix::Clear => {
                    let frame = &mut self.frames[i];
                    frame.presence = 0.0;
                    frame.landmarks = [Landmark::default(); LANDMARKS];
                    frame.mask = empty.clone();
                }
                Fix::Fill => {
                    let (before, after) = (self.frames[i - 1].clone(), self.frames[i + 1].clone());
                    let mut landmarks = before.landmarks;
                    for (l, a) in landmarks.iter_mut().zip(&after.landmarks) {
                        l.x = (l.x + a.x) / 2.0;
                        l.y = (l.y + a.y) / 2.0;
                        l.visibility = l.visibility.min(a.visibility);
                        l.presence = l.presence.min(a.presence);
                    }
                    let frame = &mut self.frames[i];
                    frame.presence = before.presence.min(after.presence);
                    frame.landmarks = landmarks;
                    frame.mask = before.mask;
                }
            }
        }
        smooth_landmarks(&mut self.frames, self.smoothing);
        PoseTrack {
            mask_size: self.mask_size,
            frames: self.frames,
        }
    }
}

/// What [`debounce`] does to a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    /// Leave it.
    Keep,
    /// A person seen in this frame only: drop it.
    Clear,
    /// A person lost in this frame only: fill it in from the frames on either side.
    Fill,
}

/// For each frame, whether a person was seen, what to do: a frame with a person whose
/// neighbours (one at either end of the sequence) have none is cleared; then a frame without
/// one between two that still have one is filled, so fills only ever read kept frames.
pub fn debounce(present: &[bool]) -> Vec<Fix> {
    let n = present.len();
    let isolated = |i: usize| {
        n > 1
            && present[i]
            && i.checked_sub(1).is_none_or(|j| !present[j])
            && present.get(i + 1).is_none_or(|next| !next)
    };
    let kept: Vec<bool> = (0..n).map(|i| present[i] && !isolated(i)).collect();
    (0..n)
        .map(|i| {
            if isolated(i) {
                Fix::Clear
            } else if !kept[i] && i > 0 && i + 1 < n && kept[i - 1] && kept[i + 1] {
                Fix::Fill
            } else {
                Fix::Keep
            }
        })
        .collect()
}

/// Smooths landmark positions over neighbouring frames with a person, weighted by time and
/// by how far each moved (a bilateral filter in time, as [`TemporalSmoother`] does for masks).
fn smooth_landmarks(frames: &mut [PoseFrame], strength: f64) {
    let sigma = strength.clamp(0.0, 1.0) * 2.0;
    if sigma < 0.05 {
        return;
    }
    let radius = (2.0 * sigma).ceil() as usize;
    // A move of 2% of the frame between frames is motion, not jitter.
    let motion = 0.02f64;
    let original: Vec<PoseFrame> = frames.to_vec();
    for (i, frame) in frames.iter_mut().enumerate() {
        if frame.presence < MIN_PRESENCE {
            continue;
        }
        let low = i.saturating_sub(radius);
        let high = (i + radius).min(original.len() - 1);
        for (n, landmark) in frame.landmarks.iter_mut().enumerate() {
            let centre = original[i].landmarks[n];
            let (mut sx, mut sy, mut total) = (f64::from(centre.x), f64::from(centre.y), 1.0);
            for (k, other) in original.iter().enumerate().take(high + 1).skip(low) {
                if k == i || other.presence < MIN_PRESENCE {
                    continue;
                }
                let p = other.landmarks[n];
                let dt = k.abs_diff(i) as f64;
                let d = f64::from(p.x - centre.x).hypot(f64::from(p.y - centre.y));
                let w = (-(dt * dt) / (2.0 * sigma * sigma)).exp()
                    * (-(d * d) / (2.0 * motion * motion)).exp();
                sx += w * f64::from(p.x);
                sy += w * f64::from(p.y);
                total += w;
            }
            landmark.x = (sx / total) as f32;
            landmark.y = (sy / total) as f32;
        }
    }
}

/// The landmarks of one moment, interpolated between analyzed frames.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseAt {
    /// How likely a person is there.
    pub presence: f32,
    /// The 33 landmarks.
    pub landmarks: [Landmark; LANDMARKS],
    /// The index of the analyzed frame shown at that moment, whose mask applies.
    pub frame: usize,
}

const POSE_MAGIC: &[u8; 8] = b"FCPOSE01";

impl PoseTrack {
    /// The track as its data file.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(POSE_MAGIC);
        w.u32(self.mask_size.0);
        w.u32(self.mask_size.1);
        w.u32(self.frames.len() as u32);
        for frame in &self.frames {
            w.i64(frame.time.get());
            w.f32(frame.presence);
            for l in &frame.landmarks {
                w.f32(l.x);
                w.f32(l.y);
                w.f32(l.visibility);
                w.f32(l.presence);
            }
            w.block(&frame.mask);
        }
        w.0
    }

    /// A track from its data file.
    pub fn decode(data: &[u8]) -> Result<PoseTrack, DataError> {
        let mut r = Reader::new(data);
        if r.take(8)? != POSE_MAGIC {
            return Err(DataError("it is not a pose file".into()));
        }
        let mask_size = (r.u32()?, r.u32()?);
        let count = r.u32()? as usize;
        let mut frames = Vec::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            let time = Flicks::new(r.i64()?);
            let presence = r.f32()?;
            let mut landmarks = [Landmark::default(); LANDMARKS];
            for l in &mut landmarks {
                *l = Landmark {
                    x: r.f32()?,
                    y: r.f32()?,
                    visibility: r.f32()?,
                    presence: r.f32()?,
                };
            }
            let mask = r.block()?.to_vec();
            frames.push(PoseFrame {
                time,
                presence,
                landmarks,
                mask,
            });
        }
        if !r.is_done() {
            return Err(DataError("it has bytes after its last frame".into()));
        }
        Ok(PoseTrack { mask_size, frames })
    }

    /// The pose at `time`: from the frame shown then, with positions interpolated towards
    /// the next analyzed frame when both have a person; `None` for an empty track.
    pub fn at(&self, time: Flicks) -> Option<PoseAt> {
        let index = frame_at(self.frames.iter().map(|f| f.time), time)?;
        let frame = &self.frames[index];
        let mut landmarks = frame.landmarks;
        if let Some(next) = self.frames.get(index + 1)
            && time > frame.time
            && frame.presence >= MIN_PRESENCE
            && next.presence >= MIN_PRESENCE
        {
            let t = ((time - frame.time).get() as f64
                / (next.time - frame.time).get().max(1) as f64)
                .clamp(0.0, 1.0) as f32;
            for (l, n) in landmarks.iter_mut().zip(&next.landmarks) {
                l.x += (n.x - l.x) * t;
                l.y += (n.y - l.y) * t;
                l.visibility += (n.visibility - l.visibility) * t;
                l.presence += (n.presence - l.presence) * t;
            }
        }
        Some(PoseAt {
            presence: frame.presence,
            landmarks,
            frame: index,
        })
    }

    /// The mask of analyzed frame `index`, unpacked: `mask_size` bytes.
    pub fn mask(&self, index: usize) -> Option<Vec<u8>> {
        let frame = self.frames.get(index)?;
        unpack(
            &frame.mask,
            self.mask_size.0 as usize * self.mask_size.1 as usize,
        )
    }

    /// The summary agents read, with `data` describing this track's data file.
    pub fn summary(&self, data: DataFile) -> Pose {
        let with_person: Vec<&PoseFrame> = self
            .frames
            .iter()
            .filter(|f| f.presence >= MIN_PRESENCE)
            .collect();
        let bounds = with_person
            .iter()
            .flat_map(|f| f.landmarks.iter())
            .filter(|l| l.visibility >= 0.5)
            .fold(None, |b: Option<[f32; 4]>, l| {
                Some(match b {
                    None => [l.x, l.y, l.x, l.y],
                    Some([x0, y0, x1, y1]) => [x0.min(l.x), y0.min(l.y), x1.max(l.x), y1.max(l.y)],
                })
            });
        // One a second, or one every few seconds so there are never more than 60.
        let second = Flicks::from_seconds(1).map_or(705_600_000, Flicks::get);
        let length = match (self.frames.first(), self.frames.last()) {
            (Some(first), Some(last)) => (last.time - first.time).get(),
            _ => 0,
        };
        let step = second * (length / second / MAX_SAMPLES + 1);
        let mut samples = Vec::new();
        let mut next = i64::MIN;
        for frame in &self.frames {
            if frame.time.get() < next {
                continue;
            }
            next = (frame.time.get().div_euclid(step) + 1) * step;
            samples.push(PoseSample {
                at: Moment::new(frame.time),
                presence: round(frame.presence),
                landmarks: if frame.presence >= MIN_PRESENCE {
                    frame
                        .landmarks
                        .iter()
                        .map(|l| [round(l.x), round(l.y), round(l.visibility)])
                        .collect()
                } else {
                    Vec::new()
                },
            });
        }
        Pose {
            model: MODEL.to_string(),
            frames: self.frames.len() as u32,
            frames_with_person: with_person.len() as u32,
            span: match (self.frames.first(), self.frames.last()) {
                (Some(first), Some(last)) => Some(Span::new(first.time, last.time)),
                _ => None,
            },
            bounds: bounds.map(|b| b.map(round)),
            mask_size: [self.mask_size.0, self.mask_size.1],
            samples,
            data,
        }
    }
}

/// `value` to three decimals, so summaries stay short.
fn round(value: f32) -> f32 {
    (value * 1000.0).round() / 1000.0
}

/// The pose model's two sessions.
#[cfg(feature = "onnx")]
pub struct PoseModel {
    detector: crate::onnx::Session,
    landmarks: crate::onnx::Session,
}

#[cfg(feature = "onnx")]
impl PoseModel {
    /// Opens the installed `mediapipe-pose-full` in `models` on `options`' device.
    pub fn open(options: &crate::onnx::SessionOptions) -> Result<PoseModel, AnalysisError> {
        let model = crate::models::manifest(MODEL)?;
        if !crate::models::is_installed(&options.models, model) {
            return Err(AnalysisError::Model(
                crate::models::ModelError::NotInstalled { what: MODEL.into() },
            ));
        }
        let file = |name: &str| crate::models::file_path(&options.models, model, name);
        Ok(PoseModel {
            detector: crate::onnx::session(&file("detector.onnx"), options)?,
            landmarks: crate::onnx::session(&file("landmarks.onnx"), options)?,
        })
    }

    /// Looks for a person in `picture`: in `region` first when one is followed, then with the
    /// detector.
    pub fn observe(
        &mut self,
        picture: &Rgb,
        region: Option<Region>,
    ) -> Result<Option<Observation>, AnalysisError> {
        if let Some(region) = region
            && let Some(seen) = self.landmarks_in(picture, region)?
            && seen.presence >= MIN_PRESENCE
        {
            return Ok(Some(seen));
        }
        let (letterbox, input) = detector_input(picture);
        let outputs = run(
            &mut self.detector,
            "input_1",
            vec![1, DETECTOR, DETECTOR, 3],
            input,
            &["Identity", "Identity_1"],
        )?;
        let found = detections(&outputs[0], &outputs[1], letterbox);
        let Some(best) = best(&found) else {
            return Ok(None);
        };
        let region = Region::around(best.keypoints[0], best.keypoints[1]);
        Ok(self
            .landmarks_in(picture, region)?
            .filter(|seen| seen.presence >= MIN_PRESENCE))
    }

    fn landmarks_in(
        &mut self,
        picture: &Rgb,
        region: Region,
    ) -> Result<Option<Observation>, AnalysisError> {
        if !(region.side.is_finite() && region.side > 1.0) {
            return Ok(None);
        }
        let input = landmark_input(picture, &region);
        let outputs = run(
            &mut self.landmarks,
            "input_1",
            vec![1, CROP, CROP, 3],
            input,
            &["Identity", "Identity_1", "Identity_2"],
        )?;
        let presence = outputs[1].first().copied().unwrap_or(0.0);
        Ok(observation(
            region,
            (picture.width, picture.height),
            &outputs[0],
            presence,
            &outputs[2],
        ))
    }
}

/// Runs `session` on one float input and returns the named float outputs.
#[cfg(feature = "onnx")]
pub(crate) fn run(
    session: &mut crate::onnx::Session,
    input: &str,
    shape: Vec<usize>,
    values: Vec<f32>,
    outputs: &[&str],
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    use crate::onnx::ort::value::Tensor;
    let failed = |message: String| AnalysisError::Inference(message);
    let tensor = Tensor::from_array((shape, values)).map_err(|e| failed(e.to_string()))?;
    let inputs: Vec<(
        std::borrow::Cow<'static, str>,
        crate::onnx::ort::session::SessionInputValue<'_>,
    )> = vec![(std::borrow::Cow::Owned(input.to_string()), tensor.into())];
    let result = session.run(inputs).map_err(|e| failed(e.to_string()))?;
    outputs
        .iter()
        .map(|name| {
            let value = result
                .get(*name)
                .ok_or_else(|| failed(format!("the model has no output {name}")))?;
            let (_, data) = value
                .try_extract_tensor::<f32>()
                .map_err(|e| failed(e.to_string()))?;
            Ok(data.to_vec())
        })
        .collect()
}

/// The `pose` analyzer: takes decoded frames, runs the model on those it samples, and
/// returns the track.
#[cfg(feature = "onnx")]
pub struct PoseAnalyzer {
    model: PoseModel,
    sampler: crate::vision::Sampler,
    smoothing: f64,
    builder: Option<PoseBuilder>,
    region: Option<Region>,
}

#[cfg(feature = "onnx")]
impl PoseAnalyzer {
    /// An analyzer with `options`, its model opened from `models` on the CPU.
    pub fn new(options: &PoseOptions, models: &Path) -> Result<PoseAnalyzer, AnalysisError> {
        // Deterministic CPU kernels: the same file always gives the same track.
        let sessions = crate::onnx::SessionOptions::new(models).on(crate::onnx::Device::Cpu);
        Ok(PoseAnalyzer {
            model: PoseModel::open(&sessions)?,
            sampler: crate::vision::Sampler::new(options.max_fps.clamp(1, 60)),
            smoothing: options.smoothing,
            builder: None,
            region: None,
        })
    }

    /// Analyzes `frame` if it is due; true when it was.
    pub fn push(&mut self, frame: &fosscut_media::VideoFrame) -> Result<bool, AnalysisError> {
        if !self.sampler.take(frame.pts) {
            return Ok(false);
        }
        let picture = Rgb::from_frame(frame, WORKING_SIDE);
        self.push_picture(frame.pts, &picture, (frame.width, frame.height))?;
        Ok(true)
    }

    /// Analyzes `picture`, the frame at `time` of a `frame`-sized source.
    pub fn push_picture(
        &mut self,
        time: Flicks,
        picture: &Rgb,
        frame: (u32, u32),
    ) -> Result<(), AnalysisError> {
        let seen = self.model.observe(picture, self.region)?;
        self.region = seen.as_ref().map(|s| s.next);
        let builder = self
            .builder
            .get_or_insert_with(|| PoseBuilder::new(frame.0, frame.1, self.smoothing));
        builder.push(time, (picture.width, picture.height), seen.as_ref());
        Ok(())
    }

    /// The track of every frame analyzed.
    pub fn finish(self) -> PoseTrack {
        match self.builder {
            Some(builder) => builder.finish(),
            None => PoseTrack {
                mask_size: (0, 0),
                frames: Vec::new(),
            },
        }
    }
}

/// Without ONNX Runtime support the analyzer cannot be made.
#[cfg(not(feature = "onnx"))]
pub struct PoseAnalyzer;

#[cfg(not(feature = "onnx"))]
impl PoseAnalyzer {
    /// Always fails: this build has no ONNX Runtime support.
    pub fn new(_: &PoseOptions, _: &Path) -> Result<PoseAnalyzer, AnalysisError> {
        Err(AnalysisError::OnnxUnavailable)
    }

    /// Never called.
    pub fn push(&mut self, _: &fosscut_media::VideoFrame) -> Result<bool, AnalysisError> {
        Err(AnalysisError::OnnxUnavailable)
    }

    /// Never called.
    pub fn finish(self) -> PoseTrack {
        PoseTrack {
            mask_size: (0, 0),
            frames: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_follow_the_ssd_layout() {
        let all = anchors();
        assert_eq!(all.len(), ANCHORS);
        // 28 by 28 cells of stride 8 with two anchors each come first.
        assert_eq!(all[0], [0.5 / 28.0, 0.5 / 28.0]);
        assert_eq!(all[1], all[0]);
        assert_eq!(all[2], [1.5 / 28.0, 0.5 / 28.0]);
        assert_eq!(all[28 * 28 * 2], [0.5 / 14.0, 0.5 / 14.0]);
        // Then 7 by 7 cells with six anchors each.
        let last = all[ANCHORS - 1];
        assert!((last[0] - 6.5 / 7.0).abs() < 1e-6 && (last[1] - 6.5 / 7.0).abs() < 1e-6);
        assert_eq!(all[ANCHORS - 6], last);
        assert_ne!(all[ANCHORS - 7], last);
    }

    #[test]
    fn letterboxes_centre_the_picture() {
        let wide = Letterbox::of(640, 360);
        assert!((wide.scale - 0.35).abs() < 1e-9);
        assert!((wide.pad.0).abs() < 1e-9 && (wide.pad.1 - 49.0).abs() < 1e-9);
        // The input's centre is the picture's centre; its top-left corner the padding.
        let (x, y) = wide.to_picture(112.0, 112.0);
        assert!((x - 320.0).abs() < 1e-9 && (y - 180.0).abs() < 1e-9);
        let (x, y) = wide.to_picture(0.0, 49.0);
        assert!(x.abs() < 1e-9 && y.abs() < 1e-9);
        let tall = Letterbox::of(100, 200);
        assert!((tall.pad.0 - 56.0).abs() < 1e-9 && tall.pad.1.abs() < 1e-9);
    }

    /// Detector outputs with one anchor's values set.
    fn outputs(anchor: usize, values: [f32; 12], logit: f32) -> (Vec<f32>, Vec<f32>) {
        let mut boxes = vec![0.0; ANCHORS * 12];
        boxes[anchor * 12..anchor * 12 + 12].copy_from_slice(&values);
        let mut scores = vec![-10.0; ANCHORS];
        scores[anchor] = logit;
        (boxes, scores)
    }

    #[test]
    fn detections_decode_relative_to_anchors_and_the_letterbox() {
        // The anchor of cell (14, 14) of the 28 by 28 map is at the input's centre plus half
        // a cell: (14.5 / 28) * 224 = 116 pixels.
        let anchor = (14 * 28 + 14) * 2;
        let (boxes, scores) = outputs(
            anchor,
            [
                -4.0, 10.0, 40.0, 80.0, -4.0, 30.0, -4.0, -50.0, 0.0, 0.0, 0.0, 0.0,
            ],
            3.0,
        );
        let letterbox = Letterbox::of(640, 360);
        let found = detections(&boxes, &scores, letterbox);
        assert_eq!(found.len(), 1, "only scores of 0.5 or more");
        let d = &found[0];
        assert!((d.score - sigmoid(3.0)).abs() < 1e-6);
        // Box centre (112, 126) in the input is (320, 220) in the picture.
        assert!(
            (d.bounds[0] - 320.0).abs() < 1e-3 && (d.bounds[1] - 220.0).abs() < 1e-3,
            "{:?}",
            d.bounds
        );
        assert!((d.bounds[2] - 40.0 / 0.35).abs() < 1e-3);
        // Hips at (112, 146) and the circle's edge at (112, 66): a body standing upright.
        let (hips, edge) = (d.keypoints[0], d.keypoints[1]);
        assert!((hips.0 - 320.0).abs() < 1e-3 && (hips.1 - (146.0 - 49.0) / 0.35).abs() < 1e-3);
        let region = Region::around(hips, edge);
        assert!(region.rotation.abs() < 1e-9, "upright: {}", region.rotation);
        let radius = (146.0 - 66.0) / 0.35;
        assert!((region.side - 2.0 * radius * REGION_SCALE).abs() < 1e-6);
        assert!(detections(&boxes, &vec![-1.0; ANCHORS], letterbox).is_empty());
    }

    #[test]
    fn overlapping_detections_merge_by_score() {
        let detection = |score: f32, x: f64| Detection {
            score,
            bounds: [x, 100.0, 50.0, 100.0],
            keypoints: [(x, 120.0), (x, 40.0), (x, 80.0), (x, 0.0)],
        };
        let merged = best(&[
            detection(0.9, 100.0),
            detection(0.6, 104.0),
            detection(0.8, 400.0),
        ])
        .unwrap();
        assert_eq!(merged.score, 0.9);
        // Weighted 0.9 : 0.6 between 100 and 104; the far one stays out.
        assert!(
            (merged.bounds[0] - 101.6).abs() < 1e-5,
            "{:?}",
            merged.bounds
        );
        assert!((merged.keypoints[0].0 - 101.6).abs() < 1e-5);
        assert!(best(&[]).is_none());
        assert_eq!(overlap(&[0.0, 0.0, 2.0, 2.0], &[0.0, 0.0, 2.0, 2.0]), 1.0);
        assert_eq!(overlap(&[0.0, 0.0, 2.0, 2.0], &[5.0, 0.0, 2.0, 2.0]), 0.0);
    }

    #[test]
    fn regions_turn_with_the_body_and_map_back() {
        // A body lying with its head to the right: the circle's edge is right of the hips,
        // so the region turns a quarter clockwise and the crop's top points at it.
        let lying = Region::around((100.0, 100.0), (180.0, 100.0));
        assert!(
            (lying.rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
            "{}",
            lying.rotation
        );
        let upside_down = Region::around((100.0, 100.0), (100.0, 180.0));
        assert!((upside_down.rotation.abs() - std::f64::consts::PI).abs() < 1e-9);
        // The crop's centre is the region's centre; its top edge's middle lies towards the
        // circle's edge, a half side away.
        let map = lying.map(CROP);
        let (x, y) = map.apply(128.0, 128.0);
        assert!((x - 100.0).abs() < 1e-9 && (y - 100.0).abs() < 1e-9);
        let (x, y) = map.apply(128.0, 0.0);
        assert!(
            (x - (100.0 + lying.side / 2.0)).abs() < 1e-9 && (y - 100.0).abs() < 1e-9,
            "({x}, {y})"
        );
        let back = map.inverse().unwrap();
        let (u, v) = back.apply(x, y);
        assert!((u - 128.0).abs() < 1e-9 && v.abs() < 1e-9);
        assert!(
            (normalize_radians(3.0 * std::f64::consts::PI) - std::f64::consts::PI).abs() < 1e-9
                || (normalize_radians(3.0 * std::f64::consts::PI) + std::f64::consts::PI).abs()
                    < 1e-9
        );
    }

    #[test]
    fn landmarks_and_masks_map_from_the_crop_to_the_frame() {
        let region = Region {
            x: 320.0,
            y: 180.0,
            side: 256.0,
            rotation: 0.0,
        };
        let mut points = vec![0.0f32; POINTS * 5];
        // Landmark 0 at the crop's top-left quarter, visible; 33 and 34 set the next region.
        points[..5].copy_from_slice(&[64.0, 64.0, 0.0, 5.0, 5.0]);
        points[33 * 5..33 * 5 + 2].copy_from_slice(&[128.0, 160.0]);
        points[34 * 5..34 * 5 + 2].copy_from_slice(&[128.0, 60.0]);
        // A mask that is the crop's left half.
        let mask: Vec<f32> = (0..CROP * CROP)
            .map(|i| if i % CROP < CROP / 2 { 20.0 } else { -20.0 })
            .collect();
        let seen = observation(region, (640, 360), &points, 0.9, &mask).unwrap();
        let l = seen.landmarks[0];
        assert!(
            (l.x - 256.0 / 640.0).abs() < 1e-6 && (l.y - 116.0 / 360.0).abs() < 1e-6,
            "{l:?}"
        );
        assert!(l.visibility > 0.99 && l.presence > 0.99);
        assert!(seen.next.rotation.abs() < 1e-9 && (seen.next.y - 212.0).abs() < 1e-9);
        // Over a 64 by 36 mask of the frame: the crop spans x 192..448 of 640, so its left
        // half covers stored columns 19 to 31.
        let frame = frame_mask(&seen, (640, 360), (64, 36));
        let at = |x: usize, y: usize| frame[y * 64 + x];
        assert!(
            at(22, 18) > 0.99 && at(29, 18) > 0.99,
            "inside the left half"
        );
        assert!(at(34, 18) < 0.01 && at(40, 18) < 0.01, "the right half");
        assert!(at(5, 18) < 0.01 && at(60, 18) < 0.01, "outside the region");
        assert!(observation(region, (640, 360), &points[..10], 0.9, &mask).is_none());
    }

    #[test]
    fn model_inputs_have_the_documented_shapes_and_ranges() {
        let picture = Rgb::from_fn(64, 32, |_, _| [255, 255, 255]);
        let (letterbox, input) = detector_input(&picture);
        assert_eq!(input.len(), DETECTOR * DETECTOR * 3);
        assert_eq!(letterbox.pad.1, 56.0);
        // Padding is black (-1); the picture is white (1).
        assert_eq!(input[0], -1.0);
        let middle = (112 * DETECTOR + 112) * 3;
        assert!((input[middle] - 1.0).abs() < 1e-6);
        let region = Region {
            x: 32.0,
            y: 16.0,
            side: 16.0,
            rotation: 0.3,
        };
        let crop = landmark_input(&picture, &region);
        assert_eq!(crop.len(), CROP * CROP * 3);
        assert!(crop.iter().all(|&v| (0.0..=1.0).contains(&v)));
        assert!((crop[(128 * CROP + 128) * 3] - 1.0).abs() < 1e-6);
    }

    fn landmarks(x: f32) -> [Landmark; LANDMARKS] {
        [Landmark {
            x,
            y: 0.5,
            visibility: 0.9,
            presence: 0.9,
        }; LANDMARKS]
    }

    fn observed(x: f32) -> Observation {
        Observation {
            presence: 0.95,
            landmarks: landmarks(x),
            region: Region {
                x: 32.0,
                y: 18.0,
                side: 32.0,
                rotation: 0.0,
            },
            next: Region {
                x: 32.0,
                y: 18.0,
                side: 32.0,
                rotation: 0.0,
            },
            mask: vec![1.0; CROP * CROP],
        }
    }

    #[test]
    fn tracks_round_trip_and_interpolate() {
        let second = Flicks::from_seconds(1).unwrap().get();
        let at = |i: i64| Flicks::new(i * second / 30);
        let mut builder = PoseBuilder::new(64, 36, 0.0);
        assert_eq!(builder.mask_size(), (256, 144));
        for i in 0..40 {
            // Nobody in frames 3 and 4 (two frames, so they are not filled in).
            let seen = (i != 3 && i != 4).then(|| observed(0.25 + i as f32 * 0.001));
            builder.push(at(i), (64, 36), seen.as_ref());
        }
        let track = builder.finish();
        assert_eq!(track.frames.len(), 40);
        let decoded = PoseTrack::decode(&track.encode()).unwrap();
        assert_eq!(decoded, track);
        let mut damaged = track.encode();
        damaged.truncate(damaged.len() - 3);
        assert!(PoseTrack::decode(&damaged).is_err());
        assert!(PoseTrack::decode(b"FCMATTE1").is_err());
        // Half-way between frames 10 and 11.
        let half = track
            .at(Flicks::new((at(10).get() + at(11).get()) / 2))
            .unwrap();
        assert_eq!(half.frame, 10);
        assert!(
            (half.landmarks[0].x - 0.2605).abs() < 1e-4,
            "{}",
            half.landmarks[0].x
        );
        // No person in frame 3, so frame 2 does not interpolate towards it.
        let empty = track.at(at(3)).unwrap();
        assert!(empty.presence < MIN_PRESENCE);
        let held = track.at(at(2) + Flicks::new(5)).unwrap().landmarks[0].x;
        assert!((held - 0.252).abs() < 1e-6, "{held}");
        // Masks: the region covers the middle of the frame.
        let mask = track.mask(10).unwrap();
        assert_eq!(mask.len(), 256 * 144);
        assert_eq!(mask[72 * 256 + 128], 255);
        assert_eq!(mask[72 * 256 + 5], 0);
        assert!(track.mask(3).unwrap().iter().all(|&v| v == 0));
        assert!(track.mask(99).is_none());
        let summary = track.summary(DataFile::of(&track.encode()));
        assert_eq!((summary.frames, summary.frames_with_person), (40, 38));
        assert_eq!(summary.samples.len(), 2, "one per second");
        // Ten minutes give 60 samples, ten seconds apart.
        let long = PoseTrack {
            mask_size: (1, 1),
            frames: (0..601)
                .map(|s| PoseFrame {
                    time: Flicks::new(s * second),
                    presence: 0.0,
                    landmarks: [Landmark::default(); LANDMARKS],
                    mask: pack(&[0]),
                })
                .collect(),
        };
        let samples = long.summary(DataFile::of(&[])).samples;
        assert!(
            samples.len() <= 61 && samples.len() >= 55,
            "{}",
            samples.len()
        );
        assert_eq!(samples[1].at.flicks, Flicks::new(11 * second));
        assert_eq!(summary.samples[0].landmarks.len(), LANDMARKS);
        assert_eq!(summary.mask_size, [256, 144]);
        let bounds = summary.bounds.unwrap();
        assert!(
            (bounds[0] - 0.25).abs() < 1e-3 && (bounds[2] - 0.289).abs() < 1e-3,
            "{bounds:?}"
        );
    }

    #[test]
    fn single_frame_detections_and_drop_outs_are_mended() {
        use Fix::{Clear, Fill, Keep};
        assert_eq!(
            debounce(&[false, true, false, false]),
            [Keep, Clear, Keep, Keep]
        );
        assert_eq!(
            debounce(&[true, true, false, true, true]),
            [Keep, Keep, Fill, Keep, Keep]
        );
        assert_eq!(
            debounce(&[true, false, true, true]),
            [Clear, Keep, Keep, Keep]
        );
        assert_eq!(debounce(&[true, false]), [Clear, Keep]);
        assert_eq!(debounce(&[true]), [Keep], "a single frame is all there is");
        assert_eq!(debounce(&[true, true, false]), [Keep, Keep, Keep]);
        assert!(debounce(&[]).is_empty());
        let second = Flicks::from_seconds(1).unwrap().get();
        let mut builder = PoseBuilder::new(64, 36, 0.0);
        let seen = observed(0.3);
        let sequence = [true, true, false, true, true, false, true, false, false];
        for (i, present) in sequence.into_iter().enumerate() {
            let time = Flicks::new(i as i64 * second / 30);
            builder.push(time, (64, 36), present.then_some(&seen));
        }
        let track = builder.finish();
        let presence: Vec<bool> = track
            .frames
            .iter()
            .map(|f| f.presence >= MIN_PRESENCE)
            .collect();
        // Frame 2 is filled between 1 and 3; frame 6 stands alone and goes.
        assert_eq!(
            presence,
            [true, true, true, true, true, false, false, false, false]
        );
        assert!(track.mask(6).unwrap().iter().all(|&v| v == 0));
        assert_eq!(track.mask(2), track.mask(1));
    }

    #[test]
    fn landmark_smoothing_steadies_jitter_and_keeps_jumps() {
        let second = Flicks::from_seconds(1).unwrap().get();
        let mut builder = PoseBuilder::new(64, 36, 1.0);
        for i in 0..20i64 {
            // Jitter of 0.4% around 0.3, then a jump to 0.7.
            let x = if i < 10 {
                0.3 + if i % 2 == 0 { 0.004 } else { -0.004 }
            } else {
                0.7
            };
            builder.push(Flicks::new(i * second / 30), (64, 36), Some(&observed(x)));
        }
        let track = builder.finish();
        let x = |i: usize| track.frames[i].landmarks[0].x;
        assert!((x(5) - 0.3).abs() < 0.0015, "jitter shrinks: {}", x(5));
        assert!(
            (x(9) - 0.3).abs() < 0.005 && (x(10) - 0.7).abs() < 1e-3,
            "the jump stays: {} {}",
            x(9),
            x(10)
        );
    }
}
