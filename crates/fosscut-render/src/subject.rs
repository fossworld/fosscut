//! Effects that follow a subject (ADR-0033, items 3 and 4): `key.subject` removes the
//! background with a clip's cached matte, and `body.outline`, `body.glow`, `body.shadow` and
//! `body.sparkle` draw around a person from its cached pose and person mask.
//!
//! The renderer never runs a model. A frame source that has a clip's analyses answers
//! [`crate::FrameProvider::matte`] and [`crate::FrameProvider::pose`] for the source frame a
//! layer shows; [`crate::DecoderFrames`] does it from any [`SubjectData`] the engine gives
//! it. Without them the effects leave the picture unchanged, so a preview keeps playing while
//! the analysis runs; exports and frame grabs check first and fail instead.
//!
//! Mattes, masks and landmarks cover the source frame as it is stored, before its display
//! rotation, like the layer's own picture, which the effects work on before placement. What
//! the passes read is prepared here on the CPU, at the matte's own small size so it costs
//! little and gives the same result on every adapter:
//!
//! - a feathered matte, blurred by three box passes that approach a Gaussian (Wells,
//!   "Efficient synthesis of Gaussian filters by cascaded uniform filters", PAMI 1986);
//! - the signed distance from the person's edge, from exact Euclidean distance transforms
//!   (Felzenszwalb and Huttenlocher, "Distance transforms of sampled functions", Theory of
//!   Computing 2012), which bilinear sampling turns into a smooth outline at any size;
//! - sparkles, placed along the body's bones by integer hashes of their index and life
//!   cycle and drawn into a coverage image, never with sine-based noise, so every frame
//!   renders the same on every machine.
//!
//! The passes in `shaders/effects.wgsl` then combine them with the picture in linear light,
//! with premultiplied alpha: outlines and shadows go behind the person (over the background
//! but under the person's own pixels), glows and sparkles add light.

use std::collections::BTreeMap;
use std::sync::Arc;

use fosscut_model::ParamValue;
use fosscut_time::Flicks;

use crate::color::TransferFunction;
use crate::effects::{PlannedEffect, SLOTS, hex_rgb};
use crate::plan::SourceRef;
use crate::{FrameProvider, RenderError};

/// Which cached matte `key.subject` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MatteKind {
    /// The person mask of the pose model (the `matte` analysis with model `person`).
    Person,
    /// BiRefNet's subject matte (the `matte` analysis with model `subject`).
    Subject,
}

impl MatteKind {
    /// Its name, as the effect's `source` parameter and the analysis option take it.
    pub fn name(self) -> &'static str {
        match self {
            MatteKind::Person => "person",
            MatteKind::Subject => "subject",
        }
    }
}

/// A matte or mask over a whole source frame as stored: `width * height` bytes, rows top to
/// bottom, from 0 (background) to 255 (subject).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Matte {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The alpha.
    pub alpha: Vec<u8>,
}

impl Matte {
    /// The alpha as values from 0 to 1, or `None` when its size is wrong.
    fn values(&self) -> Option<Vec<f32>> {
        (self.width > 0
            && self.height > 0
            && self.alpha.len() == self.width as usize * self.height as usize)
            .then(|| self.alpha.iter().map(|&v| f32::from(v) / 255.0).collect())
    }
}

/// One body landmark, in MediaPipe's order of 33.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Landmark {
    /// Distance from the source frame's left edge, as a fraction of its width.
    pub x: f32,
    /// Distance from the top edge, as a fraction of its height.
    pub y: f32,
    /// How likely the point is visible, 0 to 1.
    pub visibility: f32,
}

/// A person's pose in one source frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    /// How likely a person is there, 0 to 1; below 0.5 the effects draw nothing.
    pub presence: f32,
    /// The 33 body landmarks.
    pub landmarks: Vec<Landmark>,
    /// The person mask, if the analysis kept one.
    pub mask: Option<Arc<Matte>>,
}

/// What a subject effect reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Need {
    /// A matte of this kind.
    Matte(MatteKind),
    /// The pose with its person mask.
    Pose,
}

/// What an effect of `effect_type` with `values` reads, or `None` for effects that need no
/// analysis.
pub fn need_of(effect_type: &str, values: &BTreeMap<String, ParamValue>) -> Option<Need> {
    match effect_type {
        "key.subject" => Some(Need::Matte(match values.get("source") {
            Some(ParamValue::Name(name)) if name == "subject" => MatteKind::Subject,
            _ => MatteKind::Person,
        })),
        "body.outline" | "body.glow" | "body.shadow" | "body.sparkle" => Some(Need::Pose),
        _ => None,
    }
}

