//! What the vision analyzers ([`crate::pose`] and [`crate::matte`]) share: frames as RGB
//! pictures the models read, resampling, mattes packed by run length, smoothing in time, and
//! the binary files that hold their per-frame results beside the cache entry.
//!
//! Pixel `(i, j)` has its centre at `(i + 0.5, j + 0.5)` in pixel coordinates, so the whole
//! picture spans `0..width` and `0..height`.

use std::collections::VecDeque;

use fosscut_media::{Matrix, PixelFormat, Range, VideoFrame};
use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A frame as 8-bit RGB, display-referred (the code values a player shows), rows top to
/// bottom without padding: what the vision models were trained on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgb {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 3` bytes.
    pub data: Vec<u8>,
}

impl Rgb {
    /// A `width` by `height` picture with `pixel(x, y)` at each pixel, for tests and
    /// synthetic scenes.
    pub fn from_fn(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Rgb {
        let mut data = Vec::with_capacity(width as usize * height as usize * 3);
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&pixel(x, y));
            }
        }
        Rgb {
            width,
            height,
            data,
        }
    }

    /// An R'G'B'A `frame` averaged over `factor` by `factor` blocks into `width` by
    /// `height` pixels, ignoring alpha.
    fn from_rgba(frame: &VideoFrame, factor: usize, width: usize, height: usize) -> Rgb {
        let plane = &frame.planes[0];
        let mut data = Vec::with_capacity(width * height * 3);
        let block = (factor * factor) as f32;
        for by in 0..height {
            for bx in 0..width {
                let mut sum = [0.0f32; 3];
                for y in by * factor..(by + 1) * factor {
                    let row = plane.row(y.min(plane.rows.saturating_sub(1)));
                    for x in bx * factor..(bx + 1) * factor {
                        for (c, total) in sum.iter_mut().enumerate() {
                            *total += row.get(x * 4 + c).map_or(0.0, |&v| f32::from(v));
                        }
                    }
                }
                data.extend(sum.map(|total| (total / block).round().clamp(0.0, 255.0) as u8));
            }
        }
        Rgb {
            width: width as u32,
            height: height as u32,
            data,
        }
    }

    /// `frame` converted to RGB with its own matrix and range, averaged over square blocks of
    /// the whole number of pixels that keeps its longer side from `max_side` up to twice
    /// that (no averaging when it is already shorter). Chroma is sampled at the centre of
    /// each block from the nearest chroma sample.
    pub fn from_frame(frame: &VideoFrame, max_side: u32) -> Rgb {
        let long = frame.width.max(frame.height);
        let factor = (long / max_side.max(1)).max(1) as usize;
        let width = (frame.width as usize / factor).max(1);
        let height = (frame.height as usize / factor).max(1);
        let (kr, kb) = match frame.color.matrix {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt2020Ncl => (0.2627, 0.0593),
            _ => (0.2126, 0.0722),
        };
        if frame.format == PixelFormat::Rgba {
            return Rgb::from_rgba(frame, factor, width, height);
        }
        let full = frame.color.range == Range::Full;
        let identity = frame.color.matrix == Matrix::Identity;
        let luma = frame.planes.first();
        let byte = |plane: Option<&fosscut_media::Plane>, x: usize, y: usize| -> f32 {
            plane
                .and_then(|p| p.data.get(y.min(p.rows.saturating_sub(1)) * p.stride + x))
                .map_or(128.0, |&v| f32::from(v))
        };
        let mut data = Vec::with_capacity(width * height * 3);
        let block = (factor * factor) as f32;
        for by in 0..height {
            for bx in 0..width {
                let mut sum = 0.0f32;
                for y in by * factor..(by + 1) * factor {
                    for x in bx * factor..(bx + 1) * factor {
                        sum += byte(luma, x, y);
                    }
                }
                let y_code = sum / block;
                // Chroma of the block's centre pixel.
                let (cx, cy) = (
                    (bx * factor + factor / 2) / 2,
                    (by * factor + factor / 2) / 2,
                );
                let (cb, cr) = match frame.format {
                    PixelFormat::Nv12 => (
                        byte(frame.planes.get(1), cx * 2, cy),
                        byte(frame.planes.get(1), cx * 2 + 1, cy),
                    ),
                    PixelFormat::Yuv420p => (
                        byte(frame.planes.get(1), cx, cy),
                        byte(frame.planes.get(2), cx, cy),
                    ),
                    // Read above, before any Y'CbCr is looked at.
                    PixelFormat::Rgba => (128.0, 128.0),
                };
                let (y, u, v) = if full {
                    (y_code / 255.0, (cb - 128.0) / 255.0, (cr - 128.0) / 255.0)
                } else {
                    (
                        (y_code - 16.0) / 219.0,
                        (cb - 128.0) / 224.0,
                        (cr - 128.0) / 224.0,
                    )
                };
                let rgb = if identity {
                    // RGB stored as GBR in the planes.
                    [v + 0.5, y, u + 0.5]
                } else {
                    let kg = 1.0 - kr - kb;
                    let r = y + 2.0 * (1.0 - kr) * v;
                    let b = y + 2.0 * (1.0 - kb) * u;
                    let g = (y - kr * r - kb * b) / kg;
                    [r, g, b]
                };
                for c in rgb {
                    data.push((c * 255.0).round().clamp(0.0, 255.0) as u8);
                }
            }
        }
        Rgb {
            width: width as u32,
            height: height as u32,
            data,
        }
    }

    /// The colour at `(x, y)` in pixel coordinates, interpolated between the four nearest
    /// pixels, each channel from 0 to 1; black outside the picture.
    pub fn sample(&self, x: f32, y: f32) -> [f32; 3] {
        let (w, h) = (self.width as i64, self.height as i64);
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let mut out = [0.0f32; 3];
        for (dy, wy) in [(0, 1.0 - ty), (1, ty)] {
            for (dx, wx) in [(0, 1.0 - tx), (1, tx)] {
                let (px, py) = (x0 + dx, y0 + dy);
                if px < 0 || py < 0 || px >= w || py >= h {
                    continue;
                }
                let i = ((py * w + px) * 3) as usize;
                for (c, value) in out.iter_mut().enumerate() {
                    *value += wx * wy * f32::from(self.data[i + c]) / 255.0;
                }
            }
        }
        // The weights add up to 1 but rounding can pass it by a hair.
        out.map(|v| v.clamp(0.0, 1.0))
    }
}

