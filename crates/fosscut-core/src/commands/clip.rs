use fosscut_model::{AssetKind, Clip, ClipContent, Id, IdKind};
use fosscut_time::{Flicks, Rate, TimeInput, TimeReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::caption::split_caption;
use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, find_or_create_track,
    free_track, group_members, insert_sorted, locate_clip, resolve, tidy_groups, timeline_id,
    timeline_mut, track_index, track_kind_for,
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
    /// The other clips of its group that the edit changed too (ADR-0024).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked: Vec<ClipPlacement>,
}

impl ClipPlacement {
    pub(crate) fn of(clip: &Clip, track: &Id, rate: Rate) -> ClipPlacement {
        ClipPlacement {
            clip: clip.id.clone(),
            track: track.clone(),
            start: TimeReport::new(clip.start, rate),
            end: TimeReport::new(clip.end(), rate),
            duration: TimeReport::new(clip.duration, rate),
            linked: Vec::new(),
        }
    }
}

fn yes() -> bool {
    true
}

/// Places a section of a media asset on a track.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipAdd {
    /// The asset to place.
    #[serde(default)]
    pub asset: Option<Id>,
    /// A compound timeline to place instead of an asset (ADR-0025).
    #[serde(default)]
    pub compound: Option<Id>,
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
    /// Without `track`: use the first track of the right kind that is free over the clip's
    /// range (`start` is needed), adding one when none is, instead of the first track.
    #[serde(default)]
    pub free_track: bool,
}

impl Command for ClipAdd {
    const NAME: &'static str = "clip.add";
    const SUMMARY: &'static str = "Place a media asset (or, with compound, a compound timeline) on a track. Never overwrites: fails with fosscut.clip.overlap if the range is taken.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let source_in = self
            .source_in
            .as_ref()
            .map(|t| resolve(t, rate))
            .transpose()?
            .unwrap_or(Flicks::ZERO);
        let asset_id = match (&self.asset, &self.compound) {
            (Some(asset), None) => asset.clone(),
            (None, Some(nested)) => {
                let content =
                    super::compound::compound_content(ctx.project, &timeline, nested, source_in)?;
                let total = ctx.project.timelines[nested].duration();
                return place(ctx, &self, &timeline, content, Some(total), nested.as_str());
            }
            _ => {
                return Err(ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    "clip.add: give an asset or a compound timeline, not both",
                ));
            }
        };
        let asset = ctx.project.media.get(&asset_id).cloned().ok_or_else(|| {
            ApiError::new(
                codes::ASSET_NOT_FOUND,
                format!("asset {asset_id} does not exist"),
            )
        })?;
        let content = match asset.kind {
            AssetKind::Video | AssetKind::Web => ClipContent::Video {
                asset: asset.id.clone(),
                source_in,
            },
            AssetKind::Audio => ClipContent::Audio {
                asset: asset.id.clone(),
                source_in,
            },
            AssetKind::Lut => {
                return Err(ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    format!(
                        "clip.add: {} is a LUT; use it in a color.lut effect",
                        asset.id
                    ),
                ));
            }
            AssetKind::Font => {
                return Err(ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    format!(
                        "clip.add: {} is a font; name its family in a text style's font",
                        asset.id
                    ),
                ));
            }
            // A 3D model is an object of its own (ADR-0040).
            AssetKind::Model => {
                return Err(ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    format!(
                        "clip.add: {} is a 3D model; add it with model.add",
                        asset.id
                    ),
                )
                .with_hint(format!(
                    "model.add with object {{\"kind\": \"asset\", \"asset\": \"{}\"}} places it in the timeline's 3D scene.",
                    asset.id
                )));
            }
            // A vector drawing shows as an image, drawn sharp at any size (ADR-0036).
            AssetKind::Image | AssetKind::Vector => {
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
        place(
            ctx,
            &self,
            &timeline,
            content,
            asset.info.duration,
            asset.id.as_str(),
        )
    }
}

