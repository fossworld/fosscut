//! Masks on clips (ADR-0011, item 4: masks with feather, expansion, round corners, invert,
//! several per clip). A mask is a shape placed in its clip's picture; together the clip's
//! masks decide which parts of the picture show. On an adjustment clip they limit where
//! its effects apply.
//!
//! Positions are fractions of the picture: `center` from its top-left corner, `size` of its
//! width and height. Path points sit in the mask's own box, from -1 to 1 on each axis;
//! brush strokes sit in the picture itself, from 0 to 1.

use std::collections::BTreeMap;

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::animation::{KeyValue, Keyframe, evaluate};
use crate::{Id, is_default};

/// The shape of a mask.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaskShape {
    /// A rectangle, with corners rounded by `roundness`.
    #[default]
    Rectangle,
    /// An ellipse filling the mask's box.
    Ellipse,
    /// Everything on one side of a line through the centre (above it, before rotation).
    Linear,
    /// A band through the centre, `size` high: the mirror shape.
    Mirror,
    /// A five-pointed star.
    Star,
    /// A heart.
    Heart,
    /// A closed path through `points`, straight or curved by their handles: the pen.
    Path,
    /// The letters of `text`, filling the mask's box.
    Text,
    /// Painted `strokes`.
    Brush,
}

/// How a mask combines with the masks before it.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaskMode {
    /// Shows what this mask covers as well.
    #[default]
    Add,
    /// Hides what this mask covers.
    Subtract,
    /// Shows only what this mask and the ones before cover.
    Intersect,
}

/// A point of a path mask, with optional Bezier handles relative to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PathPoint {
    /// The point in the mask's box, -1 to 1.
    pub at: [f64; 2],
    /// The handle towards the previous point; a corner when omitted.
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub in_handle: Option<[f64; 2]>,
    /// The handle towards the next point; a corner when omitted.
    #[serde(default, rename = "out", skip_serializing_if = "Option::is_none")]
    pub out_handle: Option<[f64; 2]>,
}

/// One painted stroke of a brush mask.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrushStroke {
    /// The stroke's path in the picture, 0 to 1 from its top-left corner.
    pub points: Vec<[f64; 2]>,
    /// The brush's radius as a fraction of the picture's height.
    pub radius: f64,
    /// Erases instead of painting.
    #[serde(default, skip_serializing_if = "is_default")]
    pub erase: bool,
}

fn yes() -> bool {
    true
}

fn is_yes(value: &bool) -> bool {
    *value
}

fn middle() -> [f64; 2] {
    [0.5, 0.5]
}

fn is_middle(value: &[f64; 2]) -> bool {
    *value == middle()
}

/// A mask on a clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Mask {
    /// Mask id (`mask_…`).
    pub id: Id,
    /// Its shape.
    #[serde(default)]
    pub shape: MaskShape,
    /// False ignores the mask without removing it.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub enabled: bool,
    /// The middle of the mask in the picture, 0 to 1 from the top-left corner (the middle
    /// when omitted).
    #[serde(default = "middle", skip_serializing_if = "is_middle")]
    pub center: [f64; 2],
    /// Width and height as fractions of the picture's (half of each when omitted).
    #[serde(default = "middle", skip_serializing_if = "is_middle")]
    pub size: [f64; 2],
    /// Clockwise rotation in degrees.
    #[serde(default, skip_serializing_if = "is_default")]
    pub rotation: f64,
    /// Softness of the edge, 0 (sharp) to 1, as a fraction of the mask's smaller half size.
    #[serde(default, skip_serializing_if = "is_default")]
    pub feather: f64,
    /// Grows (positive) or shrinks (negative) the shape, -1 to 1, as a fraction of its
    /// smaller half size.
    #[serde(default, skip_serializing_if = "is_default")]
    pub expansion: f64,
    /// Rounds a rectangle's corners, 0 (square) to 1 (fully round).
    #[serde(default, skip_serializing_if = "is_default")]
    pub roundness: f64,
    /// Keeps the outside instead of the inside.
    #[serde(default, skip_serializing_if = "is_default")]
    pub invert: bool,
    /// How it combines with the masks before it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub mode: MaskMode,
    /// Points of a path mask.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<PathPoint>,
    /// Text of a text mask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Strokes of a brush mask.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<BrushStroke>,
    /// Keyframes by property (`center`, `size`, `rotation`, `feather`, `expansion`,
    /// `roundness`), times from the clip's start.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keyframes: BTreeMap<String, Vec<Keyframe>>,
}

