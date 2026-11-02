//! Light effects (ADR-0011, items 1 and 4): `light.glow`, `light.sweep` and
//! `light.streaks`, and the blur and glow that clip animations plan. This module holds the
//! CPU side: how far light spreads past a layer, how the layer's picture is padded so it
//! can, how the parameters pack into each pass's uniform, and how padded pictures are
//! placed. The passes run in `gpu/light.rs`; the shaders are in `shaders/light.wgsl`.
//!
//! Formulas, all from public sources and written for FossCut:
//!
//! - **Glow** is the classic bloom: a bright pass (soft threshold on the brightest
//!   sRGB-encoded channel), Gaussian blurs at two radii (a wide halo and a core a quarter
//!   its size, as multi-scale bloom does), and the light added to the picture. Over
//!   transparent parts the light carries its own coverage (its brightest channel), so it
//!   shows on whatever is below the layer.
//! - **Streaks** are a directional maximum with an exponential fall-off, in two passes in
//!   the manner of Kawase's streak filter (GDC 2003): a fine pass and a coarse pass whose
//!   weights multiply, since `exp(-a) * exp(-b) = exp(-(a + b))`, cover the whole length
//!   with few samples.
//! - **Sweep** adds a band of light whose profile is a smoothstep, moving along a direction
//!   from just outside the layer's picture to just outside its other side.
//!
//! Lengths are fractions of the frame (the canvas) measured at the layer's own size, so a
//! glow looks the same on a title, a logo or a video, and grows and shrinks with the
//! layer's scale. Nothing here is random, so every adapter renders the same light.

use fosscut_model::{AnimationDelta, ParamValue};

use crate::effects::{PlannedEffect, SLOTS};
use crate::geometry::Vertex;

/// The light passes that need more than one per-pixel pass.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Light {
    /// `light.glow`: bright pass, two blurs, then the light added.
    Glow,
    /// `light.streaks`: bright pass, a fine and a coarse directional pass, then the light
    /// added.
    Streaks,
}

/// Entry points of `shaders/light.wgsl`, for building pipelines.
pub const ENTRIES: [&str; 5] = [
    "fs_light_pad",
    "fs_light_bright",
    "fs_light_streak",
    "fs_light_add",
    "fs_light_sweep",
];

/// The planned type of an animation's blur (`blur_in`, `blur_out`): a Gaussian blur whose
/// picture is padded so it spreads past the edges. Not a registry type: only plans make it.
pub const ANIMATION_BLUR: &str = "animation.blur";

/// Value a renderer adds to a light effect: the layer's pixels per canvas width and
/// height, at the layer's own size.
pub const CANVAS_VALUE: &str = "_canvas";
/// Value a renderer adds to a light effect after padding: the padding's width and height
/// in pixels on each side.
pub const INSET_VALUE: &str = "_inset";

/// Samples on each side of a streak pass; two passes reach `STREAK_TAPS` squared samples.
pub const STREAK_TAPS: u32 = 16;
/// Exponential fall-off of streaks over their length: they fade to about 2% at it.
const STREAK_FALLOFF: f32 = 4.0;
/// Streaks are drawn on until their light falls below this, so they never end in a cut.
const STREAK_FAINTEST: f64 = 1e-3;
/// Gaussian reach in standard deviations that padding leaves room for.
const GLOW_REACH: f64 = 3.5;
/// The wide halo of a glow is this many times its core.
const GLOW_CORE: f32 = 4.0;
/// A padded picture holds at most this many pixels (4096 by 2048), so huge glows on large
/// pictures stay within memory; a picture already larger is not padded.
const MAX_PADDED_PIXELS: f64 = 8_388_608.0;

fn number(effect: &PlannedEffect, name: &str) -> f64 {
    effect
        .values
        .get(name)
        .and_then(ParamValue::number)
        .unwrap_or(0.0)
}

fn pair(effect: &PlannedEffect, name: &str) -> Option<[f64; 2]> {
    effect.values.get(name).and_then(ParamValue::pair)
}