/// Places `content` on the timeline for `clip.add`: its length, range check and track.
fn place(
    ctx: &mut Context<'_>,
    args: &ClipAdd,
    timeline: &Id,
    content: ClipContent,
    total: Option<Flicks>,
    label: &str,
) -> Result<ClipPlacement, ApiError> {
    {
        let rate = ctx.project.timelines[timeline].format.rate;
        let source_in = content.source_in().unwrap_or(Flicks::ZERO);
        let duration = match (&args.duration, total) {
            (Some(duration), _) => resolve(duration, rate)?,
            (None, Some(total)) => rate.frames_to_flicks(rate.frame_at(total - source_in)),
            (None, None) => rate.snap(Flicks::from_seconds(5).unwrap_or(Flicks::ZERO)),
        };
        if let Some(total) = total
            && (source_in.is_negative() || source_in + duration > total)
        {
            return Err(ApiError::new(
                codes::CLIP_SOURCE_RANGE,
                format!("the clip would read past the end of {label}"),
            )
            .with_details(json!({
                "asset_duration": TimeReport::new(total, rate),
                "source_in": TimeReport::new(source_in, rate),
                "duration": TimeReport::new(duration, rate),
            })));
        }
        let track = match (&args.track, args.free_track, &args.start) {
            (Some(track), _, _) => track.clone(),
            (None, true, Some(start)) => {
                let start = resolve(start, rate)?;
                free_track(
                    ctx.project,
                    timeline,
                    track_kind_for(&content),
                    (start, start + duration),
                )?
            }
            (None, true, None) => {
                return Err(ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    "clip.add: free_track needs start",
                ));
            }
            (None, false, _) => {
                find_or_create_track(ctx.project, timeline, track_kind_for(&content))?
            }
        };
        let id = ctx.project.allocate_id(IdKind::Clip);
        let timeline_doc = timeline_mut(ctx.project, timeline)?;
        let index = track_index(timeline_doc, &track)?;
        let track_doc = &mut timeline_doc.tracks[index];
        ensure_unlocked(track_doc)?;
        ensure_kind(track_doc, &content)?;
        let start = match &args.start {
            Some(start) => resolve(start, rate)?,
            None => track_doc.end(),
        };
        ensure_valid_range(start, duration)?;
        ensure_free(track_doc, start, start + duration, None, rate)?;
        let clip = Clip::new(id, start, duration, content);
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
    /// Also split the clips of its group that span the cut (the default); their right
    /// parts form a new group (ADR-0024).
    #[serde(default = "yes")]
    pub linked: bool,
}

/// Result of `clip.split`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClipSplitResult {
    /// The left part; it keeps the original id.
    pub left: ClipPlacement,
    /// The right part, with a new id.
    pub right: ClipPlacement,
    /// The clips of its group split at the same time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked: Vec<ClipSplitResult>,
}

impl Command for ClipSplit {
    const NAME: &'static str = "clip.split";
    const SUMMARY: &'static str = "Split a clip at a timeline position. The left part keeps the id; the right part gets a new id and continues the source. Clips of its group spanning the time split too unless linked: false.";
    type Output = ClipSplitResult;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipSplitResult, ApiError> {
        let members = group_members(ctx.project, &self.clip, self.linked);
        let location = locate_clip(ctx.project, &self.clip)?;
        let rate = ctx.project.timelines[&location.timeline].format.rate;
        let at = resolve(&self.at, rate)?;
        let mut result = self.split_one(ctx)?;
        // Members that span the cut split too; the right parts form a new group.
        for member in members {
            let spans = locate_clip(ctx.project, &member).is_ok_and(|l| {
                let c = &ctx.project.timelines[&l.timeline].tracks[l.track].clips[l.clip];
                c.start < at && at < c.end()
            });
            if spans {
                let split = ClipSplit {
                    clip: member,
                    at: TimeInput::Flicks(at.get()),
                    linked: false,
                }
                .split_one(ctx)?;
                result.linked.push(split);
            }
        }
        if !result.linked.is_empty() {
            let group = ctx.project.allocate_id(IdKind::Group);
            let rights: Vec<Id> = std::iter::once(&result)
                .chain(&result.linked)
                .map(|r| r.right.clip.clone())
                .collect();
            let timeline = timeline_mut(ctx.project, &location.timeline)?;
            for clip in timeline.tracks.iter_mut().flat_map(|t| &mut t.clips) {
                if rights.contains(&clip.id) {
                    clip.group = Some(group.clone());
                }
            }
        }
        tidy_groups(timeline_mut(ctx.project, &location.timeline)?);
        Ok(result)
    }
}

