//! Splitting every track at once, replacing a clip's media, and aligning a clip on the
//! canvas.

use fosscut_model::{AssetKind, ClipContent, Id, IdKind};
use fosscut_time::{Flicks, TimeInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::clip::{ClipPlacement, ClipSplit, ClipSplitResult};
use super::util::{
    ensure_unlocked, locate_clip, resolve, tidy_groups, timeline_id, timeline_mut, track_index,
};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// Splits every clip under a time, on every unlocked track or the tracks named.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimelineSplit {
    /// Where to cut; snapped to a frame.
    pub at: TimeInput,
    /// Only these tracks; every unlocked track when omitted.
    #[serde(default)]
    pub tracks: Vec<Id>,
    /// The timeline; the primary one when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
}

/// Result of `timeline.split`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SplitAll {
    /// Each clip split, with its two parts.
    pub splits: Vec<ClipSplitResult>,
}

impl Command for TimelineSplit {
    const NAME: &'static str = "timeline.split";
    const SUMMARY: &'static str = "Split all: cut every clip under a time on every unlocked track (or the tracks named) in one step. Grouped clips' right parts form new groups.";
    type Output = SplitAll;

    fn apply(self, ctx: &mut Context<'_>) -> Result<SplitAll, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let doc = &ctx.project.timelines[&timeline];
        let at = resolve(&self.at, doc.format.rate)?;
        let mut chosen = Vec::new();
        for id in &self.tracks {
            let index = track_index(doc, id)?;
            ensure_unlocked(&doc.tracks[index])?;
            chosen.push(index);
        }
        if self.tracks.is_empty() {
            chosen = (0..doc.tracks.len())
                .filter(|i| !doc.tracks[*i].locked)
                .collect();
        }
        // The clips under the cut, with their groups.
        let under: Vec<(Id, Option<Id>)> = chosen
            .iter()
            .flat_map(|i| &doc.tracks[*i].clips)
            .filter(|c| c.start < at && at < c.end())
            .map(|c| (c.id.clone(), c.group.clone()))
            .collect();
        if under.is_empty() {
            return Err(ApiError::new(
                codes::CLIP_SPLIT_OUTSIDE,
                "timeline.split: no clip spans that time on those tracks",
            ));
        }
        let mut splits = Vec::new();
        let mut rights: Vec<(Id, Option<Id>)> = Vec::new();
        for (clip, group) in under {
            let split = ClipSplit {
                clip,
                at: TimeInput::Flicks(at.get()),
                linked: false,
            }
            .split_one(ctx)?;
            rights.push((split.right.clip.clone(), group));
            splits.push(split);
        }
        // The right parts of a group's clips form a new group of their own.
        let mut fresh = std::collections::BTreeMap::new();
        for (_, group) in &rights {
            if let Some(group) = group
                && !fresh.contains_key(group)
            {
                let id = ctx.project.allocate_id(IdKind::Group);
                fresh.insert(group.clone(), id);
            }
        }
        let doc = timeline_mut(ctx.project, &timeline)?;
        for clip in doc.tracks.iter_mut().flat_map(|t| &mut t.clips) {
            if let Some((_, Some(group))) = rights.iter().find(|(id, _)| id == &clip.id) {
                clip.group = fresh.get(group).cloned();
            }
        }
        tidy_groups(doc);
        Ok(SplitAll { splits })
    }
}

/// Puts other media under a clip, keeping its place, length and everything done to it.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipReplace {
    /// The video, audio or image clip.
    pub clip: Id,
    /// The new asset, of a kind the clip can show.
    pub asset: Id,
    /// Where in the new media the clip starts; its start when omitted.
    #[serde(default)]
    pub source_in: Option<TimeInput>,
}

