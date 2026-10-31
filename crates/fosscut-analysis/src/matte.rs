//! Subject mattes (ADR-0033, item 3): the alpha of a clip's subject in every frame, which
//! the `key.subject` effect reads to remove the background.
//!
//! Two models make them, chosen by [`MatteOptions::model`]:
//!
//! - **`person`** (the default): the person mask of MediaPipe Pose Landmarker
//!   ([`crate::pose`]), about 20 ms a frame on a laptop CPU. It finds people only.
//! - **`subject`**: BiRefNet_lite (Zheng and others, "Bilateral reference for
//!   high-resolution dichotomous image segmentation", 2024), which separates any salient
//!   subject with fine edges but takes 9 to 14 s a frame on the same CPU. Its ONNX export
//!   reads exactly 1024 by 1024 pixels (the frame resized, RGB normalized with the ImageNet
//!   mean and deviation), so a smaller input cannot make it faster; analyzing every `every`-th
//!   frame can.
//!
//! **Frames between analyzed ones** follow the subject's motion: the subject's box in the
//! analyzed frame before is followed forward through the frames in between by
//! [`crate::track::Tracker`], and the box in the analyzed frame after is followed backward.
//! Each of the two mattes is moved and scaled with its box, then they are cross-faded by
//! time. A subject that moves as a whole stays sharp; one that changes shape (arms swinging)
//! shows a soft double edge where the two mattes disagree, so analyze every frame (or every
//! second one) for fast action, and every fifth to tenth for slow scenes such as an interview.
//!
//! Mattes are smoothed in time ([`TemporalSmoother`]), stored at `size` pixels on the frame's
//! longer side (512 by default), and packed by run length into one data file. Person mattes of
//! every frame also drop a person seen in one frame only and fill one lost for one frame, as
//! the pose analyzer does ([`crate::pose::debounce`]).

use std::path::Path;
use std::sync::Arc;

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::scenes::Luma;
use crate::track::{Region, Tracker};
use crate::vision::{
    DataError, DataFile, Reader, TemporalSmoother, Writer, frame_at, pack, resize, sample_gray,
    to_bytes, unpack,
};
use crate::{AnalysisError, Span};

/// The BiRefNet model the `subject` matte runs.
pub const SUBJECT_MODEL: &str = "birefnet-lite";
/// BiRefNet's input side.
const SUBJECT_INPUT: usize = 1024;
/// ImageNet's channel means, which BiRefNet's input is normalized by.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
/// ImageNet's channel deviations.
const DEVIATION: [f32; 3] = [0.229, 0.224, 0.225];

/// Which model makes a matte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatteModel {
    /// The pose model's person mask: people only, fast.
    #[default]
    Person,
    /// BiRefNet: any subject, fine edges, slow.
    Subject,
}

impl MatteModel {
    /// Its name in options: `person` or `subject`.
    pub fn name(self) -> &'static str {
        match self {
            MatteModel::Person => "person",
            MatteModel::Subject => "subject",
        }
    }

    /// The installed model it runs.
    pub fn model(self) -> &'static str {
        match self {
            MatteModel::Person => crate::pose::MODEL,
            MatteModel::Subject => SUBJECT_MODEL,
        }
    }

    /// The model called `name`.
    pub fn parse(name: &str) -> Option<MatteModel> {
        [MatteModel::Person, MatteModel::Subject]
            .into_iter()
            .find(|m| m.name() == name)
    }
}

/// Options of the `matte` analyzer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MatteOptions {
    /// `person` (the default: the pose model's person mask, about 20 ms a frame) or
    /// `subject` (BiRefNet: any subject with fine edges, 9 to 14 s a frame on a laptop CPU).
    pub model: MatteModel,
    /// Run the model on every `every`-th analyzed frame (1 to 300, default 1) and follow the
    /// subject's motion in between: 5 to 10 keeps `subject` practical for slow scenes, at
    /// the cost of soft edges where the subject changes shape quickly.
    pub every: u32,
    /// The stored matte's longer side in pixels, 64 to 1024 (default 512).
    pub size: u32,
    /// How much mattes are smoothed in time, from 0 (not at all) to 1 (a Gaussian of two
    /// frames); edges that move are left alone. Default 0.5.
    pub smoothing: f64,
    /// Most frames analyzed per second, 1 to 60 (default 30).
    pub max_fps: u32,
}

