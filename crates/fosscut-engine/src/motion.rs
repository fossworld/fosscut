//! Measured motion as keyframes (ADR-0030) and multicam sync (ADR-0029 item 4).
//!
//! - [`stabilize`] measures how the camera moved through a clip's media and returns the
//!   `keyframe.set_all` calls that steady it through the clip's transform (CC-031);
//! - [`track`] follows a region of a clip's picture from a time on and returns the calls that
//!   make one of its masks (mask tracking) or another clip (motion tracking, for a title or
//!   sticker) follow it;
//! - [`sync_offsets`] measures how far apart angles recorded at the same time started, from
//!   their sound, for `multicam.create`.
//!
//! Like the other measurements in the engine, these read media and propose commands; the
//! front ends apply them in one request, so the result is undoable and visible.

use std::path::Path;

use fosscut_analysis::{MotionEstimator, Tracker, audio_offset, scenes::Luma, stabilize as steady};

pub use fosscut_analysis::Region;
use fosscut_media::ffmpeg::{FfmpegAudioDecoder, FfmpegVideoDecoder};
use fosscut_media::{DecoderOptions, MediaError, VideoDecoder, VideoFrame};
use fosscut_model::{Clip, ClipContent, Id, Project, Timeline};
use fosscut_render::geometry::clockwise_rotation;
use fosscut_time::Flicks;
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;
use crate::export::resolve;

/// The strength `stabilize` uses when none is given.
pub const DEFAULT_STRENGTH: f64 = 0.5;

/// The sample rate sound is compared at for multicam sync.
const SYNC_RATE: u32 = 16_000;

/// What a measurement found, and the calls that apply it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MotionPlan {
    /// The clip whose keyframes the calls set.
    pub clip: Id,
    /// Frames measured.
    pub frames: usize,
    /// For stabilization: the zoom that hides the moving edges, from 1 to 1.5.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f64>,
    /// For tracking: where the region was lost, as timeline ranges.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lost: Vec<Lost>,
    /// `keyframe.set_all` calls, one per property.
    pub commands: Vec<Value>,
}

/// A timeline range where tracking lost the region and held its last position.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Lost {
    /// Where it was lost.
    pub start: Flicks,
    /// Where it was found again, or the end of the tracking.
    pub end: Flicks,
}

/// What follows a tracked region.
#[derive(Clone, Debug, PartialEq)]
pub enum TrackTarget {
    /// A mask of the tracked clip: its centre follows the region.
    Mask(Id),
    /// Another clip, such as a title or sticker: its position moves as the region does.
    Clip(Id),
}

/// A clip with what the measurements need about it.
struct Source<'a> {
    timeline: &'a Timeline,
    clip: &'a Clip,
    asset: Id,
    /// The stored frame's size and its clockwise display rotation.
    size: (u32, u32),
    rotation: u16,
}

fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::InvalidArguments {
        message: message.into(),
    }
}

fn find<'a>(project: &'a Project, clip: &Id) -> Result<(&'a Timeline, &'a Clip), EngineError> {
    project
        .timelines
        .values()
        .flat_map(|t| {
            t.tracks
                .iter()
                .flat_map(move |track| track.clips.iter().map(move |c| (t, c)))
        })
        .find(|(_, c)| &c.id == clip)
        .ok_or_else(|| invalid(format!("clip {clip} does not exist")))
}

fn source<'a>(project: &'a Project, clip: &Id, what: &str) -> Result<Source<'a>, EngineError> {
    let (timeline, found) = find(project, clip)?;
    let ClipContent::Video { asset, .. } = &found.content else {
        return Err(invalid(format!(
            "{clip} is a {} clip; {what} works on video clips",
            found.content.kind_name()
        )));
    };
    let video = project
        .media
        .get(asset)
        .and_then(|a| a.info.video.as_ref())
        .ok_or_else(|| EngineError::NoVideo {
            asset: asset.to_string(),
        })?;
    Ok(Source {
        timeline,
        clip: found,
        asset: asset.clone(),
        size: (video.width.max(1), video.height.max(1)),
        rotation: clockwise_rotation(video.rotation),
    })
}

