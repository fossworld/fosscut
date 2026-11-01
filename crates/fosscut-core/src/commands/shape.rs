//! Shapes, stickers, backgrounds and project templates (ADR-0028): `shape.add` draws a
//! shape or a built-in library item, `shape.set` changes one by a merge patch, and
//! `template.apply` builds a new timeline from a built-in project template with your media
//! and words. Everything they add is an ordinary clip, so it stays editable.

use std::collections::BTreeMap;

use fosscut_model::{
    AssetKind, Clip, ClipContent, Contour, FillRule, Id, IdKind, MaskShape, Paint, PathPoint,
    Project, ShapeContent, ShapeOutline, ShapeStroke, TextContent, Timeline, TimelineEntry,
    TimelineFormat, Track, TrackKind, Transform, shape_issues,
};
use fosscut_time::{Flicks, Rate, TimeInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::caption::merge;
use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, insert_sorted, locate_clip,
    resolve, timeline_id, timeline_mut, track_index,
};
use super::{ClipPlacement, TextTemplate, TransitionSet};
use crate::presets::{LibraryVisual, VisualKind, project_template, visual};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::SHAPE_INVALID, message).with_hint(
        "Shapes are rectangle, ellipse, star, heart or path (with points); capabilities lists the library's shapes, stickers and backgrounds under library.",
    )
}

/// Refuses a shape that breaks the rules of ADR-0028, with the first problem as message.
fn checked(project: &Project, command: &str, shape: &ShapeContent) -> Result<(), ApiError> {
    match shape_issues(project, shape).first() {
        Some(issue) => Err(invalid(format!("{command}: {}", issue.message))),
        None => Ok(()),
    }
}

/// A library item's shape on `format`: items are drawn for a 16:9 canvas, so other canvases
/// keep their proportions (backgrounds always cover the canvas).
fn fitted(item: &LibraryVisual, format: &TimelineFormat) -> ShapeContent {
    let mut shape = item.content.clone();
    if item.kind != VisualKind::Background {
        let design = 1920.0 / 1080.0;
        let canvas = f64::from(format.width.max(1)) / f64::from(format.height.max(1));
        shape.size[0] *= design / canvas;
    }
    shape
}

/// The video track a shape goes on: backgrounds below everything, other shapes above
/// every video track. A track at that end holding only shapes and free over `range` is
/// reused; otherwise a new one is made.
pub(super) fn graphics_track(
    project: &mut Project,
    timeline: &Id,
    range: (Flicks, Flicks),
    below: bool,
) -> Result<Id, ApiError> {
    let doc = timeline_mut(project, timeline)?;
    let end = if below {
        doc.tracks.iter().position(|t| t.kind == TrackKind::Video)
    } else {
        doc.tracks.iter().rposition(|t| t.kind == TrackKind::Video)
    };
    if let Some(index) = end {
        let track = &doc.tracks[index];
        let shapes_only = !track.clips.is_empty()
            && track
                .clips
                .iter()
                .all(|c| matches!(c.content, ClipContent::Shape(_)));
        let free = track
            .clips
            .iter()
            .all(|c| c.end() <= range.0 || c.start >= range.1);
        if (shapes_only || track.clips.is_empty()) && free && !track.locked {
            return Ok(track.id.clone());
        }
    }
    let id = project.allocate_id(IdKind::Track);
    let doc = timeline_mut(project, timeline)?;
    let index = match (end, below) {
        (Some(i), true) => i,
        (Some(i), false) => i + 1,
        (None, _) => 0,
    };
    doc.tracks.insert(
        index,
        Track {
            id: id.clone(),
            kind: TrackKind::Video,
            name: Some(if below { "Backgrounds" } else { "Graphics" }.into()),
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: Vec::new(),
        },
    );
    Ok(id)
}

