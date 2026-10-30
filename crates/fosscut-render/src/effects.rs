//! The CPU side of effect passes (ADR-0011, items 1 and 4): which shader entry point an
//! effect type uses, how its parameters pack into the pass's uniform, and the curves
//! table. The GPU side is in `gpu.rs`, the shaders in `shaders/effects.wgsl`.

use std::collections::BTreeMap;

use fosscut_model::{HSL_PARAMS, ParamValue};

/// Vec4 slots in an effect pass's uniform.
pub const SLOTS: usize = 12;

/// An effect ready to render: its type and every parameter at the frame's time.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedEffect {
    /// The effect type, such as `color.adjust`.
    pub effect_type: String,
    /// Every parameter, with defaults filled in.
    pub values: BTreeMap<String, ParamValue>,
    /// Seed for effects that vary from frame to frame (grain): the frame's index.
    pub seed: u32,
}

impl PlannedEffect {
    /// A number parameter, or 0.
    pub fn number(&self, name: &str) -> f32 {
        self.values
            .get(name)
            .and_then(ParamValue::number)
            .unwrap_or(0.0) as f32
    }

    /// A pair parameter, or zeros.
    pub fn pair(&self, name: &str) -> [f32; 2] {
        self.values
            .get(name)
            .and_then(ParamValue::pair)
            .map_or([0.0; 2], |p| p.map(|v| v as f32))
    }

    /// Curve points, or the straight line.
    pub fn points(&self, name: &str) -> Vec<[f64; 2]> {
        match self.values.get(name) {
            Some(ParamValue::Points(points)) => points.clone(),
            _ => vec![[0.0, 0.0], [1.0, 1.0]],
        }
    }

    /// The asset id of an asset parameter.
    pub fn asset(&self, name: &str) -> Option<&fosscut_model::Id> {
        match self.values.get(name) {
            Some(ParamValue::Asset(reference)) => Some(&reference.asset),
            _ => None,
        }
    }
}

/// How an effect type renders.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Pass {
    /// One per-pixel pass through the named entry point.
    Pixel(&'static str),
    /// A two-pass Gaussian blur.
    Blur,
    /// A blur, then the picture plus its difference from the blur.
    Sharpen,
}

/// The pass of an effect type, or `None` for a type this renderer does not draw.
pub fn pass_for(effect_type: &str) -> Option<Pass> {
    Some(match effect_type {
        "color.adjust" => Pass::Pixel("fs_adjust"),
        "color.curves" => Pass::Pixel("fs_curves"),
        "color.hsl" => Pass::Pixel("fs_hsl"),
        "color.wheels" => Pass::Pixel("fs_wheels"),
        "color.lut" => Pass::Pixel("fs_lut"),
        "color.vignette" => Pass::Pixel("fs_vignette"),
        "stylize.grain" => Pass::Pixel("fs_grain"),
        "key.chroma" => Pass::Pixel("fs_chroma"),
        "blur.gaussian" => Pass::Blur,
        "detail.sharpen" => Pass::Sharpen,
        _ => return None,
    })
}

/// Every per-pixel entry point, for building pipelines.
pub const PIXEL_ENTRIES: [&str; 12] = [
    "fs_copy",
    "fs_mix",
    "fs_adjust",
    "fs_curves",
    "fs_hsl",
    "fs_wheels",
    "fs_lut",
    "fs_vignette",
    "fs_grain",
    "fs_chroma",
    "fs_blur",
    "fs_sharpen",
];

