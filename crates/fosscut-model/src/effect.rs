//! Effects (ADR-0011, items 1 and 2): built-in effect types with parameter schemas, and
//! effect instances stored inside the clip that owns them.
//!
//! An instance names its type and version, keeps only the parameters that differ from
//! their defaults, and may animate numeric parameters with keyframes, addressed as
//! `effects.<id>.<param>` in the property vocabulary.

use std::collections::BTreeMap;

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::animation::{KeyValue, Keyframe, evaluate};

/// A reference to a media asset, such as a LUT file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AssetRef {
    /// The asset (`asset_…`).
    pub asset: Id,
}

/// A parameter value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ParamValue {
    /// A number.
    Number(f64),
    /// A pair such as a color wheel position `[x, y]`.
    Pair([f64; 2]),
    /// On or off.
    Flag(bool),
    /// One of a fixed set of names.
    Name(String),
    /// Curve control points `[[x, y], ...]` from 0 to 1, sorted by x.
    Points(Vec<[f64; 2]>),
    /// A media asset.
    Asset(AssetRef),
}

impl ParamValue {
    /// The number, if this is one.
    pub fn number(&self) -> Option<f64> {
        match self {
            ParamValue::Number(v) => Some(*v),
            _ => None,
        }
    }

    /// The pair, if this is one.
    pub fn pair(&self) -> Option<[f64; 2]> {
        match self {
            ParamValue::Pair(v) => Some(*v),
            _ => None,
        }
    }
}

impl From<KeyValue> for ParamValue {
    fn from(value: KeyValue) -> ParamValue {
        match value {
            KeyValue::Number(v) => ParamValue::Number(v),
            KeyValue::Pair(v) => ParamValue::Pair(v),
        }
    }
}

/// An effect on a clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Effect {
    /// Effect id (`effect_…`).
    pub id: Id,
    /// The effect type, such as `color.adjust`.
    #[serde(rename = "type")]
    pub effect_type: String,
    /// Version of the type's parameters this instance was written with.
    pub version: u32,
    /// False bypasses the effect without removing it.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub enabled: bool,
    /// Parameters that differ from their defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamValue>,
    /// Keyframes of numeric parameters by parameter name, each sorted by time from the
    /// clip's start. A parameter with keyframes takes its value from them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keyframes: BTreeMap<String, Vec<Keyframe>>,
}

fn yes() -> bool {
    true
}

fn is_yes(value: &bool) -> bool {
    *value
}

impl Effect {
    /// Every parameter of the effect at `offset` from the clip's start: keyframed values,
    /// stored values, and the type's defaults for the rest. `None` for an unknown type.
    pub fn values_at(&self, offset: Flicks) -> Option<BTreeMap<String, ParamValue>> {
        let spec = effect_type(&self.effect_type)?;
        let mut values = BTreeMap::new();
        for param in spec.params {
            let value = self
                .keyframes
                .get(param.name)
                .and_then(|keys| evaluate(keys, offset))
                .map(ParamValue::from)
                .or_else(|| self.params.get(param.name).cloned())
                .unwrap_or_else(|| param.default.value());
            values.insert(param.name.to_string(), value);
        }
        Some(values)
    }

    /// Shifts every keyframe by `delta`, as [`crate::Clip::shift_keyframes`] does.
    pub fn shift_keyframes(&mut self, delta: Flicks) {
        for keys in self.keyframes.values_mut() {
            for key in keys {
                key.time = key.time + delta;
            }
        }
    }
}

/// What a parameter holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// A number.
    Number,
    /// A pair `[x, y]`.
    Pair,
    /// true or false.
    Bool,
    /// One of a fixed set of names.
    Choice,
    /// Curve control points `[[x, y], ...]` from 0 to 1.
    Curve,
    /// A media asset `{"asset": "asset_…"}` of the kind the parameter names.
    Asset,
    /// An sRGB color `#RRGGBB`.
    Color,
}

/// A parameter's default.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ParamDefault {
    /// A number.
    Number(f64),
    /// A pair.
    Pair([f64; 2]),
    /// A flag.
    Bool(bool),
    /// A name.
    Name(&'static str),
    /// The straight curve from (0, 0) to (1, 1).
    Identity(Identity),
    /// No default: the parameter must be given.
    Required(Required),
}