impl Command for ClipReplace {
    const NAME: &'static str = "clip.replace";
    const SUMMARY: &'static str = "Replace a clip's media with another asset, keeping its place, length, speed, effects, masks, keyframes and animations. The new media must be long enough (fosscut.clip.source_range).";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let asset = ctx.project.media.get(&self.asset).cloned().ok_or_else(|| {
            ApiError::new(
                codes::ASSET_NOT_FOUND,
                format!("asset {} does not exist", self.asset),
            )
        })?;
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let rate = timeline.format.rate;
        let track = &mut timeline.tracks[location.track];
        ensure_unlocked(track)?;
        let clip = &mut track.clips[location.clip];
        let source_in = match &self.source_in {
            Some(t) => resolve(t, rate)?,
            None => Flicks::ZERO,
        };
        let fits = match &clip.content {
            ClipContent::Video { .. } => asset.kind == AssetKind::Video,
            ClipContent::Audio { .. } => asset.has_audio(),
            ClipContent::Image { .. } => asset.kind == AssetKind::Image,
            _ => false,
        };
        if !fits {
            return Err(invalid(format!(
                "clip.replace: a {} clip cannot show {:?} asset {}",
                clip.content.kind_name(),
                asset.kind,
                asset.id
            )));
        }
        match &mut clip.content {
            ClipContent::Video {
                asset: a,
                source_in: s,
            }
            | ClipContent::Audio {
                asset: a,
                source_in: s,
            } => {
                *a = asset.id.clone();
                *s = source_in;
            }
            ClipContent::Image { asset: a } => *a = asset.id.clone(),
            _ => {}
        }
        if let (Some(total), false) = (
            asset.info.duration,
            matches!(clip.content, ClipContent::Image { .. }),
        ) && (source_in.is_negative() || source_in + clip.source_length() > total)
        {
            return Err(ApiError::new(
                codes::CLIP_SOURCE_RANGE,
                format!(
                    "clip.replace: {} is too short for {} from that point",
                    asset.name, clip.id
                ),
            )
            .with_hint("Choose an earlier source_in, or trim the clip first."));
        }
        Ok(ClipPlacement::of(clip, &track.id, rate))
    }
}

/// A horizontal placement for `clip.align`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Horizontal {
    /// Against the canvas's left edge.
    Left,
    /// Centred.
    Center,
    /// Against the right edge.
    Right,
}

/// A vertical placement for `clip.align`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Vertical {
    /// Against the top edge.
    Top,
    /// Centred.
    Center,
    /// Against the bottom edge.
    Bottom,
}

/// Moves a clip so its picture lines up with the canvas's edges or centre.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipAlign {
    /// The clip.
    pub clip: Id,
    /// `left`, `center` or `right`; unchanged when omitted.
    #[serde(default)]
    pub horizontal: Option<Horizontal>,
    /// `top`, `center` or `bottom`; unchanged when omitted.
    #[serde(default)]
    pub vertical: Option<Vertical>,
}

/// Result of `clip.align`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Aligned {
    /// The clip.
    pub clip: Id,
    /// Its new `transform.position`.
    pub position: [f64; 2],
}

