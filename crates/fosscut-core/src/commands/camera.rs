//! Cameras for clips placed in 3D (ADR-0037): `camera.add` puts a camera clip on a video
//! track, and `camera.set` changes one by a merge patch. While a camera clip covers a time,
//! the timeline's 3D clips (`transform.three_d`) are seen through it; its numbers animate
//! with `keyframe.set` as `camera.<field>`.

use fosscut_model::{
    CAMERA_ANIMATABLE, CameraContent, Clip, ClipContent, Id, IdKind, KeyValue, Project, Property,
    PropertyKind, Track, TrackKind, camera_issues,
};
use fosscut_time::{Flicks, TimeInput, TimeReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::caption::merge;
use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, insert_sorted, locate_clip,
    resolve, timeline_id, timeline_mut, track_index,
};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::CAMERA_INVALID, message)
}

/// Refuses a camera with values out of range, naming the first.
fn checked(command: &str, camera: &CameraContent) -> Result<(), ApiError> {
    match camera_issues(camera).first() {
        Some(issue) => Err(invalid(format!("{command}: {issue}"))),
        None => Ok(()),
    }
}

/// A camera clip and its camera.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CameraReport {
    /// The camera clip.
    pub clip: Id,
    /// Its track.
    pub track: Id,
    /// Its timeline.
    pub timeline: Id,
    /// Where it starts.
    pub start: TimeReport,
    /// Where it ends.
    pub end: TimeReport,
    /// Its camera, without keyframes; defaults are omitted.
    pub camera: CameraContent,
}

impl CameraReport {
    fn of(clip: &Clip, track: &Id, timeline: &Id, rate: fosscut_time::Rate) -> CameraReport {
        let camera = match &clip.content {
            ClipContent::Camera(camera) => (**camera).clone(),
            _ => CameraContent::default(),
        };
        CameraReport {
            clip: clip.id.clone(),
            track: track.clone(),
            timeline: timeline.clone(),
            start: TimeReport::new(clip.start, rate),
            end: TimeReport::new(clip.end(), rate),
            camera,
        }
    }
}

/// The video track a camera goes on: the highest video track when it holds only cameras
/// and is free over `range`, otherwise a new `Camera` track above every video track.
fn camera_track(
    project: &mut Project,
    timeline: &Id,
    range: (Flicks, Flicks),
) -> Result<Id, ApiError> {
    let doc = timeline_mut(project, timeline)?;
    let top = doc.tracks.iter().rposition(|t| t.kind == TrackKind::Video);
    if let Some(index) = top {
        let track = &doc.tracks[index];
        let cameras_only = !track.clips.is_empty()
            && track
                .clips
                .iter()
                .all(|c| matches!(c.content, ClipContent::Camera(_)));
        let free = track
            .clips
            .iter()
            .all(|c| c.end() <= range.0 || c.start >= range.1);
        if cameras_only && free && !track.locked {
            return Ok(track.id.clone());
        }
    }
    let id = project.allocate_id(IdKind::Track);
    let doc = timeline_mut(project, timeline)?;
    doc.tracks.insert(
        top.map_or(0, |i| i + 1),
        Track {
            id: id.clone(),
            kind: TrackKind::Video,
            name: Some("Camera".into()),
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: Vec::new(),
        },
    );
    Ok(id)
}

/// Adds a camera clip: the view of the timeline's 3D clips while it lasts.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CameraAdd {
    /// The camera: `field_of_view` (degrees across the canvas width, 40 by default),
    /// `position` and `position_z`, `point_of_interest` and `point_of_interest_z`, `orbit`
    /// (`[yaw, pitch]` degrees around the point of interest), `roll`, and the depth of field
    /// (`depth_of_field: true`, `focus_offset`, `aperture`, `max_blur`). Omitted, the
    /// default camera, which frames the z = 0 plane exactly like 2D.
    #[serde(default)]
    pub camera: Option<CameraContent>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Video track; the highest video track holding only cameras, or a new `Camera` track
    /// above every video track, when omitted.
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline position; 0 when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// Length; to the timeline's end (or 5 seconds on an empty timeline) when omitted.
    #[serde(default)]
    pub duration: Option<TimeInput>,
}

impl Command for CameraAdd {
    const NAME: &'static str = "camera.add";
    const SUMMARY: &'static str = "Add a camera clip: while it lasts, the timeline's 3D clips (transform.three_d true) are seen through it, with its field of view, position and depth (position_z pushes in), point of interest, orbit around it, roll and depth of field. Animate it with keyframe.set on camera.position_z, camera.orbit and the other camera.* properties.";
    type Output = CameraReport;

