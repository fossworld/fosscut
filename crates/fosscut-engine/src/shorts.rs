//! Long video to shorts (CC-017).
//!
//! - [`candidates`] scores stretches of whole sentences of a clip's media from its cached
//!   transcript, and from its cached `levels` and `scenes` when they are there
//!   ([`fosscut_analysis::highlights`]), and reports each with its numbers, in the media's
//!   time and where it plays on the clip's timeline. Which one makes a good short is the
//!   agent's call (ADR-0012).
//! - [`make_short`] returns the calls that build a new timeline (1080x1920 by default) that
//!   plays a range of the media, filling the canvas and following its subject
//!   ([`crate::reframe`]), with captions of its words (`captions.add`) and a title
//!   (`text.add`) when asked.
//!
//! The calls are planned on a scratch copy of the project through the command framework,
//! so the ids they report (the new timeline and clip) are the ones they create when applied
//! in one request at the project's current revision.

use std::path::PathBuf;

use fosscut_analysis::framing::FramingOptions;
pub use fosscut_analysis::highlights::HighlightOptions;
use fosscut_analysis::highlights::{self, Highlight, HighlightInput};
use fosscut_analysis::{Analyzer, Levels, Scenes, Transcript};
use fosscut_core::{Request, Session};
use fosscut_model::{AssetKind, ClipContent, Id, MemoryStore, Project};
use fosscut_render::geometry::clockwise_rotation;
use fosscut_time::{Flicks, Rate};
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;
use crate::measured;
use crate::reframe::{ReframePlan, ReframeRequest, SubjectSource, plan_in};

/// Where analyses are read from.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortsContext {
    /// The project bundle; relative media paths resolve against it.
    pub bundle: PathBuf,
    /// The analysis cache, normally [`crate::cache_dir`].
    pub cache: PathBuf,
}

/// A candidate with where it plays on the clip's timeline.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ShortCandidate {
    /// The candidate, in the media's own time.
    #[serde(flatten)]
    pub highlight: Highlight,
    /// Where it starts on the clip's timeline.
    pub timeline_start: Flicks,
    /// Where it ends there.
    pub timeline_end: Flicks,
}

/// A clip's candidate shorts.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ShortCandidates {
    /// The clip.
    pub clip: Id,
    /// Its media.
    pub asset: Id,
    /// The analyses the scores come from.
    pub measured: Vec<&'static str>,
    /// Analyses that would add scores if they were run: `levels` (loudness peaks and
    /// variation) and `scenes` (shot cuts).
    pub missing: Vec<&'static str>,
    /// Sentences in the part of the media the clip plays.
    pub sentences: u32,
    /// Candidates of an allowed length that were scored.
    pub considered: u32,
    /// The best candidates, best first.
    pub candidates: Vec<ShortCandidate>,
}

fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::InvalidArguments {
        message: message.into(),
    }
}

