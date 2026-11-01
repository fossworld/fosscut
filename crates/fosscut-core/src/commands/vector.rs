//! Vector paths (ADR-0036): `clip.follow` moves a clip along a shape clip's outline, and
//! `vector.explode` turns an imported SVG drawing into a compound clip of editable shape
//! clips, one per path, so parts of a logo animate separately. Shape properties
//! (`shape.trim_start`, `shape.trim_end`, `shape.trim_offset`, `shape.stroke_width`) and
//! `follow.progress` are set and keyed through `clip.set` and `keyframe.set`.

use std::collections::BTreeMap;

use fosscut_model::{
    AssetKind, Clip, ClipContent, Follow, Id, IdKind, KeyValue, Project, ShapeContent, Timeline,
    TimelineEntry, TimelineFormat, TimelineRole, Track, TrackKind, Transform, follow_placement,
    shape_issues,
};
use fosscut_time::{Flicks, TimeInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, insert_sorted, locate_clip,
    resolve, timeline_id, timeline_mut, track_index,
};
use super::{ClipPlacement, shape::graphics_track};
use crate::{ApiError, Command, Context, codes};

fn follow_error(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::FOLLOW_INVALID, message).with_hint(
        "Follow a shape clip on the same timeline that follows nothing itself; clip.follow with clear: true stops following.",
    )
}

fn vector_error(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::VECTOR_INVALID, message)
}

/// Makes a clip move along the outline of a shape clip, or stops it.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipFollow {
    /// The clip that moves: a video, image, text, shape, caption or compound clip.
    pub clip: Id,
    /// The shape clip whose outline it follows, on the same timeline; it follows nothing
    /// itself. Its own placement and animations carry the path, and the clip's position
    /// becomes the point on it. Needed unless the clip follows one already or `clear` is
    /// set.
    #[serde(default)]
    pub path: Option<Id>,
    /// How far along: 0 at the path's start, 1 at its end, as a fraction of its whole
    /// length; past 1 goes round a closed path again (0 for a new follow). Animate it with
    /// keyframe.set or keyframe.set_all on `follow.progress`, for example from 0 at the
    /// clip's start to 1 at its end.
    #[serde(default)]
    pub progress: Option<f64>,
    /// Turns the clip with the path's direction, added to its rotation (0 degrees runs left
    /// to right).
    #[serde(default)]
    pub orient: Option<bool>,
    /// Added to the point on the path, as fractions of the canvas width and height.
    #[serde(default)]
    pub offset: Option<[f64; 2]>,
    /// Stops following: the clip goes back to its own position, and its progress keyframes
    /// are removed.
    #[serde(default)]
    pub clear: bool,
}

/// A clip's follow after `clip.follow`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Followed {
    /// The clip.
    pub clip: Id,
    /// What it follows now; absent when it follows nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow: Option<Follow>,
    /// Where its centre is at its start: an offset from the canvas centre as fractions of
    /// the canvas width and height, as `transform.position`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 2]>,
    /// Degrees the path turns it at its start, added to its rotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
}

