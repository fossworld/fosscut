//! Shape clips (ADR-0028): a drawn picture of a shape (the mask geometry of ADR-0011 item 4)
//! filled with a colour or gradient, with an optional outline and label. A rectangle as
//! large as the canvas is a background; stickers are shapes with labels.
//!
//! Vector paths (ADR-0036): a path may be open (a line) and have several contours, any
//! shape may carry a stroke centred on its edge with caps, joins and dashes, and trims show
//! only part of the outline, so it can draw itself on. Lengths along a stroke (its width,
//! dashes and dash offset) are fractions of the canvas height, like text sizes; trims are
//! fractions of the outline's whole length.

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
    /// False leaves a `path` shape open: a line from its first point to its last, not
    /// joined back (ADR-0036). Closed when omitted.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub closed: bool,
    /// More contours of a `path` shape after `points`, in the same box: holes, and parts
    /// drawn and trimmed with it, as the subpaths of SVG path data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contours: Vec<Contour>,
    /// Which areas of overlapping contours are filled: `even_odd` (the default) or
    /// `nonzero`, as in SVG.
    #[serde(default, skip_serializing_if = "is_default")]
    pub fill_rule: FillRule,
    /// What fills it: `null` for no fill, as lines have (white when omitted). An open
    /// path's fill closes it with a straight edge.
    #[serde(default = "white_fill")]
    pub fill: Option<Paint>,
    /// An outline along its edge.
    #[serde(default, skip_serializing_if = "is_default")]
    pub outline: ShapeOutline,
    /// A stroke centred on its edge, or along an open path (ADR-0036).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<ShapeStroke>,
    /// Where the visible part of the outline starts, 0 to 1 of its length (ADR-0036):
    /// trims cut the stroke, and the fill of an open path. Animate `shape.trim_start`.
    #[serde(default, skip_serializing_if = "is_default")]
    pub trim_start: f64,
    /// Where the visible part ends, 0 to 1 of the length (1 when omitted); animating it
    /// from 0 to 1 draws the outline on. Animate `shape.trim_end`.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub trim_end: f64,
    /// Moves the visible part along the outline by this fraction of its length, wrapping
    /// around: 0.5 starts it halfway. Animate `shape.trim_offset`.
    #[serde(default, skip_serializing_if = "is_default")]
    pub trim_offset: f64,
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

/// One more contour of a path shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Contour {
    /// Its points, -1 to 1 across the shape's box.
    pub points: Vec<PathPoint>,
    /// False leaves it open (closed when omitted).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub closed: bool,
}

/// Which areas of overlapping contours a fill covers.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FillRule {
    /// Inside an odd number of contours: a contour inside another cuts a hole.
    #[default]
    EvenOdd,
    /// Where the contours wind around a point at all, counted by direction: a contour
    /// inside another cuts a hole only when it runs the other way.
    Nonzero,
}

/// How a stroke ends at the ends of an open path and of dashes.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineCap {
    /// Square, exactly at the end.
    #[default]
    Butt,
    /// A half circle past the end.
    Round,
    /// Square, half the width past the end.
    Square,
}

/// How a stroke turns at corners.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineJoin {
    /// A sharp point, cut off as a bevel past `miter_limit`.
    #[default]
    Miter,
    /// Rounded.
    Round,
    /// Cut off flat.
    Bevel,
}

/// A stroke centred on a shape's edge or along an open path (ADR-0036).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeStroke {
    /// Its colour, where a gradient starts (white when omitted).
    #[serde(default)]
    pub color: Color,
    /// A gradient to a second colour across the shape's box, as a fill's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<Gradient>,
    /// Its width as a fraction of the canvas height, from 0 to 0.5 (0.01 when omitted);
    /// animate `shape.stroke_width`. The clip's scale scales it too.
    #[serde(default = "default_stroke", skip_serializing_if = "is_default_stroke")]
    pub width: f64,
    /// The ends of an open path and of dashes: `butt` (the default), `round` or `square`.
    #[serde(default, skip_serializing_if = "is_default")]
    pub cap: LineCap,
    /// Corners: `miter` (the default), `round` or `bevel`.
    #[serde(default, skip_serializing_if = "is_default")]
    pub join: LineJoin,
    /// How far a miter may reach, in stroke widths, before it is cut off as a bevel; at
    /// least 1 (4 when omitted).
    #[serde(default = "default_miter", skip_serializing_if = "is_default_miter")]
    pub miter_limit: f64,
    /// Dash pattern: lengths of dashes and gaps in turn, as fractions of the canvas height;
    /// solid when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dashes: Vec<f64>,
    /// Where the dash pattern starts, as a fraction of the canvas height.
    #[serde(default, skip_serializing_if = "is_default")]
    pub dash_offset: f64,
}

fn default_stroke() -> f64 {
    0.01
}

fn is_default_stroke(value: &f64) -> bool {
    *value == default_stroke()
}

fn default_miter() -> f64 {
    4.0
}

fn is_default_miter(value: &f64) -> bool {
    *value == default_miter()
}

impl ShapeStroke {
    /// A solid stroke of `color`, `width` of the canvas height, with butt caps and miter
    /// joins.
    pub fn new(color: Color, width: f64) -> ShapeStroke {
        ShapeStroke {
            color,
            gradient: None,
            width,
            cap: LineCap::default(),
            join: LineJoin::default(),
            miter_limit: default_miter(),
            dashes: Vec::new(),
            dash_offset: 0.0,
        }
    }

    /// Its colour and gradient as a paint.
    pub fn paint(&self) -> Paint {
        Paint {
            color: self.color.clone(),
            gradient: self.gradient.clone(),
        }
    }
}

fn yes() -> bool {
    true
}

fn is_yes(value: &bool) -> bool {
    *value
}

fn one() -> f64 {
    1.0
}

fn is_one(value: &f64) -> bool {
    *value == 1.0
}

#[allow(clippy::unnecessary_wraps)]
fn white_fill() -> Option<Paint> {
    Some(Paint::default())
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
            closed: true,
            contours: Vec::new(),
            fill_rule: FillRule::default(),
            fill: Some(Paint {
                color,
                gradient: None,
            }),
            outline: ShapeOutline::default(),
            stroke: None,
            trim_start: 0.0,
            trim_end: 1.0,
            trim_offset: 0.0,
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
        badge.fill.as_mut().unwrap().gradient = Some(Gradient {
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