impl ClipSplit {
    /// Splits this clip alone; its right part leaves any group.
    pub(crate) fn split_one(self, ctx: &mut Context<'_>) -> Result<ClipSplitResult, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let right_id = ctx.project.allocate_id(IdKind::Clip);
        let effects = ctx.project.timelines[&location.timeline].tracks[location.track].clips
            [location.clip]
            .effects
            .len();
        let effect_ids: Vec<Id> = (0..effects)
            .map(|_| ctx.project.allocate_id(IdKind::Effect))
            .collect();
        let masks = ctx.project.timelines[&location.timeline].tracks[location.track].clips
            [location.clip]
            .masks
            .len();
        let mask_ids: Vec<Id> = (0..masks)
            .map(|_| ctx.project.allocate_id(IdKind::Mask))
            .collect();
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
        let pages = match &original.content {
            ClipContent::Caption(caption) => {
                Some(split_caption(caption, offset).ok_or_else(|| {
                    ApiError::new(
                        codes::CAPTIONS_INVALID_WORDS,
                        format!(
                            "splitting {} at {} would leave a part without words",
                            original.id,
                            TimeReport::new(at, rate).timecode
                        ),
                    )
                    .with_hint("Split between words, or remove the caption clip instead.")
                })?)
            }
            _ => None,
        };
        let mut right = original.clone();
        right.id = right_id;
        // Through the speed map: each part keeps its stretch of the curve (ADR-0023).
        right.retime_part(offset, original.duration - offset);
        right.start = at;
        right.duration = original.duration - offset;
        if let Some((left, right_words)) = pages {
            right.content = ClipContent::Caption(right_words);
            track.clips[location.clip].content = ClipContent::Caption(left);
        }
        // Keyframes keep their place on the content; fades stay at the outer edges.
        right.shift_keyframes(Flicks::ZERO - offset);
        right.effects = right
            .effects
            .into_iter()
            .zip(effect_ids)
            .map(|(mut effect, id)| {
                effect.id = id;
                effect
            })
            .collect();
        right.masks = right
            .masks
            .into_iter()
            .zip(mask_ids)
            .map(|(mut mask, id)| {
                mask.id = id;
                mask
            })
            .collect();
        right.audio.fade_in = Flicks::ZERO;
        right.animations.entrance = None;
        right.group = None;
        track.clips[location.clip].audio.fade_out = Flicks::ZERO;
        // The transition and the exit animation belong to the end, now the right part's.
        track.clips[location.clip].transition = None;
        track.clips[location.clip].animations.exit = None;
        track.clips[location.clip].retime_part(Flicks::ZERO, offset);
        track.clips[location.clip].duration = offset;
        let left = ClipPlacement::of(&track.clips[location.clip], &track.id, rate);
        let right_placement = ClipPlacement::of(&right, &track.id, rate);
        track.clips.insert(location.clip + 1, right);
        Ok(ClipSplitResult {
            left,
            right: right_placement,
            linked: Vec::new(),
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
    /// Move the clips of its group by the same time (the default); only this clip changes
    /// track (ADR-0024).
    #[serde(default = "yes")]
    pub linked: bool,
}

impl Command for ClipMove {
    const NAME: &'static str = "clip.move";
    const SUMMARY: &'static str = "Move a clip to a new position and/or a compatible track; its group moves by the same time unless linked: false. Never overwrites: fails with fosscut.clip.overlap.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        if self.start.is_none() && self.track.is_none() {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "clip.move: give a new start, a new track, or both",
            ));
        }
        let members = group_members(ctx.project, &self.clip, self.linked);
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
        let before = clip.start;
        if let Some(start) = &self.start {
            clip.start = resolve(start, rate)?;
        }
        let shift = clip.start - before;
        // The group moves as one: every member comes out first, so members on one track
        // never collide with each other's old places.
        let mut taken = Vec::new();
        for member in &members {
            let found = timeline.tracks.iter().enumerate().find_map(|(t, track)| {
                track
                    .clips
                    .iter()
                    .position(|c| &c.id == member)
                    .map(|i| (t, i))
            });
            let Some((track, index)) = found else {
                continue;
            };
            ensure_unlocked(&timeline.tracks[track])?;
            taken.push((track, timeline.tracks[track].clips.remove(index)));
        }
        ensure_valid_range(clip.start, clip.duration)?;
        ensure_free(&timeline.tracks[target], clip.start, clip.end(), None, rate)?;
        let mut placement = ClipPlacement::of(&clip, &timeline.tracks[target].id, rate);
        insert_sorted(&mut timeline.tracks[target], clip);
        for (track, mut member) in taken {
            member.start = member.start + shift;
            ensure_valid_range(member.start, member.duration)?;
            ensure_free(
                &timeline.tracks[track],
                member.start,
                member.end(),
                None,
                rate,
            )?;
            placement
                .linked
                .push(ClipPlacement::of(&member, &timeline.tracks[track].id, rate));
            insert_sorted(&mut timeline.tracks[track], member);
        }
        Ok(placement)
    }
}