impl Command for ClipFollow {
    const NAME: &'static str = "clip.follow";
    const SUMMARY: &'static str = "Move a clip (a dot, a logo, a title, any picture) along the outline of a shape clip on the same timeline, such as a route drawn as an open path: progress 0 to 1 of the path's length (animate follow.progress with keyframes), orient to turn with the path, offset to sit beside it; clear stops following.";
    type Output = Followed;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Followed, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let doc = &ctx.project.timelines[&location.timeline];
        let clip = &doc.tracks[location.track].clips[location.clip];
        if self.clear {
            if self.path.is_some() || self.progress.is_some() || self.offset.is_some() {
                return Err(follow_error(
                    "clip.follow: clear takes no path, progress or offset",
                ));
            }
        } else {
            if matches!(
                clip.content,
                ClipContent::Audio { .. } | ClipContent::Adjustment
            ) {
                return Err(follow_error(format!(
                    "clip.follow: {} is a {} clip, with no picture to move",
                    clip.id,
                    clip.content.kind_name()
                )));
            }
            let path = self
                .path
                .clone()
                .or_else(|| clip.follow.as_ref().map(|f| f.clip.clone()))
                .ok_or_else(|| {
                    follow_error(format!(
                        "clip.follow: {} follows nothing yet; give the path, a shape clip",
                        clip.id
                    ))
                })?;
            check_path(doc, clip, &path)?;
            if self.progress.is_some() && clip.keyframes.contains_key("follow.progress") {
                return Err(ApiError::new(
                    codes::CLIP_PROPERTY_ANIMATED,
                    format!("clip.follow: follow.progress of {} has keyframes", clip.id),
                )
                .with_hint("Set its value at a time with keyframe.set, or remove its keyframes with keyframe.remove (all: true) first."));
            }
            for value in self.progress.iter().chain(self.offset.iter().flatten()) {
                if !value.is_finite() {
                    return Err(follow_error(
                        "clip.follow: progress and offset must be finite numbers",
                    ));
                }
            }
        }
        let doc = timeline_mut(ctx.project, &location.timeline)?;
        let track = &mut doc.tracks[location.track];
        ensure_unlocked(track)?;
        let clip = &mut track.clips[location.clip];
        if self.clear {
            clip.follow = None;
            clip.keyframes.remove("follow.progress");
        } else {
            let current = clip.follow.take();
            let base = current.unwrap_or(Follow {
                clip: Id::new(IdKind::Clip, 0),
                progress: 0.0,
                orient: false,
                offset: [0.0, 0.0],
            });
            clip.follow = Some(Follow {
                clip: self.path.clone().unwrap_or(base.clip),
                progress: self.progress.unwrap_or(base.progress),
                orient: self.orient.unwrap_or(base.orient),
                offset: self.offset.unwrap_or(base.offset),
            });
        }
        let clip = clip.clone();
        let placed = follow_placement(ctx.project, &clip, Flicks::ZERO);
        Ok(Followed {
            clip: clip.id.clone(),
            follow: clip.follow.clone(),
            position: placed.map(|p| p.position),
            rotation: placed.map(|p| p.rotation),
        })
    }
}

/// Refuses a path `clip` cannot follow on `doc`: missing, on another timeline, not a shape,
/// itself, following something itself, or `clip` being followed already.
fn check_path(doc: &Timeline, clip: &Clip, path: &Id) -> Result<(), ApiError> {
    if path == &clip.id {
        return Err(follow_error("clip.follow: a clip cannot follow itself"));
    }
    let Some((_, target)) = doc.find_clip(path) else {
        return Err(follow_error(format!(
            "clip.follow: {path} is not a clip on the timeline of {}",
            clip.id
        )));
    };
    if !matches!(target.content, ClipContent::Shape(_)) {
        return Err(follow_error(format!(
            "clip.follow: {path} is a {} clip; only shape clips have a path to follow",
            target.content.kind_name()
        )));
    }
    if target.follow.is_some() {
        return Err(follow_error(format!(
            "clip.follow: {path} follows a path itself; follow a shape that stays on its own"
        )));
    }
    let followed = doc
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| c.follow.as_ref().is_some_and(|f| f.clip == clip.id));
    if let Some(other) = followed {
        return Err(follow_error(format!(
            "clip.follow: {} follows {}, so {} cannot follow a path itself",
            other.id, clip.id, clip.id
        )));
    }
    Ok(())
}

/// The plain value of a vector property of `clip` (ADR-0036), as a keyframe value: what
/// `keyframe.set` keeps when it is given no value.
pub(crate) fn plain_value(clip: &Clip, path: &str) -> Option<KeyValue> {
    if path == "follow.progress" {
        return clip.follow.as_ref().map(|f| KeyValue::Number(f.progress));
    }
    let ClipContent::Shape(shape) = &clip.content else {
        return None;
    };
    Some(KeyValue::Number(match path {
        "shape.trim_start" => shape.trim_start,
        "shape.trim_end" => shape.trim_end,
        "shape.trim_offset" => shape.trim_offset,
        "shape.stroke_width" => shape.stroke.as_ref()?.width,
        _ => return None,
    }))
}