/// What `effect` reads, if anything.
pub fn need(effect: &PlannedEffect) -> Option<Need> {
    need_of(&effect.effect_type, &effect.values)
}

/// Cached analyses of one asset, looked up by the source time a layer shows. The engine
/// implements it over the analysis cache; tests over drawn data.
pub trait SubjectData: Send + Sync {
    /// The matte of `kind` for the frame shown at `time`, if it was analyzed.
    fn matte(&self, kind: MatteKind, time: Flicks) -> Option<Arc<Matte>>;

    /// The pose at `time`, if it was analyzed.
    fn pose(&self, time: Flicks) -> Option<Arc<Pose>>;
}

/// What one subject effect pass needs besides the picture.
pub(crate) struct Prepared {
    /// The pass's uniform.
    pub(crate) params: [[f32; 4]; SLOTS],
    /// The field it samples, at the matte's size: red the subject's or person's coverage,
    /// green the signed distance from its edge in picture heights, blue a blurred coverage.
    pub(crate) field: Option<Field>,
    /// Its second texture.
    pub(crate) aux: Aux,
}

/// A small RGBA float texture.
pub(crate) struct Field {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) texels: Vec<[f32; 4]>,
}

/// The second texture of a subject pass.
pub(crate) enum Aux {
    None,
    /// The picture blurred with this deviation in pixels: `key.subject`'s blurred background.
    Blur(f32),
    /// Sparkle coverage, `width * height` bytes.
    Coverage {
        width: u32,
        height: u32,
        data: Vec<u8>,
    },
}

/// The sRGB colour `name` (`#RRGGBB`) in linear light.
fn linear(effect: &PlannedEffect, name: &str, fallback: &str) -> [f32; 3] {
    let hex = match effect.values.get(name) {
        Some(ParamValue::Name(hex)) => hex.as_str(),
        _ => fallback,
    };
    hex_rgb(hex).map(|c| TransferFunction::Srgb.decode(c))
}

fn choice<'a>(effect: &'a PlannedEffect, name: &str) -> &'a str {
    match effect.values.get(name) {
        Some(ParamValue::Name(value)) => value,
        _ => "",
    }
}

fn flag(effect: &PlannedEffect, name: &str) -> bool {
    matches!(effect.values.get(name), Some(ParamValue::Flag(true)))
}