/// Maps a pixel of a model's input to a point of the picture it reads: `x = a u + b v + c`,
/// `y = d u + e v + f`, both in pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    /// The six coefficients, row by row.
    pub m: [f64; 6],
}

impl Affine {
    /// The point `(u, v)` maps to.
    pub fn apply(&self, u: f64, v: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.m;
        (a * u + b * v + c, d * u + e * v + f)
    }

    /// The map back, or `None` when it squashes the plane flat.
    pub fn inverse(&self) -> Option<Affine> {
        let [a, b, c, d, e, f] = self.m;
        let det = a * e - b * d;
        if det.abs() < 1e-12 {
            return None;
        }
        let (ia, ib, id, ie) = (e / det, -b / det, -d / det, a / det);
        Some(Affine {
            m: [ia, ib, -(ia * c + ib * f), id, ie, -(id * c + ie * f)],
        })
    }
}

/// Samples `picture` through `map` into a `width` by `height` input of the model, each pixel
/// at its centre: channels 0 to 1, RGB order, rows top to bottom.
pub fn warp(picture: &Rgb, map: &Affine, width: usize, height: usize) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(width * height);
    for v in 0..height {
        for u in 0..width {
            let (x, y) = map.apply(u as f64 + 0.5, v as f64 + 0.5);
            out.push(picture.sample(x as f32, y as f32));
        }
    }
    out
}

/// One value of a single-channel picture at `(x, y)` in pixel coordinates, interpolated;
/// `outside` beyond its edges.
pub fn sample_gray(
    values: &[f32],
    width: usize,
    height: usize,
    x: f64,
    y: f64,
    outside: f32,
) -> f32 {
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let mut sum = 0.0f32;
    for (dy, wy) in [(0, 1.0 - ty), (1, ty)] {
        for (dx, wx) in [(0, 1.0 - tx), (1, tx)] {
            let (px, py) = (x0 + dx, y0 + dy);
            let value = if px < 0 || py < 0 || px >= width as i64 || py >= height as i64 {
                outside
            } else {
                values[py as usize * width + px as usize]
            };
            sum += wx * wy * value;
        }
    }
    sum
}