/// The uniform of a per-pixel pass for `effect` on a picture `size` pixels large.
/// `lut_size` is the entries per axis of a LUT effect's table.
pub fn pack(effect: &PlannedEffect, size: (u32, u32), lut_size: u32) -> [[f32; 4]; SLOTS] {
    let mut p = [[0.0f32; 4]; SLOTS];
    let n = |name: &str| effect.number(name);
    let (w, h) = (size.0 as f32, size.1 as f32);
    match effect.effect_type.as_str() {
        "color.adjust" => {
            p[0] = [n("exposure"), n("temperature"), n("tint"), n("contrast")];
            p[1] = [n("brightness"), n("highlights"), n("shadows"), n("whites")];
            p[2] = [n("blacks"), n("saturation"), n("vibrance"), 0.0];
        }
        "color.hsl" => {
            for (band, [hue, saturation, lightness]) in HSL_PARAMS.iter().enumerate() {
                p[band] = [n(hue), n(saturation), n(lightness), 0.0];
            }
        }
        "color.wheels" => {
            for (slot, wheel) in ["lift", "gamma", "gain", "offset"].iter().enumerate() {
                let [x, y] = effect.pair(wheel);
                p[slot] = [x, y, n(&format!("{wheel}_luma")), 0.0];
            }
        }
        "color.lut" => p[0] = [lut_size as f32, n("intensity"), n("protect_skin"), 0.0],
        "color.vignette" => {
            p[0] = [n("amount"), n("midpoint"), n("roundness"), n("feather")];
            p[1] = [w, h, 0.0, 0.0];
        }
        "stylize.grain" => {
            // Grain size is relative to a 1080-pixel-high picture.
            p[0] = [n("amount"), n("size"), (effect.seed % 4096) as f32, 0.0];
            p[1] = [h / 1080.0, 0.0, 0.0, 0.0];
        }
        "detail.sharpen" => p[0] = [n("amount"), 0.0, 0.0, 0.0],
        "key.chroma" => {
            p[0] = [n("similarity"), n("smoothness"), n("spill"), 0.0];
            let color = match effect.values.get("color") {
                Some(ParamValue::Name(hex)) => hex_rgb(hex),
                _ => [0.0, 1.0, 0.0],
            };
            p[1] = [color[0], color[1], color[2], 0.0];
        }
        _ => {}
    }
    p
}

/// The sRGB-encoded channels of `#RRGGBB`, 0 to 1; black when malformed.
pub fn hex_rgb(hex: &str) -> [f32; 3] {
    let digits = hex.trim_start_matches('#');
    let channel = |i: usize| {
        digits
            .get(i..i + 2)
            .and_then(|d| u8::from_str_radix(d, 16).ok())
            .map_or(0.0, |v| f32::from(v) / 255.0)
    };
    [channel(0), channel(2), channel(4)]
}

/// One half of a separable blur with standard deviation `sigma` pixels of a picture
/// `size` pixels large whose texture has `mips` levels: the level to sample (so at most
/// a dozen taps are needed), the sigma in that level's texels, the taps on each side, and
/// the size of that level.
pub fn blur_plan(sigma: f32, size: (u32, u32), mips: u32) -> (u32, f32, u32, (u32, u32)) {
    let sigma = sigma.max(0.0);
    let mut lod = 0u32;
    while sigma / (1u32 << lod) as f32 > 4.0 && lod + 1 < mips {
        lod += 1;
    }
    let scaled = sigma / (1u32 << lod) as f32;
    let taps = (3.0 * scaled).ceil().clamp(1.0, 24.0) as u32;
    let level = ((size.0 >> lod).max(1), (size.1 >> lod).max(1));
    (lod, scaled, taps, level)
}