/// Gets what `effect` needs for the frame of `source`, `picture` pixels large, from
/// `frames`; `None` when the analysis is not available or there is nothing to draw, which
/// leaves the picture unchanged.
pub(crate) fn prepare(
    effect: &PlannedEffect,
    source: &SourceRef,
    frames: &mut dyn FrameProvider,
    picture: (u32, u32),
) -> Result<Option<Prepared>, RenderError> {
    let Some(need) = need(effect) else {
        return Ok(None);
    };
    let mut params = [[0.0f32; 4]; SLOTS];
    let n = |name: &str| effect.number(name);
    match need {
        Need::Matte(kind) => {
            let Some(matte) = frames.matte(source, kind)? else {
                return Ok(None);
            };
            let Some(values) = matte.values() else {
                return Ok(None);
            };
            let (w, h) = (matte.width as usize, matte.height as usize);
            let feathered = blur(&values, w, h, n("feather") * h as f32);
            let background = match choice(effect, "background") {
                "color" => 1.0,
                "blur" => 2.0,
                _ => 0.0,
            };
            params[0] = [
                n("threshold"),
                if flag(effect, "invert") { 1.0 } else { 0.0 },
                background,
                0.0,
            ];
            let [r, g, b] = linear(effect, "color", "#000000");
            params[1] = [r, g, b, 0.0];
            Ok(Some(Prepared {
                params,
                field: Some(Field {
                    width: matte.width,
                    height: matte.height,
                    texels: feathered.iter().map(|&v| [v, 0.0, 0.0, 0.0]).collect(),
                }),
                aux: if background == 2.0 {
                    Aux::Blur(n("blur") * picture.1 as f32)
                } else {
                    Aux::None
                },
            }))
        }
        Need::Pose => {
            let Some(pose) = frames.pose(source)? else {
                return Ok(None);
            };
            if pose.presence < 0.5 {
                return Ok(None);
            }
            let [r, g, b] = linear(effect, "color", "#FFFFFF");
            params[1] = [r, g, b, 0.0];
            if effect.effect_type == "body.sparkle" {
                let sparkles = sparkles(&pose, n("amount"), n("size"), effect.time, picture);
                let (width, height) = coverage_size(picture);
                return Ok(Some(Prepared {
                    params,
                    field: None,
                    aux: Aux::Coverage {
                        width,
                        height,
                        data: rasterize_sparkles(&sparkles, (width, height)),
                    },
                }));
            }
            let Some((mask, values)) = pose.mask.as_ref().and_then(|m| Some((m, m.values()?)))
            else {
                return Ok(None);
            };
            let (w, h) = (mask.width as usize, mask.height as usize);
            let texels: Vec<[f32; 4]> = match effect.effect_type.as_str() {
                "body.outline" => {
                    params[0] = [n("width"), n("softness"), 0.0, 0.0];
                    let distance = signed_distance(&values, w, h);
                    values
                        .iter()
                        .zip(&distance)
                        .map(|(&m, &d)| [m, d / h as f32, 0.0, 0.0])
                        .collect()
                }
                "body.glow" => {
                    params[0] = [n("strength"), 0.0, 0.0, 0.0];
                    let blurred = blur(&values, w, h, n("size") * h as f32);
                    values
                        .iter()
                        .zip(&blurred)
                        .map(|(&m, &g)| [m, 0.0, g, 0.0])
                        .collect()
                }
                _ => {
                    // body.shadow: the offset is in picture heights on both axes.
                    let [dx, dy] = effect.pair("offset");
                    let aspect = picture.1 as f32 / picture.0.max(1) as f32;
                    params[0] = [dx * aspect, dy, n("opacity"), 0.0];
                    let blurred = blur(&values, w, h, n("blur") * h as f32);
                    values
                        .iter()
                        .zip(&blurred)
                        .map(|(&m, &g)| [m, 0.0, g, 0.0])
                        .collect()
                }
            };
            Ok(Some(Prepared {
                params,
                field: Some(Field {
                    width: mask.width,
                    height: mask.height,
                    texels,
                }),
                aux: Aux::None,
            }))
        }
    }
}

/// `values` (`width` by `height`) blurred with standard deviation `sigma` pixels: three box
/// blurs along each axis, whose widths are chosen so their variance matches the Gaussian's,
/// with edges repeated.
pub fn blur(values: &[f32], width: usize, height: usize, sigma: f32) -> Vec<f32> {
    let ideal = (12.0 * sigma.max(0.0) * sigma.max(0.0) / 3.0 + 1.0).sqrt();
    let radius = ((ideal - 1.0) / 2.0).round().max(0.0) as usize;
    if radius == 0 || width == 0 || height == 0 {
        return values.to_vec();
    }
    let mut out = values.to_vec();
    let mut line = Vec::new();
    for _ in 0..3 {
        for y in 0..height {
            line.clear();
            line.extend_from_slice(&out[y * width..(y + 1) * width]);
            box_line(&line, radius, |x, v| out[y * width + x] = v);
        }
        for x in 0..width {
            line.clear();
            line.extend((0..height).map(|y| out[y * width + x]));
            box_line(&line, radius, |y, v| out[y * width + x] = v);
        }
    }
    out
}

/// A running mean of `2 * radius + 1` samples along `line`, edges repeated.
fn box_line(line: &[f32], radius: usize, mut put: impl FnMut(usize, f32)) {
    let n = line.len();
    let at = |i: isize| line[i.clamp(0, n as isize - 1) as usize];
    let r = radius as isize;
    let scale = 1.0 / (2 * radius + 1) as f32;
    let mut sum: f32 = (-r..=r).map(at).sum();
    for i in 0..n as isize {
        put(i as usize, sum * scale);
        sum += at(i + r + 1) - at(i - r);
    }
}

/// The distance in pixels from each pixel to the edge of the region where `values` are at
/// least one half, negative inside it: exact Euclidean distances between pixel centres, less
/// half a pixel, so the edge falls between the last pixel inside and the first outside.
pub fn signed_distance(values: &[f32], width: usize, height: usize) -> Vec<f32> {
    let inside: Vec<bool> = values.iter().map(|&v| v >= 0.5).collect();
    let to_inside = squared_distance(&inside, width, height, true);
    let to_outside = squared_distance(&inside, width, height, false);
    inside
        .iter()
        .enumerate()
        .map(|(i, &is_in)| {
            if is_in {
                -(to_outside[i].sqrt() - 0.5)
            } else {
                to_inside[i].sqrt() - 0.5
            }
        })
        .collect()
}