/// Finds `clip` and its timeline.
fn find<'a>(
    project: &'a Project,
    clip: &Id,
) -> Result<(&'a fosscut_model::Timeline, &'a fosscut_model::Clip), EngineError> {
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

/// The newest cached transcript of `asset`.
fn transcript(
    project: &Project,
    context: &ShortsContext,
    asset: &Id,
) -> Result<Transcript, EngineError> {
    measured::newest::<Transcript>(
        project,
        &context.bundle,
        &context.cache,
        asset,
        Analyzer::Transcript,
        &measured::any,
    )?
    .map(|(t, _)| t)
    .ok_or_else(|| measured::missing(asset, Analyzer::Transcript, json!({}), "shorts"))
}

/// Scores the candidate shorts in the part of its media that `clip` plays; see the
/// [module documentation](self). Needs the media's transcript cached; fails with
/// `fosscut.shorts.no_candidates` when no stretch of whole sentences fits the lengths.
pub fn candidates(
    project: &Project,
    context: &ShortsContext,
    clip: &Id,
    options: &HighlightOptions,
) -> Result<ShortCandidates, EngineError> {
    let (_, found) = find(project, clip)?;
    let asset = match &found.content {
        ClipContent::Video { asset, .. } | ClipContent::Audio { asset, .. } => asset.clone(),
        other => {
            return Err(invalid(format!(
                "{clip} is a {} clip; shorts are found in the speech of video or audio clips",
                other.kind_name()
            )));
        }
    };
    let spoken = transcript(project, context, &asset)?;
    let levels = measured::newest::<Levels>(
        project,
        &context.bundle,
        &context.cache,
        &asset,
        Analyzer::Levels,
        &measured::any,
    )?
    .map(|(l, _)| l);
    let scenes = measured::newest::<Scenes>(
        project,
        &context.bundle,
        &context.cache,
        &asset,
        Analyzer::Scenes,
        &measured::any,
    )?
    .map(|(s, _)| s);
    let first = found.source_at(Flicks::ZERO).unwrap_or(Flicks::ZERO);
    let last = found.source_at(found.duration).unwrap_or(first);
    let range = (first.min(last), first.max(last));
    let options = options.clamped();
    let found_highlights = highlights::candidates(
        &HighlightInput {
            transcript: &spoken,
            levels: levels.as_ref(),
            scenes: scenes.as_ref(),
            range: Some(range),
        },
        &options,
    );
    if found_highlights.candidates.is_empty() {
        return Err(EngineError::NoShortCandidates {
            clip: clip.to_string(),
            sentences: found_highlights.sentences,
            min_seconds: options.min_seconds,
            max_seconds: options.max_seconds,
        });
    }
    let on_timeline = |time: Flicks| {
        found
            .offset_of_source(time)
            .map_or(found.start, |offset| found.start + offset)
    };
    let mut measured = vec!["transcript"];
    let mut missing = Vec::new();
    for (name, present) in [("levels", levels.is_some()), ("scenes", scenes.is_some())] {
        if present {
            measured.push(name);
        } else {
            missing.push(name);
        }
    }
    Ok(ShortCandidates {
        clip: clip.clone(),
        asset,
        measured,
        missing,
        sentences: found_highlights.sentences,
        considered: found_highlights.considered,
        candidates: found_highlights
            .candidates
            .into_iter()
            .map(|highlight| {
                let (a, b) = (
                    on_timeline(highlight.span.start.flicks),
                    on_timeline(highlight.span.end.flicks),
                );
                ShortCandidate {
                    timeline_start: a.min(b),
                    timeline_end: a.max(b),
                    highlight,
                }
            })
            .collect(),
    })
}

/// The media a short is cut from.
#[derive(Clone, Debug, PartialEq)]
pub enum ShortSource {
    /// The media of this clip.
    Clip(Id),
    /// This video asset.
    Asset(Id),
}

/// What a short holds.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortRequest {
    /// The media.
    pub source: ShortSource,
    /// Where the short starts in the media's own time (a candidate's start).
    pub start: Flicks,
    /// Where it ends there.
    pub end: Flicks,
    /// The new timeline's name; `Short` with its start when `None`.
    pub name: Option<String>,
    /// The canvas width (1080).
    pub width: u32,
    /// The canvas height (1920).
    pub height: u32,
    /// What the picture follows; centred when `None`.
    pub follow: Option<SubjectSource>,
    /// How it follows.
    pub framing: FramingOptions,
    /// Frame each shot on its own, from the cached scene cuts.
    pub shots: bool,
    /// Caption the words spoken in the range, from the cached transcript.
    pub captions: bool,
    /// The caption preset, such as `bold-pop`; the default look when `None`.
    pub caption_preset: Option<String>,
    /// A title shown over the first three seconds.
    pub title: Option<String>,
}

impl ShortRequest {
    /// A 1080x1920 short of `start` to `end` of `source` that follows a person, frames shot
    /// by shot, with no captions or title.
    pub fn new(source: ShortSource, start: Flicks, end: Flicks) -> ShortRequest {
        ShortRequest {
            source,
            start,
            end,
            name: None,
            width: 1080,
            height: 1920,
            follow: Some(SubjectSource::Person),
            framing: FramingOptions::default(),
            shots: true,
            captions: false,
            caption_preset: None,
            title: None,
        }
    }
}

/// The calls that build a short, with what they create.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ShortPlan {
    /// The new timeline, as the calls create it at the project's current revision.
    pub timeline: Id,
    /// Its video clip.
    pub clip: Id,
    /// Its name.
    pub name: String,
    /// Where the short starts in the media: the range's start on the nearest frame.
    pub start: Flicks,
    /// Its length on the timeline, whole frames.
    pub duration: Flicks,
    /// How the clip follows its subject, without the calls (they are in `commands`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framing: Option<ReframePlan>,
    /// Why the picture is centred instead, such as `fosscut.reframe.no_subject`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub centred: Option<String>,
    /// Words captioned.
    pub caption_words: usize,
    /// The calls, in the order they apply.
    pub commands: Vec<Value>,
}