/// The layer's pixels per canvas width and height an effect measures with: what
/// [`annotate`] recorded, or the picture's own size, which is right for an adjustment
/// layer drawn over the whole canvas.
fn canvas_pixels(effect: &PlannedEffect, size: (u32, u32)) -> [f64; 2] {
    pair(effect, CANVAS_VALUE).unwrap_or([f64::from(size.0), f64::from(size.1)])
}

/// The padding an effect's light was given, in pixels on each side.
fn inset(effect: &PlannedEffect) -> [f64; 2] {
    pair(effect, INSET_VALUE).unwrap_or([0.0, 0.0])
}

/// How many of its own pixels one canvas pixel spans for a picture `source` pixels large,
/// turned `rotation` degrees and fitted inside a `canvas` (before the layer's scale).
pub fn fitted_scale(canvas: (u32, u32), source: (u32, u32), rotation: u16) -> f64 {
    let (mut w, mut h) = (f64::from(source.0.max(1)), f64::from(source.1.max(1)));
    if rotation == 90 || rotation == 270 {
        std::mem::swap(&mut w, &mut h);
    }
    let fit = (f64::from(canvas.0) / w).min(f64::from(canvas.1) / h);
    1.0 / fit.max(1e-9)
}

/// True for the light effects (and animation blur) this module measures.
fn measured(effect_type: &str) -> bool {
    effect_type.starts_with("light.") || effect_type == ANIMATION_BLUR
}

/// `effects` with every light effect told how many of the layer's pixels span the canvas,
/// `per_canvas_pixel` of its pixels per canvas pixel of a `canvas`, and animation blurs
/// turned from fractions of the canvas into fractions of the picture's height (`size`), as
/// `blur.gaussian` measures. `None` when no effect needs either.
pub fn annotate(
    effects: &[PlannedEffect],
    canvas: (u32, u32),
    per_canvas_pixel: f64,
    size: (u32, u32),
) -> Option<Vec<PlannedEffect>> {
    if !effects.iter().any(|e| measured(&e.effect_type)) {
        return None;
    }
    let unit = [
        f64::from(canvas.0) * per_canvas_pixel,
        f64::from(canvas.1) * per_canvas_pixel,
    ];
    Some(
        effects
            .iter()
            .map(|e| {
                let mut e = e.clone();
                if e.effect_type == ANIMATION_BLUR {
                    if let Some(ParamValue::Number(radius)) = e.values.get_mut("radius") {
                        *radius *= unit[1] / f64::from(size.1.max(1));
                    }
                } else if measured(&e.effect_type) {
                    e.values
                        .insert(CANVAS_VALUE.to_string(), ParamValue::Pair(unit));
                }
                e
            })
            .collect(),
    )
}

/// How far an effect's light spreads past the picture, in pixels left and right and above
/// and below, for a picture `size` pixels large.
pub fn reach(effect: &PlannedEffect, size: (u32, u32)) -> [f64; 2] {
    let [unit_w, unit_h] = canvas_pixels(effect, size);
    match effect.effect_type.as_str() {
        "light.glow" if number(effect, "intensity") > 0.0 => {
            let r = GLOW_REACH * number(effect, "radius") * unit_h;
            [r, r]
        }
        "light.streaks" if number(effect, "intensity") > 0.0 => {
            let length = number(effect, "length") * unit_w * streak_reach(effect);
            let (sin, cos) = number(effect, "angle").to_radians().sin_cos();
            [length * cos.abs(), length * sin.abs()]
        }
        ANIMATION_BLUR => {
            // A fraction of the picture's height, as blur.gaussian measures.
            let r = 3.0 * number(effect, "radius") * f64::from(size.1);
            [r, r]
        }
        _ => [0.0, 0.0],
    }
}

/// Where a layer's picture is padded so light spreads past its edges.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Padding {
    /// The first effect that runs on the padded picture; those before it run on the
    /// picture as it is.
    pub at: usize,
    /// Pixels added left and right.
    pub x: u32,
    /// Pixels added above and below.
    pub y: u32,
    /// The picture's size before padding.
    pub size: (u32, u32),
}

impl Padding {
    /// The padded picture's size.
    pub fn padded(&self) -> (u32, u32) {
        (self.size.0 + 2 * self.x, self.size.1 + 2 * self.y)
    }