/// Draws a shape, sticker or background.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeAdd {
    /// A library item to start from: a shape, sticker or background (`capabilities` lists
    /// them under library.visuals).
    #[serde(default)]
    pub item: Option<String>,
    /// The outline: `rectangle`, `ellipse`, `star`, `heart` or `path`; a rectangle when
    /// neither this nor `item` is given.
    #[serde(default)]
    pub shape: Option<MaskShape>,
    /// Width and height as fractions of the canvas's.
    #[serde(default)]
    pub size: Option<[f64; 2]>,
    /// Corner rounding of a rectangle, 0 to 1.
    #[serde(default)]
    pub roundness: Option<f64>,
    /// Points of a path, -1 to 1 across the shape's box.
    #[serde(default)]
    pub points: Option<Vec<PathPoint>>,
    /// SVG path data in canvas pixels (`M 200 800 C 600 200 1300 1000 1700 300`; the M, L,
    /// H, V, C, S, Q, T, A and Z commands): makes a path shape sized and placed where the
    /// data draws it, with its subpaths as contours (ADR-0036). A path whose subpaths are
    /// all open is a line without fill unless `fill` is given.
    #[serde(default)]
    pub d: Option<String>,
    /// False leaves a path open: a line from its first point to its last.
    #[serde(default)]
    pub closed: Option<bool>,
    /// More contours of a path after `points`: holes and separate parts.
    #[serde(default)]
    pub contours: Option<Vec<Contour>>,
    /// Which areas of overlapping contours fill: `even_odd` or `nonzero` (`nonzero` for
    /// `d`, as in SVG; otherwise `even_odd`).
    #[serde(default)]
    pub fill_rule: Option<FillRule>,
    /// Its fill: `{"color": "#FF3366"}`, or with `gradient: {"to", "angle", "radial"}`;
    /// `null` for none, as for a ring drawn by its stroke alone.
    #[serde(default, deserialize_with = "some_value")]
    #[schemars(with = "Option<Paint>")]
    pub fill: Option<Option<Paint>>,
    /// An outline inside its edge.
    #[serde(default)]
    pub outline: Option<ShapeOutline>,
    /// A stroke centred on its edge or along an open path: `{"color", "gradient", "width"
    /// (fraction of the canvas height, 0.01 when omitted), "cap" (butt, round, square),
    /// "join" (miter, round, bevel), "miter_limit", "dashes" and "dash_offset" (fractions
    /// of the canvas height)}`.
    #[serde(default)]
    pub stroke: Option<ShapeStroke>,
    /// Where the visible part of the outline starts, 0 to 1 of its length.
    #[serde(default)]
    pub trim_start: Option<f64>,
    /// Where the visible part ends, 0 to 1 of its length; 0 hides it before a draw-on.
    #[serde(default)]
    pub trim_end: Option<f64>,
    /// Moves the visible part along the outline by this fraction, wrapping around.
    #[serde(default)]
    pub trim_offset: Option<f64>,
    /// Text on it, as a text clip's content (`{"text": "NEW", "style": {...}}`).
    #[serde(default)]
    pub label: Option<TextContent>,
    /// Where the label's centre sits in the shape's box, -1 to 1 on each axis.
    #[serde(default)]
    pub label_at: Option<[f64; 2]>,
    /// Target video track; for backgrounds a track below the footage and for other shapes
    /// one above it (reused or made) when omitted.
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Timeline position; 0 when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// Length; 3 seconds when omitted.
    #[serde(default)]
    pub duration: Option<TimeInput>,
    /// Placement; the library item's or centred when omitted.
    #[serde(default)]
    pub transform: Option<Transform>,
}

