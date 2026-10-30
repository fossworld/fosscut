//! Keyframed clip properties (ADR-0011, item 3): a closed vocabulary of property paths,
//! keyframes with hold, linear or Bezier interpolation, and their evaluation.
//!
//! Keyframe times are offsets from the clip's start, so keyframes move with their clip.
//! A trim or split keeps every keyframe and only shifts the times, which keeps the curve
//! exact on both sides of a cut; keyframes outside the clip still shape the curve inside
//! it.

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::is_default;

/// A keyframed value: a number, or a pair such as a position.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum KeyValue {
    /// A number, such as a scale or an opacity.
    Number(f64),
    /// A pair, such as a position `[x, y]`.
    Pair([f64; 2]),
}

impl KeyValue {
    /// The number, if this is one.
    pub fn number(self) -> Option<f64> {
        match self {
            KeyValue::Number(v) => Some(v),
            KeyValue::Pair(_) => None,
        }
    }

    /// The pair, if this is one.
    pub fn pair(self) -> Option<[f64; 2]> {
        match self {
            KeyValue::Pair(v) => Some(v),
            KeyValue::Number(_) => None,
        }
    }

    fn lerp(self, other: KeyValue, t: f64) -> KeyValue {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        match (self, other) {
            (KeyValue::Number(a), KeyValue::Number(b)) => KeyValue::Number(mix(a, b)),
            (KeyValue::Pair(a), KeyValue::Pair(b)) => {
                KeyValue::Pair([mix(a[0], b[0]), mix(a[1], b[1])])
            }
            // Validation keeps one kind per property; hold on a mismatch.
            (a, _) => a,
        }
    }
}

/// How a value travels from one keyframe to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    /// Keeps this keyframe's value until the next keyframe.
    Hold,
    /// Changes at a constant rate.
    #[default]
    Linear,
    /// Follows the cubic Bezier easing curve from (0, 0) to (1, 1) with control points
    /// `[x1, y1, x2, y2]`, as in CSS: x is the fraction of the time between the keyframes
    /// and y the fraction of the change. x1 and x2 are between 0 and 1.
    Bezier([f64; 4]),
}

impl Interpolation {
    /// Starts slowly and speeds up.
    pub const EASE_IN: Interpolation = Interpolation::Bezier([0.42, 0.0, 1.0, 1.0]);
    /// Starts quickly and slows down.
    pub const EASE_OUT: Interpolation = Interpolation::Bezier([0.0, 0.0, 0.58, 1.0]);
    /// Starts and ends slowly.
    pub const EASE_IN_OUT: Interpolation = Interpolation::Bezier([0.42, 0.0, 0.58, 1.0]);

    /// The fraction of the change reached at fraction `t` of the time, both 0 to 1.
    pub fn progress(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Interpolation::Hold => 0.0,
            Interpolation::Linear => t,
            Interpolation::Bezier([x1, y1, x2, y2]) => bezier_y_at_x(x1, y1, x2, y2, t),
        }
    }
}

/// One coordinate of the cubic Bezier through 0, `a`, `b`, 1 at parameter `s`.
fn cubic(a: f64, b: f64, s: f64) -> f64 {
    let r = 1.0 - s;
    3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s
}

fn cubic_slope(a: f64, b: f64, s: f64) -> f64 {
    let r = 1.0 - s;
    3.0 * r * r * a + 6.0 * r * s * (b - a) + 3.0 * s * s * (1.0 - b)
}

/// Solves x(s) = x for the curve's parameter (Newton steps, then bisection when the slope
/// is flat), and returns y(s).
fn bezier_y_at_x(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    let mut s = x;
    for _ in 0..8 {
        let error = cubic(x1, x2, s) - x;
        if error.abs() < 1e-9 {
            return cubic(y1, y2, s);
        }
        let slope = cubic_slope(x1, x2, s);
        if slope.abs() < 1e-6 {
            break;
        }
        s = (s - error / slope).clamp(0.0, 1.0);
    }
    let (mut low, mut high) = (0.0, 1.0);
    s = x;
    for _ in 0..60 {
        let value = cubic(x1, x2, s);
        if (value - x).abs() < 1e-9 {
            break;
        }
        if value < x {
            low = s;
        } else {
            high = s;
        }
        s = (low + high) / 2.0;
    }
    cubic(y1, y2, s)
}