    /// The part of the padded picture shown for `kept` (`[left, top, right, bottom]` of the
    /// picture as displayed, as [`fosscut_model::Crop::kept`] gives it): each edge that is
    /// not cropped moves out by the padding, so light shows past it, while a cropped edge
    /// stays where it is. `turned` is true when the picture is displayed a quarter turn
    /// from how it is stored.
    pub fn kept(&self, kept: [f64; 4], turned: bool) -> [f64; 4] {
        let (mut fx, mut fy) = (
            f64::from(self.x) / f64::from(self.size.0.max(1)),
            f64::from(self.y) / f64::from(self.size.1.max(1)),
        );
        if turned {
            std::mem::swap(&mut fx, &mut fy);
        }
        let [left, top, right, bottom] = kept;
        let out = |edge: f64, at: f64, by: f64| {
            if (edge - at).abs() < 1e-9 {
                at + by
            } else {
                edge
            }
        };
        [
            out(left, 0.0, -fx),
            out(top, 0.0, -fy),
            out(right, 1.0, fx),
            out(bottom, 1.0, fy),
        ]
    }

    /// A texture coordinate of the unpadded picture (outside 0 to 1 where the padding
    /// shows) moved into the padded picture.
    pub fn uv(&self, uv: [f32; 2]) -> [f32; 2] {
        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
        let (pw, ph) = (w + 2.0 * self.x as f32, h + 2.0 * self.y as f32);
        [
            (uv[0] * w + self.x as f32) / pw,
            (uv[1] * h + self.y as f32) / ph,
        ]
    }

    /// Corners computed for the unpadded picture with their texture coordinates moved into
    /// the padded picture ([`Padding::uv`]).
    pub fn remap(&self, vertices: [Vertex; 4]) -> [Vertex; 4] {
        vertices.map(|mut v| {
            v.uv = self.uv(v.uv);
            v
        })
    }
}

/// Where and how much to pad a picture `size` pixels large for `effects`, or `None` when
/// no effect spreads past its edges.
///
/// The padding goes right before the first spreading effect that comes after every effect
/// that reads the clip's cached analyses or neighbouring frames (`key.subject`, `body.*`,
/// `motion.blur`, `detail.denoise`, `color.deflicker`), since those line up with the
/// picture as it is; a spreading effect before such an effect stays within the picture.
/// It leaves room for every spreading effect from there on, as far as the padded picture
/// stays within `limit` pixels on each side and 4096 by 2048 pixels in all; beyond that the
/// light is cut where the padding ends.
pub fn padding(effects: &[PlannedEffect], size: (u32, u32), limit: u32) -> Option<Padding> {
    use crate::effects::{Pass, pass_for};
    let barrier = effects
        .iter()
        .rposition(|e| {
            matches!(
                pass_for(&e.effect_type),
                Some(Pass::Subject(_) | Pass::Temporal(_))
            )
        })
        .map_or(0, |i| i + 1);
    let spreads = |e: &PlannedEffect| reach(e, size).iter().any(|r| *r >= 0.5);
    let at = barrier + effects[barrier..].iter().position(spreads)?;
    let (mut x, mut y) = (0.0f64, 0.0f64);
    for effect in &effects[at..] {
        let [rx, ry] = reach(effect, size);
        x += rx;
        y += ry;
    }
    let room = |extent: u32| (f64::from(limit.saturating_sub(extent)) / 2.0).max(0.0);
    let (x, y) = (x.min(room(size.0)), y.min(room(size.1)));
    // Both sides shrink by the factor k that brings (w + 2kx)(h + 2ky) down to the budget.
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    let budget = MAX_PADDED_PIXELS.max(w * h);
    let k = if (w + 2.0 * x) * (h + 2.0 * y) <= budget {
        1.0
    } else {
        let (a, b, c) = (4.0 * x * y, 2.0 * (w * y + h * x), w * h - budget);
        if a > 0.0 {
            (-b + (b * b - 4.0 * a * c).sqrt()) / (2.0 * a)
        } else if b > 0.0 {
            -c / b
        } else {
            0.0
        }
    }
    .clamp(0.0, 1.0);
    // Whole pixels, ignoring rounding noise in the sums.
    let whole = |v: f64| (v * k - 1e-6).ceil().max(0.0) as u32;
    let (x, y) = (whole(x), whole(y));
    (x > 0 || y > 0).then_some(Padding { at, x, y, size })
}

