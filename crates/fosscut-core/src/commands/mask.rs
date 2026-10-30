//! Masks on clips (ADR-0011, item 4): add, change and remove them. Several masks combine
//! in order; their placement animates with `keyframe.set` as `masks.<id>.<property>`.

use fosscut_model::{
    BrushStroke, Clip, ClipContent, Id, IdKind, Mask, MaskMode, MaskShape, PathPoint,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::util::{ensure_unlocked, locate_clip, timeline_mut};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// Fields of a mask a command may set; omitted fields keep their value.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskFields {
    /// The middle of the mask in the picture, `[x, y]` from 0 to 1.
    #[serde(default)]
    pub center: Option<[f64; 2]>,
    /// Width and height as fractions of the picture's.
    #[serde(default)]
    pub size: Option<[f64; 2]>,
    /// Clockwise rotation in degrees.
    #[serde(default)]
    pub rotation: Option<f64>,
    /// Edge softness, 0 to 1.
    #[serde(default)]
    pub feather: Option<f64>,
    /// Grows (positive) or shrinks (negative) the shape, -1 to 1.
    #[serde(default)]
    pub expansion: Option<f64>,
    /// Rounds a rectangle's corners, 0 to 1.
    #[serde(default)]
    pub roundness: Option<f64>,
    /// Keep the outside instead of the inside.
    #[serde(default)]
    pub invert: Option<bool>,
    /// `add` (default), `subtract` or `intersect` with the masks before it.
    #[serde(default)]
    pub mode: Option<MaskMode>,
    /// Points of a path (pen) mask in the mask's box, -1 to 1, with optional Bezier
    /// handles: `[{"at": [x, y], "in": [dx, dy], "out": [dx, dy]}]`.
    #[serde(default)]
    pub points: Option<Vec<PathPoint>>,
    /// Text of a text mask.
    #[serde(default)]
    pub text: Option<String>,
    /// Strokes of a brush mask in the picture, 0 to 1: `[{"points": [[x, y], ...],
    /// "radius": 0.03, "erase": false}]`.
    #[serde(default)]
    pub strokes: Option<Vec<BrushStroke>>,
}

impl MaskFields {
    fn apply(self, command: &str, mask: &mut Mask) -> Result<(), ApiError> {
        let animated = |name: &str| mask.keyframes.contains_key(name);
        for (name, given) in [
            ("center", self.center.is_some()),
            ("size", self.size.is_some()),
            ("rotation", self.rotation.is_some()),
            ("feather", self.feather.is_some()),
            ("expansion", self.expansion.is_some()),
            ("roundness", self.roundness.is_some()),
        ] {
            if given && animated(name) {
                return Err(ApiError::new(
                    codes::CLIP_PROPERTY_ANIMATED,
                    format!("{command}: the mask's {name} has keyframes"),
                )
                .with_hint(format!(
                    "Set it at a time with keyframe.set (property masks.{}.{name}).",
                    mask.id
                )));
            }
        }
        let range = |name: &str, v: f64, low: f64, high: f64| {
            if v.is_finite() && (low..=high).contains(&v) {
                Ok(v)
            } else {
                Err(invalid(format!(
                    "{command}: {name} must be from {low} to {high}, not {v}"
                )))
            }
        };
        if let Some(c) = self.center {
            mask.center = [
                range("center", c[0], -1.0, 2.0)?,
                range("center", c[1], -1.0, 2.0)?,
            ];
        }
        if let Some(s) = self.size {
            mask.size = [
                range("size", s[0], 0.0, 4.0)?,
                range("size", s[1], 0.0, 4.0)?,
            ];
        }
        if let Some(r) = self.rotation {
            mask.rotation = range("rotation", r, -36_000.0, 36_000.0)?;
        }
        if let Some(f) = self.feather {
            mask.feather = range("feather", f, 0.0, 1.0)?;
        }
        if let Some(e) = self.expansion {
            mask.expansion = range("expansion", e, -1.0, 1.0)?;
        }
        if let Some(r) = self.roundness {
            mask.roundness = range("roundness", r, 0.0, 1.0)?;
        }
        if let Some(invert) = self.invert {
            mask.invert = invert;
        }
        if let Some(mode) = self.mode {
            mask.mode = mode;
        }
        if let Some(points) = self.points {
            mask.points = points;
        }
        if let Some(text) = self.text {
            mask.text = Some(text);
        }
        if let Some(strokes) = self.strokes {
            mask.strokes = strokes;
        }
        Ok(())
    }
}

/// A clip's masks after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MasksChanged {
    /// The clip.
    pub clip: Id,
    /// The mask the command added or changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<Id>,
    /// Every mask of the clip, in the order they combine.
    pub masks: Vec<Mask>,
}