impl Command for ClipAlign {
    const NAME: &'static str = "clip.align";
    const SUMMARY: &'static str = "Align a clip on the canvas: put the visible part of its picture against the left, right, top or bottom edge, or centre it, taking its scale, crop, flips and rotation into account. Text and captions can only be centred.";
    type Output = Aligned;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Aligned, ApiError> {
        if self.horizontal.is_none() && self.vertical.is_none() {
            return Err(invalid("clip.align: give horizontal, vertical or both"));
        }
        let location = locate_clip(ctx.project, &self.clip)?;
        let timeline = &ctx.project.timelines[&location.timeline];
        let (w, h) = (
            f64::from(timeline.format.width),
            f64::from(timeline.format.height),
        );
        let clip = &timeline.tracks[location.track].clips[location.clip];
        if clip.keyframes.contains_key("transform.position") {
            return Err(ApiError::new(
                codes::CLIP_PROPERTY_ANIMATED,
                format!("clip.align: the position of {} has keyframes", clip.id),
            )
            .with_hint(
                "Set a keyframe with keyframe.set instead, or remove the position keyframes.",
            ));
        }
        let picture = clip
            .content
            .asset()
            .and_then(|a| ctx.project.media.get(a))
            .and_then(|a| a.info.video.as_ref())
            .map(|v| {
                if v.rotation.rem_euclid(180) == 90 {
                    (f64::from(v.height), f64::from(v.width))
                } else {
                    (f64::from(v.width), f64::from(v.height))
                }
            });
        let edges = [
            self.horizontal
                .filter(|x| *x != Horizontal::Center)
                .is_some(),
            self.vertical.filter(|y| *y != Vertical::Center).is_some(),
        ];
        let Some((pw, ph)) = picture else {
            if edges.iter().any(|e| *e) {
                return Err(invalid(format!(
                    "clip.align: {} has no picture size to align to an edge; it can only be centred",
                    clip.id
                )));
            }
            let mut position = clip.transform.position;
            if self.horizontal.is_some() {
                position[0] = 0.0;
            }
            if self.vertical.is_some() {
                position[1] = 0.0;
            }
            return set_position(ctx, &self.clip, position);
        };
        // The visible part's corners around the clip's centre, as the renderer places them.
        let t = clip.transform_at(Flicks::ZERO);
        let fit = (w / pw).min(h / ph);
        let [fx, fy] = t.size_factors();
        let (half_w, half_h) = (pw * fit * fx / 2.0, ph * fit * fy / 2.0);
        let [left, top, right, bottom] = clip.crop_at(Flicks::ZERO).kept();
        let (u0, u1) = if t.flip_horizontal {
            (1.0 - right, 1.0 - left)
        } else {
            (left, right)
        };
        let (v0, v1) = if t.flip_vertical {
            (1.0 - bottom, 1.0 - top)
        } else {
            (top, bottom)
        };
        let (sin, cos) = t.rotation.to_radians().sin_cos();
        let corners: Vec<(f64, f64)> = [(u0, v0), (u1, v0), (u0, v1), (u1, v1)]
            .iter()
            .map(|(u, v)| {
                let (dx, dy) = ((2.0 * u - 1.0) * half_w, (2.0 * v - 1.0) * half_h);
                (dx * cos - dy * sin, dx * sin + dy * cos)
            })
            .collect();
        let span = |f: fn(&(f64, f64)) -> f64| {
            let values: Vec<f64> = corners.iter().map(f).collect();
            (
                values.iter().copied().fold(f64::INFINITY, f64::min),
                values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            )
        };
        let ((min_x, max_x), (min_y, max_y)) = (span(|c| c.0), span(|c| c.1));
        let mut position = clip.transform.position;
        if let Some(horizontal) = self.horizontal {
            let x = match horizontal {
                Horizontal::Left => -w / 2.0 - min_x,
                Horizontal::Center => -(min_x + max_x) / 2.0,
                Horizontal::Right => w / 2.0 - max_x,
            };
            position[0] = x / w;
        }
        if let Some(vertical) = self.vertical {
            let y = match vertical {
                Vertical::Top => -h / 2.0 - min_y,
                Vertical::Center => -(min_y + max_y) / 2.0,
                Vertical::Bottom => h / 2.0 - max_y,
            };
            position[1] = y / h;
        }
        set_position(ctx, &self.clip, position)
    }
}

fn set_position(ctx: &mut Context<'_>, clip: &Id, position: [f64; 2]) -> Result<Aligned, ApiError> {
    let location = locate_clip(ctx.project, clip)?;
    let timeline = timeline_mut(ctx.project, &location.timeline)?;
    let track = &mut timeline.tracks[location.track];
    ensure_unlocked(track)?;
    // Snap away floating-point dust so centred stays exactly zero.
    let position = position.map(|v| (v * 1e9).round() / 1e9);
    track.clips[location.clip].transform.position = position;
    Ok(Aligned {
        clip: clip.clone(),
        position,
    })
}