/// A single-channel picture resized from `from` to `to` pixels, one axis at a time: an area
/// average where it shrinks (each output pixel averages what it covers) and linear
/// interpolation where it grows.
pub fn resize(values: &[f32], from: (usize, usize), to: (usize, usize)) -> Vec<f32> {
    let rows = resample_axis(values, from.0, from.1, to.0, true);
    resample_axis(&rows, to.0, from.1, to.1, false)
}

/// One pass of [`resize`]: along rows (`across`) or columns.
fn resample_axis(
    values: &[f32],
    width: usize,
    height: usize,
    target: usize,
    across: bool,
) -> Vec<f32> {
    let (length, lines) = if across {
        (width, height)
    } else {
        (height, width)
    };
    let weights = axis_weights(length, target);
    let (out_width, out_height) = if across {
        (target, height)
    } else {
        (width, target)
    };
    let mut out = vec![0.0f32; out_width * out_height];
    for line in 0..lines {
        for (i, taps) in weights.iter().enumerate() {
            let mut sum = 0.0f32;
            for &(j, w) in taps {
                let value = if across {
                    values[line * width + j]
                } else {
                    values[j * width + line]
                };
                sum += w * value;
            }
            if across {
                out[line * out_width + i] = sum;
            } else {
                out[i * out_width + line] = sum;
            }
        }
    }
    out
}

/// For each of `target` output samples, the input samples of an axis `length` long it reads
/// and their weights, which add up to 1.
fn axis_weights(length: usize, target: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = length as f64 / target.max(1) as f64;
    (0..target)
        .map(|i| {
            if scale > 1.0 {
                // The output pixel covers [i * scale, (i + 1) * scale) of the input.
                let (start, end) = (i as f64 * scale, (i as f64 + 1.0) * scale);
                let mut taps = Vec::new();
                let mut j = start.floor() as usize;
                while (j as f64) < end && j < length {
                    let overlap = (end.min(j as f64 + 1.0) - start.max(j as f64)).max(0.0);
                    if overlap > 0.0 {
                        taps.push((j, (overlap / scale) as f32));
                    }
                    j += 1;
                }
                taps
            } else {
                let centre = (i as f64 + 0.5) * scale - 0.5;
                let low = centre.floor().max(0.0);
                let t = (centre - low).clamp(0.0, 1.0) as f32;
                let low = (low as usize).min(length.saturating_sub(1));
                let high = (low + 1).min(length.saturating_sub(1));
                if centre <= 0.0 {
                    vec![(0, 1.0)]
                } else {
                    vec![(low, 1.0 - t), (high, t)]
                }
            }
        })
        .collect()
}

/// The logistic function, which turns a model's logit into a probability.
pub fn sigmoid(logit: f32) -> f32 {
    1.0 / (1.0 + (-logit.clamp(-80.0, 80.0)).exp())
}

/// The stored size of a matte of a `width` by `height` frame whose longer side is `long`
/// pixels, keeping the frame's shape.
pub fn stored_size(width: u32, height: u32, long: u32) -> (u32, u32) {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let scale = f64::from(long) / w.max(h);
    (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    )
}

/// Matte values from 0 to 1 as bytes, with values within half a step of either end snapped
/// to it so runs stay long.
pub fn to_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .map(|b| match b {
            0..=1 => 0,
            254..=255 => 255,
            other => other,
        })
        .collect()
}

/// Packs bytes by runs (PackBits, as in TIFF): a header byte `n` below 128 is followed by
/// `n + 1` literal bytes; from 128 up it repeats the next byte `n - 126` times (2 to 129).
/// Mattes, mostly long runs of 0 and 255 with soft edges, shrink about tenfold.
pub fn pack(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() / 8 + 16);
    let mut i = 0;
    while i < bytes.len() {
        let mut run = 1;
        while i + run < bytes.len() && bytes[i + run] == bytes[i] && run < 129 {
            run += 1;
        }
        if run >= 2 {
            out.push((run + 126) as u8);
            out.push(bytes[i]);
            i += run;
            continue;
        }
        // Literals up to the next run of two or more, at most 128.
        let start = i;
        while i < bytes.len()
            && i - start < 128
            && !(i + 1 < bytes.len() && bytes[i + 1] == bytes[i])
        {
            i += 1;
        }
        if i == start {
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&bytes[start..i]);
    }
    out
}

