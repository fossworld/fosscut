use fosscut_model::{Clip, ClipContent, Id, IdKind, TextContent, TextStyle, TrackKind, Transform};
use fosscut_time::{Flicks, TimeInput};
use schemars::JsonSchema;
use serde::Deserialize;

use super::ClipPlacement;
use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, find_or_create_track,
    insert_sorted, resolve, timeline_id, timeline_mut, track_index,
};
use crate::{ApiError, Command, Context, codes};

/// Adds a text clip (a title or caption).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextAdd {
    /// The text; `\n` separates lines.
    pub text: String,
    /// Target text track; the first text track (created on top if needed) when omitted.
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Timeline position; appended after the track's last clip when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// Length; 3 seconds when omitted.
    #[serde(default)]
    pub duration: Option<TimeInput>,
    /// Appearance; defaults when omitted.
    #[serde(default)]
    pub style: Option<TextStyle>,
    /// Placement; centred when omitted.
    #[serde(default)]
    pub transform: Option<Transform>,
}

impl Command for TextAdd {
    const NAME: &'static str = "text.add";
    const SUMMARY: &'static str =
        "Add a text clip. Never overwrites: fails with fosscut.clip.overlap if the range is taken.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        if self.text.trim().is_empty() {
            return Err(ApiError::new(
                codes::TEXT_EMPTY,
                "text.add: text must not be empty",
            ));
        }
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let duration = match &self.duration {
            Some(duration) => resolve(duration, rate)?,
            None => rate.snap(Flicks::from_seconds(3).unwrap_or(Flicks::ZERO)),
        };
        let track = match &self.track {
            Some(track) => track.clone(),
            None => find_or_create_track(ctx.project, &timeline, TrackKind::Text)?,
        };
        let id = ctx.project.allocate_id(IdKind::Clip);
        let timeline_doc = timeline_mut(ctx.project, &timeline)?;
        let index = track_index(timeline_doc, &track)?;
        let track_doc = &mut timeline_doc.tracks[index];
        let content = ClipContent::Text(TextContent {
            text: self.text,
            style: self.style.unwrap_or_default(),
        });
        ensure_unlocked(track_doc)?;
        ensure_kind(track_doc, &content)?;
        let start = match &self.start {
            Some(start) => resolve(start, rate)?,
            None => track_doc.end(),
        };
        ensure_valid_range(start, duration)?;
        ensure_free(track_doc, start, start + duration, None, rate)?;
        let clip = Clip {
            id,
            start,
            duration,
            content,
            transform: self.transform.unwrap_or_default(),
            opacity: 1.0,
            volume: 1.0,
        };
        let placement = ClipPlacement::of(&clip, &track, rate);
        insert_sorted(track_doc, clip);
        Ok(placement)
    }
}