/// Sets the plain value of vector property `path` of `clip` to `value`, already checked
/// against the property's range.
pub(crate) fn assign(
    command: &str,
    clip: &mut Clip,
    path: &str,
    value: f64,
) -> Result<(), ApiError> {
    let id = clip.id.clone();
    if path == "follow.progress" {
        let follow = clip.follow.as_mut().ok_or_else(|| {
            follow_error(format!(
                "{command}: {id} follows no path; make it follow one with clip.follow"
            ))
        })?;
        follow.progress = value;
        return Ok(());
    }
    let ClipContent::Shape(shape) = &mut clip.content else {
        return Err(ApiError::new(
            codes::INVALID_ARGUMENTS,
            format!("{command}: {path} is a property of shape clips only"),
        ));
    };
    match path {
        "shape.trim_start" => shape.trim_start = value,
        "shape.trim_end" => shape.trim_end = value,
        "shape.trim_offset" => shape.trim_offset = value,
        "shape.stroke_width" => {
            let stroke = shape.stroke.as_mut().ok_or_else(|| {
                ApiError::new(
                    codes::SHAPE_INVALID,
                    format!("{command}: {id} has no stroke"),
                )
                .with_hint("Give the shape a stroke first: shape.set with {\"stroke\": {\"color\": \"#FFFFFF\", \"width\": 0.01}}.")
            })?;
            stroke.width = value;
        }
        other => {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                format!("{command}: {other} cannot be set"),
            ));
        }
    }
    Ok(())
}

/// Points the follows of clips in `timeline` at the copies `copies` maps their paths to,
/// and drops follows whose path is not on the timeline: after clips are copied with new
/// ids (`timeline.duplicate`, `clips.paste`). Their progress keyframes go with them.
pub(crate) fn remap_follows(timeline: &mut Timeline, copies: &BTreeMap<Id, Id>) {
    let present: std::collections::BTreeSet<Id> = timeline
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.id.clone()))
        .collect();
    for clip in timeline.tracks.iter_mut().flat_map(|t| &mut t.clips) {
        let Some(follow) = &mut clip.follow else {
            continue;
        };
        if let Some(copy) = copies.get(&follow.clip) {
            follow.clip = copy.clone();
        }
        if !present.contains(&follow.clip) {
            clip.follow = None;
            clip.keyframes.remove("follow.progress");
        }
    }
}

/// Stops the clips of `project` that follow one of `removed` from following it, so
/// removing a path clip leaves its followers where their own transforms put them.
pub(crate) fn release_follows(project: &mut Project, removed: &[Id]) {
    for timeline in project.timelines.values_mut() {
        for clip in timeline.tracks.iter_mut().flat_map(|t| &mut t.clips) {
            if clip
                .follow
                .as_ref()
                .is_some_and(|f| removed.contains(&f.clip))
            {
                clip.follow = None;
                clip.keyframes.remove("follow.progress");
            }
        }
    }
}

/// One path of a vector drawing, as a shape.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VectorPart {
    /// The element's id in the drawing, or a name for it; its track is named after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its shape, sized as a fraction of the drawing's width and height, with stroke
    /// lengths as fractions of the drawing's height.
    pub shape: ShapeContent,
    /// Where the middle of its box is: an offset from the drawing's middle as fractions of
    /// its width and height.
    #[serde(default)]
    pub position: [f64; 2],
    /// Its opacity, 0 to 1 (1 when omitted).
    #[serde(default = "full")]
    pub opacity: f64,
}

fn full() -> f64 {
    1.0
}