impl Source<'_> {
    /// The picture's size as displayed, before fitting.
    fn displayed(&self) -> (f64, f64) {
        let (w, h) = (f64::from(self.size.0), f64::from(self.size.1));
        if self.rotation % 180 == 90 {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// The fitted picture's width and height as fractions of the canvas's.
    fn fitted(&self) -> (f64, f64) {
        let format = &self.timeline.format;
        let (canvas_w, canvas_h) = (
            f64::from(format.width.max(1)),
            f64::from(format.height.max(1)),
        );
        let (w, h) = self.displayed();
        let fit = (canvas_w / w).min(canvas_h / h);
        (w * fit / canvas_w, h * fit / canvas_h)
    }

    /// The source times the clip plays, from its first frame to its last.
    fn source_range(&self) -> (Flicks, Flicks) {
        let first = self.clip.source_at(Flicks::ZERO).unwrap_or(Flicks::ZERO);
        let last = self.clip.source_at(self.clip.duration).unwrap_or(first);
        (first.min(last), first.max(last))
    }

    /// The offset from the clip's start, on a timeline frame, where source time `pts` plays.
    fn offset_of(&self, pts: Flicks) -> Option<Flicks> {
        let offset = self.clip.offset_of_source(pts)?;
        let rate = self.timeline.format.rate;
        let at = rate.snap(self.clip.start + offset) - self.clip.start;
        (!at.is_negative() && at < self.clip.duration).then_some(at)
    }

    /// Decodes the frames the clip plays from source time `from`, in order, calling `each`
    /// with each frame's time and luma plane until it returns `false`.
    fn frames(
        &self,
        project: &Project,
        bundle: &Path,
        from: Flicks,
        each: &mut dyn FnMut(Flicks, Luma<'_>) -> bool,
    ) -> Result<(), EngineError> {
        let path = resolve(project, bundle, &self.asset)?;
        let mut decoder = FfmpegVideoDecoder::open(&path, DecoderOptions::default())
            .map_err(|e| no_video(e, &self.asset))?;
        let (_, end) = self.source_range();
        let mut frame: Option<VideoFrame> = Some(decoder.frame_at(from)?);
        while let Some(current) = frame {
            if current.pts > end {
                break;
            }
            if let Some(plane) = current.planes.first() {
                let luma = Luma {
                    width: current.width,
                    height: current.height,
                    stride: plane.stride,
                    data: &plane.data,
                };
                if !each(current.pts, luma) {
                    return Err(EngineError::AnalysisCancelled);
                }
            }
            frame = decoder.next_frame()?;
        }
        Ok(())
    }
}

fn no_video(error: MediaError, asset: &Id) -> EngineError {
    match error {
        MediaError::NoVideoStream { .. } => EngineError::NoVideo {
            asset: asset.to_string(),
        },
        other => EngineError::from(other),
    }
}

/// A vector of the stored frame turned as the picture is displayed.
fn displayed_vector(v: (f64, f64), rotation: u16) -> (f64, f64) {
    match rotation {
        90 => (-v.1, v.0),
        180 => (-v.0, -v.1),
        270 => (v.1, -v.0),
        _ => v,
    }
}

/// A point of the stored frame, in fractions, where it lands in the displayed picture.
fn displayed_point(p: (f64, f64), rotation: u16) -> (f64, f64) {
    match rotation {
        90 => (1.0 - p.1, p.0),
        180 => (1.0 - p.0, 1.0 - p.1),
        270 => (p.1, 1.0 - p.0),
        _ => p,
    }
}

/// Keeps one key per offset, the last measured.
fn dedup_keys(keys: &mut Vec<(Flicks, Value)>) {
    keys.dedup_by(|later, earlier| {
        if later.0 == earlier.0 {
            earlier.1 = later.1.clone();
            true
        } else {
            false
        }
    });
}

fn set_all(clip: &Id, property: &str, keys: &[(Flicks, Value)]) -> Value {
    let keys: Vec<Value> = keys
        .iter()
        .map(|(offset, value)| json!({ "offset": { "flicks": offset.get() }, "value": value }))
        .collect();
    json!({
        "command": "keyframe.set_all",
        "args": { "clip": clip, "property": property, "keys": keys },
    })
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// Measures the camera's motion through `clip` and returns the `keyframe.set_all` calls
/// for its `transform.position`, `transform.rotation` and `transform.scale` that steady
/// it: `strength` from 0 (no change) to 1 (very smooth; 3 accepted for a locked-off look)
/// scales how much of the motion is removed, and one zoom hides the moving edges. Existing
/// keyframes of those properties are replaced; the clip's own placement is kept around the
/// correction.
pub fn stabilize(
    project: &Project,
    bundle: &Path,
    clip: &Id,
    strength: f64,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<MotionPlan, EngineError> {
    if !strength.is_finite() || !(0.0..=3.0).contains(&strength) {
        return Err(invalid(format!(
            "stabilize: strength {strength} is outside 0 to 3"
        )));
    }
    let source = source(project, clip, "stabilization")?;
    let total = frame_count(&source);
    let mut estimator = MotionEstimator::new();
    let mut done = 0u64;
    let (from, _) = source.source_range();
    source.frames(project, bundle, from, &mut |pts, luma| {
        estimator.push(pts, luma);
        done += 1;
        !done.is_multiple_of(10) || progress(crate::Progress { done, total })
    })?;
    let path = estimator.finish();
    let frames = path.frames.len();
    if frames < 2 {
        return Err(invalid(format!("{clip} has too few frames to stabilize")));
    }
    let steadied = steady(&path, strength);
    let zoom = steadied.zoom;
    let transform = &source.clip.transform;
    let (fit_w, fit_h) = source.fitted();
    let (stored_w, stored_h) = (f64::from(source.size.0), f64::from(source.size.1));
    let (display_w, display_h) = source.displayed();
    let [stretch_x, stretch_y] = transform.stretch;
    let (sin, cos) = transform.rotation.to_radians().sin_cos();
    let mut position = Vec::with_capacity(frames);
    let mut rotation = Vec::with_capacity(frames);
    let mut scale = Vec::with_capacity(frames);
    for correction in &steadied.frames {
        let Some(offset) = source.offset_of(correction.pts) else {
            continue;
        };
        // The move in stored pixels, turned as the picture is displayed, as fractions of
        // the displayed picture, then of the canvas through the fit and the clip's scale,
        // turned by the clip's rotation.
        let moved = displayed_vector(
            (
                zoom * correction.dx * stored_w,
                zoom * correction.dy * stored_h,
            ),
            source.rotation,
        );
        let (mx, my) = (
            moved.0 / display_w * fit_w * transform.scale * stretch_x,
            moved.1 / display_h * fit_h * transform.scale * stretch_y,
        );
        let (px, py) = (mx * cos - my * sin, mx * sin + my * cos);
        position.push((
            offset,
            json!([
                round(transform.position[0] + px, 6),
                round(transform.position[1] + py, 6)
            ]),
        ));
        rotation.push((
            offset,
            json!(round(transform.rotation + correction.rotation, 4)),
        ));
        scale.push((
            offset,
            json!(round(transform.scale * correction.scale * zoom, 6)),
        ));
    }
    for keys in [&mut position, &mut rotation, &mut scale] {
        dedup_keys(keys);
    }
    progress(crate::Progress { done: total, total });
    Ok(MotionPlan {
        clip: clip.clone(),
        frames,
        zoom: Some(zoom),
        lost: Vec::new(),
        commands: vec![
            set_all(clip, "transform.position", &position),
            set_all(clip, "transform.rotation", &rotation),
            set_all(clip, "transform.scale", &scale),
        ],
    })
}

fn frame_count(source: &Source<'_>) -> u64 {
    let (from, to) = source.source_range();
    let rate = source.timeline.format.rate;
    (rate.frame_at(to - from).max(1)) as u64
}

/// Follows `region` of `clip`'s picture (centre and size as fractions of the displayed
/// picture) from timeline time `at` to the clip's end, and returns the `keyframe.set_all`
/// call that makes `target` follow it: a mask of the clip has its `center` keyed to the
/// region's, or another clip has its `transform.position` keyed to move as the region does
/// on the canvas, from where it is at `at`. Where the region is lost its last position
/// holds, and the plan lists the range.
pub fn track(
    project: &Project,
    bundle: &Path,
    clip: &Id,
    region: Region,
    at: Flicks,
    target: &TrackTarget,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<MotionPlan, EngineError> {
    let values = [region.x, region.y, region.width, region.height];
    if values.iter().any(|v| !v.is_finite())
        || region.width <= 0.0
        || region.height <= 0.0
        || region.width > 1.0
        || region.height > 1.0
        || !(0.0..=1.0).contains(&region.x)
        || !(0.0..=1.0).contains(&region.y)
    {
        return Err(invalid(
            "track: the region's centre and size are fractions of the picture, from 0 to 1",
        ));
    }
    let source = source(project, clip, "tracking")?;
    if at < source.clip.start || at >= source.clip.end() {
        return Err(invalid(format!("track: {at:?} is outside {clip}")));
    }
    // Where the keys go, and how a region centre in the displayed picture becomes a value.
    let (keyed, start, base) = match target {
        TrackTarget::Mask(mask) => {
            if !source.clip.masks.iter().any(|m| &m.id == mask) {
                return Err(invalid(format!("track: {clip} has no mask {mask}")));
            }
            (clip.clone(), source.clip.start, None)
        }
        TrackTarget::Clip(other) => {
            if other == clip {
                return Err(invalid(
                    "track: a clip cannot follow itself; track into one of its masks instead",
                ));
            }
            let (_, follower) = find(project, other)?;
            (
                other.clone(),
                follower.start,
                Some((follower.transform.position, follower.start, follower.end())),
            )
        }
    };
    let offset = at - source.clip.start;
    let from = source
        .clip
        .source_at(offset)
        .ok_or_else(|| invalid(format!("track: {clip} shows a frozen frame at {at:?}")))?;
    // The region is given in the displayed picture; the tracker works on stored frames.
    let stored = stored_region(region, source.rotation);
    let total = frame_count(&source);
    let mut tracker: Option<Tracker> = None;
    let mut points: Vec<(Flicks, (f64, f64), bool)> = Vec::new();
    let mut done = 0u64;
    source.frames(project, bundle, from, &mut |pts, luma| {
        let point = match tracker.as_mut() {
            None => {
                tracker = Some(Tracker::new(luma, stored));
                ((stored.x, stored.y), false)
            }
            Some(tracker) => {
                let found = tracker.push(luma);
                ((found.region.x, found.region.y), found.lost)
            }
        };
        points.push((pts, displayed_point(point.0, source.rotation), point.1));
        done += 1;
        !done.is_multiple_of(10) || progress(crate::Progress { done, total })
    })?;
    let first = points
        .first()
        .map(|p| p.1)
        .ok_or_else(|| invalid(format!("track: no frames of {clip} from {at:?}")))?;
    let (fit_w, fit_h) = source.fitted();
    let transform = &source.clip.transform;
    let (sin, cos) = transform.rotation.to_radians().sin_cos();
    // A centre's place on the canvas, as fractions of the canvas from its centre.
    let on_canvas = |p: (f64, f64)| {
        let (mx, my) = (
            (p.0 - 0.5) * fit_w * transform.scale * transform.stretch[0],
            (p.1 - 0.5) * fit_h * transform.scale * transform.stretch[1],
        );
        (mx * cos - my * sin, mx * sin + my * cos)
    };
    let origin = on_canvas(first);
    let mut keys = Vec::with_capacity(points.len());
    let mut lost: Vec<Lost> = Vec::new();
    let mut lost_since: Option<Flicks> = None;
    let mut last_time = at;
    for (pts, centre, is_lost) in &points {
        let Some(offset) = source.offset_of(*pts) else {
            continue;
        };
        let time = source.clip.start + offset;
        last_time = time;
        match (is_lost, lost_since) {
            (true, None) => lost_since = Some(time),
            (false, Some(since)) => {
                lost.push(Lost {
                    start: since,
                    end: time,
                });
                lost_since = None;
            }
            _ => {}
        }
        let value = match base {
            None => json!([round(centre.0, 6), round(centre.1, 6)]),
            Some((position, follower_start, follower_end)) => {
                if time < follower_start || time >= follower_end {
                    continue;
                }
                let now = on_canvas(*centre);
                json!([
                    round(position[0] + now.0 - origin.0, 6),
                    round(position[1] + now.1 - origin.1, 6)
                ])
            }
        };
        keys.push((time - start, value));
    }
    if let Some(since) = lost_since {
        lost.push(Lost {
            start: since,
            end: last_time,
        });
    }
    dedup_keys(&mut keys);
    if keys.is_empty() {
        return Err(invalid(format!(
            "track: {keyed} does not overlap the tracked part of {clip}"
        )));
    }
    let property = match target {
        TrackTarget::Mask(mask) => format!("masks.{mask}.center"),
        TrackTarget::Clip(_) => "transform.position".to_string(),
    };
    progress(crate::Progress { done: total, total });
    Ok(MotionPlan {
        clip: keyed.clone(),
        frames: points.len(),
        zoom: None,
        lost,
        commands: vec![set_all(&keyed, &property, &keys)],
    })
}

/// A region of the displayed picture as a region of the stored frame.
fn stored_region(region: Region, rotation: u16) -> Region {
    let (x, y) = match rotation {
        90 => (region.y, 1.0 - region.x),
        180 => (1.0 - region.x, 1.0 - region.y),
        270 => (1.0 - region.y, region.x),
        _ => (region.x, region.y),
    };
    let (width, height) = if rotation % 180 == 90 {
        (region.height, region.width)
    } else {
        (region.width, region.height)
    };
    Region {
        x,
        y,
        width,
        height,
    }
}

/// How far apart angles started, from their sound.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SyncOffsets {
    /// Each angle's offset in the shared scene, in order, the first at zero: the `offsets`
    /// of `multicam.create`.
    pub offsets: Vec<Flicks>,
    /// How sure each match with the first angle is, from 0 to 1 (1 for the first).
    pub confidence: Vec<f64>,
}

/// Offsets below this confidence are reported as doubtful by the front ends.
pub const SYNC_CONFIDENCE: f64 = 0.3;

/// Measures where each of `angles` (media assets with sound) starts in the scene the first
/// one recorded, by comparing their sound, looking at most `max_offset_seconds` either way
/// (any offset when 0).
pub fn sync_offsets(
    project: &Project,
    bundle: &Path,
    angles: &[Id],
    max_offset_seconds: f64,
) -> Result<SyncOffsets, EngineError> {
    if angles.len() < 2 {
        return Err(invalid("multicam sync: give at least two angles"));
    }
    let sounds = angles
        .iter()
        .map(|asset| mono(project, bundle, asset))
        .collect::<Result<Vec<_>, _>>()?;
    let mut offsets = vec![Flicks::ZERO];
    let mut confidence = vec![1.0];
    for other in &sounds[1..] {
        let found = audio_offset(&sounds[0], other, SYNC_RATE, max_offset_seconds);
        offsets.push(Flicks::new(
            (found.offset_seconds * 705_600_000.0).round() as i64
        ));
        confidence.push(round(found.confidence, 3));
    }
    Ok(SyncOffsets {
        offsets,
        confidence,
    })
}

/// The whole sound of `asset` as mono samples at [`SYNC_RATE`].
fn mono(project: &Project, bundle: &Path, asset: &Id) -> Result<Vec<f32>, EngineError> {
    let found = project
        .media
        .get(asset)
        .ok_or_else(|| EngineError::AssetNotFound {
            asset: asset.to_string(),
        })?;
    if found.info.audio.is_none() {
        return Err(EngineError::NoAudio {
            asset: asset.to_string(),
        });
    }
    let duration = found.info.duration.unwrap_or_default();
    let path = resolve(project, bundle, asset)?;
    let mut decoder = FfmpegAudioDecoder::open(&path, SYNC_RATE, 1).map_err(|e| match e {
        MediaError::NoAudioStream { .. } => EngineError::NoAudio {
            asset: asset.to_string(),
        },
        other => EngineError::from(other),
    })?;
    let chunk = Flicks::new(30 * 705_600_000);
    let mut samples = Vec::new();
    let mut start = Flicks::ZERO;
    while start < duration {
        let length = (duration - start).min(chunk);
        samples.extend(decoder.read(start, length)?.samples);
        start = start + length;
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_pictures_map_both_ways() {
        let region = Region {
            x: 0.2,
            y: 0.3,
            width: 0.1,
            height: 0.4,
        };
        for rotation in [0, 90, 180, 270] {
            let stored = stored_region(region, rotation);
            let back = displayed_point((stored.x, stored.y), rotation);
            assert!(
                (back.0 - 0.2).abs() < 1e-12 && (back.1 - 0.3).abs() < 1e-12,
                "{rotation}"
            );
            let size = if rotation % 180 == 90 {
                (0.4, 0.1)
            } else {
                (0.1, 0.4)
            };
            assert_eq!((stored.width, stored.height), size);
        }
        // A move right in a frame shown a quarter turn clockwise is a move down on screen.
        assert_eq!(displayed_vector((1.0, 0.0), 90), (-0.0, 1.0));
        assert_eq!(displayed_vector((1.0, 0.0), 270), (0.0, -1.0));
    }

    #[test]
    fn keys_keep_one_per_frame() {
        let mut keys = vec![
            (Flicks::new(0), json!(1)),
            (Flicks::new(0), json!(2)),
            (Flicks::new(5), json!(3)),
        ];
        dedup_keys(&mut keys);
        assert_eq!(
            keys,
            vec![(Flicks::new(0), json!(2)), (Flicks::new(5), json!(3))]
        );
    }
}