/// Marker for [`ParamDefault::Identity`]; serialized as `"identity"`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Identity {
    /// The straight curve.
    Identity,
}

/// Marker for [`ParamDefault::Required`]; serialized as `"required"`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Required {
    /// The parameter must be given.
    Required,
}

impl ParamDefault {
    /// The default as a value; a required parameter defaults to the number 0, which
    /// validation never lets through.
    pub fn value(self) -> ParamValue {
        match self {
            ParamDefault::Number(v) => ParamValue::Number(v),
            ParamDefault::Pair(v) => ParamValue::Pair(v),
            ParamDefault::Bool(v) => ParamValue::Flag(v),
            ParamDefault::Name(v) => ParamValue::Name(v.to_string()),
            ParamDefault::Identity(_) => ParamValue::Points(vec![[0.0, 0.0], [1.0, 1.0]]),
            ParamDefault::Required(_) => ParamValue::Number(0.0),
        }
    }
}

/// One parameter of an effect type.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ParamSpec {
    /// Name within the effect, such as `exposure`.
    pub name: &'static str,
    /// What it holds.
    pub kind: ParamKind,
    /// Its default.
    pub default: ParamDefault,
    /// Lowest allowed value of a number (or of each half of a pair).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Highest allowed value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Unit of a number, such as `stops`.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub unit: &'static str,
    /// Names a choice accepts.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub choices: &'static [&'static str],
    /// Whether it takes keyframes.
    pub animatable: bool,
    /// What it means.
    pub meaning: &'static str,
}

/// Groups effect types for people browsing them.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EffectCategory {
    /// Color and tone.
    Color,
    /// Blur and sharpness.
    Detail,
    /// Texture and stylization.
    Stylize,
    /// Keying: making parts of the picture transparent.
    Key,
}

/// A built-in effect type and its parameters (ADR-0011, item 1).
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct EffectSpec {
    /// Namespaced type name, such as `color.adjust`; `type` in JSON, as `effect.add` takes it.
    #[serde(rename = "type")]
    pub name: &'static str,
    /// Parameter version; instances of a newer version are refused.
    pub version: u32,
    /// Display name.
    pub title: &'static str,
    /// Group.
    pub category: EffectCategory,
    /// What it does.
    pub summary: &'static str,
    /// Its parameters.
    pub params: Vec<ParamSpec>,
}

impl EffectSpec {
    /// The parameter called `name`.
    pub fn param(&self, name: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.name == name)
    }
}

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

/// An amount from -1 to 1 with 0 leaving the picture unchanged.
const fn amount(name: &'static str, meaning: &'static str) -> ParamSpec {
    number(name, 0.0, -1.0, 1.0, "", meaning)
}

const fn wheel(name: &'static str, meaning: &'static str) -> ParamSpec {
    ParamSpec {
        name,
        kind: ParamKind::Pair,
        default: ParamDefault::Pair([0.0, 0.0]),
        min: Some(-1.0),
        max: Some(1.0),
        unit: "",
        choices: &[],
        animatable: true,
        meaning,
    }
}

const fn curve(name: &'static str, meaning: &'static str) -> ParamSpec {
    ParamSpec {
        name,
        kind: ParamKind::Curve,
        default: ParamDefault::Identity(Identity::Identity),
        min: Some(0.0),
        max: Some(1.0),
        unit: "",
        choices: &[],
        animatable: false,
        meaning,
    }
}

/// The eight hue bands of `color.hsl`, with their centre hues in degrees.
pub const HSL_BANDS: [(&str, f64); 8] = [
    ("red", 0.0),
    ("orange", 30.0),
    ("yellow", 60.0),
    ("green", 120.0),
    ("aqua", 180.0),
    ("blue", 240.0),
    ("purple", 270.0),
    ("magenta", 300.0),
];