/// The `length` bytes [`pack`] packed, or `None` when `packed` is damaged.
pub fn unpack(packed: &[u8], length: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(length);
    let mut i = 0;
    while i < packed.len() {
        let header = usize::from(packed[i]);
        i += 1;
        if header < 128 {
            let literal = packed.get(i..i + header + 1)?;
            out.extend_from_slice(literal);
            i += header + 1;
        } else {
            let value = *packed.get(i)?;
            out.extend(std::iter::repeat_n(value, header - 126));
            i += 1;
        }
        if out.len() > length {
            return None;
        }
    }
    (out.len() == length).then_some(out)
}

/// Smooths a sequence of mattes in time without blurring motion: each pixel becomes a
/// weighted mean of itself in the neighbouring frames, weighted by how far away in time each
/// frame is (a Gaussian) and by how much the value differs (another Gaussian), a bilateral
/// filter in time (Tomasi and Manduchi, "Bilateral filtering for gray and color images",
/// ICCV 1998). Flicker, small changes from frame to frame, averages out; an edge that moves
/// differs by much more than the value deviation where it passes, so it keeps its place
/// instead of leaving a trail. The filter is symmetric, so nothing lags.
///
/// Frames go in with [`TemporalSmoother::push`] and come out in the same order, delayed by
/// the filter's radius; [`TemporalSmoother::finish`] returns the rest.
#[derive(Clone, Debug)]
pub struct TemporalSmoother {
    radius: usize,
    time_weights: Vec<f32>,
    window: VecDeque<(Flicks, Vec<f32>)>,
    /// Frames already returned from the window's front part.
    emitted: usize,
}

/// The value difference at which a neighbour's weight falls to about 60%: changes much
/// larger than this are motion, not flicker.
const VALUE_SIGMA: f32 = 0.15;

impl TemporalSmoother {
    /// A smoother of `strength` from 0 (none) to 1 (a Gaussian of two frames in time).
    pub fn new(strength: f64) -> TemporalSmoother {
        let sigma = (strength.clamp(0.0, 1.0) * 2.0) as f32;
        let radius = if sigma < 0.05 {
            0
        } else {
            (2.0 * sigma).ceil() as usize
        };
        let time_weights = (0..=radius)
            .map(|k| (-(k as f32 * k as f32) / (2.0 * sigma * sigma).max(1e-6)).exp())
            .collect();
        TemporalSmoother {
            radius,
            time_weights,
            window: VecDeque::new(),
            emitted: 0,
        }
    }

    /// Adds the next frame; returns the frame that is now complete, if any.
    pub fn push(&mut self, time: Flicks, values: Vec<f32>) -> Option<(Flicks, Vec<f32>)> {
        self.window.push_back((time, values));
        // The frame at `emitted` is complete once `radius` frames follow it.
        if self.window.len() > self.emitted + self.radius {
            let out = self.smoothed(self.emitted);
            self.emitted += 1;
            if self.emitted > self.radius {
                self.window.pop_front();
                self.emitted -= 1;
            }
            return Some(out);
        }
        None
    }

    /// The frames still in the window, smoothed with what remains around them.
    pub fn finish(mut self) -> Vec<(Flicks, Vec<f32>)> {
        let mut out = Vec::new();
        while self.emitted < self.window.len() {
            out.push(self.smoothed(self.emitted));
            self.emitted += 1;
        }
        out
    }

    /// Frame `index` of the window, smoothed with its neighbours there.
    fn smoothed(&self, index: usize) -> (Flicks, Vec<f32>) {
        let (time, centre) = &self.window[index];
        if self.radius == 0 {
            return (*time, centre.clone());
        }
        let low = index.saturating_sub(self.radius);
        let high = (index + self.radius).min(self.window.len() - 1);
        let neighbours: Vec<(&[f32], f32)> = (low..=high)
            .filter(|&k| k != index && self.window[k].1.len() == centre.len())
            .map(|k| {
                (
                    self.window[k].1.as_slice(),
                    self.time_weights[k.abs_diff(index)],
                )
            })
            .collect();
        let scale = -1.0 / (2.0 * VALUE_SIGMA * VALUE_SIGMA);
        let values = centre
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let (mut sum, mut total) = (c, 1.0f32);
                for (other, weight) in &neighbours {
                    let d = other[i] - c;
                    let w = weight * (d * d * scale).exp();
                    sum += w * other[i];
                    total += w;
                }
                sum / total
            })
            .collect();
        (*time, values)
    }
}