fn with_clip(
    ctx: &mut Context<'_>,
    command: &str,
    id: &Id,
    edit: impl FnOnce(&mut Clip) -> Result<Option<Id>, ApiError>,
) -> Result<MasksChanged, ApiError> {
    let location = locate_clip(ctx.project, id)?;
    let timeline = timeline_mut(ctx.project, &location.timeline)?;
    let track = &mut timeline.tracks[location.track];
    ensure_unlocked(track)?;
    let clip = &mut track.clips[location.clip];
    if matches!(clip.content, ClipContent::Audio { .. }) {
        return Err(invalid(format!(
            "{command}: {id} is an audio clip; masks shape pictures"
        )));
    }
    let mask = edit(clip)?;
    Ok(MasksChanged {
        clip: clip.id.clone(),
        mask,
        masks: clip.masks.clone(),
    })
}

fn mask_index(command: &str, clip: &Clip, mask: &Id) -> Result<usize, ApiError> {
    clip.masks
        .iter()
        .position(|m| &m.id == mask)
        .ok_or_else(|| {
            ApiError::new(
                codes::MASK_NOT_FOUND,
                format!("{command}: {} has no mask {mask}", clip.id),
            )
        })
}

/// Adds a mask to a clip.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskAdd {
    /// The clip: a video, image, text, caption or adjustment clip.
    pub clip: Id,
    /// `rectangle`, `ellipse`, `linear`, `mirror`, `star`, `heart`, `path` (pen, needs
    /// points), `text` (needs text) or `brush` (needs strokes).
    pub shape: MaskShape,
    /// The mask's settings; defaults when omitted.
    #[serde(flatten)]
    pub fields: MaskFields,
    /// Position among the clip's masks (0 combines first); last when omitted.
    #[serde(default)]
    pub index: Option<usize>,
}

impl Command for MaskAdd {
    const NAME: &'static str = "mask.add";
    const SUMMARY: &'static str = "Add a mask to a clip's picture: rectangle (round corners), ellipse, linear, mirror (a band), star, heart, path (pen, with Bezier handles), text or brush (painted strokes), placed by center and size as fractions of the picture, with rotation, feather, expansion and invert. Several masks combine in order (add, subtract, intersect); on an adjustment clip they limit where its effects apply.";
    type Output = MasksChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MasksChanged, ApiError> {
        let id = ctx.project.allocate_id(IdKind::Mask);
        with_clip(ctx, Self::NAME, &self.clip, |clip| {
            let mut mask = Mask::new(id.clone(), self.shape);
            if self.shape == MaskShape::Brush {
                // Strokes are drawn in the picture itself, so the box covers all of it.
                mask.size = [1.0, 1.0];
            }
            self.fields.apply(Self::NAME, &mut mask)?;
            let index = self.index.unwrap_or(clip.masks.len());
            if index > clip.masks.len() {
                return Err(invalid(format!(
                    "mask.add: index {index} is beyond the clip's {} masks",
                    clip.masks.len()
                )));
            }
            clip.masks.insert(index, mask);
            Ok(Some(id))
        })
    }
}

/// Changes a mask.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskSet {
    /// The clip.
    pub clip: Id,
    /// The mask (`mask_…`).
    pub mask: Id,
    /// A new shape.
    #[serde(default)]
    pub shape: Option<MaskShape>,
    /// false ignores the mask; true uses it again.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Settings to change.
    #[serde(flatten)]
    pub fields: MaskFields,
}

impl Command for MaskSet {
    const NAME: &'static str = "mask.set";
    const SUMMARY: &'static str = "Change a mask: shape, placement (center, size, rotation), feather, expansion, roundness, invert, mode, path points, text or brush strokes, or turn it off (enabled: false). Animated values are set with keyframe.set as masks.<id>.<property>.";
    type Output = MasksChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MasksChanged, ApiError> {
        with_clip(ctx, Self::NAME, &self.clip, |clip| {
            let index = mask_index(Self::NAME, clip, &self.mask)?;
            let mask = &mut clip.masks[index];
            if let Some(shape) = self.shape {
                mask.shape = shape;
            }
            if let Some(enabled) = self.enabled {
                mask.enabled = enabled;
            }
            self.fields.apply(Self::NAME, mask)?;
            Ok(Some(self.mask.clone()))
        })
    }
}

/// Removes a mask.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskRemove {
    /// The clip.
    pub clip: Id,
    /// The mask.
    pub mask: Id,
}

impl Command for MaskRemove {
    const NAME: &'static str = "mask.remove";
    const SUMMARY: &'static str = "Remove a mask and its keyframes from a clip.";
    type Output = MasksChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MasksChanged, ApiError> {
        with_clip(ctx, Self::NAME, &self.clip, |clip| {
            let index = mask_index(Self::NAME, clip, &self.mask)?;
            clip.masks.remove(index);
            Ok(None)
        })
    }
}