/// Moves a clip's edges: shorter, or longer into more of its media.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipTrim {
    /// The clip to trim.
    pub clip: Id,
    /// New timeline position of the clip's first frame. What plays under the rest of the
    /// clip stays where it is: media starts later or earlier in its source. Unchanged when
    /// omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// New timeline position just after the clip's last frame; unchanged when omitted.
    #[serde(default)]
    pub end: Option<TimeInput>,
    /// Keep the clip's start where it is and move the later clips on its track by the
    /// change in length, so no gap opens and nothing is covered: trimming the start with
    /// ripple removes the clip's beginning and pulls everything after it earlier.
    #[serde(default)]
    pub ripple: bool,
    /// Trim the same edge of the clips of its group that share it (the default, ADR-0024).
    #[serde(default = "yes")]
    pub linked: bool,
}

impl Command for ClipTrim {
    const NAME: &'static str = "clip.trim";
    const SUMMARY: &'static str = "Move a clip's start or end edge: trim it shorter, or extend it into more of its media. The picture under the rest of the clip stays put. Never overwrites: fails with fosscut.clip.overlap, or fosscut.clip.source_range past the media. With ripple the clip keeps its start and later clips move by the change in length (delete left or right). Group members sharing the edge trim too unless linked: false.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        let members = group_members(ctx.project, &self.clip, self.linked);
        let location = locate_clip(ctx.project, &self.clip)?;
        let before = ctx.project.timelines[&location.timeline].tracks[location.track].clips
            [location.clip]
            .clone();
        let rate = ctx.project.timelines[&location.timeline].format.rate;
        let start = self.start.as_ref().map(|t| resolve(t, rate)).transpose()?;
        let end = self.end.as_ref().map(|t| resolve(t, rate)).transpose()?;
        let ripple = self.ripple;
        let mut placement = self.trim_one(ctx)?;
        // Members sharing a moved edge move it too.
        for member in members {
            let Ok(l) = locate_clip(ctx.project, &member) else {
                continue;
            };
            let clip = &ctx.project.timelines[&l.timeline].tracks[l.track].clips[l.clip];
            let start = start.filter(|s| *s != before.start && clip.start == before.start);
            let end = end.filter(|e| *e != before.end() && clip.end() == before.end());
            if start.is_none() && end.is_none() {
                continue;
            }
            let trimmed = ClipTrim {
                clip: member,
                start: start.map(|s| TimeInput::Flicks(s.get())),
                end: end.map(|e| TimeInput::Flicks(e.get())),
                ripple,
                linked: false,
            }
            .trim_one(ctx)?;
            placement.linked.push(trimmed);
        }
        Ok(placement)
    }
}