/// Parameter names of `color.hsl`, per band in [`HSL_BANDS`] order: hue, saturation and
/// lightness.
pub const HSL_PARAMS: [[&str; 3]; 8] = [
    ["red_hue", "red_saturation", "red_lightness"],
    ["orange_hue", "orange_saturation", "orange_lightness"],
    ["yellow_hue", "yellow_saturation", "yellow_lightness"],
    ["green_hue", "green_saturation", "green_lightness"],
    ["aqua_hue", "aqua_saturation", "aqua_lightness"],
    ["blue_hue", "blue_saturation", "blue_lightness"],
    ["purple_hue", "purple_saturation", "purple_lightness"],
    ["magenta_hue", "magenta_saturation", "magenta_lightness"],
];

fn hsl_params() -> Vec<ParamSpec> {
    let mut params = Vec::with_capacity(24);
    for [hue, saturation, lightness] in HSL_PARAMS {
        params.push(amount(
            hue,
            "turns this band's hues towards its neighbours, up to 30 degrees",
        ));
        params.push(amount(
            saturation,
            "-1 removes this band's color, 1 doubles it",
        ));
        params.push(amount(
            lightness,
            "darkens (negative) or lightens (positive) this band",
        ));
    }
    params
}

/// Every built-in effect type (ADR-0011, item 4: the color and detail set).
pub fn effect_types() -> Vec<EffectSpec> {
    vec![
        EffectSpec {
            name: "color.adjust",
            version: 1,
            title: "Adjust",
            category: EffectCategory::Color,
            summary: "Exposure, white balance, contrast, tone ranges and saturation: the basic color corrections.",
            params: vec![
                number(
                    "exposure",
                    0.0,
                    -4.0,
                    4.0,
                    "stops",
                    "brightness in stops of light: 1 doubles it",
                ),
                amount(
                    "temperature",
                    "cooler (negative, bluer) or warmer (positive, more orange)",
                ),
                amount("tint", "greener (negative) or more magenta (positive)"),
                amount(
                    "contrast",
                    "flatter (negative) or punchier (positive) around mid gray",
                ),
                amount(
                    "brightness",
                    "darkens or lifts the midtones, keeping black and white",
                ),
                amount(
                    "highlights",
                    "darkens (negative) or brightens (positive) the bright parts",
                ),
                amount(
                    "shadows",
                    "deepens (negative) or lifts (positive) the dark parts",
                ),
                amount(
                    "whites",
                    "moves the white point: positive clips more to white",
                ),
                amount("blacks", "moves the black point: positive lifts the blacks"),
                amount("saturation", "-1 is black and white, 1 doubles the color"),
                amount("vibrance", "like saturation, but mostly on muted colors"),
            ],
        },
        EffectSpec {
            name: "color.curves",
            version: 1,
            title: "Curves",
            category: EffectCategory::Color,
            summary: "Tone curves for brightness and for each of red, green and blue: control points from 0 to 1, joined smoothly without overshoot.",
            params: vec![
                curve(
                    "master",
                    "the brightness curve, applied to all three channels first",
                ),
                curve("red", "the red channel's curve"),
                curve("green", "the green channel's curve"),
                curve("blue", "the blue channel's curve"),
            ],
        },
        EffectSpec {
            name: "color.hsl",
            version: 1,
            title: "HSL",
            category: EffectCategory::Color,
            summary: "Hue, saturation and lightness for eight color bands: red, orange, yellow, green, aqua, blue, purple and magenta.",
            params: hsl_params(),
        },
        EffectSpec {
            name: "color.wheels",
            version: 1,
            title: "Color wheels",
            category: EffectCategory::Color,
            summary: "Lift, gamma and gain wheels for shadows, midtones and highlights, and an offset for everything: each a color push [x, y] inside the unit circle plus a brightness amount.",
            params: vec![
                wheel(
                    "lift",
                    "color pushed into the shadows: direction is the hue, length the strength",
                ),
                amount("lift_luma", "darkens or lifts the shadows"),
                wheel("gamma", "color pushed into the midtones"),
                amount("gamma_luma", "darkens or lifts the midtones"),
                wheel("gain", "color pushed into the highlights"),
                amount("gain_luma", "darkens or brightens the highlights"),
                wheel("offset", "color added everywhere"),
                amount("offset_luma", "darkens or brightens everything"),
            ],
        },
        EffectSpec {
            name: "color.lut",
            version: 1,
            title: "LUT",
            category: EffectCategory::Color,
            summary: "Applies a 3D LUT from a .cube file imported as a media asset, with an intensity and protection for skin tones.",
            params: vec![
                ParamSpec {
                    name: "lut",
                    kind: ParamKind::Asset,
                    default: ParamDefault::Required(Required::Required),
                    min: None,
                    max: None,
                    unit: "",
                    choices: &["lut"],
                    animatable: false,
                    meaning: "the LUT asset: {\"asset\": \"asset_…\"} from importing a .cube file",
                },
                number(
                    "intensity",
                    1.0,
                    0.0,
                    1.0,
                    "",
                    "how much of the LUT applies: 0 none, 1 all",
                ),
                number(
                    "protect_skin",
                    0.0,
                    0.0,
                    1.0,
                    "",
                    "keeps skin tones closer to the original",
                ),
            ],
        },
        EffectSpec {
            name: "color.vignette",
            version: 1,
            title: "Vignette",
            category: EffectCategory::Color,
            summary: "Darkens (or lightens) the edges of the picture.",
            params: vec![
                amount(
                    "amount",
                    "positive darkens the edges, negative lightens them",
                ),
                number(
                    "midpoint",
                    0.5,
                    0.0,
                    1.0,
                    "",
                    "how far from the centre the vignette begins",
                ),
                amount(
                    "roundness",
                    "0 follows the picture's shape, 1 is a circle, -1 squarer",
                ),
                number("feather", 0.5, 0.0, 1.0, "", "how soft the transition is"),
            ],
        },
        EffectSpec {
            name: "key.chroma",
            version: 1,
            title: "Chroma key",
            category: EffectCategory::Key,
            summary: "Makes one color transparent, such as a green or blue screen, with soft edges and spill removal.",
            params: vec![
                ParamSpec {
                    name: "color",
                    kind: ParamKind::Color,
                    default: ParamDefault::Name("#00FF00"),
                    min: None,
                    max: None,
                    unit: "",
                    choices: &[],
                    animatable: false,
                    meaning: "the color to remove, #RRGGBB",
                },
                number(
                    "similarity",
                    0.35,
                    0.0,
                    1.0,
                    "",
                    "how close a color must be to the key to vanish",
                ),
                number(
                    "smoothness",
                    0.1,
                    0.0,
                    1.0,
                    "",
                    "how gradually colors near the key fade",
                ),
                number(
                    "spill",
                    0.5,
                    0.0,
                    1.0,
                    "",
                    "how much of the key's color is removed from what stays",
                ),
            ],
        },
        EffectSpec {
            name: "blur.gaussian",
            version: 1,
            title: "Blur",
            category: EffectCategory::Detail,
            summary: "Gaussian blur.",
            params: vec![number(
                "radius",
                0.01,
                0.0,
                0.2,
                "of height",
                "blur radius (one standard deviation) as a fraction of the picture's height",
            )],
        },
        EffectSpec {
            name: "detail.sharpen",
            version: 1,
            title: "Sharpen",
            category: EffectCategory::Detail,
            summary: "Sharpens edges with an unsharp mask.",
            params: vec![
                number("amount", 0.5, 0.0, 2.0, "", "strength: 0 none"),
                number(
                    "radius",
                    0.002,
                    0.0005,
                    0.02,
                    "of height",
                    "size of the detail sharpened, as a fraction of the picture's height",
                ),
            ],
        },
        EffectSpec {
            name: "stylize.grain",
            version: 1,
            title: "Film grain",
            category: EffectCategory::Stylize,
            summary: "Adds moving film grain, the same on every render of a frame.",
            params: vec![
                number("amount", 0.2, 0.0, 1.0, "", "how visible the grain is"),
                number(
                    "size",
                    1.0,
                    0.5,
                    4.0,
                    "",
                    "grain size: 1 is fine grain at 1080p",
                ),
            ],
        },
    ]
}