/// A curve through `points` sampled at 256 positions from 0 to 1, with monotone cubic
/// (Fritsch-Carlson) interpolation: smooth, and never overshooting between points, so a
/// rising curve stays rising. Flat before the first point and after the last.
pub fn sample_curve(points: &[[f64; 2]]) -> [f32; 256] {
    let mut out = [0.0f32; 256];
    if points.len() < 2 {
        for (i, v) in out.iter_mut().enumerate() {
            *v = i as f32 / 255.0;
        }
        return out;
    }
    let n = points.len();
    let x: Vec<f64> = points.iter().map(|p| p[0]).collect();
    let y: Vec<f64> = points.iter().map(|p| p[1]).collect();
    let secants: Vec<f64> = (0..n - 1)
        .map(|i| (y[i + 1] - y[i]) / (x[i + 1] - x[i]).max(1e-9))
        .collect();
    let mut tangents = vec![0.0; n];
    tangents[0] = secants[0];
    tangents[n - 1] = secants[n - 2];
    for i in 1..n - 1 {
        tangents[i] = if secants[i - 1] * secants[i] <= 0.0 {
            0.0
        } else {
            (secants[i - 1] + secants[i]) / 2.0
        };
    }
    for i in 0..n - 1 {
        if secants[i] == 0.0 {
            tangents[i] = 0.0;
            tangents[i + 1] = 0.0;
            continue;
        }
        let a = tangents[i] / secants[i];
        let b = tangents[i + 1] / secants[i];
        let r = a * a + b * b;
        if r > 9.0 {
            let t = 3.0 / r.sqrt();
            tangents[i] = t * a * secants[i];
            tangents[i + 1] = t * b * secants[i];
        }
    }
    for (index, value) in out.iter_mut().enumerate() {
        let t = index as f64 / 255.0;
        let v = if t <= x[0] {
            y[0]
        } else if t >= x[n - 1] {
            y[n - 1]
        } else {
            let i = x
                .partition_point(|&xi| xi <= t)
                .saturating_sub(1)
                .min(n - 2);
            let h = x[i + 1] - x[i];
            let s = (t - x[i]) / h;
            let (s2, s3) = (s * s, s * s * s);
            (2.0 * s3 - 3.0 * s2 + 1.0) * y[i]
                + (s3 - 2.0 * s2 + s) * h * tangents[i]
                + (-2.0 * s3 + 3.0 * s2) * y[i + 1]
                + (s3 - s2) * h * tangents[i + 1]
        };
        *value = v.clamp(0.0, 1.0) as f32;
    }
    out
}