/// The effects from `padding.at` on, told where the original picture lies in the padded
/// one, with blur radii, which are fractions of the picture's height, rescaled to the
/// padded height so they blur as much as before.
pub fn after_padding(effects: &[PlannedEffect], padding: &Padding) -> Vec<PlannedEffect> {
    let ratio = f64::from(padding.size.1) / f64::from(padding.padded().1.max(1));
    effects[padding.at.min(effects.len())..]
        .iter()
        .map(|e| {
            let mut e = e.clone();
            match e.effect_type.as_str() {
                "blur.gaussian" | "detail.sharpen" | ANIMATION_BLUR => {
                    if let Some(ParamValue::Number(radius)) = e.values.get_mut("radius") {
                        *radius *= ratio;
                    }
                }
                kind if measured(kind) => {
                    e.values.insert(
                        INSET_VALUE.to_string(),
                        ParamValue::Pair([f64::from(padding.x), f64::from(padding.y)]),
                    );
                }
                _ => {}
            }
            e
        })
        .collect()
}

/// The uniform of `fs_light_pad`: copies a `size` picture into the middle of the padded
/// one.
pub fn pack_pad(padding: &Padding) -> [[f32; 4]; SLOTS] {
    let mut p = [[0.0f32; 4]; SLOTS];
    p[0] = [
        padding.x as f32,
        padding.y as f32,
        padding.size.0 as f32,
        padding.size.1 as f32,
    ];
    p
}

/// A `#RRGGBB` parameter as linear-light RGB; white when missing.
fn linear(effect: &PlannedEffect, name: &str) -> [f32; 3] {
    match effect.values.get(name) {
        Some(ParamValue::Name(hex)) => crate::effects::hex_rgb(hex).map(|c| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }),
        _ => [1.0; 3],
    }
}

/// The uniform of `fs_light_bright`: the threshold and the tint.
pub fn pack_bright(effect: &PlannedEffect) -> [[f32; 4]; SLOTS] {
    let mut p = [[0.0f32; 4]; SLOTS];
    let [r, g, b] = linear(effect, "color");
    p[0] = [
        number(effect, "threshold").clamp(0.0, 1.0) as f32,
        number(effect, "tint").clamp(0.0, 1.0) as f32,
        0.0,
        0.0,
    ];
    p[1] = [r, g, b, 0.0];
    p
}

/// The two blur radii of a glow in pixels, wide and core, on a picture `size` large.
pub fn glow_sigmas(effect: &PlannedEffect, size: (u32, u32)) -> (f32, f32) {
    let wide = (number(effect, "radius") * canvas_pixels(effect, size)[1]) as f32;
    (wide, wide / GLOW_CORE)
}

/// The uniform of `fs_light_add`: weights of its two lights, the intensity, and whether
/// only the light shows.
pub fn pack_add(effect: &PlannedEffect, weights: [f32; 2]) -> [[f32; 4]; SLOTS] {
    let mut p = [[0.0f32; 4]; SLOTS];
    let only = matches!(effect.values.get("glow_only"), Some(ParamValue::Flag(true)));
    p[0] = [
        weights[0],
        weights[1],
        number(effect, "intensity").max(0.0) as f32,
        if only { 1.0 } else { 0.0 },
    ];
    p
}

/// How far streaks are drawn, as a multiple of their length: until their light falls below
/// [`STREAK_FAINTEST`], so brighter streaks reach further (1.7 lengths at intensity 1, 2.2 at
/// 8), at most three lengths.
fn streak_reach(effect: &PlannedEffect) -> f64 {
    let brightest = number(effect, "intensity").max(0.0) / STREAK_FAINTEST;
    (brightest.max(std::f64::consts::E).ln() / f64::from(STREAK_FALLOFF)).min(3.0)
}