impl Default for MatteOptions {
    fn default() -> MatteOptions {
        MatteOptions {
            model: MatteModel::Person,
            every: 1,
            size: 512,
            smoothing: 0.5,
            max_fps: 30,
        }
    }
}

impl MatteOptions {
    /// The options with every value brought into its range.
    pub fn clamped(&self) -> MatteOptions {
        MatteOptions {
            model: self.model,
            every: self.every.clamp(1, 300),
            size: self.size.clamp(64, 1024),
            smoothing: if self.smoothing.is_finite() {
                self.smoothing.clamp(0.0, 1.0)
            } else {
                0.0
            },
            max_fps: self.max_fps.clamp(1, 60),
        }
    }
}

/// One frame's matte.
#[derive(Clone, Debug, PartialEq)]
pub struct MatteFrame {
    /// The frame's time from the start of the file.
    pub time: Flicks,
    /// Whether the model ran on it (otherwise it follows the analyzed frames around it).
    pub key: bool,
    /// The alpha, packed with [`crate::vision::pack`]: `size` bytes from 0 (background) to 255
    /// (subject).
    pub alpha: Vec<u8>,
}

/// Every analyzed frame's matte: the data file of a `matte` result.
#[derive(Clone, Debug, PartialEq)]
pub struct MatteTrack {
    /// The mattes' width and height.
    pub size: (u32, u32),
    /// The frames in time order.
    pub frames: Vec<MatteFrame>,
}

/// What the `matte` analyzer reports: a summary and the data file with every frame's matte,
/// which `key.subject` reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Matte {
    /// `person` or `subject`.
    pub kind: MatteModel,
    /// The installed model that made it.
    pub model: String,
    /// Frames with a matte.
    pub frames: u32,
    /// Frames the model ran on; the others follow them.
    pub keyframes: u32,
    /// The time from the first to the last frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    /// The mattes' width and height in pixels.
    pub size: [u32; 2],
    /// The share of the frame the subject covers, averaged over all frames, 0 to 1.
    pub coverage: f32,
    /// The data file beside the cache entry.
    pub data: DataFile,
}

const MATTE_MAGIC: &[u8; 8] = b"FCMATTE1";

impl MatteTrack {
    /// The track as its data file.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(MATTE_MAGIC);
        w.u32(self.size.0);
        w.u32(self.size.1);
        w.u32(self.frames.len() as u32);
        for frame in &self.frames {
            w.i64(frame.time.get());
            w.u32(u32::from(frame.key));
            w.block(&frame.alpha);
        }
        w.0
    }

    /// A track from its data file.
    pub fn decode(data: &[u8]) -> Result<MatteTrack, DataError> {
        let mut r = Reader::new(data);
        if r.take(8)? != MATTE_MAGIC {
            return Err(DataError("it is not a matte file".into()));
        }
        let size = (r.u32()?, r.u32()?);
        let count = r.u32()? as usize;
        let mut frames = Vec::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            let time = Flicks::new(r.i64()?);
            let key = r.u32()? != 0;
            let alpha = r.block()?.to_vec();
            frames.push(MatteFrame { time, key, alpha });
        }
        if !r.is_done() {
            return Err(DataError("it has bytes after its last frame".into()));
        }
        Ok(MatteTrack { size, frames })
    }

    /// The index of the frame shown at `time`: the last at or before it, or the first.
    pub fn at(&self, time: Flicks) -> Option<usize> {
        frame_at(self.frames.iter().map(|f| f.time), time)
    }

    /// The matte of frame `index`, unpacked: `size` bytes.
    pub fn alpha(&self, index: usize) -> Option<Vec<u8>> {
        let frame = self.frames.get(index)?;
        unpack(&frame.alpha, self.size.0 as usize * self.size.1 as usize)
    }

    /// The summary agents read, with `data` describing this track's data file.
    pub fn summary(&self, kind: MatteModel, data: DataFile) -> Matte {
        let pixels = (self.size.0 as usize * self.size.1 as usize).max(1);
        let total: f64 = (0..self.frames.len())
            .filter_map(|i| self.alpha(i))
            .map(|a| a.iter().map(|&v| f64::from(v)).sum::<f64>() / (255.0 * pixels as f64))
            .sum();
        Matte {
            kind,
            model: kind.model().to_string(),
            frames: self.frames.len() as u32,
            keyframes: self.frames.iter().filter(|f| f.key).count() as u32,
            span: match (self.frames.first(), self.frames.last()) {
                (Some(first), Some(last)) => Some(Span::new(first.time, last.time)),
                _ => None,
            },
            size: [self.size.0, self.size.1],
            coverage: ((total / self.frames.len().max(1) as f64) * 1000.0).round() as f32 / 1000.0,
            data,
        }
    }
}