/// Splits an imported SVG drawing into editable shape clips.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VectorExplode {
    /// The vector asset: an imported `.svg` file.
    pub asset: Id,
    /// Timeline to place the drawing on; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Video track for the compound clip; a graphics track above the footage when omitted.
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline position; 0 when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// Length; 3 seconds when omitted.
    #[serde(default)]
    pub duration: Option<TimeInput>,
    /// Name of the compound timeline that holds the shapes; the asset's name when omitted.
    #[serde(default)]
    pub name: Option<String>,
    /// The drawing's paths as shapes, in drawing order. Front ends read them from the file
    /// for you (the CLI, the MCP edit tool and the desktop do), so give only the asset;
    /// pass parts only for a drawing you parsed yourself.
    #[serde(default)]
    pub parts: Option<Vec<VectorPart>>,
}

/// One shape clip `vector.explode` made.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExplodedPart {
    /// The shape clip.
    pub clip: Id,
    /// Its track in the compound timeline.
    pub track: Id,
    /// The element's id or name in the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// What `vector.explode` made.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Exploded {
    /// The compound clip showing the drawing, where it was placed.
    pub compound: ClipPlacement,
    /// The compound timeline holding the shapes, on a canvas of the drawing's proportions.
    pub timeline: Id,
    /// The shape clips, bottom first, one per path and track.
    pub parts: Vec<ExplodedPart>,
}