/// Squared distance from each pixel to the nearest pixel where `mask` equals `target`; a
/// large value when there is none.
fn squared_distance(mask: &[bool], width: usize, height: usize, target: bool) -> Vec<f32> {
    const FAR: f32 = 1e12;
    let mut grid: Vec<f32> = mask
        .iter()
        .map(|&m| if m == target { 0.0 } else { FAR })
        .collect();
    let mut f = Vec::new();
    let mut d = Vec::new();
    for x in 0..width {
        f.clear();
        f.extend((0..height).map(|y| grid[y * width + x]));
        lower_envelope(&f, &mut d);
        for (y, &v) in d.iter().enumerate() {
            grid[y * width + x] = v;
        }
    }
    for y in 0..height {
        f.clear();
        f.extend_from_slice(&grid[y * width..(y + 1) * width]);
        lower_envelope(&f, &mut d);
        grid[y * width..(y + 1) * width].copy_from_slice(&d);
    }
    grid
}

/// The one-dimensional squared distance transform of `f`: for each `q`, the least
/// `(q - p)^2 + f[p]`, as the lower envelope of parabolas (Felzenszwalb and Huttenlocher).
fn lower_envelope(f: &[f32], out: &mut Vec<f32>) {
    let n = f.len();
    out.clear();
    out.resize(n, 0.0);
    if n == 0 {
        return;
    }
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f32; n + 1];
    let mut k = 0usize;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    let at = |q: usize| f[q] + (q * q) as f32;
    for q in 1..n {
        loop {
            let p = v[k];
            let s = (at(q) - at(p)) / (2.0 * q as f32 - 2.0 * p as f32);
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            if s <= z[k] {
                // k == 0: the new parabola is lower everywhere left of the old boundary.
                v[0] = q;
                z[1] = f32::INFINITY;
                break;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f32::INFINITY;
            break;
        }
    }
    let mut k = 0usize;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f32 - p as f32;
        *o = dq * dq + f[p];
    }
}

/// The bones sparkles gather along, as pairs of landmarks: shoulders, arms, the torso's sides,
/// hips, legs, and the head to each shoulder.
pub const BONES: [(usize, usize); 14] = [
    (11, 12),
    (11, 13),
    (13, 15),
    (12, 14),
    (14, 16),
    (11, 23),
    (12, 24),
    (23, 24),
    (23, 25),
    (25, 27),
    (24, 26),
    (26, 28),
    (0, 11),
    (0, 12),
];

/// One sparkle at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sparkle {
    /// Centre as fractions of the picture's width and height.
    pub x: f32,
    /// See `x`.
    pub y: f32,
    /// Radius in picture heights.
    pub radius: f32,
    /// Brightness, 0 to 1.
    pub brightness: f32,
}

/// An integer hash with good avalanche (Chris Wellons' `lowbias32`, public domain).
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// A number from 0 (inclusive) to 1 from two integers.
fn unit(a: u32, b: u32) -> f32 {
    (hash(hash(a).wrapping_add(b.wrapping_mul(0x9e37_79b9))) >> 8) as f32 / 16_777_216.0
}

/// Most sparkles at `amount` 1.
const MAX_SPARKLES: f32 = 80.0;