/// The two passes of `light.streaks` on a picture `size` pixels large with `mips` levels:
/// for each, the uniform of `fs_light_streak`. The fine pass samples a mip level as coarse
/// as its step; the coarse pass samples the fine pass's output, which has one level.
pub fn pack_streaks(effect: &PlannedEffect, size: (u32, u32), mips: u32) -> [[[f32; 4]; SLOTS]; 2] {
    let length = number(effect, "length").max(0.0) * canvas_pixels(effect, size)[0];
    let (sin, cos) = number(effect, "angle").to_radians().sin_cos();
    let taps = STREAK_TAPS as f64;
    let fine = length * streak_reach(effect) / (taps * (taps + 1.0));
    let coarse = fine * (taps + 1.0);
    let pass = |step: f64, lod: f32| {
        let mut p = [[0.0f32; 4]; SLOTS];
        p[0] = [
            (cos * step / f64::from(size.0.max(1))) as f32,
            (sin * step / f64::from(size.1.max(1))) as f32,
            lod,
            STREAK_TAPS as f32,
        ];
        // The fall-off per pixel, so the two passes' weights multiply into one curve.
        p[1] = [
            (f64::from(STREAK_FALLOFF) * step / length.max(1e-6)) as f32,
            0.0,
            0.0,
            0.0,
        ];
        p
    };
    let lod = (fine.max(1.0).log2() as f32).clamp(0.0, mips.saturating_sub(1) as f32);
    [pass(fine, lod), pass(coarse, 0.0)]
}

/// The uniform of `fs_light_sweep` on a picture `size` pixels large.
pub fn pack_sweep(effect: &PlannedEffect, size: (u32, u32)) -> [[f32; 4]; SLOTS] {
    let mut p = [[0.0f32; 4]; SLOTS];
    let [r, g, b] = linear(effect, "color");
    p[0] = [
        number(effect, "position") as f32,
        number(effect, "angle").to_radians() as f32,
        number(effect, "width").clamp(0.0, 1.0) as f32,
        number(effect, "softness").clamp(0.0, 1.0) as f32,
    ];
    p[1] = [number(effect, "intensity").max(0.0) as f32, r, g, b];
    let [x, y] = inset(effect);
    p[2] = [
        x as f32,
        y as f32,
        (f64::from(size.0) - 2.0 * x).max(1.0) as f32,
        (f64::from(size.1) - 2.0 * y).max(1.0) as f32,
    ];
    p
}

/// The effects a clip's animations add at a moment (ADR-0011, clip animations): an
/// [`ANIMATION_BLUR`] for a blur and a `light.glow` for a glow, after the clip's own
/// effects. Their sizes are fractions of the canvas, like the light effects'.
pub fn animation_effects(delta: &AnimationDelta, seed: u32, time: f64) -> Vec<PlannedEffect> {
    let mut planned = Vec::new();
    let effect = |kind: &str, values: Vec<(&str, ParamValue)>| PlannedEffect {
        effect_type: kind.to_string(),
        values: values
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        seed,
        time,
    };
    if delta.glow > 1e-4
        && let Some(spec) = fosscut_model::effect_type(fosscut_model::light::GLOW)
    {
        let mut values: Vec<(&str, ParamValue)> = spec
            .params
            .iter()
            .map(|p| (p.name, p.default.value()))
            .collect();
        for (name, value) in &mut values {
            match *name {
                "radius" => *value = ParamValue::Number(0.03),
                "intensity" => *value = ParamValue::Number(2.5 * delta.glow),
                _ => {}
            }
        }
        planned.push(effect(fosscut_model::light::GLOW, values));
    }
    if delta.blur > 1e-5 {
        planned.push(effect(
            ANIMATION_BLUR,
            vec![("radius", ParamValue::Number(delta.blur))],
        ));
    }
    planned
}