/// A frame's luma plane kept for following the subject between analyzed frames.
#[derive(Clone, Debug)]
pub struct Plane {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes from one row to the next.
    pub stride: usize,
    /// The plane.
    pub data: Arc<[u8]>,
}

impl Plane {
    /// The luma plane of `frame`, shared rather than copied.
    pub fn of(frame: &fosscut_media::VideoFrame) -> Option<Plane> {
        let plane = frame.planes.first()?;
        Some(Plane {
            width: frame.width,
            height: frame.height,
            stride: plane.stride,
            data: plane.data.clone(),
        })
    }

    fn luma(&self) -> Luma<'_> {
        Luma {
            width: self.width,
            height: self.height,
            stride: self.stride,
            data: &self.data,
        }
    }
}

/// An analyzed frame: its matte and its picture.
struct Key {
    time: Flicks,
    alpha: Vec<f32>,
    plane: Option<Plane>,
    region: Option<Region>,
}

/// Gathers mattes into a [`MatteTrack`]: frames the model ran on, and frames between them
/// that follow the subject's motion, smoothed in time.
pub struct MatteBuilder {
    size: (u32, u32),
    smoother: TemporalSmoother,
    /// Keys of frames still in the smoother, in order.
    keys: std::collections::VecDeque<bool>,
    last: Option<Key>,
    between: Vec<(Flicks, Option<Plane>)>,
    frames: Vec<MatteFrame>,
}

impl MatteBuilder {
    /// A builder for mattes `size` pixels large, smoothed by `smoothing` (0 to 1).
    pub fn new(size: (u32, u32), smoothing: f64) -> MatteBuilder {
        MatteBuilder {
            size,
            smoother: TemporalSmoother::new(smoothing),
            keys: std::collections::VecDeque::new(),
            last: None,
            between: Vec::new(),
            frames: Vec::new(),
        }
    }

    /// Adds a frame the model ran on: its matte (`size` values 0 to 1) and its picture.
    /// Frames added with [`MatteBuilder::push_between`] since the last one are filled in.
    pub fn push_key(&mut self, time: Flicks, alpha: Vec<f32>, plane: Option<Plane>) {
        let region = subject_region(&alpha, self.size);
        let key = Key {
            time,
            alpha,
            plane,
            region,
        };
        let between = std::mem::take(&mut self.between);
        if let Some(last) = self.last.take() {
            let filled = fill(&last, Some(&key), &between, self.size);
            self.emit(last.time, last.alpha, true);
            for (time, alpha) in filled {
                self.emit(time, alpha, false);
            }
        }
        self.last = Some(key);
    }

    /// Adds a frame between analyzed ones.
    pub fn push_between(&mut self, time: Flicks, plane: Option<Plane>) {
        if self.last.is_some() {
            self.between.push((time, plane));
        }
    }

    fn emit(&mut self, time: Flicks, alpha: Vec<f32>, key: bool) {
        self.keys.push_back(key);
        if let Some(done) = self.smoother.push(time, alpha) {
            self.complete(done);
        }
    }

    fn complete(&mut self, (time, alpha): (Flicks, Vec<f32>)) {
        let key = self.keys.pop_front().unwrap_or(false);
        self.frames.push(MatteFrame {
            time,
            key,
            alpha: pack(&to_bytes(&alpha)),
        });
    }

    /// The finished track. Frames after the last analyzed one follow it forward.
    pub fn finish(mut self) -> MatteTrack {
        let between = std::mem::take(&mut self.between);
        if let Some(last) = self.last.take() {
            let filled = fill(&last, None, &between, self.size);
            self.emit(last.time, last.alpha, true);
            for (time, alpha) in filled {
                self.emit(time, alpha, false);
            }
        }
        let smoother = std::mem::replace(&mut self.smoother, TemporalSmoother::new(0.0));
        for done in smoother.finish() {
            self.complete(done);
        }
        MatteTrack {
            size: self.size,
            frames: self.frames,
        }
    }
}