    fn apply(self, ctx: &mut Context<'_>) -> Result<CameraReport, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let doc = &ctx.project.timelines[&timeline];
        let rate = doc.format.rate;
        let camera = self.camera.unwrap_or_default();
        checked(Self::NAME, &camera)?;
        let start = match &self.start {
            Some(start) => resolve(start, rate)?,
            None => Flicks::ZERO,
        };
        let duration = match &self.duration {
            Some(duration) => resolve(duration, rate)?,
            None => {
                let end = doc.duration();
                if end > start {
                    end - start
                } else {
                    rate.snap(Flicks::from_seconds(5).unwrap_or(Flicks::ZERO))
                }
            }
        };
        ensure_valid_range(start, duration)?;
        let track = match &self.track {
            Some(track) => track.clone(),
            None => camera_track(ctx.project, &timeline, (start, start + duration))?,
        };
        let id = ctx.project.allocate_id(IdKind::Clip);
        let doc = timeline_mut(ctx.project, &timeline)?;
        let index = track_index(doc, &track)?;
        let track_doc = &mut doc.tracks[index];
        let content = ClipContent::Camera(Box::new(camera));
        ensure_unlocked(track_doc)?;
        ensure_kind(track_doc, &content)?;
        ensure_free(track_doc, start, start + duration, None, rate)?;
        let clip = Clip::new(id, start, duration, content);
        let report = CameraReport::of(&clip, &track, &timeline, rate);
        insert_sorted(track_doc, clip);
        Ok(report)
    }
}

/// Changes a camera clip.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CameraSet {
    /// The camera clip.
    pub clip: Id,
    /// Changes to its camera as a JSON merge patch, for example
    /// `{"field_of_view": 60, "depth_of_field": true, "focus_offset": 0.2}`; null returns a
    /// field to its default. Fields with keyframes change through keyframe.set instead.
    pub set: Value,
}

impl Command for CameraSet {
    const NAME: &'static str = "camera.set";
    const SUMMARY: &'static str = "Change a camera clip's field of view, position, position_z, point of interest, orbit, roll or depth of field (depth_of_field, focus_offset, aperture, max_blur) by a merge patch.";
    type Output = CameraReport;

    fn apply(self, ctx: &mut Context<'_>) -> Result<CameraReport, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let doc = timeline_mut(ctx.project, &location.timeline)?;
        let rate = doc.format.rate;
        let track = &mut doc.tracks[location.track];
        ensure_unlocked(track)?;
        let track_id = track.id.clone();
        let clip = &mut track.clips[location.clip];
        let ClipContent::Camera(current) = &clip.content else {
            return Err(ApiError::new(
                codes::CAMERA_NOT_CAMERA,
                format!(
                    "camera.set: {} is a {} clip, not a camera clip",
                    self.clip,
                    clip.content.kind_name()
                ),
            ));
        };
        let mut value = serde_json::to_value(current.as_ref()).unwrap_or_else(|_| json!({}));
        merge(&mut value, &self.set);
        let camera: CameraContent =
            serde_json::from_value(value).map_err(|e| invalid(format!("camera.set: {e}")))?;
        checked(Self::NAME, &camera)?;
        for (path, _) in CAMERA_ANIMATABLE {
            if clip.keyframes.contains_key(*path) && camera.value(path) != current.value(path) {
                return Err(ApiError::new(
                    codes::CLIP_PROPERTY_ANIMATED,
                    format!("camera.set: {path} of {} has keyframes", self.clip),
                )
                .with_hint("Set its value at a time with keyframe.set, or remove its keyframes with keyframe.remove (all: true) first."));
            }
        }
        clip.content = ClipContent::Camera(Box::new(camera));
        Ok(CameraReport::of(clip, &track_id, &location.timeline, rate))
    }
}

/// Sets the camera property `property` of a camera clip to `value` (`clip.set` and
/// keyframe removal).
pub(crate) fn assign(
    command: &str,
    clip: &mut Clip,
    property: &Property,
    value: &Value,
) -> Result<(), ApiError> {
    let ClipContent::Camera(camera) = &mut clip.content else {
        return Err(ApiError::new(
            codes::CAMERA_NOT_CAMERA,
            format!("{command}: {} is not a camera clip", clip.id),
        ));
    };
    let number = |v: &Value| {
        v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| {
            ApiError::new(
                codes::INVALID_ARGUMENTS,
                format!("{command}: {} must be a number, not {v}", property.path),
            )
        })
    };
    let mut next = (**camera).clone();
    match property.kind {
        PropertyKind::Bool if property.path == "camera.depth_of_field" => {
            next.depth_of_field = value.as_bool().ok_or_else(|| {
                ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    format!("{command}: camera.depth_of_field must be true or false"),
                )
            })?;
        }
        PropertyKind::Number => {
            next.set_value(property.path, KeyValue::Number(number(value)?));
        }
        PropertyKind::Pair => {
            let items = value.as_array().filter(|a| a.len() == 2).ok_or_else(|| {
                ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    format!("{command}: {} must be [x, y], not {value}", property.path),
                )
            })?;
            next.set_value(
                property.path,
                KeyValue::Pair([number(&items[0])?, number(&items[1])?]),
            );
        }
        _ => {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                format!("{command}: {} cannot be set", property.path),
            ));
        }
    }
    checked(command, &next)?;
    **camera = next;
    Ok(())
}

/// The plain value of camera property `path` of a camera clip.
pub(crate) fn plain_value(clip: &Clip, path: &str) -> Option<KeyValue> {
    match &clip.content {
        ClipContent::Camera(camera) => camera.value(path),
        _ => None,
    }
}
