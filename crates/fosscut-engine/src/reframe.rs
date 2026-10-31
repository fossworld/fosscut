//! Following a subject on a canvas: automatic reframing (CC-084), vertical versions of a
//! whole edit (CC-175) and camera tracking (CC-087).
//!
//! The subject comes from cached analyses of the clip's media, or from tracking a region
//! the agent picks; the crop that follows it shot by shot is computed by
//! [`fosscut_analysis::framing`] (dead zone, speed limit, ease in and out, headroom and zoom
//! limits, no move across a shot cut). The crop then becomes `keyframe.set_all` calls for the
//! clip's `transform.position` and `transform.scale` (ADR-0030), through the same fit of the
//! picture into the canvas that [`crate::motion::stabilize`] uses: at scale 1 the displayed
//! picture fits inside the canvas, `scale` and `stretch` size it, and `position` moves its
//! centre by fractions of the canvas.
//!
//! - [`reframe`] makes a clip follow its subject on its timeline's canvas, typically one of
//!   another aspect (a 9:16 timeline showing 16:9 footage);
//! - [`camera_track`] does the same in place with a zoom: the camera follows the subject;
//! - [`vertical`] copies a timeline onto a 1080x1920 canvas (`timeline.duplicate` with
//!   `fit: fill`) and reframes every video clip of the copy.
//!
//! Every function returns calls and applies nothing. Keys are kept where the path bends
//! (within a fifth of a pixel at 1080 across), with a hold at the end of each shot so a cut
//! jumps straight to the next framing. Analyses that are not cached fail with
//! `fosscut.analysis.missing`, naming what to run; a clip in which no subject is found fails
//! with `fosscut.reframe.no_subject`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use fosscut_analysis::Region;
pub use fosscut_analysis::framing::FramingOptions;
use fosscut_analysis::framing::{self, Framing, Subject};
use fosscut_analysis::pose::MIN_PRESENCE;
use fosscut_analysis::scenes::Luma;
use fosscut_analysis::{Analyzer, MatteModel, PoseTrack, Scenes, Tracker};
use fosscut_core::Session;
use fosscut_media::ffmpeg::FfmpegVideoDecoder;
use fosscut_media::{DecoderOptions, MediaError, VideoDecoder, VideoFrame};
use fosscut_model::{Clip, ClipContent, Id, MemoryStore, Project, Timeline, TrackKind};
use fosscut_render::geometry::clockwise_rotation;
use fosscut_time::Flicks;
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;
use crate::beats::apply;
use crate::measured;

/// What the crop follows.
#[derive(Clone, Debug, PartialEq)]
pub enum SubjectSource {
    /// A person, from the cached `pose` analysis.
    Person,
    /// The subject of the cached `matte` analysis: the `subject` model's matte, or else the
    /// `person` model's.
    Subject,
    /// A region of the picture tracked from timeline time `at` (the clip's start when
    /// `None`): centre and size as fractions of the displayed picture.
    Region {
        /// The region where it is at `at`.
        region: Region,
        /// When the region is where it is given.
        at: Option<Flicks>,
    },
}

impl SubjectSource {
    fn name(&self) -> &'static str {
        match self {
            SubjectSource::Person => "person",
            SubjectSource::Subject => "subject",
            SubjectSource::Region { .. } => "region",
        }
    }
}

/// What to follow and how.
#[derive(Clone, Debug, PartialEq)]
pub struct ReframeRequest {
    /// The project bundle; relative media paths resolve against it.
    pub bundle: PathBuf,
    /// The analysis cache, normally [`crate::cache_dir`].
    pub cache: PathBuf,
    /// What the crop follows.
    pub source: SubjectSource,
    /// Dead zone, speed, ease, headroom and zoom.
    pub framing: FramingOptions,
    /// Frame each shot on its own, at the cuts of the cached `scenes` analysis; when false
    /// the clip is one shot and no scenes analysis is needed.
    pub shots: bool,
}