/// The box around the subject in `alpha` (where it is at least half), a little larger, as
/// fractions of the frame; `None` when there is none.
fn subject_region(alpha: &[f32], size: (u32, u32)) -> Option<Region> {
    let (w, h) = (size.0 as usize, size.1 as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if alpha[y * w + x] >= 0.5 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 == usize::MAX {
        return None;
    }
    let margin = 1.1;
    let width = ((x1 - x0) as f64 * margin / w as f64).min(1.0);
    let height = ((y1 - y0) as f64 * margin / h as f64).min(1.0);
    Some(Region {
        x: (x0 + x1) as f64 / 2.0 / w as f64,
        y: (y0 + y1) as f64 / 2.0 / h as f64,
        width,
        height,
    })
}

/// The mattes of the frames `between` two keys, `before` and `after` (or only `before` at
/// the end), each key's matte moved with its subject's box as followed through the frames,
/// then cross-faded by time.
fn fill(
    before: &Key,
    after: Option<&Key>,
    between: &[(Flicks, Option<Plane>)],
    size: (u32, u32),
) -> Vec<(Flicks, Vec<f32>)> {
    if between.is_empty() {
        return Vec::new();
    }
    let forward = follow(before, between.iter().map(|(_, p)| p.as_ref()));
    let backward = after.map(|after| {
        let mut regions = follow(after, between.iter().rev().map(|(_, p)| p.as_ref()));
        regions.reverse();
        regions
    });
    between
        .iter()
        .enumerate()
        .map(|(i, (time, _))| {
            let from_before = moved(&before.alpha, size, before.region, forward[i]);
            let alpha = match (after, &backward) {
                (Some(after), Some(backward)) => {
                    let from_after = moved(&after.alpha, size, after.region, backward[i]);
                    let span = (after.time - before.time).get().max(1) as f32;
                    let s = ((*time - before.time).get() as f32 / span).clamp(0.0, 1.0);
                    from_before
                        .iter()
                        .zip(&from_after)
                        .map(|(a, b)| a + (b - a) * s)
                        .collect()
                }
                _ => from_before,
            };
            (*time, alpha)
        })
        .collect()
}

/// Where `key`'s subject is in each of `planes`, followed from the key's own picture;
/// `None` where it cannot be followed.
fn follow<'a>(key: &Key, planes: impl Iterator<Item = Option<&'a Plane>>) -> Vec<Option<Region>> {
    let mut tracker = match (&key.plane, key.region) {
        (Some(plane), Some(region)) => Some(Tracker::new(plane.luma(), region)),
        _ => None,
    };
    planes
        .map(|plane| {
            let (tracker, plane) = (tracker.as_mut()?, plane?);
            let point = tracker.push(plane.luma());
            (!point.lost).then_some(point.region)
        })
        .collect()
}

/// `alpha` with its subject moved and scaled from box `from` to box `to`; unchanged without
/// both.
fn moved(alpha: &[f32], size: (u32, u32), from: Option<Region>, to: Option<Region>) -> Vec<f32> {
    let (Some(from), Some(to)) = (from, to) else {
        return alpha.to_vec();
    };
    let (w, h) = (size.0 as usize, size.1 as usize);
    let (sx, sy) = (
        from.width / to.width.max(1e-6),
        from.height / to.height.max(1e-6),
    );
    let mut out = Vec::with_capacity(w * h);
    for j in 0..h {
        for i in 0..w {
            let (u, v) = ((i as f64 + 0.5) / w as f64, (j as f64 + 0.5) / h as f64);
            let (x, y) = (from.x + (u - to.x) * sx, from.y + (v - to.y) * sy);
            out.push(sample_gray(alpha, w, h, x * w as f64, y * h as f64, 0.0));
        }
    }
    out
}

/// BiRefNet's input for `picture`: resized to 1024 by 1024 (edges repeated, not darkened),
/// channels first, normalized.
pub fn subject_input(picture: &crate::vision::Rgb) -> Vec<f32> {
    let n = SUBJECT_INPUT;
    let (w, h) = (picture.width as f32, picture.height as f32);
    let mut out = vec![0.0f32; 3 * n * n];
    for v in 0..n {
        let y = ((v as f32 + 0.5) * h / n as f32).clamp(0.5, (h - 0.5).max(0.5));
        for u in 0..n {
            let x = ((u as f32 + 0.5) * w / n as f32).clamp(0.5, (w - 0.5).max(0.5));
            let rgb = picture.sample(x, y);
            for c in 0..3 {
                out[c * n * n + v * n + u] = (rgb[c] - MEAN[c]) / DEVIATION[c];
            }
        }
    }
    out
}