/// Turns the angles of light effects on a picture stored `rotation` degrees from how it
/// is displayed, so sweeps and streaks run the way they look on the canvas.
pub fn orient(effects: &mut [PlannedEffect], rotation: u16) {
    if rotation == 0 {
        return;
    }
    for effect in effects
        .iter_mut()
        .filter(|e| matches!(e.effect_type.as_str(), "light.sweep" | "light.streaks"))
    {
        if let Some(ParamValue::Number(angle)) = effect.values.get_mut("angle") {
            // Displayed clockwise by `rotation`, so stored directions turn back by it.
            *angle -= f64::from(rotation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(kind: &str, values: &[(&str, ParamValue)]) -> PlannedEffect {
        let mut all: std::collections::BTreeMap<String, ParamValue> =
            fosscut_model::effect_type(kind)
                .map(|spec| {
                    spec.params
                        .iter()
                        .map(|p| (p.name.to_string(), p.default.value()))
                        .collect()
                })
                .unwrap_or_default();
        for (k, v) in values {
            all.insert(k.to_string(), v.clone());
        }
        PlannedEffect {
            effect_type: kind.into(),
            values: all,
            seed: 0,
            time: 0.0,
        }
    }

    #[test]
    fn glows_and_streaks_reach_past_the_picture_by_their_size() {
        let n = ParamValue::Number;
        let glow = planned("light.glow", &[("radius", n(0.1))]);
        // Without a recorded canvas, the picture is the canvas.
        assert_eq!(reach(&glow, (200, 100)), [35.0, 35.0]);
        let annotated =
            annotate(std::slice::from_ref(&glow), (1920, 1080), 0.5, (200, 100)).unwrap();
        let r = reach(&annotated[0], (200, 100));
        assert!(
            (r[0] - 3.5 * 0.1 * 540.0).abs() < 1e-9,
            "measured on the canvas: {r:?}"
        );
        let dark = planned("light.glow", &[("intensity", n(0.0))]);
        assert_eq!(reach(&dark, (200, 100)), [0.0, 0.0]);
        // Streaks are drawn until their light is a thousandth: 1.73 lengths at intensity 1.
        let level = planned("light.streaks", &[("length", n(0.5))]);
        let [x, y] = reach(&level, (400, 100));
        let far = 200.0 * 1000f64.ln() / 4.0;
        assert!(
            (x - far).abs() < 1e-9 && y.abs() < 1e-9,
            "horizontal streaks: {x}"
        );
        let steep = planned("light.streaks", &[("angle", n(90.0)), ("length", n(0.5))]);
        let [x, y] = reach(&steep, (400, 100));
        assert!(x.abs() < 1e-9 && (y - far).abs() < 1e-9);
        let bright = planned(
            "light.streaks",
            &[("length", n(0.5)), ("intensity", n(8.0))],
        );
        assert!(
            reach(&bright, (400, 100))[0] > x.max(far) * 1.25,
            "brighter reach further"
        );
        assert_eq!(reach(&planned("light.sweep", &[]), (400, 100)), [0.0, 0.0]);
        assert_eq!(reach(&planned("color.adjust", &[]), (400, 100)), [0.0, 0.0]);
        assert!(annotate(&[planned("color.adjust", &[])], (1920, 1080), 1.0, (9, 9)).is_none());
        // An animation blur becomes a fraction of the picture's height.
        let blur = PlannedEffect {
            effect_type: ANIMATION_BLUR.into(),
            values: [("radius".to_string(), n(0.01))].into(),
            seed: 0,
            time: 0.0,
        };
        let turned = annotate(&[blur], (1920, 1080), 0.5, (400, 135)).unwrap();
        assert_eq!(turned[0].values["radius"], n(0.01 * 540.0 / 135.0));
        assert!(!turned[0].values.contains_key(CANVAS_VALUE));
    }

    #[test]
    fn padding_goes_after_effects_that_read_analyses_and_is_capped() {
        let n = ParamValue::Number;
        let grade = planned("color.adjust", &[]);
        let glow = planned("light.glow", &[("radius", n(0.02))]);
        let subject = planned("key.subject", &[]);
        let effects = vec![grade.clone(), glow.clone()];
        let pad = padding(&effects, (100, 100), 8192).unwrap();
        assert_eq!((pad.at, pad.x, pad.y), (1, 7, 7));
        assert_eq!(pad.padded(), (114, 114));
        // A glow before a subject key stays inside; one after it pads.
        let effects = vec![glow.clone(), subject.clone(), grade.clone()];
        assert_eq!(padding(&effects, (100, 100), 8192), None);
        let effects = vec![glow.clone(), subject, glow.clone()];
        assert_eq!(padding(&effects, (100, 100), 8192).unwrap().at, 2);
        // Spreading effects add up; padding stays within the texture limit and the pixel
        // budget.
        let effects = vec![glow.clone(), glow.clone()];
        assert_eq!(padding(&effects, (100, 100), 8192).unwrap().x, 14);
        let huge = planned("light.glow", &[("radius", n(0.5))]);
        let pad = padding(std::slice::from_ref(&huge), (400, 100), 8192).unwrap();
        assert_eq!((pad.x, pad.y), (175, 175));
        let pad = padding(std::slice::from_ref(&huge), (400, 100), 500).unwrap();
        assert_eq!((pad.x, pad.y), (50, 175));
        let big = padding(std::slice::from_ref(&huge), (4000, 2000), 16384).unwrap();
        let (w, h) = big.padded();
        assert!(f64::from(w) * f64::from(h) <= MAX_PADDED_PIXELS + 3.0 * 6000.0);
        assert!(big.x == big.y && big.x > 20 && big.x < 100, "{big:?}");
        let streak = planned("light.streaks", &[("length", n(1.0))]);
        let wide = padding(std::slice::from_ref(&streak), (4000, 2000), 16384).unwrap();
        assert_eq!(wide.y, 0, "level streaks pad only the sides");
        assert!(wide.x > 0 && f64::from(wide.padded().0) * 2000.0 <= MAX_PADDED_PIXELS + 4000.0);
        assert_eq!(padding(&[grade], (100, 100), 8192), None);
        let blur = PlannedEffect {
            effect_type: ANIMATION_BLUR.into(),
            values: [("radius".to_string(), n(0.01))].into(),
            seed: 0,
            time: 0.0,
        };
        assert_eq!(padding(&[blur], (100, 100), 8192).unwrap().x, 3);
    }

    #[test]
    fn padded_pictures_keep_their_place() {
        let pad = Padding {
            at: 0,
            x: 10,
            y: 5,
            size: (100, 50),
        };
        let close = |a: [f64; 4], b: [f64; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        // Uncropped edges move out by the padding; cropped ones stay.
        assert!(close(
            pad.kept([0.0, 0.0, 1.0, 1.0], false),
            [-0.1, -0.1, 1.1, 1.1]
        ));
        assert!(close(
            pad.kept([0.2, 0.0, 1.0, 0.5], false),
            [0.2, -0.1, 1.1, 0.5]
        ));
        let turned = Padding { y: 20, ..pad };
        assert!(close(
            turned.kept([0.0, 0.0, 1.0, 1.0], true),
            [-0.4, -0.1, 1.4, 1.1]
        ));
        // Texture coordinates of the picture's corners land on its pixels in the padded one.
        let corner = |uv: [f32; 2]| Vertex {
            position: [0.0; 2],
            local: [0.0; 2],
            uv,
        };
        let moved = pad.remap([
            corner([0.0, 0.0]),
            corner([1.0, 0.0]),
            corner([-0.1, -0.1]),
            corner([1.1, 1.1]),
        ]);
        assert_eq!(moved[0].uv, [10.0 / 120.0, 5.0 / 60.0]);
        assert_eq!(moved[1].uv, [110.0 / 120.0, 5.0 / 60.0]);
        assert!(moved[2].uv[0].abs() < 1e-6 && moved[2].uv[1].abs() < 1e-6);
        assert!((moved[3].uv[0] - 1.0).abs() < 1e-6 && (moved[3].uv[1] - 1.0).abs() < 1e-6);
        // Blurs after the padding keep their size on the picture; light effects learn the
        // inset.
        let effects = vec![
            planned("light.glow", &[]),
            planned("blur.gaussian", &[("radius", ParamValue::Number(0.06))]),
            planned("light.sweep", &[]),
        ];
        let after = after_padding(&effects, &pad);
        assert_eq!(after[0].values[INSET_VALUE], ParamValue::Pair([10.0, 5.0]));
        assert_eq!(
            after[1].values["radius"],
            ParamValue::Number(0.06 * 50.0 / 60.0)
        );
        assert_eq!(
            pack_sweep(&after[2], (120, 60))[2],
            [10.0, 5.0, 100.0, 50.0]
        );
    }

    #[test]
    fn parameters_pack_where_the_light_shaders_read_them() {
        let n = ParamValue::Number;
        let glow = planned(
            "light.glow",
            &[
                ("threshold", n(0.6)),
                ("tint", n(1.0)),
                ("color", ParamValue::Name("#FF8000".into())),
                ("glow_only", ParamValue::Flag(true)),
                ("intensity", n(2.0)),
            ],
        );
        let bright = pack_bright(&glow);
        assert_eq!(bright[0][..2], [0.6, 1.0]);
        assert_eq!(bright[1][0], 1.0, "orange is full red");
        assert!((bright[1][1] - 0.2158).abs() < 1e-3, "and decoded green");
        assert_eq!(pack_add(&glow, [0.5, 0.5])[0], [0.5, 0.5, 2.0, 1.0]);
        let annotated =
            annotate(std::slice::from_ref(&glow), (1920, 1080), 1.0, (300, 100)).unwrap();
        let (wide, core) = glow_sigmas(&annotated[0], (300, 100));
        assert!((wide - 0.04 * 1080.0).abs() < 1e-3 && (core - wide / 4.0).abs() < 1e-6);
        let streaks = planned("light.streaks", &[("length", n(0.5))]);
        let [fine, coarse] = pack_streaks(&streaks, (312, 100), 9);
        // 156 pixels drawn 1.73 times as far, over 16 x 17 fine steps, then 17 times those.
        let step = 156.0 * 1000f64.ln() / 4.0 / (16.0 * 17.0);
        assert!((f64::from(fine[0][0]) - step / 312.0).abs() < 1e-6 && fine[0][1].abs() < 1e-6);
        assert!((coarse[0][0] - 17.0 * fine[0][0]).abs() < 1e-6);
        assert_eq!((fine[0][2], fine[0][3], coarse[0][2]), (0.0, 16.0, 0.0));
        // Weights multiply into one fall-off along the whole length, 4 per length.
        assert!((fine[1][0] * 17.0 - coarse[1][0]).abs() < 1e-6);
        assert!((f64::from(fine[1][0]) - 4.0 * step / 156.0).abs() < 1e-6);
        let sweep = planned("light.sweep", &[("angle", n(90.0)), ("position", n(0.25))]);
        let p = pack_sweep(&sweep, (200, 100));
        assert!((p[0][1] - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(p[0][0], 0.25);
        assert_eq!(p[2], [0.0, 0.0, 200.0, 100.0]);
        assert_eq!(
            pack_pad(&Padding {
                at: 0,
                x: 3,
                y: 4,
                size: (10, 20)
            })[0],
            [3.0, 4.0, 10.0, 20.0]
        );
    }

    #[test]
    fn animations_plan_blur_and_glow_and_rotations_turn_angles() {
        let still = AnimationDelta::default();
        assert!(animation_effects(&still, 0, 0.0).is_empty());
        let flare = AnimationDelta {
            glow: 0.5,
            blur: 0.01,
            ..AnimationDelta::default()
        };
        let added = animation_effects(&flare, 4, 1.5);
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].effect_type, "light.glow");
        assert_eq!(added[0].values["intensity"], ParamValue::Number(1.25));
        assert_eq!(added[0].values.len(), 6, "every glow parameter");
        assert_eq!(added[1].effect_type, ANIMATION_BLUR);
        assert_eq!((added[1].seed, added[1].time), (4, 1.5));
        let mut effects = vec![
            planned("light.sweep", &[("angle", ParamValue::Number(10.0))]),
            planned("light.glow", &[]),
        ];
        orient(&mut effects, 90);
        assert_eq!(effects[0].values["angle"], ParamValue::Number(-80.0));
        assert!(!effects[1].values.contains_key("angle"));
        assert!((fitted_scale((1920, 1080), (960, 540), 0) - 0.5).abs() < 1e-9);
        assert!((fitted_scale((1080, 1920), (1920, 1080), 90) - 1.0).abs() < 1e-9);
    }
}
