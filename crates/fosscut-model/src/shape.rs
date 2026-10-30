//! Shape clips (ADR-0028): a drawn picture of a shape (the mask geometry of ADR-0011 item 4)
//! filled with a colour or gradient, with an optional outline and label. A rectangle as
//! large as the canvas is a background; stickers are shapes with labels.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::is_default;
use crate::mask::{MaskShape, PathPoint};
use crate::text::TextContent;
use crate::timeline::Color;

/// A drawn shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeContent {
    /// The outline: `rectangle` (with `roundness`), `ellipse`, `star`, `heart` or `path`
    /// (through `points`).
    pub shape: MaskShape,
    /// Width and height as fractions of the canvas's; `[1, 1]` covers it.
    pub size: [f64; 2],
    /// Corner rounding of a rectangle, from 0 (square) to 1 (fully round ends).
    #[serde(default, skip_serializing_if = "is_default")]
    pub roundness: f64,
    /// The points of a `path` shape, -1 to 1 across the shape's box.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<PathPoint>,
    /// What fills it.
    #[serde(default)]
    pub fill: Paint,
    /// An outline along its edge.
    #[serde(default, skip_serializing_if = "is_default")]
    pub outline: ShapeOutline,
    /// Text on it, styled as a text clip's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<TextContent>,
    /// Where the label's centre sits in the shape's box, -1 to 1 on each axis; the middle
    /// when omitted.
    #[serde(default, skip_serializing_if = "is_default")]
    pub label_at: [f64; 2],
}

/// A colour, or a gradient from it to a second colour.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Paint {
    /// The colour, where a gradient starts (white when omitted).
    #[serde(default)]
    pub color: Color,
    /// A gradient to a second colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<Gradient>,
}

/// How a gradient runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Gradient {
    /// The colour it ends with.
    pub to: Color,
    /// Its direction in degrees: 0 runs left to right, 90 top to bottom.
    #[serde(default, skip_serializing_if = "is_default")]
    pub angle: f64,
    /// From the centre outwards instead of along a line.
    #[serde(default, skip_serializing_if = "is_default")]
    pub radial: bool,
}

/// An outline along a shape's edge, inside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeOutline {
    /// Whether it is drawn.
    pub enabled: bool,
    /// Its colour (black when omitted).
    #[serde(default = "black", skip_serializing_if = "is_black")]
    pub color: Color,
    /// Its width as a fraction of the shape's shorter side, from 0 to 0.5 (0.04 when
    /// omitted).
    #[serde(
        default = "default_outline",
        skip_serializing_if = "is_default_outline"
    )]
    pub width: f64,
}

fn black() -> Color {
    Color::parse("#000000").unwrap_or_default()
}

fn is_black(color: &Color) -> bool {
    *color == black()
}

fn default_outline() -> f64 {
    0.04
}

fn is_default_outline(value: &f64) -> bool {
    *value == default_outline()
}

impl Default for ShapeOutline {
    fn default() -> ShapeOutline {
        ShapeOutline {
            enabled: false,
            color: black(),
            width: default_outline(),
        }
    }
}

impl ShapeContent {
    /// A shape of `kind`, `size` of the canvas, filled with `color`.
    pub fn new(kind: MaskShape, size: [f64; 2], color: Color) -> ShapeContent {
        ShapeContent {
            shape: kind,
            size,
            roundness: 0.0,
            points: Vec::new(),
            fill: Paint {
                color,
                gradient: None,
            },
            outline: ShapeOutline::default(),
            label: None,
            label_at: [0.0, 0.0],
        }
    }

    /// The shapes a shape clip can be: not the mask-only `linear`, `mirror`, `text` and
    /// `brush`.
    pub fn drawable(kind: MaskShape) -> bool {
        matches!(
            kind,
            MaskShape::Rectangle
                | MaskShape::Ellipse
                | MaskShape::Star
                | MaskShape::Heart
                | MaskShape::Path
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_round_trip_sparsely() {
        let plain = ShapeContent::new(MaskShape::Ellipse, [0.2, 0.2], Color::default());
        let json = serde_json::to_value(&plain).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"shape": "ellipse", "size": [0.2, 0.2], "fill": {"color": "#FFFFFF"}})
        );
        let mut badge = ShapeContent::new(MaskShape::Rectangle, [0.3, 0.1], Color::default());
        badge.roundness = 1.0;
        badge.fill.gradient = Some(Gradient {
            to: Color::parse("#FF0066").unwrap(),
            angle: 90.0,
            radial: false,
        });
        badge.outline.enabled = true;
        badge.label = Some(TextContent::plain("NEW"));
        let back: ShapeContent =
            serde_json::from_value(serde_json::to_value(&badge).unwrap()).unwrap();
        assert_eq!(back, badge);
        assert!(ShapeContent::drawable(MaskShape::Path));
        assert!(!ShapeContent::drawable(MaskShape::Brush));
    }
}