/// A property's value at one time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Keyframe {
    /// Time from the clip's start; it may lie outside the clip after a trim.
    pub time: Flicks,
    /// The value at that time.
    pub value: KeyValue,
    /// How the value moves from this keyframe to the next (linear when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub interpolation: Interpolation,
}

/// The value of keyframes sorted by time at `time`: the first value before the first
/// keyframe, the last after the last, and the interpolation of the keyframe before `time`
/// in between. `None` without keyframes.
pub fn evaluate(keyframes: &[Keyframe], time: Flicks) -> Option<KeyValue> {
    let first = keyframes.first()?;
    if time <= first.time {
        return Some(first.value);
    }
    let after = keyframes.partition_point(|k| k.time <= time);
    let before = &keyframes[after - 1];
    let Some(next) = keyframes.get(after) else {
        return Some(before.value);
    };
    let span = (next.time - before.time).get() as f64;
    let t = (time - before.time).get() as f64 / span;
    Some(
        before
            .value
            .lerp(next.value, before.interpolation.progress(t)),
    )
}

/// What a property holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PropertyKind {
    /// A number.
    Number,
    /// A pair `[x, y]`.
    Pair,
    /// true or false.
    Bool,
    /// A duration (`{"seconds": 0.5}`, `{"frames": 15}` and the other time forms).
    Time,
    /// One of a fixed set of names.
    Choice,
}

/// Which clips have a property.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Applies {
    /// Clips with a picture: video, image, text and caption clips.
    Picture,
    /// Clips with sound: video and audio clips.
    Sound,
    /// Every clip.
    Any,
}

/// One entry of the closed property vocabulary.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Property {
    /// The path, such as `transform.position`.
    pub path: &'static str,
    /// What it holds.
    pub kind: PropertyKind,
    /// Which clips have it.
    pub applies: Applies,
    /// Whether it takes keyframes.
    pub animatable: bool,
    /// Lowest allowed value of a number (or of each half of a pair).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Highest allowed value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Names a choice accepts.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub choices: &'static [&'static str],
    /// What it means.
    pub meaning: &'static str,
}

const fn prop(
    path: &'static str,
    kind: PropertyKind,
    applies: Applies,
    animatable: bool,
    range: (Option<f64>, Option<f64>),
    meaning: &'static str,
) -> Property {
    Property {
        path,
        kind,
        applies,
        animatable,
        min: range.0,
        max: range.1,
        choices: &[],
        meaning,
    }
}

/// Names of the blend modes, in [`crate::BlendMode`] order.
pub const BLEND_MODES: &[&str] = &[
    "normal",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color_dodge",
    "color_burn",
    "hard_light",
    "soft_light",
    "linear_burn",
    "add",
    "difference",
    "exclusion",
];

/// Names of the channel modes, in [`crate::ChannelMode`] order.
pub const CHANNEL_MODES: &[&str] = &["stereo", "left", "right", "mono"];