/// The animatable properties of a mask: name, whether it is a pair, and its range.
pub const MASK_PROPERTIES: [(&str, bool, f64, f64); 6] = [
    ("center", true, -1.0, 2.0),
    ("size", true, 0.0, 4.0),
    ("rotation", false, f64::NEG_INFINITY, f64::INFINITY),
    ("feather", false, 0.0, 1.0),
    ("expansion", false, -1.0, 1.0),
    ("roundness", false, 0.0, 1.0),
];

impl Mask {
    /// A mask of `shape` with the defaults: in the middle, half the picture's size.
    pub fn new(id: Id, shape: MaskShape) -> Mask {
        Mask {
            id,
            shape,
            enabled: true,
            center: middle(),
            size: middle(),
            rotation: 0.0,
            feather: 0.0,
            expansion: 0.0,
            roundness: 0.0,
            invert: false,
            mode: MaskMode::Add,
            points: Vec::new(),
            text: None,
            strokes: Vec::new(),
            keyframes: BTreeMap::new(),
        }
    }

    fn number_at(&self, name: &str, offset: Flicks, value: f64) -> f64 {
        self.keyframes
            .get(name)
            .and_then(|k| evaluate(k, offset))
            .and_then(KeyValue::number)
            .unwrap_or(value)
    }

    fn pair_at(&self, name: &str, offset: Flicks, value: [f64; 2]) -> [f64; 2] {
        self.keyframes
            .get(name)
            .and_then(|k| evaluate(k, offset))
            .and_then(KeyValue::pair)
            .unwrap_or(value)
    }

    /// The mask at `offset` from the clip's start, with keyframes applied and values kept
    /// in range; the result has no keyframes.
    pub fn at(&self, offset: Flicks) -> Mask {
        Mask {
            center: self.pair_at("center", offset, self.center),
            size: self.pair_at("size", offset, self.size).map(|v| v.max(0.0)),
            rotation: self.number_at("rotation", offset, self.rotation),
            feather: self
                .number_at("feather", offset, self.feather)
                .clamp(0.0, 1.0),
            expansion: self
                .number_at("expansion", offset, self.expansion)
                .clamp(-1.0, 1.0),
            roundness: self
                .number_at("roundness", offset, self.roundness)
                .clamp(0.0, 1.0),
            keyframes: BTreeMap::new(),
            ..self.clone()
        }
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

/// Splits `masks.<id>.<property>` into the mask id and the property.
pub fn mask_path(path: &str) -> Option<(&str, &str)> {
    path.strip_prefix("masks.")?.split_once('.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IdKind, Interpolation};

    #[test]
    fn masks_are_sparse_and_animate() {
        let mut mask = Mask::new(Id::new(IdKind::Mask, 1), MaskShape::Ellipse);
        let json = serde_json::to_value(&mask).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "id": "mask_0001", "shape": "ellipse" })
        );
        mask.keyframes.insert(
            "center".into(),
            vec![
                Keyframe {
                    time: Flicks::ZERO,
                    value: KeyValue::Pair([0.2, 0.5]),
                    interpolation: Interpolation::Linear,
                },
                Keyframe {
                    time: Flicks::from_seconds(2).unwrap(),
                    value: KeyValue::Pair([0.8, 0.5]),
                    interpolation: Interpolation::Linear,
                },
            ],
        );
        mask.keyframes.insert(
            "feather".into(),
            vec![Keyframe {
                time: Flicks::ZERO,
                value: KeyValue::Number(3.0),
                interpolation: Interpolation::Linear,
            }],
        );
        let now = mask.at(Flicks::from_seconds(1).unwrap());
        assert_eq!(now.center, [0.5, 0.5]);
        assert_eq!(now.feather, 1.0, "clamped");
        assert!(now.keyframes.is_empty());
        mask.shift_keyframes(Flicks::from_seconds(1).unwrap());
        assert_eq!(mask.at(Flicks::from_seconds(1).unwrap()).center, [0.2, 0.5]);
        let path: Mask = serde_json::from_str(
            r#"{"id": "mask_0002", "shape": "path", "points": [
                {"at": [-1, -1]}, {"at": [1, -1], "out": [0, 0.5]}, {"at": [0, 1], "in": [0.2, 0]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(path.points[1].out_handle, Some([0.0, 0.5]));
        assert_eq!(path.points[2].in_handle, Some([0.2, 0.0]));
        assert_eq!(
            mask_path("masks.mask_0002.center"),
            Some(("mask_0002", "center"))
        );
        assert_eq!(mask_path("effects.effect_0001.exposure"), None);
    }
}
