//! Automatic color (ADR-0011, item 4, and ADR-0012: measured on this machine, no model):
//! statistics of a clip's picture, and `color.adjust` settings that correct it (auto
//! adjust) or bring it close to another clip (color match). Front ends add the settings
//! with `effect.add`, so the write goes through the command framework and is undoable.
//!
//! Frames are rendered without the clip's effects, through the same color pipeline as an
//! export, and measured in sRGB-encoded values.

use std::collections::BTreeMap;
use std::path::Path;

use fosscut_model::{ClipContent, Id, Project};
use fosscut_render::Renderer;
use fosscut_render::color::TransferFunction;
use fosscut_render::geometry::UNCROPPED;
use fosscut_render::plan::{Layer, RenderPlan, SourceRef, source_geometry};
use fosscut_time::Flicks;
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;
use crate::frame::Media;

/// Frames measured per clip.
const SAMPLES: i64 = 7;
/// Width of the measured frames, in pixels.
const WIDTH: u32 = 160;

/// What a clip's picture looks like on average, in sRGB-encoded values from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ColorStats {
    /// Mean red, green and blue.
    pub mean: [f64; 3],
    /// Median luma.
    pub median: f64,
    /// The 1st percentile of luma: the black level.
    pub low: f64,
    /// The 99th percentile of luma: the white level.
    pub high: f64,
    /// Mean colorfulness: the difference between the largest and smallest channel.
    pub chroma: f64,
}

/// Statistics of 8-bit RGBA pixels.
pub fn stats_of(rgba: &[u8]) -> ColorStats {
    let mut histogram = [0u64; 256];
    let (mut sum, mut chroma, mut count) = ([0.0f64; 3], 0.0f64, 0u64);
    for pixel in rgba.as_chunks::<4>().0 {
        let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(|c| f64::from(c) / 255.0);
        sum[0] += r;
        sum[1] += g;
        sum[2] += b;
        chroma += r.max(g).max(b) - r.min(g).min(b);
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        histogram[(luma * 255.0).round().clamp(0.0, 255.0) as usize] += 1;
        count += 1;
    }
    let count_f = count.max(1) as f64;
    let percentile = |p: f64| -> f64 {
        let wanted = (p * count as f64).ceil().max(1.0) as u64;
        let mut seen = 0;
        for (value, n) in histogram.iter().enumerate() {
            seen += n;
            if seen >= wanted {
                return value as f64 / 255.0;
            }
        }
        1.0
    };
    ColorStats {
        mean: sum.map(|s| s / count_f),
        median: percentile(0.5),
        low: percentile(0.01),
        high: percentile(0.99),
        chroma: chroma / count_f,
    }
}