impl Command for ShapeAdd {
    const NAME: &'static str = "shape.add";
    const SUMMARY: &'static str = "Draw a shape (rectangle, ellipse, star, heart, a pen path, or SVG path data in canvas pixels as d), a sticker (badges, buttons, arrows, speech bubbles, check marks) or a background (solid or gradient) from the built-in library, with its fill (null for none), gradient, outline, stroke (width, caps, joins, dashes) and trims (trim_end 0 then draw_on draws it on), and label. Open paths are lines. Backgrounds go below the footage, other shapes above it.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let format = ctx.project.timelines[&timeline].format.clone();
        let rate = format.rate;
        let item = self.item.as_deref().map(visual).transpose()?;
        let mut shape = match &item {
            Some(item) => fitted(item, &format),
            None => ShapeContent::new(
                MaskShape::Rectangle,
                [0.3, 0.2],
                fosscut_model::Color::default(),
            ),
        };
        if let Some(kind) = self.shape {
            shape.shape = kind;
        }
        if let Some(size) = self.size {
            shape.size = size;
        }
        if let Some(roundness) = self.roundness {
            shape.roundness = roundness;
        }
        if let Some(points) = self.points {
            shape.points = points;
        }
        let mut placed_at = None;
        if let Some(data) = &self.d {
            let path = vector_path(data, &format)?;
            shape.shape = MaskShape::Path;
            shape.size = path.size;
            shape.closed = path.contours[0].closed;
            shape.points = path.contours[0].points.clone();
            shape.contours = path.contours[1..].to_vec();
            shape.fill_rule = FillRule::Nonzero;
            placed_at = Some(path.position);
        }
        if let Some(closed) = self.closed {
            shape.closed = closed;
        }
        if let Some(contours) = self.contours {
            shape.contours = contours;
        }
        if let Some(rule) = self.fill_rule {
            shape.fill_rule = rule;
        }
        let open = shape.shape == MaskShape::Path
            && !shape.closed
            && shape.contours.iter().all(|c| !c.closed);
        match self.fill {
            Some(fill) => shape.fill = fill,
            // Lines have no fill unless asked for.
            None if open && item.is_none() => shape.fill = None,
            None => {}
        }
        if let Some(outline) = self.outline {
            shape.outline = outline;
        }
        if let Some(stroke) = self.stroke {
            shape.stroke = Some(stroke);
        }
        if let Some(trim) = self.trim_start {
            shape.trim_start = trim;
        }
        if let Some(trim) = self.trim_end {
            shape.trim_end = trim;
        }
        if let Some(trim) = self.trim_offset {
            shape.trim_offset = trim;
        }
        if let Some(label) = self.label {
            shape.label = Some(label);
        }
        if let Some(label_at) = self.label_at {
            shape.label_at = label_at;
        }
        checked(ctx.project, Self::NAME, &shape)?;
        let start = match &self.start {
            Some(start) => resolve(start, rate)?,
            None => Flicks::ZERO,
        };
        let duration = match &self.duration {
            Some(duration) => resolve(duration, rate)?,
            None => rate.snap(Flicks::from_seconds(3).unwrap_or(Flicks::ZERO)),
        };
        ensure_valid_range(start, duration)?;
        let background = item
            .as_ref()
            .is_some_and(|i| i.kind == VisualKind::Background);
        let track = match &self.track {
            Some(track) => track.clone(),
            None => graphics_track(
                ctx.project,
                &timeline,
                (start, start + duration),
                background,
            )?,
        };
        let transform = self.transform.unwrap_or_else(|| Transform {
            position: placed_at
                .or_else(|| item.as_ref().and_then(|i| i.position))
                .unwrap_or([0.0, 0.0]),
            ..Transform::default()
        });
        let id = ctx.project.allocate_id(IdKind::Clip);
        let doc = timeline_mut(ctx.project, &timeline)?;
        let index = track_index(doc, &track)?;
        let track_doc = &mut doc.tracks[index];
        let content = ClipContent::Shape(Box::new(shape));
        ensure_unlocked(track_doc)?;
        ensure_kind(track_doc, &content)?;
        ensure_free(track_doc, start, start + duration, None, rate)?;
        let clip = Clip {
            transform,
            ..Clip::new(id, start, duration, content)
        };
        let placement = ClipPlacement::of(&clip, &track, rate);
        insert_sorted(track_doc, clip);
        Ok(placement)
    }
}

/// Reads a present field as `Some`, so `null` becomes `Some(None)` and an omitted field
/// stays `None` through `#[serde(default)]`.
fn some_value<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A path from SVG path data in canvas pixels: its contours in its box, the box's size as
/// fractions of the canvas, and where its middle is.
struct DrawnPath {
    contours: Vec<fosscut_model::Contour>,
    size: [f64; 2],
    position: [f64; 2],
}

fn vector_path(data: &str, format: &TimelineFormat) -> Result<DrawnPath, ApiError> {
    let fitted = fosscut_model::parse_path_data(data, 1.0)
        .map_err(|e| invalid(format!("shape.add: d: {e}")))?;
    let (width, height) = (
        f64::from(format.width.max(1)),
        f64::from(format.height.max(1)),
    );
    let [x, y, w, h] = fitted.bounds;
    Ok(DrawnPath {
        contours: fitted.contours,
        size: [w / width, h / height],
        position: [(x + w / 2.0) / width - 0.5, (y + h / 2.0) / height - 0.5],
    })
}