/// Every property a clip has (ADR-0011, item 3). `clip.set` sets them, and the animatable
/// ones take keyframes.
pub fn clip_properties() -> Vec<Property> {
    use Applies::{Any, Picture, Sound};
    use PropertyKind::{Bool, Choice, Number, Pair, Time};
    let crop = |path, meaning| prop(path, Number, Picture, true, (Some(0.0), Some(1.0)), meaning);
    vec![
        prop(
            "transform.position",
            Pair,
            Picture,
            true,
            (None, None),
            "[x, y]: offset of the clip's centre from the canvas centre, as fractions of the canvas width and height; y grows downwards",
        ),
        prop(
            "transform.scale",
            Number,
            Picture,
            true,
            (Some(0.0), None),
            "uniform size relative to the fitted size, greater than 0",
        ),
        prop(
            "transform.stretch",
            Pair,
            Picture,
            true,
            (Some(0.0), None),
            "[x, y]: extra width and height factors on top of the scale, for non-uniform scaling; [1, 1] keeps the shape",
        ),
        prop(
            "transform.rotation",
            Number,
            Picture,
            true,
            (None, None),
            "degrees clockwise",
        ),
        prop(
            "transform.flip_horizontal",
            Bool,
            Picture,
            false,
            (None, None),
            "mirror the picture left to right",
        ),
        prop(
            "transform.flip_vertical",
            Bool,
            Picture,
            false,
            (None, None),
            "mirror the picture top to bottom",
        ),
        crop(
            "crop.left",
            "fraction of the picture's width removed on the left, before any flip; the rest keeps its size and place",
        ),
        crop("crop.right", "fraction of the width removed on the right"),
        crop("crop.top", "fraction of the height removed at the top"),
        crop(
            "crop.bottom",
            "fraction of the height removed at the bottom",
        ),
        prop(
            "opacity",
            Number,
            Picture,
            true,
            (Some(0.0), Some(1.0)),
            "0 (invisible) to 1",
        ),
        Property {
            choices: BLEND_MODES,
            ..prop(
                "blend",
                Choice,
                Picture,
                false,
                (None, None),
                "how the picture combines with the layers below it",
            )
        },
        prop(
            "enabled",
            Bool,
            Any,
            false,
            (None, None),
            "false deactivates the clip: it stays on the timeline but is neither drawn nor heard",
        ),
        prop(
            "volume",
            Number,
            Sound,
            true,
            (Some(0.0), None),
            "linear gain: 1 unchanged, 0 silent, 2 is about +6 dB",
        ),
        prop(
            "audio.pan",
            Number,
            Sound,
            true,
            (Some(-1.0), Some(1.0)),
            "-1 (left) to 1 (right): 0 leaves both sides unchanged, and moving to one side lowers the other",
        ),
        prop(
            "audio.fade_in",
            Time,
            Sound,
            false,
            (Some(0.0), None),
            "length of the fade in from silence at the clip's start",
        ),
        prop(
            "audio.fade_out",
            Time,
            Sound,
            false,
            (Some(0.0), None),
            "length of the fade out to silence at the clip's end",
        ),
        Property {
            choices: CHANNEL_MODES,
            ..prop(
                "audio.channels",
                Choice,
                Sound,
                false,
                (None, None),
                "stereo as recorded, left or right to fill both channels with one side, or mono to mix both",
            )
        },
    ]
}