fn linear(v: f64) -> f64 {
    let v = v.clamp(1e-4, 1.0);
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The `color.adjust` parameters that keep only the values that change something.
fn params(values: [(&str, f64); 7]) -> BTreeMap<String, f64> {
    values
        .into_iter()
        .map(|(k, v)| (k.to_string(), round(v)))
        .filter(|(_, v)| *v != 0.0)
        .collect()
}

/// White balance in the adjust effect's model: temperature and tint that move linear
/// channel ratios by `red` and `blue` (log2 of red over green and blue over green).
fn balance(red: f64, blue: f64) -> (f64, f64) {
    // The effect multiplies red by 2^(0.3 t + 0.15 k), green by 2^(-0.3 k) and blue by
    // 2^(-0.3 t + 0.15 k); solve for the change of red/green and blue/green.
    let tint = (red + blue) / 0.9;
    let temperature = (red - blue) / 0.6;
    (temperature.clamp(-1.0, 1.0), tint.clamp(-1.0, 1.0))
}

/// Settings that correct a clip: neutral white balance (gray world), median brightness
/// near middle gray, blacks and whites stretched towards the full range, and a touch of
/// vibrance for muted pictures. Each correction is partial, so pictures keep their mood;
/// pictures with almost no tonal range (a flat color) get no stretch, which would only
/// exaggerate them.
pub fn auto_adjust(stats: &ColorStats) -> BTreeMap<String, f64> {
    let [r, g, b] = stats.mean.map(|v| linear(v).log2());
    let (temperature, tint) = balance((g - r) * 0.8, (g - b) * 0.8);
    let exposure = ((linear(0.45) / linear(stats.median)).log2() * 0.7).clamp(-2.0, 2.0);
    let ranged = stats.high - stats.low > 0.25;
    let blacks = if ranged {
        (-stats.low / 0.2 * 0.6).clamp(-1.0, 0.0)
    } else {
        0.0
    };
    let whites = if ranged {
        ((1.0 - stats.high) / 0.2 * 0.6).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let vibrance = if stats.chroma < 0.2 { 0.15 } else { 0.0 };
    params([
        ("temperature", temperature),
        ("tint", tint),
        ("exposure", exposure),
        ("blacks", blacks),
        ("whites", whites),
        ("vibrance", vibrance),
        ("contrast", 0.0),
    ])
}

/// Settings that bring a clip close to a reference: the reference's channel balance,
/// median brightness, tonal spread and colorfulness.
pub fn match_to(stats: &ColorStats, reference: &ColorStats) -> BTreeMap<String, f64> {
    let [ar, ag, ab] = stats.mean.map(|v| linear(v).log2());
    let [br, bg, bb] = reference.mean.map(|v| linear(v).log2());
    let (temperature, tint) = balance((br - bg) - (ar - ag), (bb - bg) - (ab - ag));
    let exposure = (linear(reference.median) / linear(stats.median))
        .log2()
        .clamp(-2.0, 2.0);
    let spread = |s: &ColorStats| (s.high - s.low).max(0.05);
    let contrast = (spread(reference) / spread(stats) - 1.0).clamp(-0.5, 0.5);
    let saturation = (reference.chroma.max(0.01) / stats.chroma.max(0.01) - 1.0).clamp(-0.8, 0.8);
    params([
        ("temperature", temperature),
        ("tint", tint),
        ("exposure", exposure),
        ("contrast", contrast),
        ("saturation", saturation),
        ("blacks", 0.0),
        ("whites", 0.0),
    ])
}

/// Measures the picture of `clip` from frames spread over it, rendered without effects.
pub fn clip_stats(
    project: &Project,
    bundle: &Path,
    clip: &Id,
    renderer: &mut Renderer,
) -> Result<ColorStats, EngineError> {
    let found = project
        .timelines
        .values()
        .flat_map(|t| &t.tracks)
        .flat_map(|t| &t.clips)
        .find(|c| &c.id == clip)
        .ok_or_else(|| EngineError::InvalidArguments {
            message: format!("clip {clip} does not exist"),
        })?;
    let asset = match &found.content {
        ClipContent::Video { asset, .. } => asset.clone(),
        ClipContent::Image { asset } => asset.clone(),
        other => {
            return Err(EngineError::InvalidArguments {
                message: format!(
                    "{clip} is a {} clip; color is measured on video and image clips",
                    other.kind_name()
                ),
            });
        }
    };
    let ((w, h), rotation) =
        source_geometry(project, &asset).ok_or_else(|| EngineError::InvalidArguments {
            message: format!("{asset} has no picture to measure"),
        })?;
    let (w, h) = if rotation % 180 == 90 { (h, w) } else { (w, h) };
    let size = (
        WIDTH,
        ((u64::from(WIDTH) * u64::from(h)) / u64::from(w.max(1))).max(1) as u32,
    );
    let mut media = Media::default();
    let mut pixels = Vec::new();
    for sample in 0..SAMPLES {
        let offset = Flicks::new(
            i64::try_from(
                i128::from(found.duration.get()) * i128::from(2 * sample + 1)
                    / i128::from(2 * SAMPLES),
            )
            .unwrap_or(0),
        );
        // Through the clip's speed (ADR-0023); images have one frame.
        let time = found.source_at(offset).unwrap_or(Flicks::ZERO);
        let plan = RenderPlan {
            canvas: (w, h),
            layers: vec![Layer {
                rotation,
                crop: UNCROPPED,
                ..Layer::new(SourceRef {
                    asset: asset.clone(),
                    time,
                })
            }],
            skipped: Vec::new(),
        };
        media.open_for(project, bundle, &plan)?;
        let image = renderer.render_image(&plan, media.frames(), size, TransferFunction::Srgb)?;
        pixels.extend_from_slice(&image.rgba);
    }
    Ok(stats_of(&pixels))
}

/// What `suggest` found: the measurements and the `effect.add` that applies them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Suggestion {
    /// The clip's picture.
    pub stats: ColorStats,
    /// The reference's picture, for a match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<ColorStats>,
    /// The arguments of the `effect.add` command that applies the correction first in
    /// the clip's effects.
    pub effect_add: Value,
}

/// Measures `clip` (and `reference`, for a match) and returns the `effect.add` arguments
/// of a `color.adjust` that corrects it or matches it to the reference.
pub fn suggest(
    project: &Project,
    bundle: &Path,
    clip: &Id,
    reference: Option<&Id>,
    renderer: &mut Renderer,
) -> Result<Suggestion, EngineError> {
    let stats = clip_stats(project, bundle, clip, renderer)?;
    let (params, reference) = match reference {
        Some(other) => {
            if other == clip {
                return Err(EngineError::InvalidArguments {
                    message: "a clip cannot be matched to itself".to_string(),
                });
            }
            let theirs = clip_stats(project, bundle, other, renderer)?;
            (match_to(&stats, &theirs), Some(theirs))
        }
        None => (auto_adjust(&stats), None),
    };
    Ok(Suggestion {
        stats,
        reference,
        effect_add: json!({
            "clip": clip,
            "type": "color.adjust",
            "params": params,
            "index": 0,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(rgb: [u8; 3]) -> Vec<u8> {
        [rgb[0], rgb[1], rgb[2], 255].repeat(100)
    }

    #[test]
    fn stats_measure_levels_and_color() {
        let mut pixels = flat([0, 0, 0]);
        pixels.extend(flat([255, 255, 255]));
        pixels.extend([255, 0, 0, 255]);
        let stats = stats_of(&pixels);
        assert_eq!(stats.low, 0.0);
        assert_eq!(stats.high, 1.0);
        assert!(stats.chroma > 0.0 && stats.chroma < 0.01);
        let gray = stats_of(&flat([128, 128, 128]));
        assert!((gray.median - 128.0 / 255.0).abs() < 1e-9);
        assert!(gray.mean.iter().all(|m| (m - 128.0 / 255.0).abs() < 1e-9));
        assert_eq!(stats_of(&[]).mean, [0.0; 3]);
    }

    #[test]
    fn auto_adjust_neutralizes_and_levels() {
        // A blue, dark, low-contrast picture.
        let cast = ColorStats {
            mean: [0.25, 0.3, 0.45],
            median: 0.28,
            low: 0.1,
            high: 0.7,
            chroma: 0.1,
        };
        let fix = auto_adjust(&cast);
        assert!(
            fix["temperature"] > 0.0,
            "warmer against a blue cast: {fix:?}"
        );
        assert!(fix["exposure"] > 0.0, "brighter");
        assert!(fix["blacks"] < 0.0, "deeper blacks");
        assert!(fix["whites"] > 0.0, "brighter whites");
        assert_eq!(fix["vibrance"], 0.15);
        let neutral = ColorStats {
            mean: [0.45; 3],
            median: 0.45,
            low: 0.0,
            high: 1.0,
            chroma: 0.3,
        };
        assert!(
            auto_adjust(&neutral).is_empty(),
            "a balanced picture needs nothing"
        );
    }

    #[test]
    fn matching_moves_towards_the_reference() {
        let ours = ColorStats {
            mean: [0.4, 0.4, 0.4],
            median: 0.4,
            low: 0.1,
            high: 0.8,
            chroma: 0.2,
        };
        let warm = ColorStats {
            mean: [0.55, 0.45, 0.35],
            median: 0.5,
            low: 0.0,
            high: 0.95,
            chroma: 0.3,
        };
        let m = match_to(&ours, &warm);
        assert!(m["temperature"] > 0.0 && m["exposure"] > 0.0);
        assert!(m["contrast"] > 0.0 && m["saturation"] > 0.0);
        assert!(
            match_to(&ours, &ours).is_empty(),
            "a clip already matches itself"
        );
        // The balance solves the effect's own white balance model.
        let (t, k) = balance(0.3, -0.3);
        assert!((t - 1.0).abs() < 1e-9 && k.abs() < 1e-9);
    }
}