/// BiRefNet's output logits (1024 by 1024) as a matte `size` pixels large.
pub fn subject_matte(logits: &[f32], size: (u32, u32)) -> Vec<f32> {
    let n = SUBJECT_INPUT;
    let alpha: Vec<f32> = logits[..(n * n).min(logits.len())]
        .iter()
        .map(|&l| crate::vision::sigmoid(l))
        .collect();
    if alpha.len() < n * n {
        return vec![0.0; size.0 as usize * size.1 as usize];
    }
    resize(&alpha, (n, n), (size.0 as usize, size.1 as usize))
}

/// The `matte` analyzer: takes decoded frames, runs its model on every `every`-th frame it
/// samples, and fills in the rest.
#[cfg(feature = "onnx")]
pub struct MatteAnalyzer {
    options: MatteOptions,
    model: Model,
    sampler: crate::vision::Sampler,
    sampled: u64,
    builder: Option<MatteBuilder>,
    /// Whether a person was seen in each frame, when the person model runs on every frame,
    /// so single-frame detections can be mended at the end.
    people: Vec<bool>,
}

#[cfg(feature = "onnx")]
enum Model {
    Person {
        model: Box<crate::pose::PoseModel>,
        region: Option<crate::pose::Region>,
    },
    Subject(Box<crate::onnx::Session>),
}

#[cfg(feature = "onnx")]
impl MatteAnalyzer {
    /// An analyzer with `options`, its model opened from `models` on the CPU.
    pub fn new(options: &MatteOptions, models: &Path) -> Result<MatteAnalyzer, AnalysisError> {
        let options = options.clamped();
        // Deterministic CPU kernels: the same file always gives the same mattes.
        let sessions = crate::onnx::SessionOptions::new(models).on(crate::onnx::Device::Cpu);
        let model = match options.model {
            MatteModel::Person => Model::Person {
                model: Box::new(crate::pose::PoseModel::open(&sessions)?),
                region: None,
            },
            MatteModel::Subject => {
                let manifest = crate::models::manifest(SUBJECT_MODEL)?;
                if !crate::models::is_installed(models, manifest) {
                    return Err(AnalysisError::Model(
                        crate::models::ModelError::NotInstalled {
                            what: SUBJECT_MODEL.into(),
                        },
                    ));
                }
                let file = crate::models::file_path(models, manifest, "model.onnx");
                Model::Subject(Box::new(crate::onnx::session(&file, &sessions)?))
            }
        };
        Ok(MatteAnalyzer {
            sampler: crate::vision::Sampler::new(options.max_fps),
            options,
            model,
            sampled: 0,
            builder: None,
            people: Vec::new(),
        })
    }

    /// Takes `frame` if it is due, running the model on it when it is a key; true when the
    /// model ran.
    pub fn push(&mut self, frame: &fosscut_media::VideoFrame) -> Result<bool, AnalysisError> {
        if !self.sampler.take(frame.pts) {
            return Ok(false);
        }
        let size = crate::vision::stored_size(frame.width, frame.height, self.options.size);
        let smoothing = self.options.smoothing;
        let builder = self
            .builder
            .get_or_insert_with(|| MatteBuilder::new(size, smoothing));
        let is_key = self.sampled.is_multiple_of(u64::from(self.options.every));
        self.sampled += 1;
        if !is_key {
            builder.push_between(frame.pts, Plane::of(frame));
            return Ok(false);
        }
        let alpha = match &mut self.model {
            Model::Person { model, region } => {
                let picture = crate::vision::Rgb::from_frame(frame, crate::pose::WORKING_SIDE);
                let seen = model.observe(&picture, *region)?;
                *region = seen.as_ref().map(|s| s.next);
                let alpha = match &seen {
                    Some(seen) => {
                        crate::pose::frame_mask(seen, (picture.width, picture.height), size)
                    }
                    None => vec![0.0; size.0 as usize * size.1 as usize],
                };
                if self.options.every == 1 {
                    self.people.push(seen.is_some());
                }
                alpha
            }
            Model::Subject(session) => {
                let picture = crate::vision::Rgb::from_frame(frame, 1024);
                let input = subject_input(&picture);
                let outputs = crate::pose::run(
                    session,
                    "input_image",
                    vec![1, 3, SUBJECT_INPUT, SUBJECT_INPUT],
                    input,
                    &["output_image"],
                )?;
                subject_matte(&outputs[0], size)
            }
        };
        builder.push_key(frame.pts, alpha, Plane::of(frame));
        Ok(true)
    }

