//! Canvas fills (CapCut parity CC-063): what shows around a video or image clip whose
//! picture does not cover the canvas, such as a 16:9 shot in a vertical video.

use fosscut_model::{CanvasFill, ClipContent, Id};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::util::{ensure_unlocked, locate_clip, timeline_mut};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::CANVAS_INVALID, message)
}

/// Sets or removes the canvas fill of clips.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipCanvas {
    /// Video or image clips, of any timelines.
    pub clips: Vec<Id>,
    /// `{"kind": "blur", "amount": 0.05}` for the clip's own picture blurred to cover the
    /// canvas (amount from 0 to 0.2 of the picture's height), `{"kind": "color", "color":
    /// "#202020"}` for a colour; omitted or null removes the fill.
    #[serde(default)]
    pub fill: Option<CanvasFill>,
}

/// What `clip.canvas` changed.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CanvasChanged {
    /// The clips.
    pub clips: Vec<Id>,
    /// Their fill now; none when removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<CanvasFill>,
}

impl Command for ClipCanvas {
    const NAME: &'static str = "clip.canvas";
    const SUMMARY: &'static str = "Fill the canvas around video or image clips whose picture does not cover it (a 16:9 shot in a vertical video): with their own picture blurred to cover, or a colour; no fill removes it. Several clips at once.";
    type Output = CanvasChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<CanvasChanged, ApiError> {
        if self.clips.is_empty() {
            return Err(invalid("clip.canvas: name one or more clips"));
        }
        if let Some(CanvasFill::Blur { amount }) = &self.fill
            && !(amount.is_finite() && (0.0..=0.2).contains(amount))
        {
            return Err(invalid(format!(
                "clip.canvas: a blur's amount is from 0 to 0.2, not {amount}"
            )));
        }
        let mut clips = self.clips.clone();
        clips.dedup();
        for id in &clips {
            let location = locate_clip(ctx.project, id)?;
            let timeline = timeline_mut(ctx.project, &location.timeline)?;
            let track = &mut timeline.tracks[location.track];
            ensure_unlocked(track)?;
            let clip = &mut track.clips[location.clip];
            if !matches!(
                clip.content,
                ClipContent::Video { .. } | ClipContent::Image { .. }
            ) {
                return Err(invalid(format!(
                    "clip.canvas: {id} is a {} clip; only video and image clips have a canvas fill",
                    clip.content.kind_name()
                )));
            }
            clip.canvas = self.fill.clone();
        }
        Ok(CanvasChanged {
            clips,
            fill: self.fill,
        })
    }
}