impl Command for VectorExplode {
    const NAME: &'static str = "vector.explode";
    const SUMMARY: &'static str = "Split an imported SVG drawing (a logo, an icon, a map) into editable shape clips, one per path with its fill, stroke and placement, in a new compound timeline placed as one compound clip; open it to animate parts separately (draw_on, trims, keyframes, follow).";
    type Output = Exploded;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Exploded, ApiError> {
        let asset = ctx.project.media.get(&self.asset).cloned().ok_or_else(|| {
            ApiError::new(
                codes::ASSET_NOT_FOUND,
                format!("asset {} does not exist", self.asset),
            )
        })?;
        if asset.kind != AssetKind::Vector {
            return Err(vector_error(format!(
                "vector.explode: {} is a {:?} asset, not a vector drawing",
                asset.id, asset.kind
            )));
        }
        let (width, height) = asset
            .info
            .video
            .as_ref()
            .map(|v| (f64::from(v.width.max(1)), f64::from(v.height.max(1))))
            .ok_or_else(|| vector_error(format!("vector.explode: {} has no size", asset.id)))?;
        let parts = self.parts.filter(|p| !p.is_empty()).ok_or_else(|| {
            vector_error(format!(
                "vector.explode: no paths of {} were given",
                asset.id
            ))
            .with_hint("Run vector.explode through the CLI (fosscut vector explode) or the MCP edit tool, which read the paths from the file; a drawing without paths cannot be split.")
        })?;
        for (index, part) in parts.iter().enumerate() {
            if let Some(issue) = shape_issues(ctx.project, &part.shape).first() {
                return Err(vector_error(format!(
                    "vector.explode: part {index}: {}",
                    issue.message
                )));
            }
            let finite = part.position.iter().all(|v| v.is_finite());
            if !finite || !(0.0..=1.0).contains(&part.opacity) {
                return Err(vector_error(format!(
                    "vector.explode: part {index} needs a finite position and an opacity from 0 to 1"
                )));
            }
        }
        let parent = timeline_id(ctx.project, self.timeline.as_ref())?;
        let format = ctx.project.timelines[&parent].format.clone();
        let rate = format.rate;
        let start = match &self.start {
            Some(start) => resolve(start, rate)?,
            None => Flicks::ZERO,
        };
        let duration = match &self.duration {
            Some(duration) => resolve(duration, rate)?,
            None => rate.snap(Flicks::from_seconds(3).unwrap_or(Flicks::ZERO)),
        };
        ensure_valid_range(start, duration)?;
        // The drawing's proportions, as large as the parent canvas holds, so the compound
        // clip shows it one to one when fitted.
        let fit = (f64::from(format.width) / width).min(f64::from(format.height) / height);
        let canvas = TimelineFormat {
            width: ((width * fit).round() as u32).max(1),
            height: ((height * fit).round() as u32).max(1),
            ..format.clone()
        };
        let nested_id = ctx.project.allocate_id(IdKind::Timeline);
        let name = self
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| asset.name.clone());
        let mut nested = Timeline {
            id: nested_id.clone(),
            name: name.clone(),
            format: canvas,
            tracks: Vec::with_capacity(parts.len()),
            markers: Vec::new(),
        };
        let mut made = Vec::with_capacity(parts.len());
        for (index, part) in parts.into_iter().enumerate() {
            let track = ctx.project.allocate_id(IdKind::Track);
            let clip = ctx.project.allocate_id(IdKind::Clip);
            let shape = Clip {
                transform: Transform {
                    position: part.position,
                    ..Transform::default()
                },
                opacity: part.opacity,
                ..Clip::new(
                    clip.clone(),
                    Flicks::ZERO,
                    duration,
                    ClipContent::Shape(Box::new(part.shape)),
                )
            };
            nested.tracks.push(Track {
                id: track.clone(),
                kind: TrackKind::Video,
                name: Some(
                    part.name
                        .clone()
                        .unwrap_or_else(|| format!("path {}", index + 1)),
                ),
                muted: false,
                hidden: false,
                locked: false,
                caption_style: None,
                clips: vec![shape],
            });
            made.push(ExplodedPart {
                clip,
                track,
                name: part.name,
            });
        }
        ctx.project.manifest.timelines.push(TimelineEntry {
            id: nested_id.clone(),
            name,
            role: TimelineRole::Compound,
        });
        ctx.project.timelines.insert(nested_id.clone(), nested);
        let track = match &self.track {
            Some(track) => track.clone(),
            None => graphics_track(ctx.project, &parent, (start, start + duration), false)?,
        };
        let id = ctx.project.allocate_id(IdKind::Clip);
        let doc = timeline_mut(ctx.project, &parent)?;
        let index = track_index(doc, &track)?;
        let track_doc = &mut doc.tracks[index];
        let content = ClipContent::Timeline {
            timeline: nested_id.clone(),
            source_in: Flicks::ZERO,
        };
        ensure_unlocked(track_doc)?;
        ensure_kind(track_doc, &content)?;
        ensure_free(track_doc, start, start + duration, None, rate)?;
        let compound = Clip::new(id, start, duration, content);
        let placement = ClipPlacement::of(&compound, &track, rate);
        insert_sorted(track_doc, compound);
        Ok(Exploded {
            compound: placement,
            timeline: nested_id,
            parts: made,
        })
    }
}

/// What `capabilities` says about vector paths: units, defaults and how the pieces fit.
pub(crate) fn vector_capabilities() -> serde_json::Value {
    json!({
        "units": "Shape sizes are fractions of the canvas; path points run from -1 to 1 across the shape's box; stroke width, dashes and dash offset are fractions of the canvas height; trims and follow progress are fractions of the outline's whole length.",
        "stroke": { "caps": ["butt", "round", "square"], "joins": ["miter", "round", "bevel"], "default_width": 0.01, "default_miter_limit": 4.0 },
        "fill_rules": ["even_odd", "nonzero"],
        "outline_starts": "Rectangles start at the top-left end of their top edge, ellipses at the top, stars at their top point, hearts at the dip between their lobes; all run clockwise. Paths start at their first point.",
        "path_data": "shape.add takes d: SVG path data in canvas pixels (M, L, H, V, C, S, Q, T, A, Z); the shape is sized and placed where the data draws it.",
        "properties": ["shape.trim_start", "shape.trim_end", "shape.trim_offset", "shape.stroke_width", "follow.progress"],
        "animations": ["draw_on", "draw_off"],
        "commands": ["shape.add", "shape.set", "clip.follow", "vector.explode"],
    })
}