/// How one shot of a clip is framed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ShotPlan {
    /// Where the shot starts on the timeline.
    pub start: Flicks,
    /// Where it ends.
    pub end: Flicks,
    /// Its zoom: 1 is the largest crop of the canvas's shape.
    pub zoom: f64,
    /// Frames in which the subject was found.
    pub subject_frames: usize,
    /// Of those, frames whose subject stays wholly inside the crop.
    pub inside_frames: usize,
}

/// A clip's framing and the calls that apply it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReframePlan {
    /// The clip.
    pub clip: Id,
    /// Its frames.
    pub frames: usize,
    /// Frames in which the subject was found.
    pub subject_frames: usize,
    /// Its shots.
    pub shots: Vec<ShotPlan>,
    /// `keyframe.set_all` calls for `transform.position` and `transform.scale`.
    pub commands: Vec<Value>,
}

fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::InvalidArguments {
        message: message.into(),
    }
}

/// A video clip with what framing needs about it.
struct Source<'a> {
    timeline: &'a Timeline,
    clip: &'a Clip,
    asset: Id,
    /// The stored frame's size and its clockwise display rotation.
    size: (u32, u32),
    rotation: u16,
}

fn source<'a>(project: &'a Project, clip: &Id) -> Result<Source<'a>, EngineError> {
    let (timeline, found) = project
        .timelines
        .values()
        .flat_map(|t| {
            t.tracks
                .iter()
                .flat_map(move |track| track.clips.iter().map(move |c| (t, c)))
        })
        .find(|(_, c)| &c.id == clip)
        .ok_or_else(|| invalid(format!("clip {clip} does not exist")))?;
    let ClipContent::Video { asset, .. } = &found.content else {
        return Err(invalid(format!(
            "{clip} is a {} clip; reframing follows a subject in video clips",
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
    /// The picture's size as displayed.
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

    /// The clip's stretch, never zero.
    fn stretch(&self) -> (f64, f64) {
        let [x, y] = self.clip.transform.stretch;
        let fix = |v: f64| {
            if v.is_finite() && v.abs() > 1e-9 {
                v.abs()
            } else {
                1.0
            }
        };
        (fix(x), fix(y))
    }

    /// The offset from the clip's start of each of its frames, and the source time shown.
    fn frames(&self) -> Vec<(Flicks, Flicks)> {
        let rate = self.timeline.format.rate;
        (0..)
            .map(|k| rate.frames_to_flicks(k))
            .take_while(|offset| *offset < self.clip.duration)
            .filter_map(|offset| self.clip.source_at(offset).map(|s| (offset, s)))
            .collect()
    }
}

/// The subject in each frame of `source`, from `request.source`.
fn subjects(
    project: &Project,
    source: &Source<'_>,
    frames: &[(Flicks, Flicks)],
    request: &ReframeRequest,
    needed_by: &str,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<Vec<Option<Subject>>, EngineError> {
    let (bundle, cache) = (request.bundle.as_path(), request.cache.as_path());
    let turn = |s: Subject| s.turned(source.rotation);
    match &request.source {
        SubjectSource::Person => {
            let track =
                measured::pose(project, bundle, cache, &source.asset)?.ok_or_else(|| {
                    measured::missing(&source.asset, Analyzer::Pose, json!({}), needed_by)
                })?;
            Ok(frames
                .iter()
                .map(|(_, time)| pose_at(&track, *time).map(turn))
                .collect())
        }
        SubjectSource::Subject => {
            let track = match measured::matte(
                project,
                bundle,
                cache,
                &source.asset,
                MatteModel::Subject,
            )? {
                Some(track) => track,
                None => measured::matte(project, bundle, cache, &source.asset, MatteModel::Person)?
                    .ok_or_else(|| {
                        measured::missing(
                            &source.asset,
                            Analyzer::Matte,
                            measured::matte_options(MatteModel::Subject),
                            needed_by,
                        )
                    })?,
            };
            let mut found: HashMap<usize, Option<Subject>> = HashMap::new();
            Ok(frames
                .iter()
                .map(|(_, time)| {
                    let index = shown(track.frames.iter().map(|f| f.time), *time)?;
                    *found.entry(index).or_insert_with(|| {
                        track
                            .alpha(index)
                            .and_then(|alpha| framing::from_alpha(&alpha, track.size))
                            .map(turn)
                    })
                })
                .collect())
        }
        SubjectSource::Region { region, at } => tracked(
            project,
            source,
            frames,
            region,
            *at,
            &request.bundle,
            progress,
        ),
    }
}

/// The index of the last of `times` (in order) at or before `time`, or the first; found by
/// bisection, so long clips stay fast.
fn shown(times: impl ExactSizeIterator<Item = Flicks>, time: Flicks) -> Option<usize> {
    let times: Vec<Flicks> = times.collect();
    if times.is_empty() {
        return None;
    }
    Some(times.partition_point(|t| *t <= time).saturating_sub(1))
}

/// The person at `time` in `track`, as [`PoseTrack::at`] gives it (landmarks moving
/// towards the next analyzed frame when both have a person), found by bisection.
fn pose_at(track: &PoseTrack, time: Flicks) -> Option<Subject> {
    let index = track
        .frames
        .partition_point(|f| f.time <= time)
        .checked_sub(1)
        .or((!track.frames.is_empty()).then_some(0))?;
    let frame = &track.frames[index];
    let mut landmarks = frame.landmarks;
    if let Some(next) = track.frames.get(index + 1)
        && time > frame.time
        && frame.presence >= MIN_PRESENCE
        && next.presence >= MIN_PRESENCE
    {
        let t = ((time - frame.time).get() as f64 / (next.time - frame.time).get().max(1) as f64)
            .clamp(0.0, 1.0) as f32;
        for (l, n) in landmarks.iter_mut().zip(&next.landmarks) {
            l.x += (n.x - l.x) * t;
            l.y += (n.y - l.y) * t;
            l.visibility += (n.visibility - l.visibility) * t;
            l.presence += (n.presence - l.presence) * t;
        }
    }
    framing::from_pose(frame.presence, &landmarks)
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

/// The region followed from `at` through the frames of `source`.
fn tracked(
    project: &Project,
    source: &Source<'_>,
    frames: &[(Flicks, Flicks)],
    region: &Region,
    at: Option<Flicks>,
    bundle: &Path,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<Vec<Option<Subject>>, EngineError> {
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
            "reframe: the region's centre and size are fractions of the picture, from 0 to 1",
        ));
    }
    let clip = source.clip;
    let at = at.unwrap_or(clip.start);
    if at < clip.start || at >= clip.end() {
        return Err(invalid(format!("reframe: {at:?} is outside {}", clip.id)));
    }
    let from = clip
        .source_at(at - clip.start)
        .ok_or_else(|| invalid(format!("reframe: {} shows no media at {at:?}", clip.id)))?;
    let end = frames.iter().map(|f| f.1).max().unwrap_or(from);
    let path = crate::export::resolve(project, bundle, &source.asset)?;
    let mut decoder =
        FfmpegVideoDecoder::open(&path, DecoderOptions::default()).map_err(|e| match e {
            MediaError::NoVideoStream { .. } => EngineError::NoVideo {
                asset: source.asset.to_string(),
            },
            other => EngineError::from(other),
        })?;
    let stored = stored_region(*region, source.rotation);
    let total = frames.len() as u64;
    let mut tracker: Option<Tracker> = None;
    let mut points: Vec<(Flicks, Option<Subject>)> = Vec::new();
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
            let found = match tracker.as_mut() {
                None => {
                    tracker = Some(Tracker::new(luma, stored));
                    Some(stored)
                }
                Some(tracker) => {
                    let point = tracker.push(luma);
                    (!point.lost).then_some(point.region)
                }
            };
            let subject = found.map(|r| framing::from_region(&r).turned(source.rotation));
            points.push((current.pts, subject));
            let done = points.len() as u64;
            if done.is_multiple_of(10) && !progress(crate::Progress { done, total }) {
                return Err(EngineError::AnalysisCancelled);
            }
        }
        frame = decoder.next_frame()?;
    }
    // Each frame of the clip shows the last decoded frame at or before its source time.
    Ok(frames
        .iter()
        .map(|(_, time)| {
            if *time < from {
                return None;
            }
            let after = points.partition_point(|(pts, _)| pts <= time);
            after.checked_sub(1).and_then(|i| points[i].1)
        })
        .collect())
}

/// The first frame of every shot after the first, from the cached scene cuts.
fn shot_starts(
    project: &Project,
    source: &Source<'_>,
    frames: &[(Flicks, Flicks)],
    request: &ReframeRequest,
    needed_by: &str,
) -> Result<Vec<usize>, EngineError> {
    if !request.shots {
        return Ok(Vec::new());
    }
    let (scenes, _) = measured::newest::<Scenes>(
        project,
        &request.bundle,
        &request.cache,
        &source.asset,
        Analyzer::Scenes,
        &measured::any,
    )?
    .ok_or_else(|| measured::missing(&source.asset, Analyzer::Scenes, json!({}), needed_by))?;
    let cuts: Vec<Flicks> = scenes.cuts.iter().map(|c| c.time.flicks).collect();
    // A frame shows the shot of the last cut at or before its source time.
    let shot = |time: Flicks| {
        cuts.iter()
            .filter(|&&c| c <= time + Flicks::new(1000))
            .count()
    };
    let mut starts = Vec::new();
    for k in 1..frames.len() {
        if shot(frames[k].1) != shot(frames[k - 1].1) {
            starts.push(k);
        }
    }
    Ok(starts)
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// The indices of `values` to keep so that straight lines between them stay within
/// `tolerance` of every value (Ramer, Douglas and Peucker), always the first and last.
fn simplify(values: &[[f64; 2]], tolerance: f64) -> Vec<usize> {
    let n = values.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0, n - 1)];
    while let Some((lo, hi)) = stack.pop() {
        let mut worst = (0.0, lo);
        for i in lo + 1..hi {
            let t = (i - lo) as f64 / (hi - lo) as f64;
            let deviation = (0..2)
                .map(|d| (values[lo][d] + (values[hi][d] - values[lo][d]) * t - values[i][d]).abs())
                .fold(0.0, f64::max);
            if deviation > worst.0 {
                worst = (deviation, i);
            }
        }
        if worst.0 > tolerance {
            keep[worst.1] = true;
            stack.push((lo, worst.1));
            stack.push((worst.1, hi));
        }
    }
    (0..n).filter(|&i| keep[i]).collect()
}