impl ClipTrim {
    /// Trims this clip alone.
    fn trim_one(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        if self.start.is_none() && self.end.is_none() {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "clip.trim: give a new start, a new end, or both",
            ));
        }
        let location = locate_clip(ctx.project, &self.clip)?;
        let asset_duration =
            |asset: &Id| ctx.project.media.get(asset).and_then(|a| a.info.duration);
        let original = ctx.project.timelines[&location.timeline].tracks[location.track].clips
            [location.clip]
            .clone();
        let total = match &original.content {
            ClipContent::Video { asset, .. } | ClipContent::Audio { asset, .. } => {
                asset_duration(asset)
            }
            // A compound clip reads its timeline (ADR-0025).
            ClipContent::Timeline { timeline, .. } => {
                ctx.project.timelines.get(timeline).map(|t| t.duration())
            }
            _ => None,
        };
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let rate = timeline.format.rate;
        let track = &mut timeline.tracks[location.track];
        ensure_unlocked(track)?;
        let start = match &self.start {
            Some(start) => resolve(start, rate)?,
            None => original.start,
        };
        let end = match &self.end {
            Some(end) => resolve(end, rate)?,
            None => original.end(),
        };
        let duration = end - start;
        ensure_valid_range(start, duration)?;
        let shift = start - original.start;
        let mut trimmed = original.clone();
        // The edges move in timeline time; the source follows the speed map (ADR-0023).
        trimmed.retime_part(shift, duration);
        trimmed.start = start;
        trimmed.duration = duration;
        // Keyframes stay where they are on the timeline.
        trimmed.shift_keyframes(Flicks::ZERO - shift);
        match &mut trimmed.content {
            ClipContent::Video { source_in, .. }
            | ClipContent::Audio { source_in, .. }
            | ClipContent::Timeline { source_in, .. } => {
                let moved = *source_in;
                let used = trimmed.source_length();
                if moved.is_negative() || total.is_some_and(|total| moved + used > total) {
                    return Err(ApiError::new(
                        codes::CLIP_SOURCE_RANGE,
                        format!("{} would read outside its media", original.id),
                    )
                    .with_details(json!({
                        "source_in": TimeReport::new(moved, rate),
                        "duration": TimeReport::new(duration, rate),
                        "source_used": TimeReport::new(used, rate),
                        "asset_duration": total.map(|t| TimeReport::new(t, rate)),
                    }))
                    .with_hint("Keep the edges within the media: the clip can start no earlier than the media's first frame and end no later than its last."));
                }
            }
            ClipContent::Caption(caption) => {
                *caption = trim_caption(caption, shift, duration).ok_or_else(|| {
                    ApiError::new(
                        codes::CAPTIONS_INVALID_WORDS,
                        format!("trimming {} would leave it without words", original.id),
                    )
                    .with_hint("Keep at least one word inside the clip, or remove the caption clip instead.")
                })?;
            }
            ClipContent::Image { .. }
            | ClipContent::Text(_)
            | ClipContent::Shape(_)
            | ClipContent::Particles { .. }
            | ClipContent::Model { .. }
            | ClipContent::Camera(_)
            | ClipContent::Adjustment => {}
        }
        if self.ripple {
            // The clip stays at its start; what follows moves by the change in length.
            let change = duration - original.duration;
            // Keyframes already follow the content: it moves by as much as the start did.
            trimmed.start = original.start;
            for clip in track.clips.iter_mut().skip(location.clip + 1) {
                clip.start = clip.start + change;
            }
        } else {
            ensure_free(track, start, end, Some(&original.id), rate)?;
        }
        track.clips.remove(location.clip);
        let placement = ClipPlacement::of(&trimmed, &track.id, rate);
        insert_sorted(track, trimmed);
        Ok(placement)
    }
}

/// A caption page after its start moved by `shift` and its length became `duration`:
/// words before the new start or after the new end go, times follow the start. `None`
/// when no word is left.
fn trim_caption(
    caption: &fosscut_model::CaptionContent,
    shift: Flicks,
    duration: Flicks,
) -> Option<fosscut_model::CaptionContent> {
    let mut page = caption.clone();
    if shift.is_positive() {
        page = split_caption(&page, shift)?.1;
    } else if shift.is_negative() {
        for word in page.lines.iter_mut().flat_map(|line| line.words.iter_mut()) {
            word.start = word.start - shift;
            word.end = word.end - shift;
        }
    }
    let length = page
        .lines
        .iter()
        .flat_map(|line| &line.words)
        .map(|word| word.end)
        .max()
        .unwrap_or(Flicks::ZERO);
    if duration < length {
        page = split_caption(&page, duration)?.0;
    }
    Some(page)
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
    /// Remove the clips of its group too (the default, ADR-0024).
    #[serde(default = "yes")]
    pub linked: bool,
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
    /// The clips of its group removed with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked: Vec<Id>,
}

impl Command for ClipRemove {
    const NAME: &'static str = "clip.remove";
    const SUMMARY: &'static str = "Remove a clip, and its group unless linked: false. Clips on the primary (lowest) video track need allow_destructive; ripple closes the gap.";
    type Output = ClipRemoved;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipRemoved, ApiError> {
        let members = group_members(ctx.project, &self.clip, self.linked);
        let ripple = self.ripple;
        let mut removed = self.remove_one(ctx)?;
        for member in members {
            let gone = ClipRemove {
                clip: member,
                ripple,
                linked: false,
            }
            .remove_one(ctx)?;
            removed.shifted.retain(|id| id != &gone.removed);
            removed.linked.push(gone.removed);
        }
        Ok(removed)
    }
}

impl ClipRemove {
    /// Removes this clip alone; a group left with one clip dissolves.
    fn remove_one(self, ctx: &mut Context<'_>) -> Result<ClipRemoved, ApiError> {
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
        let track = track.id.clone();
        tidy_groups(timeline);
        // Clips that followed its path stay where their own transforms put them (ADR-0036).
        super::vector::release_follows(ctx.project, std::slice::from_ref(&removed.id));
        Ok(ClipRemoved {
            removed: removed.id,
            track,
            shifted,
            linked: Vec::new(),
        })
    }
}