/// The property at `path`, if the vocabulary has it.
pub fn clip_property(path: &str) -> Option<Property> {
    clip_properties().into_iter().find(|p| p.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seconds: i64, value: f64, interpolation: Interpolation) -> Keyframe {
        Keyframe {
            time: Flicks::from_seconds(seconds).unwrap(),
            value: KeyValue::Number(value),
            interpolation,
        }
    }

    fn at(keys: &[Keyframe], seconds: f64) -> f64 {
        evaluate(keys, Flicks::from_seconds_f64(seconds).unwrap())
            .and_then(KeyValue::number)
            .unwrap()
    }

    #[test]
    fn values_hold_outside_and_interpolate_between() {
        let keys = [
            key(1, 0.0, Interpolation::Linear),
            key(3, 1.0, Interpolation::Hold),
            key(5, 0.0, Interpolation::Linear),
        ];
        assert_eq!(evaluate(&[], Flicks::ZERO), None);
        assert_eq!(at(&keys, 0.0), 0.0, "the first value before the first key");
        assert_eq!(at(&keys, 2.0), 0.5);
        assert_eq!(at(&keys, 3.0), 1.0, "exactly on a key");
        assert_eq!(at(&keys, 4.9), 1.0, "hold keeps the value");
        assert_eq!(at(&keys, 5.0), 0.0);
        assert_eq!(at(&keys, 9.0), 0.0, "the last value after the last key");
    }

    #[test]
    fn pairs_interpolate_per_axis() {
        let keys = [
            Keyframe {
                time: Flicks::ZERO,
                value: KeyValue::Pair([0.0, 1.0]),
                interpolation: Interpolation::Linear,
            },
            Keyframe {
                time: Flicks::from_seconds(2).unwrap(),
                value: KeyValue::Pair([1.0, -1.0]),
                interpolation: Interpolation::Linear,
            },
        ];
        let half = evaluate(&keys, Flicks::from_seconds(1).unwrap()).unwrap();
        assert_eq!(half.pair(), Some([0.5, 0.0]));
        assert_eq!(half.number(), None);
    }

    #[test]
    fn easing_curves_match_their_shape() {
        assert_eq!(Interpolation::Hold.progress(0.7), 0.0);
        assert_eq!(Interpolation::Linear.progress(1.5), 1.0, "clamped");
        for ease in [
            Interpolation::EASE_IN,
            Interpolation::EASE_OUT,
            Interpolation::EASE_IN_OUT,
        ] {
            assert!(ease.progress(0.0).abs() < 1e-6);
            assert!((ease.progress(1.0) - 1.0).abs() < 1e-6);
        }
        assert!(Interpolation::EASE_IN.progress(0.5) < 0.5, "slow start");
        assert!(Interpolation::EASE_OUT.progress(0.5) > 0.5, "fast start");
        let middle = Interpolation::EASE_IN_OUT.progress(0.5);
        assert!((middle - 0.5).abs() < 1e-6, "symmetric: {middle}");
        // The CSS "ease-in" curve at x = 0.25 is about 0.0934 (from its definition).
        let quarter = Interpolation::EASE_IN.progress(0.25);
        assert!((quarter - 0.0934).abs() < 2e-3, "{quarter}");
        // A curve with an overshoot goes past the target.
        let back = Interpolation::Bezier([0.3, 1.6, 0.6, 1.0]);
        assert!(back.progress(0.6) > 1.0);
        // Flat x control points still solve (bisection).
        let flat = Interpolation::Bezier([1.0, 0.0, 0.0, 1.0]);
        let v = flat.progress(0.3);
        assert!((0.0..=1.0).contains(&v));
    }

    #[test]
    fn eased_keys_follow_the_curve() {
        let keys = [
            key(0, 0.0, Interpolation::EASE_IN_OUT),
            key(4, 100.0, Interpolation::Linear),
        ];
        assert!(at(&keys, 1.0) < 25.0);
        assert!((at(&keys, 2.0) - 50.0).abs() < 1e-3);
        assert!(at(&keys, 3.0) > 75.0);
    }

    #[test]
    fn the_vocabulary_is_closed_and_consistent() {
        let properties = clip_properties();
        let mut paths: Vec<_> = properties.iter().map(|p| p.path).collect();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), properties.len(), "paths are unique");
        for p in &properties {
            if p.animatable {
                assert!(matches!(p.kind, PropertyKind::Number | PropertyKind::Pair));
            }
            assert_eq!(
                p.kind == PropertyKind::Choice,
                !p.choices.is_empty(),
                "{}",
                p.path
            );
        }
        assert_eq!(clip_property("blend").unwrap().choices.len(), 14);
        assert!(clip_property("transform.skew").is_none());
        let json = serde_json::to_value(clip_property("opacity").unwrap()).unwrap();
        assert_eq!(json["max"], 1.0);
        assert!(json.get("choices").is_none());
    }

    #[test]
    fn keyframes_round_trip_sparsely() {
        let keys = vec![
            key(0, 1.0, Interpolation::Linear),
            key(1, 2.0, Interpolation::EASE_OUT),
            key(2, 2.0, Interpolation::Hold),
        ];
        let json = serde_json::to_value(&keys).unwrap();
        assert!(json[0].get("interpolation").is_none(), "linear is omitted");
        assert_eq!(json[1]["interpolation"]["bezier"][2], 0.58);
        assert_eq!(json[2]["interpolation"], "hold");
        let back: Vec<Keyframe> = serde_json::from_value(json).unwrap();
        assert_eq!(back, keys);
        let pair: Keyframe = serde_json::from_str(r#"{"time": 0, "value": [0.5, -0.25]}"#).unwrap();
        assert_eq!(pair.value, KeyValue::Pair([0.5, -0.25]));
    }
}