    /// The track of every frame taken.
    pub fn finish(self) -> MatteTrack {
        let Some(builder) = self.builder else {
            return MatteTrack {
                size: (0, 0),
                frames: Vec::new(),
            };
        };
        let mut track = builder.finish();
        // Every frame was a key: one person flag per frame.
        if self.people.len() == track.frames.len() {
            let empty = pack(&vec![0u8; track.size.0 as usize * track.size.1 as usize]);
            for (i, fix) in crate::pose::debounce(&self.people).into_iter().enumerate() {
                match fix {
                    crate::pose::Fix::Keep => {}
                    crate::pose::Fix::Clear => track.frames[i].alpha = empty.clone(),
                    crate::pose::Fix::Fill => {
                        track.frames[i].alpha = track.frames[i - 1].alpha.clone();
                    }
                }
            }
        }
        track
    }
}

/// Without ONNX Runtime support the analyzer cannot be made.
#[cfg(not(feature = "onnx"))]
pub struct MatteAnalyzer;

#[cfg(not(feature = "onnx"))]
impl MatteAnalyzer {
    /// Always fails: this build has no ONNX Runtime support.
    pub fn new(_: &MatteOptions, _: &Path) -> Result<MatteAnalyzer, AnalysisError> {
        Err(AnalysisError::OnnxUnavailable)
    }

    /// Never called.
    pub fn push(&mut self, _: &fosscut_media::VideoFrame) -> Result<bool, AnalysisError> {
        Err(AnalysisError::OnnxUnavailable)
    }