/// The curves table of a `color.curves` effect: 256 RGBA entries holding the red, green,
/// blue and master curves.
pub fn curves_table(effect: &PlannedEffect) -> Vec<[f32; 4]> {
    let red = sample_curve(&effect.points("red"));
    let green = sample_curve(&effect.points("green"));
    let blue = sample_curve(&effect.points("blue"));
    let master = sample_curve(&effect.points("master"));
    (0..256)
        .map(|i| [red[i], green[i], blue[i], master[i]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(kind: &str, values: &[(&str, ParamValue)]) -> PlannedEffect {
        PlannedEffect {
            effect_type: kind.into(),
            values: values
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            seed: 0,
        }
    }

    #[test]
    fn curves_pass_through_points_without_overshoot() {
        let straight = sample_curve(&[[0.0, 0.0], [1.0, 1.0]]);
        assert!((straight[128] - 128.0 / 255.0).abs() < 1e-6);
        let points = [[0.0, 0.0], [0.25, 0.4], [0.5, 0.5], [1.0, 1.0]];
        let curve = sample_curve(&points);
        let at = |x: f64| curve[(x * 255.0).round() as usize];
        assert!((at(0.5) - 0.5).abs() < 0.01, "through the points");
        assert!(curve.windows(2).all(|w| w[1] >= w[0] - 1e-6), "monotone");
        let flat = sample_curve(&[[0.2, 0.3], [0.8, 0.7]]);
        assert_eq!(flat[0], 0.3, "flat before the first point");
        assert_eq!(flat[255], 0.7, "flat after the last");
        let wave = sample_curve(&[[0.0, 0.0], [0.3, 0.8], [0.6, 0.2], [1.0, 1.0]]);
        assert!(wave.iter().all(|v| (0.0..=1.0).contains(v)), "clamped");
        let peak = wave[(0.3 * 255.0f64).round() as usize];
        assert!(peak <= 0.81, "no overshoot past a local maximum: {peak}");
        assert_eq!(
            sample_curve(&[[0.5, 0.5]])[255],
            1.0,
            "fewer than two points: straight"
        );
    }

    #[test]
    fn parameters_pack_where_the_shaders_read_them() {
        let adjust = planned(
            "color.adjust",
            &[
                ("exposure", ParamValue::Number(1.0)),
                ("vibrance", ParamValue::Number(0.5)),
            ],
        );
        let p = pack(&adjust, (100, 50), 0);
        assert_eq!(p[0][0], 1.0);
        assert_eq!(p[2][2], 0.5);
        let wheels = planned(
            "color.wheels",
            &[
                ("gain", ParamValue::Pair([0.2, -0.1])),
                ("gain_luma", ParamValue::Number(0.3)),
            ],
        );
        assert_eq!(pack(&wheels, (1, 1), 0)[2], [0.2, -0.1, 0.3, 0.0]);
        let hsl = planned(
            "color.hsl",
            &[("aqua_saturation", ParamValue::Number(-0.5))],
        );
        assert_eq!(pack(&hsl, (1, 1), 0)[4][1], -0.5, "aqua is the fifth band");
        let lut = planned("color.lut", &[("intensity", ParamValue::Number(0.25))]);
        assert_eq!(pack(&lut, (1, 1), 33)[0][..2], [33.0, 0.25]);
        let vignette = planned("color.vignette", &[]);
        assert_eq!(pack(&vignette, (1920, 1080), 0)[1][..2], [1920.0, 1080.0]);
        let grain = PlannedEffect {
            seed: 5000,
            ..planned("stylize.grain", &[("size", ParamValue::Number(2.0))])
        };
        let p = pack(&grain, (3840, 2160), 0);
        assert_eq!((p[0][1], p[0][2], p[1][0]), (2.0, 904.0, 2.0));
        assert_eq!(
            pack(&planned("color.nope", &[]), (1, 1), 0),
            [[0.0; 4]; SLOTS]
        );
        let curves = curves_table(&planned("color.curves", &[]));
        assert_eq!(curves.len(), 256);
        assert_eq!(curves[255], [1.0; 4]);
        let lut_ref = planned(
            "color.lut",
            &[(
                "lut",
                ParamValue::Asset(fosscut_model::AssetRef {
                    asset: fosscut_model::Id::new(fosscut_model::IdKind::Asset, 4),
                }),
            )],
        );
        assert_eq!(lut_ref.asset("lut").map(|a| a.as_str()), Some("asset_0004"));
        assert_eq!(lut_ref.asset("intensity"), None);
        assert_eq!(lut_ref.points("master"), vec![[0.0, 0.0], [1.0, 1.0]]);
    }

    #[test]
    fn passes_and_blur_levels() {
        assert_eq!(pass_for("color.adjust"), Some(Pass::Pixel("fs_adjust")));
        assert_eq!(pass_for("blur.gaussian"), Some(Pass::Blur));
        assert_eq!(pass_for("detail.sharpen"), Some(Pass::Sharpen));
        assert_eq!(pass_for("color.unknown"), None);
        for kind in fosscut_model::effect_types() {
            assert!(pass_for(kind.name).is_some(), "{} renders", kind.name);
        }
        let (lod, sigma, taps, level) = blur_plan(2.0, (1920, 1080), 11);
        assert_eq!((lod, sigma, taps, level), (0, 2.0, 6, (1920, 1080)));
        let (lod, sigma, taps, level) = blur_plan(64.0, (1920, 1080), 11);
        assert_eq!(lod, 4);
        assert!((sigma - 4.0).abs() < 1e-6);
        assert_eq!((taps, level), (12, (120, 67)));
        let (lod, _, taps, _) = blur_plan(64.0, (1920, 1080), 1);
        assert_eq!((lod, taps), (0, 24), "without mips the taps are capped");
    }
}