/// Where a vision analyzer's per-frame results are kept: a binary file beside its cache
/// entry, with the same name and the extension `.bin`, which the entry's result names by
/// size and BLAKE3 hash so a stale or damaged file is never used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataFile {
    /// Its size in bytes.
    pub bytes: u64,
    /// The BLAKE3 hash of its contents, lowercase hex.
    pub blake3: String,
}

impl DataFile {
    /// The description of `data`.
    pub fn of(data: &[u8]) -> DataFile {
        DataFile {
            bytes: data.len() as u64,
            blake3: blake3::hash(data).to_hex().to_string(),
        }
    }

    /// Whether `data` is the file this describes.
    pub fn matches(&self, data: &[u8]) -> bool {
        data.len() as u64 == self.bytes && blake3::hash(data).to_hex().as_str() == self.blake3
    }
}

/// Why a data file could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the analysis data is damaged: {0}")]
pub struct DataError(pub String);

/// Writes the little-endian binary layout of the data files.
#[derive(Default)]
pub(crate) struct Writer(pub(crate) Vec<u8>);

impl Writer {
    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn i64(&mut self, value: i64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn f32(&mut self, value: f32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    /// A length, then the bytes.
    pub(crate) fn block(&mut self, bytes: &[u8]) {
        self.u32(bytes.len() as u32);
        self.bytes(bytes);
    }
}

/// Reads what [`Writer`] wrote, failing on a short or damaged file.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, at: 0 }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], DataError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|&end| end <= self.data.len())
            .ok_or_else(|| DataError("it ends too soon".into()))?;
        let bytes = &self.data[self.at..end];
        self.at = end;
        Ok(bytes)
    }

    pub(crate) fn u32(&mut self) -> Result<u32, DataError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn i64(&mut self) -> Result<i64, DataError> {
        let bytes = self.take(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(i64::from_le_bytes(array))
    }

    pub(crate) fn f32(&mut self) -> Result<f32, DataError> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub(crate) fn block(&mut self) -> Result<&'a [u8], DataError> {
        let length = self.u32()? as usize;
        self.take(length)
    }

    pub(crate) fn is_done(&self) -> bool {
        self.at == self.data.len()
    }
}

/// Index of the last of `times` (sorted) at or before `time`, or the first when `time` is
/// before them all; `None` when there are none.
pub fn frame_at(
    times: impl ExactSizeIterator<Item = Flicks> + Clone,
    time: Flicks,
) -> Option<usize> {
    let count = times.len();
    if count == 0 {
        return None;
    }
    let after = times.take_while(|&t| t <= time).count();
    Some(after.saturating_sub(1))
}

/// Takes frames at most `max_fps` per second: true for a frame at `pts` when it is the first
/// or at least a frame interval after the last one taken (with a small tolerance, so a 30 fps
/// source keeps every frame at 30).
#[derive(Clone, Debug)]
pub struct Sampler {
    interval: i64,
    last: Option<Flicks>,
}

impl Sampler {
    /// A sampler of at most `max_fps` frames per second (1 to 240).
    pub fn new(max_fps: u32) -> Sampler {
        let second = Flicks::from_seconds(1).map_or(705_600_000, Flicks::get);
        Sampler {
            interval: second / i64::from(max_fps.clamp(1, 240)),
            last: None,
        }
    }