/// The sparkles of `pose` at `time` seconds since the clip's start on a picture `picture`
/// pixels large: `amount` (0 to 1) of up to 80, each living a cycle of 0.7 to 1.6 s at a
/// place along a visible bone chosen anew every cycle, brightest half-way through it.
/// Everything comes from integer hashes of the sparkle's index and cycle.
pub fn sparkles(
    pose: &Pose,
    amount: f32,
    size: f32,
    time: f64,
    picture: (u32, u32),
) -> Vec<Sparkle> {
    let visible = |i: usize| pose.landmarks.get(i).filter(|l| l.visibility >= 0.5);
    let bones: Vec<(Landmark, Landmark)> = BONES
        .iter()
        .filter_map(|&(a, b)| Some((*visible(a)?, *visible(b)?)))
        .collect();
    if bones.is_empty() || pose.presence < 0.5 {
        return Vec::new();
    }
    let aspect = picture.1 as f32 / picture.0.max(1) as f32;
    let count = (amount.clamp(0.0, 1.0) * MAX_SPARKLES).round() as u32;
    // How far sparkles stray from the bone, in picture heights.
    let spread = 0.03 + size.max(0.0);
    (0..count)
        .map(|j| {
            let period = 0.7 + 0.9 * f64::from(unit(j, 1));
            let cycles = time.max(0.0) / period + f64::from(unit(j, 2));
            let cycle = cycles.floor();
            let age = (cycles - cycle) as f32;
            let seed = hash(j).wrapping_add(hash(cycle as u32 ^ 0x5bd1_e995));
            let (a, b) = bones[(hash(seed) % bones.len() as u32) as usize];
            let t = unit(seed, 3);
            let jitter = (
                (unit(seed, 4) - 0.5) * 2.0 * spread,
                (unit(seed, 5) - 0.5) * 2.0 * spread,
            );
            let brightness = 4.0 * age * (1.0 - age);
            Sparkle {
                x: a.x + (b.x - a.x) * t + jitter.0 * aspect,
                y: a.y + (b.y - a.y) * t + jitter.1,
                radius: size * (0.5 + unit(seed, 6)) * (0.6 + 0.4 * brightness),
                brightness,
            }
        })
        .collect()
}

/// The size sparkles are drawn at for a `picture`-sized layer: its own, or smaller with the
/// same shape when its longer side passes 1280 pixels.
pub fn coverage_size(picture: (u32, u32)) -> (u32, u32) {
    let long = picture.0.max(picture.1).max(1);
    if long <= 1280 {
        return (picture.0.max(1), picture.1.max(1));
    }
    let scale = 1280.0 / long as f32;
    (
        ((picture.0 as f32 * scale).round() as u32).max(1),
        ((picture.1 as f32 * scale).round() as u32).max(1),
    )
}