/// Plans the framing of `clip` on its timeline's canvas.
fn plan(
    project: &Project,
    clip: &Id,
    request: &ReframeRequest,
    needed_by: &str,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<ReframePlan, EngineError> {
    let source = source(project, clip)?;
    let frames = source.frames();
    if frames.is_empty() {
        return Err(invalid(format!("{clip} has no frames to frame")));
    }
    let subjects = subjects(project, &source, &frames, request, needed_by, progress)?;
    let starts = shot_starts(project, &source, &frames, request, needed_by)?;
    let rate = source.timeline.format.rate;
    let fps = f64::from(rate.num()) / f64::from(rate.den().max(1));
    let (fit_w, fit_h) = source.fitted();
    let (sx, sy) = source.stretch();
    let aspect = (fit_h * sy) / (fit_w * sx);
    let Framing { crops, shots } =
        framing::frame(&subjects, fps, &starts, aspect, &request.framing);
    let found = shots.iter().map(|s| s.subject_frames).sum::<usize>();
    if found == 0 {
        return Err(EngineError::NoSubject {
            clip: clip.to_string(),
            subject: request.source.name().to_string(),
        });
    }
    let transform = &source.clip.transform;
    let (sin, cos) = transform.rotation.to_radians().sin_cos();
    let placed: Vec<([f64; 2], f64)> = crops
        .iter()
        .map(|crop| {
            let scale = 1.0 / (crop.width * fit_w * sx);
            let cx = if transform.flip_horizontal {
                1.0 - crop.x
            } else {
                crop.x
            };
            let cy = if transform.flip_vertical {
                1.0 - crop.y
            } else {
                crop.y
            };
            let (mx, my) = (
                (cx - 0.5) * fit_w * scale * sx,
                (cy - 0.5) * fit_h * scale * sy,
            );
            let (px, py) = (mx * cos - my * sin, mx * sin + my * cos);
            ([-px, -py], scale)
        })
        .collect();
    let mut position = Vec::new();
    let mut scale = Vec::new();
    let mut reports = Vec::with_capacity(shots.len());
    for (k, shot) in shots.iter().enumerate() {
        let last_shot = k + 1 == shots.len();
        let part = &placed[shot.start..shot.end];
        let positions: Vec<[f64; 2]> = part.iter().map(|p| p.0).collect();
        let scales: Vec<[f64; 2]> = part.iter().map(|p| [p.1, 0.0]).collect();
        let largest = scales.iter().map(|s| s[0]).fold(0.0, f64::max);
        for (keys, kept, values) in [
            (&mut position, simplify(&positions, 2e-4), None),
            (
                &mut scale,
                simplify(&scales, 1e-4 * largest.max(1.0)),
                Some(()),
            ),
        ] {
            for (n, &i) in kept.iter().enumerate() {
                let offset = frames[shot.start + i].0;
                let value = match values {
                    None => json!([round(positions[i][0], 6), round(positions[i][1], 6)]),
                    Some(()) => json!(round(scales[i][0], 6)),
                };
                let mut key = json!({ "offset": { "flicks": offset.get() }, "value": value });
                if !last_shot && n + 1 == kept.len() {
                    key["easing"] = json!("hold");
                }
                keys.push(key);
            }
        }
        reports.push(ShotPlan {
            start: source.clip.start + frames[shot.start].0,
            end: if last_shot {
                source.clip.end()
            } else {
                source.clip.start + frames[shot.end].0
            },
            zoom: shot.zoom,
            subject_frames: shot.subject_frames,
            inside_frames: shot.inside_frames,
        });
    }
    let set_all = |property: &str, keys: Vec<Value>| {
        json!({
            "command": "keyframe.set_all",
            "args": { "clip": clip, "property": property, "keys": keys },
        })
    };
    progress(crate::Progress {
        done: frames.len() as u64,
        total: frames.len() as u64,
    });
    Ok(ReframePlan {
        clip: clip.clone(),
        frames: frames.len(),
        subject_frames: found,
        shots: reports,
        commands: vec![
            set_all("transform.position", position),
            set_all("transform.scale", scale),
        ],
    })
}

/// Returns the calls that make `clip` follow its subject on its timeline's canvas: a crop
/// of the canvas's shape moves over the picture shot by shot, as `transform.position` and
/// `transform.scale` keyframes that replace the clip's own. Zoom is chosen per shot within
/// `request.framing`'s limits (1, the largest crop, by default).
pub fn reframe(
    project: &Project,
    clip: &Id,
    request: &ReframeRequest,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<ReframePlan, EngineError> {
    plan(project, clip, request, "reframe", progress)
}

/// Returns the calls that make the camera follow the subject of `clip` in place: the clip
/// is zoomed by `zoom` (1 to 4) on its own canvas, and the zoomed crop follows the subject.
pub fn camera_track(
    project: &Project,
    clip: &Id,
    zoom: f64,
    request: &ReframeRequest,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<ReframePlan, EngineError> {
    if !zoom.is_finite() || !(1.0..=4.0).contains(&zoom) {
        return Err(invalid(format!(
            "camera_track: zoom {zoom} is outside 1 to 4"
        )));
    }
    let request = ReframeRequest {
        framing: FramingOptions {
            zoom: Some(zoom),
            ..request.framing.clone()
        },
        ..request.clone()
    };
    plan(project, clip, &request, "camera_track", progress)
}

/// A vertical copy of a timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct VerticalRequest {
    /// How every clip follows its subject; a region cannot be followed through a timeline.
    pub reframe: ReframeRequest,
    /// The copy's canvas width (1080 for 9:16).
    pub width: u32,
    /// Its height (1920 for 9:16).
    pub height: u32,
    /// Its name; the original's with ` vertical` when `None`.
    pub name: Option<String>,
}

/// A clip of the copy left as `fit: fill` placed it, centred.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Centred {
    /// The clip in the copy.
    pub clip: Id,
    /// Why: the error code that framing it gave, such as `fosscut.reframe.no_subject`.
    pub reason: String,
}

/// The calls that make a vertical copy, with what they do.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VerticalPlan {
    /// The timeline copied.
    pub source: Id,
    /// The copy's id, as the calls create it when applied in one request at the project's
    /// current revision.
    pub timeline: Id,
    /// Each reframed clip of the copy, without its calls (they are in `commands`).
    pub clips: Vec<ReframePlan>,
    /// Video clips left centred.
    pub centred: Vec<Centred>,
    /// `timeline.duplicate`, then the `keyframe.set_all` calls of every reframed clip.
    pub commands: Vec<Value>,
}