    /// Whether to take the frame at `pts`.
    pub fn take(&mut self, pts: Flicks) -> bool {
        // A tenth of an interval early still counts, for rates such as 29.97.
        let due = self
            .last
            .is_none_or(|last| (pts - last).get() * 10 >= self.interval * 9);
        if due {
            self.last = Some(pts);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_round_trips_and_shrinks_mattes() {
        let mut matte = vec![0u8; 4000];
        matte[1000..3000].fill(255);
        for (i, v) in matte[2990..3010].iter_mut().enumerate() {
            *v = (i * 12) as u8;
        }
        let packed = pack(&matte);
        assert!(packed.len() < 120, "{} bytes", packed.len());
        assert_eq!(unpack(&packed, matte.len()).unwrap(), matte);
        for bytes in [
            vec![],
            vec![7],
            vec![1, 2],
            vec![3, 3],
            (0..=255u8).collect::<Vec<_>>(),
            vec![9; 1000],
            (0..600).map(|i| (i % 3) as u8).collect(),
        ] {
            assert_eq!(unpack(&pack(&bytes), bytes.len()).unwrap(), bytes);
        }
        assert_eq!(unpack(&[5, 1], 6), None, "a literal that ends too soon");
        assert_eq!(unpack(&pack(&[1, 1, 1]), 2), None, "longer than expected");
        assert_eq!(unpack(&[200], 75), None, "a run without its byte");
    }

    #[test]
    fn bytes_snap_near_the_ends() {
        assert_eq!(
            to_bytes(&[0.0, 0.003, 0.5, 0.996, 1.0, -1.0, 2.0]),
            [0, 0, 128, 255, 255, 0, 255]
        );
    }

    #[test]
    fn resizing_averages_when_shrinking_and_interpolates_when_growing() {
        // A 4x1 row of 0, 0, 1, 1 halves to 0, 1 and keeps its mean.
        let row = [0.0, 0.0, 1.0, 1.0];
        assert_eq!(resize(&row, (4, 1), (2, 1)), [0.0, 1.0]);
        let small = resize(&row, (4, 1), (3, 1));
        assert!((small.iter().sum::<f32>() / 3.0 - 0.5).abs() < 1e-6);
        // Growing 0, 1 to four samples interpolates between the two.
        let grown = resize(&[0.0, 1.0], (2, 1), (4, 1));
        assert_eq!(grown[0], 0.0);
        assert_eq!(grown[3], 1.0);
        assert!(grown[1] > 0.0 && grown[1] < grown[2] && grown[2] < 1.0);
        // Both axes.
        let square = vec![1.0; 16];
        assert!(
            resize(&square, (4, 4), (3, 2))
                .iter()
                .all(|v| (v - 1.0).abs() < 1e-6)
        );
        assert_eq!(stored_size(1920, 1080, 256), (256, 144));
        assert_eq!(stored_size(1080, 1920, 256), (144, 256));
    }

    #[test]
    fn affine_maps_invert() {
        let map = Affine {
            m: [2.0, 0.5, 10.0, -0.25, 1.5, 3.0],
        };
        let back = map.inverse().unwrap();
        let (x, y) = map.apply(7.0, -4.0);
        let (u, v) = back.apply(x, y);
        assert!((u - 7.0).abs() < 1e-9 && (v + 4.0).abs() < 1e-9);
        assert!(
            Affine {
                m: [1.0, 2.0, 0.0, 2.0, 4.0, 0.0]
            }
            .inverse()
            .is_none()
        );
    }

    #[test]
    fn rgb_samples_between_pixels_and_frames_convert() {
        let picture = Rgb::from_fn(
            2,
            1,
            |x, _| if x == 0 { [0, 0, 0] } else { [255, 255, 255] },
        );
        assert_eq!(picture.sample(0.5, 0.5), [0.0; 3]);
        assert!((picture.sample(1.0, 0.5)[0] - 0.5).abs() < 1e-6);
        assert_eq!(picture.sample(-5.0, 0.5), [0.0; 3], "black outside");
        // Limited-range BT.709 white, black and a saturated red.
        let frame = |y: u8, cb: u8, cr: u8, matrix: Matrix| VideoFrame {
            pts: Flicks::ZERO,
            duration: Flicks::new(1),
            width: 4,
            height: 2,
            format: PixelFormat::Yuv420p,
            planes: vec![
                fosscut_media::Plane {
                    stride: 4,
                    rows: 2,
                    data: vec![y; 8].into(),
                },
                fosscut_media::Plane {
                    stride: 2,
                    rows: 1,
                    data: vec![cb; 2].into(),
                },
                fosscut_media::Plane {
                    stride: 2,
                    rows: 1,
                    data: vec![cr; 2].into(),
                },
            ],
            color: fosscut_media::ColorInfo {
                matrix,
                range: Range::Limited,
                primaries: fosscut_media::Primaries::Bt709,
                transfer: fosscut_media::Transfer::Bt709,
            },
        };
        assert_eq!(
            Rgb::from_frame(&frame(235, 128, 128, Matrix::Bt709), 64).data[..3],
            [255; 3]
        );
        assert_eq!(
            Rgb::from_frame(&frame(16, 128, 128, Matrix::Bt709), 64).data[..3],
            [0; 3]
        );
        let red = Rgb::from_frame(&frame(63, 102, 240, Matrix::Bt709), 64);
        assert!(
            red.data[0] > 250 && red.data[1] < 5 && red.data[2] < 5,
            "{:?}",
            &red.data[..3]
        );
        // Averaged in blocks when larger than asked.
        let reduced = Rgb::from_frame(&frame(235, 128, 128, Matrix::Bt601), 2);
        assert_eq!((reduced.width, reduced.height), (2, 1));
    }

    #[test]
    fn smoothing_removes_flicker_but_keeps_motion() {
        let second = Flicks::from_seconds(1).unwrap();
        let at = |i: i64| Flicks::new(i * second.get() / 30);
        // A pixel that flickers between 0.45 and 0.55, and one that steps from 0 to 1.
        let frames: Vec<Vec<f32>> = (0..12)
            .map(|i| {
                vec![
                    if i % 2 == 0 { 0.45 } else { 0.55 },
                    if i < 6 { 0.0 } else { 1.0 },
                ]
            })
            .collect();
        let mut smoother = TemporalSmoother::new(1.0);
        let mut out = Vec::new();
        for (i, frame) in frames.iter().enumerate() {
            out.extend(smoother.push(at(i as i64), frame.clone()));
        }
        out.extend(smoother.finish());
        assert_eq!(out.len(), frames.len(), "every frame comes out once");
        for (i, (time, _)) in out.iter().enumerate() {
            assert_eq!(*time, at(i as i64), "in order");
        }
        for (_, values) in &out[3..9] {
            assert!(
                (values[0] - 0.5).abs() < 0.02,
                "flicker averages out: {}",
                values[0]
            );
        }
        assert!(
            out[5].1[1] < 0.02 && out[6].1[1] > 0.98,
            "the step stays sharp: {:?} {:?}",
            out[5].1,
            out[6].1
        );
        // Strength 0 changes nothing.
        let mut none = TemporalSmoother::new(0.0);
        assert_eq!(none.push(at(0), vec![0.3]), Some((at(0), vec![0.3])));
        assert!(none.finish().is_empty());
    }

    #[test]
    fn frames_are_found_and_sampled_by_time() {
        let times = [Flicks::new(10), Flicks::new(20), Flicks::new(30)];
        let find = |t| frame_at(times.iter().copied(), Flicks::new(t));
        assert_eq!(find(5), Some(0));
        assert_eq!(find(20), Some(1));
        assert_eq!(find(29), Some(1));
        assert_eq!(find(99), Some(2));
        assert_eq!(frame_at(std::iter::empty::<Flicks>(), Flicks::ZERO), None);
        let second = Flicks::from_seconds(1).unwrap().get();
        let mut thirty = Sampler::new(30);
        let taken = (0..60)
            .filter(|i| thirty.take(Flicks::new(i * second / 60)))
            .count();
        assert_eq!(taken, 30, "every other frame of 60 fps");
        let mut all = Sampler::new(30);
        let taken = (0..30)
            .filter(|i| all.take(Flicks::new(i * second * 1001 / 30_000)))
            .count();
        assert_eq!(taken, 30, "every frame of 29.97 fps");
        let data = b"some bytes";
        let file = DataFile::of(data);
        assert!(file.matches(data) && !file.matches(b"other bytes"));
    }

    #[test]
    fn readers_fail_on_short_data() {
        let mut writer = Writer::default();
        writer.u32(7);
        writer.i64(-3);
        writer.f32(0.5);
        writer.block(b"abc");
        let mut reader = Reader::new(&writer.0);
        assert_eq!(reader.u32().unwrap(), 7);
        assert_eq!(reader.i64().unwrap(), -3);
        assert_eq!(reader.f32().unwrap(), 0.5);
        assert_eq!(reader.block().unwrap(), b"abc");
        assert!(reader.is_done());
        assert!(reader.u32().is_err());
        assert!(Reader::new(&[9, 0, 0, 0, 1]).block().is_err());
    }
}