/// Changes a shape clip.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeSet {
    /// The shape clip.
    pub clip: Id,
    /// Changes to its shape as a JSON merge patch, for example
    /// `{"fill": {"color": "#00AAFF"}, "label": {"text": "SALE"}}`.
    pub set: Value,
}

/// A shape clip's content after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShapeChanged {
    /// The clip.
    pub clip: Id,
    /// Its shape now; defaults are omitted.
    pub content: ShapeContent,
}

impl Command for ShapeSet {
    const NAME: &'static str = "shape.set";
    const SUMMARY: &'static str = "Change a shape clip's outline, size, roundness, points, contours, fill (null for none), gradient, outline, stroke, trims or label by a merge patch.";
    type Output = ShapeChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ShapeChanged, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let clip =
            &ctx.project.timelines[&location.timeline].tracks[location.track].clips[location.clip];
        let ClipContent::Shape(current) = &clip.content else {
            return Err(invalid(format!(
                "shape.set: {} is a {} clip, not a shape clip",
                self.clip,
                clip.content.kind_name()
            )));
        };
        let mut value = serde_json::to_value(current.as_ref()).unwrap_or_else(|_| json!({}));
        merge(&mut value, &self.set);
        let mut shape: ShapeContent =
            serde_json::from_value(value).map_err(|e| invalid(format!("shape.set: {e}")))?;
        // A merge patch's null removes a field, which would bring back the default white
        // fill: `"fill": null` means no fill (ADR-0036).
        if self.set.get("fill").is_some_and(Value::is_null) {
            shape.fill = None;
        }
        checked(ctx.project, Self::NAME, &shape)?;
        let doc = timeline_mut(ctx.project, &location.timeline)?;
        let track = &mut doc.tracks[location.track];
        ensure_unlocked(track)?;
        track.clips[location.clip].content = ClipContent::Shape(Box::new(shape.clone()));
        Ok(ShapeChanged {
            clip: self.clip,
            content: shape,
        })
    }
}

/// Builds a new timeline from a built-in project template, with your media in its slots
/// and your words in its titles.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateApply {
    /// The template (`capabilities` lists them under library.templates).
    pub template: String,
    /// Video or image assets for the template's slots, in order; slots without media are
    /// left out.
    #[serde(default)]
    pub media: Vec<Id>,
    /// Lines for the template's titles, in order across all of them. A title your lines
    /// reach shows only them; titles they do not reach keep their placeholders.
    #[serde(default)]
    pub lines: Vec<String>,
    /// The new timeline's name; the template's when omitted.
    #[serde(default)]
    pub name: Option<String>,
    /// Make it the primary timeline.
    #[serde(default)]
    pub primary: bool,
}

/// What `template.apply` made.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TemplateApplied {
    /// The new timeline.
    pub timeline: Id,
    /// Its name.
    pub name: String,
    /// The clips it holds, in the order they were made.
    pub clips: Vec<Id>,
}

impl Command for TemplateApply {
    const NAME: &'static str = "template.apply";
    const SUMMARY: &'static str = "Build a new timeline from a project template (vertical-promo, quote-card, photo-montage, product-reveal, talking-head-intro) with your media in its slots and your lines in its titles; everything it adds stays editable.";
    type Output = TemplateApplied;