/// The built-in effect type called `name`.
pub fn effect_type(name: &str) -> Option<EffectSpec> {
    effect_types().into_iter().find(|t| t.name == name)
}

/// Splits `effects.<id>.<param>` into the effect id and the parameter.
pub fn effect_path(path: &str) -> Option<(&str, &str)> {
    path.strip_prefix("effects.")?.split_once('.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IdKind, Interpolation};

    fn effect(kind: &str) -> Effect {
        Effect {
            id: Id::new(IdKind::Effect, 1),
            effect_type: kind.into(),
            version: 1,
            enabled: true,
            params: BTreeMap::new(),
            keyframes: BTreeMap::new(),
        }
    }

    #[test]
    fn values_fill_defaults_and_follow_keyframes() {
        let mut adjust = effect("color.adjust");
        adjust
            .params
            .insert("contrast".into(), ParamValue::Number(0.4));
        adjust.keyframes.insert(
            "exposure".into(),
            vec![
                Keyframe {
                    time: Flicks::ZERO,
                    value: KeyValue::Number(0.0),
                    interpolation: Interpolation::Linear,
                },
                Keyframe {
                    time: Flicks::from_seconds(2).unwrap(),
                    value: KeyValue::Number(2.0),
                    interpolation: Interpolation::Linear,
                },
            ],
        );
        let values = adjust.values_at(Flicks::from_seconds(1).unwrap()).unwrap();
        assert_eq!(values["exposure"], ParamValue::Number(1.0));
        assert_eq!(values["contrast"], ParamValue::Number(0.4));
        assert_eq!(values["saturation"], ParamValue::Number(0.0));
        assert_eq!(values.len(), 11);
        let curves = effect("color.curves").values_at(Flicks::ZERO).unwrap();
        assert_eq!(
            curves["red"],
            ParamValue::Points(vec![[0.0, 0.0], [1.0, 1.0]])
        );
        assert!(effect("color.nope").values_at(Flicks::ZERO).is_none());
    }

    #[test]
    fn the_registry_is_consistent() {
        let types = effect_types();
        let mut names: Vec<_> = types.iter().map(|t| t.name).collect();
        names.dedup();
        assert_eq!(names.len(), types.len());
        for spec in &types {
            assert!(spec.name.contains('.'), "{} is namespaced", spec.name);
            let mut params: Vec<_> = spec.params.iter().map(|p| p.name).collect();
            params.sort_unstable();
            params.dedup();
            assert_eq!(
                params.len(),
                spec.params.len(),
                "{} params are unique",
                spec.name
            );
            for p in &spec.params {
                assert_eq!(
                    p.animatable,
                    matches!(p.kind, ParamKind::Number | ParamKind::Pair),
                    "{}.{}",
                    spec.name,
                    p.name
                );
            }
        }
        assert_eq!(effect_type("color.hsl").unwrap().params.len(), 24);
        assert!(
            effect_type("color.hsl")
                .unwrap()
                .param("aqua_lightness")
                .is_some()
        );
        let json = serde_json::to_value(effect_type("color.lut").unwrap()).unwrap();
        assert_eq!(json["params"][0]["default"], "required");
        let json = serde_json::to_value(effect_type("color.curves").unwrap()).unwrap();
        assert_eq!(json["params"][0]["default"], "identity");
        assert_eq!(
            effect_path("effects.effect_0001.exposure"),
            Some(("effect_0001", "exposure"))
        );
        assert_eq!(effect_path("opacity"), None);
    }

    #[test]
    fn values_round_trip_by_shape() {
        let text = r#"{"id": "effect_0002", "type": "color.lut", "version": 1,
            "params": {"lut": {"asset": "asset_0003"}, "intensity": 0.5},
            "enabled": false}"#;
        let lut: Effect = serde_json::from_str(text).unwrap();
        assert!(!lut.enabled);
        assert_eq!(
            lut.params["lut"],
            ParamValue::Asset(AssetRef {
                asset: Id::new(IdKind::Asset, 3)
            })
        );
        let curve: ParamValue = serde_json::from_str("[[0, 0], [0.5, 0.6], [1, 1]]").unwrap();
        assert!(matches!(curve, ParamValue::Points(p) if p.len() == 3));
        let pair: ParamValue = serde_json::from_str("[0.2, -0.1]").unwrap();
        assert_eq!(pair.pair(), Some([0.2, -0.1]));
        assert_eq!(pair.number(), None);
        let back = serde_json::to_value(&lut).unwrap();
        assert_eq!(back["enabled"], false);
        assert!(back.get("keyframes").is_none());
    }
}