/// `sparkles` drawn as four-pointed stars with a soft halo into `size` bytes of coverage,
/// overlapping ones combined like screens.
pub fn rasterize_sparkles(sparkles: &[Sparkle], size: (u32, u32)) -> Vec<u8> {
    let (w, h) = (size.0 as usize, size.1 as usize);
    let mut coverage = vec![0.0f32; w * h];
    for s in sparkles {
        let radius = s.radius * h as f32;
        if radius < 0.5 || s.brightness <= 0.0 {
            continue;
        }
        let (cx, cy) = (s.x * w as f32, s.y * h as f32);
        let x0 = (cx - radius).floor().max(0.0) as usize;
        let x1 = ((cx + radius).ceil().max(0.0) as usize).min(w);
        let y0 = (cy - radius).floor().max(0.0) as usize;
        let y1 = ((cy + radius).ceil().max(0.0) as usize).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = (x as f32 + 0.5 - cx) / radius;
                let dy = (y as f32 + 0.5 - cy) / radius;
                let d2 = dx * dx + dy * dy;
                if d2 >= 1.0 {
                    continue;
                }
                let core = (1.0 - d2 * 9.0).max(0.0);
                let across = (1.0 - dx.abs()).max(0.0) * (1.0 - dy.abs() * 10.0).max(0.0);
                let down = (1.0 - dy.abs()).max(0.0) * (1.0 - dx.abs() * 10.0).max(0.0);
                let halo = 0.35 * (1.0 - d2.sqrt()).powi(2);
                let value = (s.brightness * (core + 0.8 * (across + down) + halo)).min(1.0);
                let c = &mut coverage[y * w + x];
                *c = 1.0 - (1.0 - *c) * (1.0 - value);
            }
        }
    }
    coverage
        .iter()
        .map(|&c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_follow_the_effect_and_its_source() {
        let mut values = BTreeMap::new();
        assert_eq!(
            need_of("key.subject", &values),
            Some(Need::Matte(MatteKind::Person))
        );
        values.insert("source".to_string(), ParamValue::Name("subject".into()));
        assert_eq!(
            need_of("key.subject", &values),
            Some(Need::Matte(MatteKind::Subject))
        );
        for kind in ["body.outline", "body.glow", "body.shadow", "body.sparkle"] {
            assert_eq!(need_of(kind, &BTreeMap::new()), Some(Need::Pose));
        }
        assert_eq!(need_of("color.adjust", &values), None);
        assert_eq!(MatteKind::Subject.name(), "subject");
    }

    #[test]
    fn blurs_keep_the_mean_and_spread_edges() {
        let (w, h) = (40, 20);
        let step: Vec<f32> = (0..w * h)
            .map(|i| if i % w < 20 { 1.0 } else { 0.0 })
            .collect();
        let blurred = blur(&step, w, h, 3.0);
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean(&blurred) - mean(&step)).abs() < 0.01);
        let row = &blurred[10 * w..11 * w];
        assert!(
            (row[0] - 1.0).abs() < 1e-5 && row[39].abs() < 1e-5,
            "far from the edge"
        );
        assert!(
            (row[19] - 0.5).abs() < 0.15 && row[16] > row[19] && row[19] > row[22],
            "{row:?}"
        );
        assert_eq!(
            blur(&step, w, h, 0.2),
            step,
            "below a pixel nothing changes"
        );
    }

    #[test]
    fn distances_are_exact_and_signed() {
        let (w, h) = (9, 9);
        // One pixel inside, at the centre.
        let mut dot = vec![0.0f32; w * h];
        dot[4 * w + 4] = 1.0;
        let d = signed_distance(&dot, w, h);
        assert_eq!(d[4 * w + 4], -0.5);
        assert_eq!(d[4 * w + 5], 0.5);
        assert!((d[0] - (32.0f32.sqrt() - 0.5)).abs() < 1e-5);
        assert!((d[4 * w + 8] - 3.5).abs() < 1e-6);
        // A filled left half: distances grow away from its edge on both sides.
        let half: Vec<f32> = (0..w * h)
            .map(|i| if i % w < 4 { 1.0 } else { 0.0 })
            .collect();
        let d = signed_distance(&half, w, h);
        assert_eq!(d[2 * w], -3.5);
        assert_eq!(d[2 * w + 3], -0.5);
        assert_eq!(d[2 * w + 4], 0.5);
        assert_eq!(d[2 * w + 8], 4.5);
        // Nothing inside: everything is far.
        assert!(signed_distance(&[0.0; 4], 2, 2).iter().all(|&v| v > 1e5));
    }

    fn standing() -> Pose {
        let mut landmarks = vec![
            Landmark {
                x: 0.5,
                y: 0.5,
                visibility: 0.0,
            };
            33
        ];
        let mut put = |i: usize, x: f32, y: f32| {
            landmarks[i] = Landmark {
                x,
                y,
                visibility: 0.9,
            }
        };
        put(0, 0.5, 0.15);
        put(11, 0.42, 0.3);
        put(12, 0.58, 0.3);
        put(23, 0.45, 0.6);
        put(24, 0.55, 0.6);
        Pose {
            presence: 0.95,
            landmarks,
            mask: None,
        }
    }

    #[test]
    fn sparkles_are_deterministic_and_stay_by_the_bones() {
        let pose = standing();
        let a = sparkles(&pose, 0.5, 0.02, 1.25, (320, 180));
        assert_eq!(a.len(), 40);
        assert_eq!(
            a,
            sparkles(&pose, 0.5, 0.02, 1.25, (320, 180)),
            "the same every time"
        );
        assert_ne!(
            a,
            sparkles(&pose, 0.5, 0.02, 1.5, (320, 180)),
            "they move on"
        );
        for s in &a {
            assert!(
                (0.3..=0.7).contains(&s.x) && (0.08..=0.67).contains(&s.y),
                "{s:?}"
            );
            assert!((0.0..=1.0).contains(&s.brightness) && s.radius > 0.0);
        }
        assert!(sparkles(&pose, 0.0, 0.02, 1.0, (320, 180)).is_empty());
        let hidden = Pose {
            presence: 0.2,
            ..standing()
        };
        assert!(sparkles(&hidden, 1.0, 0.02, 1.0, (320, 180)).is_empty());
        // Drawn: a star at the centre, nothing far away.
        let star = Sparkle {
            x: 0.5,
            y: 0.5,
            radius: 0.2,
            brightness: 1.0,
        };
        let coverage = rasterize_sparkles(&[star], (100, 100));
        assert_eq!(coverage[50 * 100 + 50], 255);
        assert!(coverage[50 * 100 + 60] > 80, "along a ray");
        assert!(coverage[60 * 100 + 60] < 20, "between rays");
        assert_eq!(coverage[5 * 100 + 5], 0);
        assert_eq!(coverage_size((1920, 1080)), (1280, 720));
        assert_eq!(coverage_size((320, 180)), (320, 180));
    }
}