    /// Never called.
    pub fn finish(self) -> MatteTrack {
        MatteTrack {
            size: (0, 0),
            frames: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (u32, u32) = (64, 36);

    /// A matte with a filled box from `x0` to `x1` and `y0` to `y1` in pixels.
    fn boxed(x0: usize, x1: usize, y0: usize, y1: usize) -> Vec<f32> {
        let (w, h) = (SIZE.0 as usize, SIZE.1 as usize);
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect()
    }

    /// A luma plane of a bright square on dark grey with a texture, its left edge at `x`.
    fn picture(x: u32) -> Plane {
        let (w, h) = (320u32, 180u32);
        let data: Vec<u8> = (0..w * h)
            .map(|i| {
                let (px, py) = (i % w, i / w);
                let inside = (x..x + 80).contains(&px) && (50..130).contains(&py);
                // The square's texture moves with it; the background's stays.
                if inside {
                    200 + (((px - x) * 7 + py * 13) % 23) as u8
                } else {
                    40 + ((px * 5 + py * 11) % 17) as u8
                }
            })
            .collect();
        Plane {
            width: w,
            height: h,
            stride: w as usize,
            data: data.into(),
        }
    }

    fn at(i: i64) -> Flicks {
        Flicks::new(i * Flicks::from_seconds(1).unwrap().get() / 30)
    }

    /// The box's left edge in a matte, in matte pixels.
    fn left_edge(alpha: &[u8]) -> usize {
        let w = SIZE.0 as usize;
        let row = &alpha[18 * w..19 * w];
        row.iter().position(|&v| v >= 128).unwrap_or(w)
    }

    #[test]
    fn frames_between_keys_follow_the_subject() {
        // The square moves 8 pixels per frame at 320 wide: 1.6 matte pixels at 64 wide.
        let mut builder = MatteBuilder::new(SIZE, 0.0);
        let matte_of = |x: u32| {
            let left = (x as usize * 64) / 320;
            boxed(left, left + 16, 10, 26)
        };
        for i in 0..11u32 {
            let x = 40 + i * 8;
            if i % 5 == 0 {
                builder.push_key(at(i64::from(i)), matte_of(x), Some(picture(x)));
            } else {
                builder.push_between(at(i64::from(i)), Some(picture(x)));
            }
        }
        // Two frames after the last key.
        for i in 11..13u32 {
            builder.push_between(at(i64::from(i)), Some(picture(40 + i * 8)));
        }
        let track = builder.finish();
        assert_eq!(track.frames.len(), 13);
        assert_eq!(
            track.frames.iter().filter(|f| f.key).count(),
            3,
            "frames 0, 5 and 10"
        );
        for (i, frame) in track.frames.iter().enumerate() {
            assert_eq!(frame.time, at(i as i64));
            let expected = (40 + i * 8) * 64 / 320;
            let edge = left_edge(&track.alpha(i).unwrap());
            assert!(
                edge.abs_diff(expected) <= 1,
                "frame {i}: the edge is at {edge}, the subject at {expected}"
            );
        }
        // Without following, the mattes would only fade: frame 2 would sit at the first
        // key's place with a faint copy at the second's.
        let decoded = MatteTrack::decode(&track.encode()).unwrap();
        assert_eq!(decoded, track);
        assert!(MatteTrack::decode(b"FCMATTE1\x01").is_err());
        let summary = track.summary(MatteModel::Subject, DataFile::of(&track.encode()));
        assert_eq!((summary.frames, summary.keyframes), (13, 3));
        assert_eq!(summary.model, SUBJECT_MODEL);
        assert!(
            (summary.coverage - 16.0 * 16.0 / (64.0 * 36.0)).abs() < 0.02,
            "{}",
            summary.coverage
        );
        assert_eq!(track.at(at(4) + Flicks::new(3)), Some(4));
    }

    #[test]
    fn a_subject_that_cannot_be_followed_fades_between_keys() {
        let mut builder = MatteBuilder::new(SIZE, 0.0);
        builder.push_key(at(0), boxed(0, 16, 0, 36), None);
        builder.push_between(at(1), None);
        builder.push_key(at(2), boxed(48, 64, 0, 36), None);
        let track = builder.finish();
        let middle = track.alpha(1).unwrap();
        let w = SIZE.0 as usize;
        // Half of each.
        assert!(middle[18 * w + 4].abs_diff(128) <= 1 && middle[18 * w + 56].abs_diff(128) <= 1);
        assert_eq!(middle[18 * w + 30], 0);
        // Frames before the first key have nothing to follow and are left out.
        let mut late = MatteBuilder::new(SIZE, 0.0);
        late.push_between(at(0), None);
        late.push_key(at(1), boxed(0, 8, 0, 8), None);
        assert_eq!(late.finish().frames.len(), 1);
        assert!(subject_region(&vec![0.0; 64 * 36], SIZE).is_none());
    }

    #[test]
    fn subject_inputs_are_normalized_and_outputs_resized() {
        let picture =
            crate::vision::Rgb::from_fn(
                32,
                18,
                |x, _| if x < 16 { [255, 255, 255] } else { [0, 0, 0] },
            );
        let input = subject_input(&picture);
        assert_eq!(input.len(), 3 * 1024 * 1024);
        // Red channel of the top-left pixel (white) and the blue of the top-right (black).
        assert!((input[0] - (1.0 - MEAN[0]) / DEVIATION[0]).abs() < 1e-5);
        let blue_right = 2 * 1024 * 1024 + 1023;
        assert!((input[blue_right] - (0.0 - MEAN[2]) / DEVIATION[2]).abs() < 1e-5);
        // Logits: the left half positive.
        let logits: Vec<f32> = (0..1024 * 1024)
            .map(|i| if i % 1024 < 512 { 12.0 } else { -12.0 })
            .collect();
        let matte = subject_matte(&logits, (64, 36));
        assert_eq!(matte.len(), 64 * 36);
        assert!(matte[18 * 64 + 10] > 0.99 && matte[18 * 64 + 50] < 0.01);
        assert!(
            subject_matte(&logits[..10], (4, 4))
                .iter()
                .all(|&v| v == 0.0)
        );
        assert_eq!(MatteModel::parse("subject"), Some(MatteModel::Subject));
        assert_eq!(MatteModel::Person.model(), crate::pose::MODEL);
        let wild = MatteOptions {
            every: 0,
            size: 9000,
            smoothing: f64::NAN,
            max_fps: 500,
            ..MatteOptions::default()
        };
        let clamped = wild.clamped();
        assert_eq!(
            (
                clamped.every,
                clamped.size,
                clamped.smoothing,
                clamped.max_fps
            ),
            (1, 1024, 0.0, 60)
        );
    }
}