    fn apply(self, ctx: &mut Context<'_>) -> Result<TemplateApplied, ApiError> {
        let template = project_template(&self.template)?;
        let rate = ctx
            .project
            .primary_timeline()
            .map_or(Rate::FPS_30, |t| t.format.rate);
        let format = TimelineFormat {
            width: template.canvas.width,
            height: template.canvas.height,
            rate,
            sample_rate: 48_000,
        };
        let mut media = Vec::new();
        for id in &self.media {
            let asset = ctx.project.media.get(id).cloned().ok_or_else(|| {
                ApiError::new(codes::ASSET_NOT_FOUND, format!("asset {id} does not exist"))
            })?;
            if !matches!(asset.kind, AssetKind::Video | AssetKind::Image) {
                return Err(ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    format!("template.apply: {id} is not a video or image to put in a slot"),
                ));
            }
            media.push(asset);
        }
        if media.len() > template.slots.len() {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                format!(
                    "template.apply: {} has {} slots, not {}",
                    template.name,
                    template.slots.len(),
                    media.len()
                ),
            ));
        }
        let id = ctx.project.allocate_id(IdKind::Timeline);
        let name = self
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| template.name.clone());
        let mut made = Vec::new();
        // The slots, one after another on the main video track.
        let mut clips = Vec::new();
        let mut at = Flicks::ZERO;
        for (asset, &slot) in media.iter().zip(&template.slots) {
            let length = rate.snap(Flicks::from_millis(slot));
            let available = asset.info.duration.unwrap_or(length);
            let length = if asset.kind == AssetKind::Video {
                length.min(rate.frames_to_flicks(rate.frame_at(available)))
            } else {
                length
            };
            if !length.is_positive() {
                continue;
            }
            let clip_id = ctx.project.allocate_id(IdKind::Clip);
            let content = match asset.kind {
                AssetKind::Image => ClipContent::Image {
                    asset: asset.id.clone(),
                },
                _ => ClipContent::Video {
                    asset: asset.id.clone(),
                    source_in: Flicks::ZERO,
                },
            };
            made.push(clip_id.clone());
            clips.push(Clip::new(clip_id, at, length, content));
            at = at + length;
        }
        let slots_end = at;
        let mut tracks = Vec::new();
        if !clips.is_empty() {
            tracks.push(Track {
                id: ctx.project.allocate_id(IdKind::Track),
                kind: TrackKind::Video,
                name: None,
                muted: false,
                hidden: false,
                locked: false,
                caption_style: None,
                clips,
            });
        }
        ctx.project.manifest.timelines.push(TimelineEntry {
            role: Default::default(),
            id: id.clone(),
            name: name.clone(),
        });
        ctx.project.timelines.insert(
            id.clone(),
            Timeline {
                id: id.clone(),
                name: name.clone(),
                format,
                tracks,
                markers: Vec::new(),
            },
        );
        if self.primary {
            ctx.project.manifest.primary_timeline = id.clone();
        }
        // Transitions between the slots.
        if let Some(kind) = &template.transition {
            let slot_clips: Vec<Id> = made.clone();
            for clip in slot_clips.iter().take(slot_clips.len().saturating_sub(1)) {
                let set = TransitionSet {
                    clip: clip.clone(),
                    transition_type: kind.clone(),
                    duration: Some(TimeInput::Seconds("0.4".into())),
                    params: BTreeMap::new(),
                };
                set.apply(ctx)?;
            }
        }
        // Backgrounds, titles and stickers.
        let mut lines = self.lines.into_iter();
        for element in &template.elements {
            let start = if element.after_slots {
                slots_end
            } else {
                rate.snap(Flicks::from_millis(element.at))
            };
            let length = TimeInput::Flicks(rate.snap(Flicks::from_millis(element.length)).get());
            let start = TimeInput::Flicks(start.get());
            if let Some(title) = &element.title {
                // Your lines fill the titles in order; a title you reach keeps only what you
                // gave it, and one you do not reach keeps its placeholders.
                let chosen: Vec<String> =
                    element.lines.iter().map_while(|_| lines.next()).collect();
                let placed = TextTemplate {
                    template: title.clone(),
                    lines: chosen,
                    track: None,
                    timeline: Some(id.clone()),
                    start: Some(start),
                    duration: Some(length),
                    style: None,
                }
                .apply(ctx)?;
                made.push(placed.clip);
            } else if let Some(item) = element.background.as_ref().or(element.sticker.as_ref()) {
                let placed = ShapeAdd {
                    item: Some(item.clone()),
                    shape: None,
                    size: None,
                    roundness: None,
                    points: None,
                    d: None,
                    closed: None,
                    contours: None,
                    fill_rule: None,
                    fill: None,
                    outline: None,
                    stroke: None,
                    trim_start: None,
                    trim_end: None,
                    trim_offset: None,
                    label: None,
                    label_at: None,
                    track: None,
                    timeline: Some(id.clone()),
                    start: Some(start),
                    duration: Some(length),
                    transform: element.position.map(|position| Transform {
                        position,
                        ..Transform::default()
                    }),
                }
                .apply(ctx)?;
                made.push(placed.clip);
            }
        }
        Ok(TemplateApplied {
            timeline: id,
            name,
            clips: made,
        })
    }
}
