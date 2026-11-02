//! Light effects (ADR-0011, items 1 and 4): a glow around a layer's bright parts, a band of
//! light sweeping across it, and anamorphic streaks from its highlights, for motion
//! graphics such as a logo glowing orange on a dark background or a sheen across an end
//! card.
//!
//! They are ordinary picture effects of the registry (category `light`), appended to
//! [`crate::effect_types`]; their numbers take keyframes like any other effect's. The
//! renderer pads a layer's picture so the glow and the streaks spread past its edges
//! (`fosscut-render`, module `light`).

use crate::{EffectCategory, EffectSpec, ParamDefault, ParamKind, ParamSpec};

/// `light.glow`: soft light around the layer's bright parts.
pub const GLOW: &str = "light.glow";
/// `light.sweep`: a band of light moving across the layer.
pub const SWEEP: &str = "light.sweep";
/// `light.streaks`: anamorphic streaks from the layer's highlights.
pub const STREAKS: &str = "light.streaks";

const fn number(
    name: &'static str,
    default: f64,
    min: f64,
    max: f64,
    unit: &'static str,
    meaning: &'static str,
) -> ParamSpec {
    ParamSpec {
        name,
        kind: ParamKind::Number,
        default: ParamDefault::Number(default),
        min: Some(min),
        max: Some(max),
        unit,
        choices: &[],
        animatable: true,
        meaning,
    }
}

const fn color(name: &'static str, default: &'static str, meaning: &'static str) -> ParamSpec {
    ParamSpec {
        name,
        kind: ParamKind::Color,
        default: ParamDefault::Name(default),
        min: None,
        max: None,
        unit: "",
        choices: &[],
        animatable: false,
        meaning,
    }
}

/// The light effect types, appended to the registry.
pub(crate) fn specs() -> Vec<EffectSpec> {
    let spec = |name, title, summary, params| EffectSpec {
        name,
        version: 1,
        title,
        category: EffectCategory::Light,
        summary,
        params,
    };
    vec![
        spec(
            GLOW,
            "Glow",
            "Soft light around the layer's bright parts that spreads past its edges: a halo for logos, lines and titles on dark backgrounds, or a bloom on a video's highlights. Works on any picture clip.",
            vec![
                number(
                    "radius",
                    0.04,
                    0.0,
                    0.5,
                    "of height",
                    "how far the light spreads (one standard deviation of its wide halo) as a fraction of the picture's height; a tighter core of a quarter of it stays close to the edges",
                ),
                number(
                    "intensity",
                    1.0,
                    0.0,
                    8.0,
                    "",
                    "how bright the light is: 0 none, 1 about as bright as what glows next to it",
                ),
                number(
                    "threshold",
                    0.0,
                    0.0,
                    1.0,
                    "",
                    "only parts at least this bright glow, by their brightest sRGB channel: 0 lets everything glow, 0.8 only the highlights",
                ),
                color(
                    "color",
                    "#FFFFFF",
                    "the colour the light takes as tint grows, #RRGGBB",
                ),
                number(
                    "tint",
                    0.0,
                    0.0,
                    1.0,
                    "",
                    "how much of the light takes the colour instead of the picture's own colours: 0 none, 1 all",
                ),
                ParamSpec {
                    name: "glow_only",
                    kind: ParamKind::Bool,
                    default: ParamDefault::Bool(false),
                    min: None,
                    max: None,
                    unit: "",
                    choices: &[],
                    animatable: false,
                    meaning: "shows only the light, without the layer itself",
                },
            ],
        ),
        spec(
            SWEEP,
            "Light sweep",
            "A band of light that moves across the layer's visible pixels, such as a sheen over a logo on an end card: keyframe position from 0 to 1. Transparent parts stay transparent.",
            vec![
                number(
                    "position",
                    0.5,
                    0.0,
                    1.0,
                    "",
                    "where the band is: 0 just before it enters the layer, 1 just after it leaves, 0.5 across the middle",
                ),
                number(
                    "angle",
                    20.0,
                    -180.0,
                    180.0,
                    "degrees",
                    "the direction the band moves, clockwise from left to right: 0 moves an upright band from left to right, 90 moves a level band from top to bottom",
                ),
                number(
                    "width",
                    0.15,
                    0.01,
                    1.0,
                    "",
                    "the band's width as a fraction of the distance it travels across the layer",
                ),
                number(
                    "softness",
                    0.6,
                    0.0,
                    1.0,
                    "",
                    "how soft the band's edges are: 0 hard, 1 fading all the way from its centre",
                ),
                number(
                    "intensity",
                    1.0,
                    0.0,
                    4.0,
                    "",
                    "how much light the band adds: 1 brings white parts to full brightness",
                ),
                color("color", "#FFFFFF", "the light's colour, #RRGGBB"),
            ],
        ),
        spec(
            STREAKS,
            "Light streaks",
            "Anamorphic streaks: thin lines of light from the layer's highlights, fading with distance, that spread past its edges. Works on any picture clip.",
            vec![
                number(
                    "length",
                    0.3,
                    0.0,
                    1.0,
                    "of width",
                    "how far the streaks reach on each side, as a fraction of the picture's width",
                ),
                number(
                    "angle",
                    0.0,
                    -180.0,
                    180.0,
                    "degrees",
                    "the streaks' direction, clockwise: 0 is horizontal",
                ),
                number(
                    "intensity",
                    1.0,
                    0.0,
                    8.0,
                    "",
                    "how bright the streaks are: 1 starts them as bright as their source",
                ),
                number(
                    "threshold",
                    0.7,
                    0.0,
                    1.0,
                    "",
                    "only parts at least this bright streak, by their brightest sRGB channel",
                ),
                color(
                    "color",
                    "#80B0FF",
                    "the colour the streaks take as tint grows, #RRGGBB",
                ),
                number(
                    "tint",
                    0.6,
                    0.0,
                    1.0,
                    "",
                    "how much of the streaks takes the colour instead of the source's own colours",
                ),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_effects_are_registered_with_their_parameters() {
        for name in [GLOW, SWEEP, STREAKS] {
            let spec = crate::effect_type(name).unwrap_or_else(|| panic!("{name} registered"));
            assert_eq!(spec.category, EffectCategory::Light);
            assert!(!spec.is_audio());
            for p in &spec.params {
                // Numbers take keyframes; defaults lie in their ranges.
                if let ParamDefault::Number(v) = p.default {
                    assert!(p.animatable, "{name}.{}", p.name);
                    assert!(p.min.is_none_or(|m| v >= m) && p.max.is_none_or(|m| v <= m));
                }
            }
        }
        let glow = crate::effect_type(GLOW).unwrap();
        let names: Vec<_> = glow.params.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [
                "radius",
                "intensity",
                "threshold",
                "color",
                "tint",
                "glow_only"
            ]
        );
        assert_eq!(
            crate::effect_type(SWEEP)
                .unwrap()
                .param("position")
                .unwrap()
                .default,
            ParamDefault::Number(0.5)
        );
        let json = serde_json::to_value(crate::effect_type(STREAKS).unwrap()).unwrap();
        assert_eq!(json["category"], "light");
    }
}
