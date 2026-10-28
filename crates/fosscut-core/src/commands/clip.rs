use fosscut_model::{AssetKind, Clip, ClipContent, Id, IdKind, Transform};
use fosscut_time::{Flicks, Rate, TimeInput, TimeReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, find_or_create_track,
    insert_sorted, locate_clip, resolve, timeline_id, timeline_mut, track_index, track_kind_for,
};
use crate::{ApiError, Command, Context, codes};

/// Where a clip ended up.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClipPlacement {
    /// Clip id.
    pub clip: Id,
    /// Track id.
    pub track: Id,
    /// First frame on the timeline.
    pub start: TimeReport,
    /// Position just after the last frame.
    pub end: TimeReport,
    /// Length on the timeline.
    pub duration: TimeReport,
}

impl ClipPlacement {
    pub(crate) fn of(clip: &Clip, track: &Id, rate: Rate) -> ClipPlacement {
        ClipPlacement {
            clip: clip.id.clone(),
            track: track.clone(),
            start: TimeReport::new(clip.start, rate),
            end: TimeReport::new(clip.end(), rate),
            duration: TimeReport::new(clip.duration, rate),
        }
    }
}

/// Places a section of a media asset on a track.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipAdd {
    /// The asset to place.
    pub asset: Id,
    /// Target track; the first track of the matching kind (created if needed) when omitted.
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Timeline position; appended after the track's last clip when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// Source in-point inside the asset; its beginning when omitted.
    #[serde(default)]
    pub source_in: Option<TimeInput>,
    /// Length; the rest of the asset (images: 5 seconds) when omitted.
    #[serde(default)]
    pub duration: Option<TimeInput>,
}

impl Command for ClipAdd {
    const NAME: &'static str = "clip.add";
    const SUMMARY: &'static str = "Place a media asset on a track. Never overwrites: fails with fosscut.clip.overlap if the range is taken.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let asset = ctx.project.media.get(&self.asset).cloned().ok_or_else(|| {
            ApiError::new(
                codes::ASSET_NOT_FOUND,
                format!("asset {} does not exist", self.asset),
            )
        })?;
        let source_in = self
            .source_in
            .as_ref()
            .map(|t| resolve(t, rate))
            .transpose()?
            .unwrap_or(Flicks::ZERO);
        let content = match asset.kind {
            AssetKind::Video => ClipContent::Video {
                asset: asset.id.clone(),
                source_in,
            },
            AssetKind::Audio => ClipContent::Audio {
                asset: asset.id.clone(),
                source_in,
            },
            AssetKind::Image => {
                if source_in != Flicks::ZERO {
                    return Err(ApiError::new(
                        codes::INVALID_ARGUMENTS,
                        "clip.add: images have no source_in",
                    ));
                }
                ClipContent::Image {
                    asset: asset.id.clone(),
                }
            }
        };
        let duration = match (&self.duration, asset.info.duration) {
            (Some(duration), _) => resolve(duration, rate)?,
            (None, Some(total)) => rate.frames_to_flicks(rate.frame_at(total - source_in)),
            (None, None) => rate.snap(Flicks::from_seconds(5).unwrap_or(Flicks::ZERO)),
        };
        if let Some(total) = asset.info.duration
            && (source_in.is_negative() || source_in + duration > total)
        {
            return Err(ApiError::new(
                codes::CLIP_SOURCE_RANGE,
                format!("the clip would read past the end of {}", asset.id),
            )
            .with_details(json!({
                "asset_duration": TimeReport::new(total, rate),
                "source_in": TimeReport::new(source_in, rate),
                "duration": TimeReport::new(duration, rate),
            })));
        }
        let track = match &self.track {
            Some(track) => track.clone(),
            None => find_or_create_track(ctx.project, &timeline, track_kind_for(&content))?,
        };
        let id = ctx.project.allocate_id(IdKind::Clip);
        let timeline_doc = timeline_mut(ctx.project, &timeline)?;
        let index = track_index(timeline_doc, &track)?;
        let track_doc = &mut timeline_doc.tracks[index];
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
            transform: Transform::default(),
            opacity: 1.0,
            volume: 1.0,
        };
        let placement = ClipPlacement::of(&clip, &track, rate);
        insert_sorted(track_doc, clip);
        Ok(placement)
    }
}

/// Splits a clip in two at a timeline position.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipSplit {
    /// The clip to split.
    pub clip: Id,
    /// Timeline position of the cut, strictly inside the clip; snapped to the nearest frame.
    pub at: TimeInput,
}

/// Result of `clip.split`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClipSplitResult {
    /// The left part; it keeps the original id.
    pub left: ClipPlacement,
    /// The right part, with a new id.
    pub right: ClipPlacement,
}