/// Returns the calls that copy `timeline` (the primary one when `None`) onto a canvas of
/// `request.width` by `request.height` with `fit: fill`, and make every video clip of the
/// copy follow its subject. A clip without a subject stays centred and is listed; a missing
/// analysis fails the whole plan, naming what to run. The original timeline is unchanged.
pub fn vertical(
    project: &Project,
    timeline: Option<&Id>,
    request: &VerticalRequest,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<VerticalPlan, EngineError> {
    if matches!(request.reframe.source, SubjectSource::Region { .. }) {
        return Err(invalid(
            "vertical: follow a person or a subject; a region belongs to one clip, so reframe that clip on its own",
        ));
    }
    if request.width == 0 || request.height == 0 {
        return Err(invalid("vertical: width and height must be at least 1"));
    }
    let source = timeline
        .cloned()
        .unwrap_or_else(|| project.manifest.primary_timeline.clone());
    let original = project
        .timelines
        .get(&source)
        .ok_or_else(|| EngineError::TimelineNotFound {
            timeline: source.to_string(),
        })?;
    let duplicate = json!({
        "command": "timeline.duplicate",
        "args": {
            "timeline": source,
            "width": request.width,
            "height": request.height,
            "fit": "fill",
            "name": request.name.clone().unwrap_or_else(|| format!("{} vertical", original.name)),
        },
    });
    let mut scratch = Session::create(MemoryStore::new(), project.clone()).map_err(|e| {
        invalid(format!(
            "vertical: the project cannot be planned on: {}",
            e.message
        ))
    })?;
    let before: Vec<Id> = scratch
        .project()
        .manifest
        .timelines
        .iter()
        .map(|t| t.id.clone())
        .collect();
    apply(&mut scratch, std::slice::from_ref(&duplicate)).map_err(|code| EngineError::Failed {
        message: format!("vertical: the copy of {source} failed with {code}"),
        code,
    })?;
    let copied = scratch.project().clone();
    let copy = copied
        .manifest
        .timelines
        .iter()
        .map(|t| t.id.clone())
        .find(|id| !before.contains(id))
        .ok_or_else(|| invalid("vertical: the copy was not made"))?;
    let doc = &copied.timelines[&copy];
    let clips: Vec<Id> = doc
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .flat_map(|t| &t.clips)
        .filter(|c| {
            matches!(&c.content, ClipContent::Video { asset, .. }
                if copied.media.get(asset).is_some_and(|a| a.info.video.is_some()))
        })
        .map(|c| c.id.clone())
        .collect();
    let mut commands = vec![duplicate];
    let mut plans = Vec::new();
    let mut centred = Vec::new();
    let total = clips.len() as u64;
    for (n, clip) in clips.iter().enumerate() {
        match plan(&copied, clip, &request.reframe, "vertical", &mut |_| true) {
            Ok(mut planned) => {
                apply(&mut scratch, &planned.commands).map_err(|code| EngineError::Failed {
                    message: format!("vertical: the keyframes of {clip} failed with {code}"),
                    code,
                })?;
                commands.append(&mut planned.commands);
                plans.push(planned);
            }
            Err(EngineError::NoSubject { .. }) => centred.push(Centred {
                clip: clip.clone(),
                reason: "fosscut.reframe.no_subject".into(),
            }),
            Err(other) => return Err(other),
        }
        if !progress(crate::Progress {
            done: n as u64 + 1,
            total,
        }) {
            return Err(EngineError::AnalysisCancelled);
        }
    }
    Ok(VerticalPlan {
        source,
        timeline: copy,
        clips: plans,
        centred,
        commands,
    })
}

/// Plans the framing of `clip` in a scratch project, for [`crate::shorts`].
pub(crate) fn plan_in(
    project: &Project,
    clip: &Id,
    request: &ReframeRequest,
    needed_by: &str,
) -> Result<ReframePlan, EngineError> {
    plan(project, clip, request, needed_by, &mut |_| true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplification_keeps_bends_and_ends() {
        let mut values: Vec<[f64; 2]> = (0..10).map(|i| [0.1, 0.0 * i as f64]).collect();
        values.extend((0..10).map(|i| [0.1 + 0.01 * i as f64, 0.0]));
        assert_eq!(simplify(&values, 1e-6), vec![0, 10, 19]);
        assert_eq!(simplify(&values[..2], 1e-6), vec![0, 1]);
        assert!(simplify(&[], 1e-6).is_empty());
    }

    #[test]
    fn bisection_finds_the_frame_shown() {
        let times = [10, 20, 30].map(Flicks::new);
        assert_eq!(shown(times.into_iter(), Flicks::new(5)), Some(0));
        assert_eq!(shown(times.into_iter(), Flicks::new(20)), Some(1));
        assert_eq!(shown(times.into_iter(), Flicks::new(99)), Some(2));
        assert_eq!(shown(std::iter::empty(), Flicks::new(1)), None);
        let empty = PoseTrack {
            mask_size: (1, 1),
            frames: Vec::new(),
        };
        assert_eq!(pose_at(&empty, Flicks::ZERO), None);
        // The same person as the track's own lookup gives, between and around its frames.
        use fosscut_analysis::pose::{LANDMARKS, Landmark, PoseFrame};
        let frame = |time: i64, x: f32, presence: f32| {
            let mut landmarks = [Landmark::default(); LANDMARKS];
            for (n, dx, y) in [(0, 0.0, 0.3), (11, -0.05, 0.45), (12, 0.05, 0.45)] {
                landmarks[n] = Landmark {
                    x: x + dx,
                    y,
                    visibility: 0.9,
                    presence: 0.9,
                };
            }
            PoseFrame {
                time: Flicks::new(time),
                presence,
                landmarks,
                mask: Vec::new(),
            }
        };
        let track = PoseTrack {
            mask_size: (1, 1),
            frames: vec![
                frame(100, 0.3, 0.9),
                frame(200, 0.5, 0.9),
                frame(300, 0.7, 0.1),
            ],
        };
        for time in [0, 100, 150, 200, 250, 300, 400] {
            let time = Flicks::new(time);
            let expected = track
                .at(time)
                .and_then(|p| framing::from_pose(p.presence, &p.landmarks));
            assert_eq!(pose_at(&track, time), expected, "{time:?}");
        }
    }

    #[test]
    fn regions_turn_with_the_picture() {
        let region = Region {
            x: 0.2,
            y: 0.3,
            width: 0.1,
            height: 0.4,
        };
        let turned = stored_region(region, 90);
        assert!((turned.x - 0.3).abs() < 1e-12 && (turned.y - 0.8).abs() < 1e-12);
        assert_eq!((turned.width, turned.height), (0.4, 0.1));
        assert_eq!(stored_region(region, 0), region);
        assert_eq!(SubjectSource::Person.name(), "person");
    }
}