/// Runs `calls` on the scratch session and returns their results.
fn run(
    scratch: &mut Session<MemoryStore>,
    calls: &[Value],
    what: &str,
) -> Result<Vec<Value>, EngineError> {
    let request: Request = serde_json::from_value(json!({ "commands": calls, "actor": "planner" }))
        .map_err(|e| invalid(format!("make_short: {what}: {e}")))?;
    scratch
        .execute(request)
        .map(|r| r.results)
        .map_err(|e| EngineError::Failed {
            message: format!("make_short: {what}: {}", e.message),
            code: e.code,
        })
}

/// The scale that makes a picture of `size` (fitted inside the canvas at scale 1) cover
/// `canvas`.
fn fill_scale(size: (f64, f64), canvas: (u32, u32)) -> f64 {
    let (cw, ch) = (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1)));
    let fit = (cw / size.0).min(ch / size.1);
    let cover = (cw / size.0).max(ch / size.1);
    cover / fit
}

/// The id a call's result names under `field`.
fn id_of(result: Option<&Value>, field: &str) -> Result<Id, EngineError> {
    result
        .and_then(|r| r.get(field))
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or_else(|| invalid(format!("make_short: no {field} in the result")))
}

/// Returns the calls that build a new timeline holding `request.start` to `request.end` of
/// the media at its own frame rate: the picture fills the canvas (centred, or following its
/// subject shot by shot), and the timeline gets captions of the words spoken in the range
/// and a title, when asked. Following a subject needs its analyses cached, and captions
/// the transcript, or the plan fails with `fosscut.analysis.missing`; a range without a
/// subject keeps the picture centred and says why.
pub fn make_short(
    project: &Project,
    context: &ShortsContext,
    request: &ShortRequest,
) -> Result<ShortPlan, EngineError> {
    let asset_id = match &request.source {
        ShortSource::Asset(asset) => asset.clone(),
        ShortSource::Clip(clip) => match &find(project, clip)?.1.content {
            ClipContent::Video { asset, .. } => asset.clone(),
            other => {
                return Err(invalid(format!(
                    "{clip} is a {} clip; a short is cut from video",
                    other.kind_name()
                )));
            }
        },
    };
    let asset = project
        .media
        .get(&asset_id)
        .ok_or_else(|| EngineError::AssetNotFound {
            asset: asset_id.to_string(),
        })?;
    let video = match (&asset.kind, &asset.info.video) {
        (AssetKind::Video, Some(video)) => video,
        _ => {
            return Err(EngineError::NoVideo {
                asset: asset_id.to_string(),
            });
        }
    };
    if request.start.is_negative() || request.end <= request.start {
        return Err(invalid(
            "make_short: the range must start at 0 or later and end after it",
        ));
    }
    if asset.info.duration.is_some_and(|d| request.end > d) {
        return Err(invalid(format!(
            "make_short: the range ends after {asset_id} does"
        )));
    }
    if request.width == 0 || request.height == 0 {
        return Err(invalid("make_short: width and height must be at least 1"));
    }
    let rate: Rate = video
        .rate
        .or_else(|| project.primary_timeline().map(|t| t.format.rate))
        .unwrap_or(Rate::FPS_30);
    // Commands place times on frames, so the media starts on the frame nearest the range's
    // start; the short holds the whole frames from there to the range's end.
    let start = rate.snap(request.start);
    let duration = rate.frames_to_flicks(rate.frame_at(request.end - start));
    if !duration.is_positive() {
        return Err(invalid("make_short: the range is shorter than a frame"));
    }
    let name = request
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("Short {} s", request.start.to_seconds_string()));
    let mut scratch = Session::create(MemoryStore::new(), project.clone()).map_err(|e| {
        invalid(format!(
            "make_short: the project cannot be planned on: {}",
            e.message
        ))
    })?;
    let mut commands = Vec::new();

    let add_timeline = json!({
        "command": "timeline.add",
        "args": {
            "name": name,
            "width": request.width,
            "height": request.height,
            "rate": rate.to_string(),
        },
    });
    let results = run(
        &mut scratch,
        std::slice::from_ref(&add_timeline),
        "timeline.add",
    )?;
    let timeline = id_of(results.first(), "timeline")?;
    commands.push(add_timeline);

    let (w, h) = (
        f64::from(video.width.max(1)),
        f64::from(video.height.max(1)),
    );
    let shown = if clockwise_rotation(video.rotation) % 180 == 90 {
        (h, w)
    } else {
        (w, h)
    };
    let place = [json!({
        "command": "clip.add",
        "args": {
            "asset": asset_id,
            "timeline": timeline,
            "start": { "flicks": 0 },
            "source_in": { "flicks": start.get() },
            "duration": { "flicks": duration.get() },
        },
    })];
    let results = run(&mut scratch, &place, "clip.add")?;
    let clip = id_of(results.first(), "clip")?;
    commands.extend(place);
    let cover = (fill_scale(shown, (request.width, request.height)) * 1e6).round() / 1e6;
    let fill = json!({
        "command": "clip.set",
        "args": { "clip": clip, "set": { "transform.scale": cover } },
    });
    run(&mut scratch, std::slice::from_ref(&fill), "clip.set")?;
    commands.push(fill);

    let mut framing = None;
    let mut centred = None;
    if let Some(follow) = &request.follow {
        let reframe = ReframeRequest {
            bundle: context.bundle.clone(),
            cache: context.cache.clone(),
            source: follow.clone(),
            framing: request.framing.clone(),
            shots: request.shots,
        };
        match plan_in(scratch.project(), &clip, &reframe, "shorts") {
            Ok(mut planned) => {
                run(&mut scratch, &planned.commands, "keyframe.set_all")?;
                commands.append(&mut planned.commands);
                framing = Some(planned);
            }
            Err(EngineError::NoSubject { .. }) => {
                centred = Some("fosscut.reframe.no_subject".to_string());
            }
            Err(other) => return Err(other),
        }
    }

    let mut caption_words = 0;
    if request.captions {
        let spoken = transcript(project, context, &asset_id)?;
        let words: Vec<Value> = spoken
            .words
            .iter()
            .filter(|w| w.start.flicks >= start && w.start.flicks < start + duration)
            .map(|w| {
                let from = w.start.flicks - start;
                let to = (w.end.flicks - start).min(duration).max(from);
                json!({
                    "text": w.text.trim(),
                    "start": { "flicks": from.get() },
                    "end": { "flicks": to.get() },
                })
            })
            .collect();
        if !words.is_empty() {
            caption_words = words.len();
            let mut args = json!({ "timeline": timeline, "words": words });
            if let Some(preset) = &request.caption_preset {
                args["preset"] = json!(preset);
            }
            let call = [json!({ "command": "captions.add", "args": args })];
            run(&mut scratch, &call, "captions.add")?;
            commands.extend(call);
        }
    }

    if let Some(title) = request.title.as_ref().filter(|t| !t.trim().is_empty()) {
        let three = rate.snap(Flicks::from_millis(3000)).min(duration);
        let call = [json!({
            "command": "text.add",
            "args": {
                "text": title,
                "timeline": timeline,
                "start": { "flicks": 0 },
                "duration": { "flicks": three.get() },
                "transform": { "position": [0.0, -0.3] },
            },
        })];
        run(&mut scratch, &call, "text.add")?;
        commands.extend(call);
    }

    Ok(ShortPlan {
        timeline,
        clip,
        name,
        start,
        duration,
        framing: framing.map(|mut f| {
            f.commands.clear();
            f
        }),
        centred,
        caption_words,
        commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_pictures_fill_tall_canvases() {
        let scale = fill_scale((1920.0, 1080.0), (1080, 1920));
        assert!((scale - 3.160_493_827).abs() < 1e-6, "{scale}");
        assert!((fill_scale((1080.0, 1920.0), (1080, 1920)) - 1.0).abs() < 1e-12);
        let request = ShortRequest::new(
            ShortSource::Asset(Id::new(fosscut_model::IdKind::Asset, 1)),
            Flicks::ZERO,
            Flicks::from_millis(20_000),
        );
        assert_eq!((request.width, request.height), (1080, 1920));
        assert_eq!(request.follow, Some(SubjectSource::Person));
    }
}