impl Command for ClipSplit {
    const NAME: &'static str = "clip.split";
    const SUMMARY: &'static str = "Split a clip at a timeline position. The left part keeps the id; the right part gets a new id and continues the source.";
    type Output = ClipSplitResult;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipSplitResult, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let right_id = ctx.project.allocate_id(IdKind::Clip);
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let rate = timeline.format.rate;
        let at = resolve(&self.at, rate)?;
        let track = &mut timeline.tracks[location.track];
        ensure_unlocked(track)?;
        let original = track.clips[location.clip].clone();
        if !(original.start < at && at < original.end()) {
            return Err(ApiError::new(
                codes::CLIP_SPLIT_OUTSIDE,
                format!(
                    "{} is not strictly inside {}",
                    TimeReport::new(at, rate).timecode,
                    original.id
                ),
            )
            .with_details(json!({
                "clip_start": TimeReport::new(original.start, rate),
                "clip_end": TimeReport::new(original.end(), rate),
            })));
        }
        let offset = at - original.start;
        let mut right = original.clone();
        right.id = right_id;
        right.start = at;
        right.duration = original.duration - offset;
        match &mut right.content {
            ClipContent::Video { source_in, .. } | ClipContent::Audio { source_in, .. } => {
                *source_in = *source_in + offset
            }
            ClipContent::Image { .. } | ClipContent::Text(_) => {}
        }
        track.clips[location.clip].duration = offset;
        let left = ClipPlacement::of(&track.clips[location.clip], &track.id, rate);
        let right_placement = ClipPlacement::of(&right, &track.id, rate);
        track.clips.insert(location.clip + 1, right);
        Ok(ClipSplitResult {
            left,
            right: right_placement,
        })
    }
}

/// Moves a clip to another position and/or track of the same timeline.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipMove {
    /// The clip to move.
    pub clip: Id,
    /// New timeline position; unchanged when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// Target track in the same timeline; unchanged when omitted.
    #[serde(default)]
    pub track: Option<Id>,
}

impl Command for ClipMove {
    const NAME: &'static str = "clip.move";
    const SUMMARY: &'static str = "Move a clip to a new position and/or a compatible track. Never overwrites: fails with fosscut.clip.overlap.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        if self.start.is_none() && self.track.is_none() {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "clip.move: give a new start, a new track, or both",
            ));
        }
        let location = locate_clip(ctx.project, &self.clip)?;
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let rate = timeline.format.rate;
        let target = match &self.track {
            Some(track) => track_index(timeline, track)?,
            None => location.track,
        };
        ensure_unlocked(&timeline.tracks[location.track])?;
        ensure_unlocked(&timeline.tracks[target])?;
        let mut clip = timeline.tracks[location.track].clips.remove(location.clip);
        ensure_kind(&timeline.tracks[target], &clip.content)?;
        if let Some(start) = &self.start {
            clip.start = resolve(start, rate)?;
        }
        ensure_valid_range(clip.start, clip.duration)?;
        ensure_free(&timeline.tracks[target], clip.start, clip.end(), None, rate)?;
        let placement = ClipPlacement::of(&clip, &timeline.tracks[target].id, rate);
        insert_sorted(&mut timeline.tracks[target], clip);
        Ok(placement)
    }
}

/// Removes a clip, optionally closing the gap.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipRemove {
    /// The clip to remove.
    pub clip: Id,
    /// Shift later clips on the same track left to close the gap.
    #[serde(default)]
    pub ripple: bool,
}

/// Result of `clip.remove`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClipRemoved {
    /// The removed clip id.
    pub removed: Id,
    /// The track it was on.
    pub track: Id,
    /// Clips shifted left by the ripple.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shifted: Vec<Id>,
}

impl Command for ClipRemove {
    const NAME: &'static str = "clip.remove";
    const SUMMARY: &'static str = "Remove a clip. Clips on the primary (lowest) video track need allow_destructive; ripple closes the gap.";
    type Output = ClipRemoved;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipRemoved, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let timeline = &ctx.project.timelines[&location.timeline];
        let track = &timeline.tracks[location.track];
        ensure_unlocked(track)?;
        let primary_video = timeline
            .tracks
            .iter()
            .position(|t| t.kind == fosscut_model::TrackKind::Video);
        if primary_video == Some(location.track) {
            ctx.require_destructive(
                "Removing a clip from the primary video track",
                json!({ "clip": self.clip, "track": track.id }),
            )?;
        }
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let track = &mut timeline.tracks[location.track];
        let removed = track.clips.remove(location.clip);
        let mut shifted = Vec::new();
        if self.ripple {
            for clip in track.clips.iter_mut().skip(location.clip) {
                clip.start = clip.start - removed.duration;
                shifted.push(clip.id.clone());
            }
        }
        Ok(ClipRemoved {
            removed: removed.id,
            track: track.id.clone(),
            shifted,
        })
    }
}
