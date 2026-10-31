//! Edits planned from the media's analyses, as the MCP `auto_edit` tool plans them: a clip
//! that follows its subject (auto reframe and camera tracking), a vertical version of a
//! timeline, markers on the beats of music and cuts moved onto them, and shorts cut from
//! the speech of a long clip.
//!
//! The engine plans each edit from cached analyses (or, for beats, measures them first) and
//! applies nothing; the plan is applied here as the user's one undoable step. A plan that
//! needs an analysis which is not cached fails with the engine's `fosscut.analysis.missing`,
//! to which [`explain_missing`] adds the model the analysis runs, so the window can offer
//! to install it or to run the analysis (`subject_analyze`) and try again.

use std::path::{Path, PathBuf};

use fosscut_core::{ApiError, codes};
use fosscut_engine::analysis::{Analyzer, Options};
use fosscut_engine::beats::{BeatContext, BeatOptions, BeatSource, CutOptions};
use fosscut_engine::reframe::{
    FramingOptions, ReframePlan, ReframeRequest, Region, SubjectSource, VerticalRequest,
};
use fosscut_engine::shorts::{HighlightOptions, ShortRequest, ShortSource, ShortsContext};
use fosscut_time::Flicks;
use serde_json::{Value, json};

use crate::commands::{
    apply_measured, ensure_revision, given, invalid, no_progress, open_project, parse_id,
};
use crate::models::ModelListing;
use crate::workspace::Workspace;

type Answer<T> = Result<T, ApiError>;

/// The code of the engine's error for an analysis that is not cached.
pub const ANALYSIS_MISSING: &str = "fosscut.analysis.missing";

/// A short's canvas when the window gives none: 1080 by 1920, 9:16.
const VERTICAL: (u32, u32) = (1080, 1920);

/// Where plans read analyses and models from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    /// The engine's cache folder, which holds the analyses.
    pub cache: PathBuf,
    /// The models folder.
    pub models: PathBuf,
}

fn internal(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INTERNAL, message)
}

/// What a crop follows, named as the window names it: `person` (the cached `pose`
/// analysis), `subject` (the cached `matte`) or `region` (tracked from `region`, drawn at
/// timeline time `at` in flicks, the clip's start when omitted). `region` and `at` belong
/// to a region only.
pub fn subject_source(
    source: &str,
    region: Option<Region>,
    at: Option<i64>,
) -> Answer<SubjectSource> {
    match source.trim() {
        "region" => {
            let region = region.ok_or_else(|| {
                invalid("source region needs the region to follow")
                    .with_hint("Draw a box around what the crop should follow.")
            })?;
            Ok(SubjectSource::Region {
                region,
                at: at.map(Flicks::new),
            })
        }
        named @ ("person" | "subject") if region.is_some() || at.is_some() => Err(invalid(
            format!("a region and its time are followed only with source region, not {named}"),
        )),
        "person" => Ok(SubjectSource::Person),
        "subject" => Ok(SubjectSource::Subject),
        other => Err(invalid(format!(
            "source {other:?}: follow a person, a subject or a region"
        ))),
    }
}

/// What a vertical version or a short follows: `person` or `subject`; a region belongs to
/// one clip at one time, so it cannot be followed through a timeline.
fn whole_subject(source: &str, what: &str) -> Answer<SubjectSource> {
    match source.trim() {
        "person" => Ok(SubjectSource::Person),
        "subject" => Ok(SubjectSource::Subject),
        "region" => Err(invalid(format!(
            "{what} follows a person or a subject; reframe one clip to follow a region"
        ))),
        other => Err(invalid(format!(
            "source {other:?}: {what} follows a person or a subject"
        ))),
    }
}

/// The model an analysis runs, the task it is listed under in `models_list`, and whether
/// it is ready to run with what `listing` holds: a vision model needs ONNX Runtime too,
/// and a transcript needs any speech model (the installed one it would use is named).
/// Analyses without a model are always ready.
pub(crate) fn model_needed(
    analyzer: Analyzer,
    options: &Options,
    listing: &ModelListing,
) -> (Option<&'static str>, Option<&'static str>, bool) {
    if let Some(name) = analyzer.vision_model(options) {
        let model = listing.models.iter().find(|m| m.name == name);
        let ready = model.is_some_and(|m| m.installed) && listing.runtime.is_some();
        return (Some(name), model.map(|m| m.task), ready);
    }
    if analyzer == Analyzer::Transcript {
        let speech = listing
            .models
            .iter()
            .find(|m| m.task == "speech" && m.installed);
        return (speech.map(|m| m.name), Some("speech"), speech.is_some());
    }
    (None, None, true)
}

/// Adds to a `fosscut.analysis.missing` error what the window needs to offer the analysis,
/// beside the engine's own details (`asset`, `analyzer`, `options` keyed by the analyzer's
/// name, and `effect`, what needs it): `needs_model`, `model` (the model it runs, or the
/// installed speech model a transcript would use; null when none), `model_task` (`pose`,
/// `matte`, `speech` or null) and `model_installed` (it can run now). Other errors are left
/// as they are.
pub fn explain_missing(mut error: ApiError, models: &Path) -> ApiError {
    if error.code != ANALYSIS_MISSING {
        return error;
    }
    let Some(analyzer) = error
        .details
        .get("analyzer")
        .and_then(Value::as_str)
        .and_then(Analyzer::parse)
    else {
        return error;
    };
    let options: Options = error
        .details
        .get("options")
        .cloned()
        .and_then(|options| serde_json::from_value(options).ok())
        .unwrap_or_default();
    let (model, task, installed) =
        model_needed(analyzer, &options, &crate::models::listing(models));
    if let Some(details) = error.details.as_object_mut() {
        details.insert("needs_model".into(), json!(task.is_some()));
        details.insert("model".into(), json!(model));
        details.insert("model_task".into(), json!(task));
        details.insert("model_installed".into(), json!(installed));
    }
    error
}

/// A clip's framing without its calls, which the response shows applied.
fn framing_report(plan: &ReframePlan) -> Value {
    json!({
        "clip": plan.clip,
        "frames": plan.frames,
        "subject_frames": plan.subject_frames,
        "shots": plan.shots,
    })
}

fn reframe_request(
    bundle: PathBuf,
    places: &Places,
    source: SubjectSource,
    shots: bool,
) -> ReframeRequest {
    ReframeRequest {
        bundle,
        cache: places.cache.clone(),
        source,
        framing: FramingOptions::default(),
        shots,
    }
}

/// What follows its subject (`clip_reframe`, `clip_camera_track`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Follow {
    /// The video clip.
    pub clip: String,
    /// `person`, `subject` or `region`.
    pub source: String,
    /// For `region`: its centre and size as fractions of the picture as displayed.
    pub region: Option<Region>,
    /// For `region`: the timeline time it was drawn at, in flicks; the clip's start when
    /// omitted.
    pub at: Option<i64>,
    /// Frame each shot on its own, at the cuts of the cached `scenes` analysis.
    pub shots: bool,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
}

/// Makes a video clip of the project open in `workspace` follow its subject on its
/// timeline's canvas, as one undoable step ("Auto reframe"): `{reframe: {clip, frames,
/// subject_frames, shots: [{start, end, zoom, subject_frames, inside_frames}]}, response}`.
pub fn reframe(workspace: &Workspace, places: &Places, follow: &Follow) -> Answer<Value> {
    follow_clip(workspace, places, follow, None)
}

/// Zooms a video clip by `zoom` (1 to 4) on its own canvas and makes the zoomed crop
/// follow its subject, as one undoable step ("Camera tracking"); answered as [`reframe`].
pub fn camera_track(
    workspace: &Workspace,
    places: &Places,
    follow: &Follow,
    zoom: f64,
) -> Answer<Value> {
    follow_clip(workspace, places, follow, Some(zoom))
}

fn follow_clip(
    workspace: &Workspace,
    places: &Places,
    follow: &Follow,
    zoom: Option<f64>,
) -> Answer<Value> {
    let clip = parse_id(&follow.clip, "clip")?;
    let source = subject_source(&follow.source, follow.region, follow.at)?;
    let (bundle, project, _) = open_project(workspace)?;
    ensure_revision(&project, follow.expected_revision)?;
    let request = reframe_request(bundle, places, source, follow.shots);
    let planned = match zoom {
        None => fosscut_engine::reframe::reframe(&project, &clip, &request, &mut no_progress),
        Some(zoom) => {
            fosscut_engine::reframe::camera_track(&project, &clip, zoom, &request, &mut no_progress)
        }
    }
    .map_err(|e| explain_missing(e.into(), &places.models))?;
    let reason = if zoom.is_some() {
        "Camera tracking"
    } else {
        "Auto reframe"
    };
    let report = framing_report(&planned);
    let response = apply_measured(
        workspace,
        planned.commands,
        follow.expected_revision,
        reason,
    )?;
    Ok(json!({ "reframe": report, "response": response }))
}

/// What vertical version to make (`timeline_vertical`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vertical {
    /// The timeline copied; the primary one when omitted.
    pub timeline: Option<String>,
    /// What every clip follows: `person` or `subject`.
    pub source: String,
    /// The copy's width; 1080 when omitted.
    pub width: Option<u32>,
    /// Its height; 1920 when omitted.
    pub height: Option<u32>,
    /// Its name; the original's with ` vertical` when omitted.
    pub name: Option<String>,
    /// Frame each shot of each clip on its own, from the cached scene cuts.
    pub shots: bool,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
}

/// Copies a timeline of the project open in `workspace` onto a vertical canvas and makes
/// every video clip of the copy follow its subject, as one undoable step ("Vertical
/// version"), applied at the revision it was planned at (the copy's id is the one the
/// calls make there): `{vertical: {source, timeline, clips, centred: [{clip, reason}]},
/// response}`, with `clips` the number of clips framed, and `centred` those left centred
/// because no subject was found in them.
pub fn vertical(workspace: &Workspace, places: &Places, request: &Vertical) -> Answer<Value> {
    let timeline = given(request.timeline.as_deref())
        .map(|t| parse_id(t, "timeline"))
        .transpose()?;
    let source = whole_subject(&request.source, "a vertical version")?;
    let (bundle, project, _) = open_project(workspace)?;
    ensure_revision(&project, request.expected_revision)?;
    let planned = fosscut_engine::reframe::vertical(
        &project,
        timeline.as_ref(),
        &VerticalRequest {
            reframe: reframe_request(bundle, places, source, request.shots),
            width: request.width.unwrap_or(VERTICAL.0),
            height: request.height.unwrap_or(VERTICAL.1),
            name: given(request.name.as_deref()).map(str::to_string),
        },
        &mut no_progress,
    )
    .map_err(|e| explain_missing(e.into(), &places.models))?;
    let report = json!({
        "source": planned.source,
        "timeline": planned.timeline,
        "clips": planned.clips.len(),
        "centred": planned.centred,
    });
    let planned_at = Some(project.manifest.revision);
    let response = apply_measured(workspace, planned.commands, planned_at, "Vertical version")?;
    Ok(json!({ "vertical": report, "response": response }))
}

fn beat_context(bundle: PathBuf, places: &Places) -> BeatContext {
    BeatContext {
        bundle,
        cache: places.cache.clone(),
        models: places.models.clone(),
        options: BeatOptions::default(),
    }
}

/// Measures the beats of a music clip of the project open in `workspace` (unless cached)
/// and marks every `every`-th one it plays (1 to 16; 1 when omitted, counted from a
/// downbeat, so 4 marks bars), as one undoable step ("Beat markers"): `{beats: {timeline,
/// tempo_bpm, beats_per_bar, confidence, markers, existing}, response}`, with `markers` the
/// number added, `existing` the beats whose frame has a marker already, and `response`
/// null when there is nothing to add. `tempo_bpm` and `beats_per_bar` are null when not
/// found.
pub fn beat_markers(
    workspace: &Workspace,
    places: &Places,
    clip: &str,
    every: Option<u32>,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let clip = parse_id(clip, "clip")?;
    let (bundle, project, _) = open_project(workspace)?;
    ensure_revision(&project, expected_revision)?;
    let planned = fosscut_engine::beats::markers(
        &project,
        &beat_context(bundle, places),
        &BeatSource::Clip(clip),
        every.unwrap_or(1),
        &mut no_progress,
    )
    .map_err(|e| explain_missing(e.into(), &places.models))?;
    let report = json!({
        "timeline": planned.timeline,
        "tempo_bpm": planned.tempo_bpm,
        "beats_per_bar": planned.beats_per_bar,
        "confidence": planned.confidence,
        "markers": planned.at.len(),
        "existing": planned.existing,
    });
    let response = apply_measured(
        workspace,
        planned.commands,
        expected_revision,
        "Beat markers",
    )?;
    Ok(json!({ "beats": report, "response": response }))
}

/// Measures the beats of music clip `music` (unless cached) and moves each cut between the
/// clips of video track `track` onto the nearest beat (every `every`-th, 1 when omitted),
/// as one undoable step ("Cut to the beat") applied at the revision it was planned at:
/// `{cuts: {track, music, tempo_bpm, beats, tolerance, snapped: [{before, after, from,
/// to}], unsnapped: [{before, after, at, reason}]}, response}`, times in flicks, `before`
/// of an unsnapped cut null after a gap, and `response` null when no cut moved.
pub fn beat_cuts(
    workspace: &Workspace,
    places: &Places,
    track: &str,
    music: &str,
    every: Option<u32>,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let track = parse_id(track, "track")?;
    let music = parse_id(music, "music")?;
    let (bundle, project, _) = open_project(workspace)?;
    ensure_revision(&project, expected_revision)?;
    let planned = fosscut_engine::beats::cut_to_beats(
        &project,
        &beat_context(bundle, places),
        &track,
        &music,
        &CutOptions {
            every: every.unwrap_or(1),
            tolerance: None,
            ..CutOptions::default()
        },
        &mut no_progress,
    )
    .map_err(|e| explain_missing(e.into(), &places.models))?;
    let mut report = json!(planned);
    if let Some(object) = report.as_object_mut() {
        object.remove("commands");
        object.entry("tempo_bpm").or_insert(Value::Null);
    }
    if let Some(unsnapped) = report["unsnapped"].as_array_mut() {
        for cut in unsnapped.iter_mut().filter_map(Value::as_object_mut) {
            cut.entry("before").or_insert(Value::Null);
        }
    }
    let planned_at = Some(project.manifest.revision);
    let response = apply_measured(workspace, planned.commands, planned_at, "Cut to the beat")?;
    Ok(json!({ "cuts": report, "response": response }))
}

/// What candidate shorts to look for (`shorts_candidates`); the engine's defaults (15 to
/// 60 seconds, 10 candidates) where omitted, clamped by it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidates {
    /// The video or audio clip.
    pub clip: String,
    /// The shortest candidate, in seconds.
    pub min_seconds: Option<f64>,
    /// The longest, in seconds.
    pub max_seconds: Option<f64>,
    /// The most candidates.
    pub count: Option<u32>,
}

/// Scores the stretches of whole sentences in the part of its media a clip of the project
/// open in `workspace` plays, from its cached transcript (and `levels` and `scenes` when
/// cached), best first. Reads only: `{clip, asset, measured, missing, sentences,
/// considered, candidates: [{span: {start, end}, seconds, score, text, sentences, words,
/// reasons, timeline_start, timeline_end}]}`, where `span` is in the media's own time
/// (`{flicks, seconds}` each) and `timeline_start` and `timeline_end` are flicks on the
/// clip's timeline.
pub fn short_candidates(
    workspace: &Workspace,
    places: &Places,
    request: &Candidates,
) -> Answer<Value> {
    let clip = parse_id(&request.clip, "clip")?;
    let (bundle, project, _) = open_project(workspace)?;
    let defaults = HighlightOptions::default();
    let options = HighlightOptions {
        min_seconds: request.min_seconds.unwrap_or(defaults.min_seconds),
        max_seconds: request.max_seconds.unwrap_or(defaults.max_seconds),
        count: request.count.unwrap_or(defaults.count),
        ..HighlightOptions::default()
    };
    let found = fosscut_engine::shorts::candidates(
        &project,
        &ShortsContext {
            bundle,
            cache: places.cache.clone(),
        },
        &clip,
        &options,
    )
    .map_err(|e| explain_missing(e.into(), &places.models))?;
    serde_json::to_value(found).map_err(|e| internal(e.to_string()))
}

/// What short to make (`shorts_make`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Short {
    /// The video clip whose media the short is cut from.
    pub clip: String,
    /// Where it starts in the media's own time, in flicks (a candidate's `span.start`).
    pub start: i64,
    /// Where it ends there.
    pub end: i64,
    /// The new timeline's name; `Short` with its start when omitted.
    pub name: Option<String>,
    /// Its canvas width; 1080 when omitted.
    pub width: Option<u32>,
    /// Its height; 1920 when omitted.
    pub height: Option<u32>,
    /// `person` or `subject` to follow; centred when omitted.
    pub follow: Option<String>,
    /// When following, frame each shot on its own from the cached scene cuts; true when
    /// omitted.
    pub shots: Option<bool>,
    /// Caption the words spoken, from the cached transcript.
    pub captions: bool,
    /// The caption preset; the default look when omitted.
    pub caption_preset: Option<String>,
    /// A title over the first three seconds.
    pub title: Option<String>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
}

/// Builds a new timeline of the project open in `workspace` that plays a range of a
/// clip's media on a vertical canvas, filling it (centred, or following its subject), with
/// captions and a title when asked, as one undoable step ("Make a short") applied at the
/// revision it was planned at: `{short: {timeline, clip, name, start, duration, centred,
/// caption_words}, response}`, with `start` and `duration` in flicks on whole frames, and
/// `centred` null, or why a followed picture stays centred (`fosscut.reframe.no_subject`).
pub fn make_short(workspace: &Workspace, places: &Places, request: &Short) -> Answer<Value> {
    let clip = parse_id(&request.clip, "clip")?;
    let follow = given(request.follow.as_deref())
        .map(|source| whole_subject(source, "a short"))
        .transpose()?;
    let (bundle, project, _) = open_project(workspace)?;
    ensure_revision(&project, request.expected_revision)?;
    let defaults = ShortRequest::new(
        ShortSource::Clip(clip),
        Flicks::new(request.start),
        Flicks::new(request.end),
    );
    let short = ShortRequest {
        name: given(request.name.as_deref()).map(str::to_string),
        width: request.width.unwrap_or(VERTICAL.0),
        height: request.height.unwrap_or(VERTICAL.1),
        follow,
        shots: request.shots.unwrap_or(defaults.shots),
        captions: request.captions,
        caption_preset: given(request.caption_preset.as_deref()).map(str::to_string),
        title: given(request.title.as_deref()).map(str::to_string),
        ..defaults
    };
    let planned = fosscut_engine::shorts::make_short(
        &project,
        &ShortsContext {
            bundle,
            cache: places.cache.clone(),
        },
        &short,
    )
    .map_err(|e| explain_missing(e.into(), &places.models))?;
    let report = json!({
        "timeline": planned.timeline,
        "clip": planned.clip,
        "name": planned.name,
        "start": planned.start,
        "duration": planned.duration,
        "centred": planned.centred,
        "caption_words": planned.caption_words,
    });
    let planned_at = Some(project.manifest.revision);
    let response = apply_measured(workspace, planned.commands, planned_at, "Make a short")?;
    Ok(json!({ "short": report, "response": response }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::workspace::NewProject;
    use fosscut_core::Request;

    const SECOND: i64 = fosscut_time::FLICKS_PER_SECOND;

    /// A folder of its own for a test, empty.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fcd-auto-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A workspace with a new 1280 by 720 project open at 30 frames a second, a short
    /// runtime directory (Unix socket paths are limited to about 100 bytes), the folder it
    /// is in, and empty cache and models folders.
    fn opened(name: &str) -> (Workspace, PathBuf, PathBuf, Places) {
        let base = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let runtime = base.join(format!("fcd-a{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        let work = folder(name);
        let workspace = Workspace::new(runtime.clone(), None);
        workspace
            .create(&NewProject {
                path: work.join("Auto.fosscut"),
                name: None,
                width: 1280,
                height: 720,
                rate: "30".into(),
            })
            .unwrap();
        let places = Places {
            cache: work.join("cache"),
            models: work.join("models"),
        };
        (workspace, runtime, work, places)
    }

    fn done(workspace: Workspace, runtime: PathBuf, work: PathBuf) {
        workspace.close();
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
    }

    fn revision(workspace: &Workspace) -> u64 {
        workspace.query("project.info", Value::Null).unwrap()["revision"]
            .as_u64()
            .unwrap()
    }

    fn last_edit(workspace: &Workspace) -> Value {
        workspace
            .query("history.list", json!({ "limit": 1 }))
            .unwrap()["entries"][0]
            .clone()
    }

    /// Imports a video of `seconds` that is nowhere on disk, with its identity recorded,
    /// and returns the asset.
    fn video_asset(workspace: &Workspace, seconds: i64) -> String {
        let added = workspace
            .execute(Request::single(
                "media.add",
                json!({
                    "name": "talk.mp4",
                    "kind": "video",
                    "path": { "absolute": "/nowhere/talk.mp4" },
                    "identity": { "size": 4321, "sample_blake3": "ab".repeat(32) },
                    "info": {
                        "duration": seconds * SECOND,
                        "container": "mov,mp4,m4a,3gp,3g2,mj2",
                        "video": { "width": 1920, "height": 1080, "rate": "30", "codec": "h264" },
                        "audio": { "sample_rate": 48000, "channels": 2, "codec": "aac" },
                    }
                }),
            ))
            .unwrap();
        added.results[0]["asset"].as_str().unwrap().to_string()
    }

    /// Places `asset` from `source_in` for `length` at `start` (seconds) and returns the
    /// clip.
    fn place(
        workspace: &Workspace,
        asset: &str,
        start: f64,
        source_in: f64,
        length: f64,
    ) -> String {
        let flicks = |s: f64| json!({ "flicks": (s * SECOND as f64).round() as i64 });
        workspace
            .execute(Request::single(
                "clip.add",
                json!({
                    "asset": asset,
                    "start": flicks(start),
                    "source_in": flicks(source_in),
                    "duration": flicks(length),
                }),
            ))
            .unwrap()
            .results[0]["clip"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn missing(error: &ApiError, analyzer: &str, effect: &str) {
        assert_eq!(error.code, ANALYSIS_MISSING, "{error:?}");
        assert_eq!(error.details["analyzer"], analyzer, "{error:?}");
        assert_eq!(error.details["effect"], effect, "{error:?}");
        assert!(error.details["asset"].is_string() && error.hint.is_some());
    }

    #[test]
    fn sources_are_named_as_the_window_names_them() {
        let region = Region {
            x: 0.5,
            y: 0.4,
            width: 0.2,
            height: 0.3,
        };
        assert_eq!(
            subject_source("person", None, None).unwrap(),
            SubjectSource::Person
        );
        assert_eq!(
            subject_source(" subject ", None, None).unwrap(),
            SubjectSource::Subject
        );
        assert_eq!(
            subject_source("region", Some(region), Some(SECOND)).unwrap(),
            SubjectSource::Region {
                region,
                at: Some(Flicks::new(SECOND))
            }
        );
        let none = subject_source("region", None, None).unwrap_err();
        assert!(none.message.contains("needs the region"), "{none:?}");
        let stray = subject_source("person", Some(region), None).unwrap_err();
        assert!(
            stray.message.contains("only with source region"),
            "{stray:?}"
        );
        assert!(subject_source("subject", None, Some(0)).is_err());
        let face = subject_source("face", None, None).unwrap_err();
        assert_eq!(face.code, codes::INVALID_ARGUMENTS);
        assert!(
            face.message.contains("person, a subject or a region"),
            "{face:?}"
        );
        assert_eq!(
            whole_subject("subject", "x").unwrap(),
            SubjectSource::Subject
        );
        assert!(
            whole_subject("region", "x")
                .unwrap_err()
                .message
                .contains("one clip")
        );
        assert!(whole_subject("", "x").is_err());
    }

    #[test]
    fn a_missing_analysis_names_the_model_it_runs() {
        let dir = folder("explain");
        let error = |analyzer: &str, options: Value| {
            let engine = fosscut_engine::EngineError::AnalysisMissing {
                asset: "asset_0001".into(),
                analyzer: analyzer.into(),
                options,
                effect: "reframe".into(),
            };
            explain_missing(ApiError::from(engine), &dir)
        };
        let pose = error("pose", json!({}));
        assert_eq!(pose.code, ANALYSIS_MISSING);
        assert_eq!(
            (
                &pose.details["needs_model"],
                &pose.details["model"],
                &pose.details["model_task"],
                &pose.details["model_installed"]
            ),
            (
                &json!(true),
                &json!("mediapipe-pose-full"),
                &json!("pose"),
                &json!(false)
            )
        );
        assert_eq!(
            pose.details["analyzer"], "pose",
            "the engine's details stay"
        );
        assert_eq!(pose.details["effect"], "reframe");
        assert!(pose.hint.is_some(), "and its hint");
        let subject = error("matte", json!({ "matte": { "model": "subject" } }));
        assert_eq!(subject.details["model"], "birefnet-lite");
        assert_eq!(subject.details["model_task"], "matte");
        let person = error("matte", json!({}));
        assert_eq!(person.details["model"], "mediapipe-pose-full");
        let speech = error("transcript", json!({}));
        assert_eq!(
            (&speech.details["model_task"], &speech.details["model"]),
            (&json!("speech"), &Value::Null),
            "no speech model is installed"
        );
        assert_eq!(speech.details["needs_model"], true);
        assert_eq!(speech.details["model_installed"], false);
        for analyzer in ["scenes", "levels", "beats"] {
            let plain = error(analyzer, json!({}));
            assert_eq!(plain.details["needs_model"], false, "{analyzer}");
            assert_eq!(plain.details["model_installed"], true, "{analyzer}");
            assert!(plain.details["model"].is_null() && plain.details["model_task"].is_null());
        }
        let other = explain_missing(invalid("nothing to add"), &dir);
        assert!(other.details.is_null(), "other errors stay as they are");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reframing_checks_its_arguments_and_names_the_missing_analysis() {
        let (workspace, runtime, work, places) = opened("reframe");
        let ask = |clip: &str, source: &str| Follow {
            clip: clip.into(),
            source: source.into(),
            ..Follow::default()
        };
        let closed = Workspace::new(runtime.join("none"), None);
        let nothing = reframe(&closed, &places, &ask("clip_0001", "person")).unwrap_err();
        assert!(nothing.message.contains("no project"), "{nothing:?}");

        let asset = video_asset(&workspace, 6);
        let clip = place(&workspace, &asset, 0.0, 0.0, 4.0);
        let before = revision(&workspace);
        let bad = reframe(&workspace, &places, &ask("not a clip", "person")).unwrap_err();
        assert_eq!(bad.code, codes::INVALID_ARGUMENTS);
        let unknown = reframe(&workspace, &places, &ask("clip_0zzz", "person")).unwrap_err();
        assert!(unknown.message.contains("does not exist"), "{unknown:?}");
        let face = reframe(&workspace, &places, &ask(&clip, "face")).unwrap_err();
        assert!(
            face.message.contains("person, a subject or a region"),
            "{face:?}"
        );
        let stale = reframe(
            &workspace,
            &places,
            &Follow {
                expected_revision: Some(before + 5),
                ..ask(&clip, "person")
            },
        )
        .unwrap_err();
        assert_eq!(stale.code, codes::REVISION_CONFLICT);

        // Nothing is analyzed yet: each plan names what to run, and the model it needs.
        let person = reframe(&workspace, &places, &ask(&clip, "person")).unwrap_err();
        missing(&person, "pose", "reframe");
        assert_eq!(person.details["model"], "mediapipe-pose-full");
        assert_eq!(person.details["model_installed"], false);
        let subject = reframe(&workspace, &places, &ask(&clip, "subject")).unwrap_err();
        missing(&subject, "matte", "reframe");
        assert_eq!(subject.details["options"]["matte"]["model"], "subject");
        assert_eq!(subject.details["model"], "birefnet-lite");

        let zoomed = camera_track(&workspace, &places, &ask(&clip, "person"), 9.0).unwrap_err();
        assert_eq!(zoomed.code, codes::INVALID_ARGUMENTS);
        assert!(zoomed.message.contains("outside 1 to 4"), "{zoomed:?}");
        let tracked = camera_track(&workspace, &places, &ask(&clip, "person"), 2.0).unwrap_err();
        missing(&tracked, "pose", "camera_track");

        let tall = |source: &str| Vertical {
            source: source.into(),
            ..Vertical::default()
        };
        let region = vertical(&workspace, &places, &tall("region")).unwrap_err();
        assert!(region.message.contains("person or a subject"), "{region:?}");
        let gone = vertical(
            &workspace,
            &places,
            &Vertical {
                timeline: Some("timeline_0zzz".into()),
                ..tall("person")
            },
        )
        .unwrap_err();
        assert_eq!(gone.code, codes::TIMELINE_NOT_FOUND, "{gone:?}");
        let copy = vertical(&workspace, &places, &tall("person")).unwrap_err();
        missing(&copy, "pose", "vertical");
        assert_eq!(revision(&workspace), before, "nothing was applied");
        done(workspace, runtime, work);
    }

    /// A transcript of 12 sentences of four words, each word half a second with a quarter
    /// second between sentences, as the `transcript` analyzer caches it for `identity`.
    fn cache_transcript(places: &Places, identity: (&str, u64)) {
        let moment = |flicks: i64| {
            json!({
                "flicks": flicks,
                "seconds": Flicks::new(flicks).to_seconds_string(),
            })
        };
        let mut words = Vec::new();
        let mut at = SECOND / 2;
        for sentence in 0..12 {
            for word in 0..4 {
                let text = if word == 3 {
                    format!("word{sentence}.")
                } else {
                    format!("word{sentence}{word}")
                };
                words.push(json!({
                    "text": text,
                    "start": moment(at),
                    "end": moment(at + SECOND / 2),
                    "probability": 0.9,
                }));
                at += SECOND / 2;
            }
            at += SECOND / 4;
        }
        let entry = json!({
            "analyzer": "transcript",
            "version": Analyzer::Transcript.version(),
            "options": {},
            "result": {
                "model": "whisper-tiny.en",
                "language": "en",
                "segments": [],
                "words": words,
                "fillers": [],
                "pauses": [],
                "min_pause_ms": 300,
            },
        });
        let dir = places
            .cache
            .join("analysis")
            .join(format!("{}-{}", identity.0, identity.1));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!(
            "transcript@{}-0000000000000000.json",
            Analyzer::Transcript.version()
        ));
        std::fs::write(file, serde_json::to_vec(&entry).unwrap()).unwrap();
    }

    #[test]
    fn shorts_are_found_in_the_speech_and_made_as_one_step() {
        let (workspace, runtime, work, places) = opened("shorts");
        let asset = video_asset(&workspace, 40);
        let clip = place(&workspace, &asset, 2.0, 0.0, 40.0);
        let ask = |count: Option<u32>| Candidates {
            clip: clip.clone(),
            min_seconds: Some(4.0),
            max_seconds: Some(8.0),
            count,
        };
        let unheard = short_candidates(&workspace, &places, &ask(None)).unwrap_err();
        missing(&unheard, "transcript", "shorts");
        assert_eq!(unheard.details["model_task"], "speech");
        assert!(
            short_candidates(
                &workspace,
                &places,
                &Candidates {
                    clip: "nope".into(),
                    ..ask(None)
                }
            )
            .is_err()
        );

        cache_transcript(&places, (&"ab".repeat(32), 4321));
        let found = short_candidates(&workspace, &places, &ask(Some(3))).unwrap();
        assert_eq!(found["clip"], clip.as_str());
        assert_eq!(found["asset"], asset.as_str());
        assert_eq!(found["measured"], json!(["transcript"]));
        assert_eq!(found["missing"], json!(["levels", "scenes"]));
        assert_eq!(found["sentences"], 12);
        let candidates = found["candidates"].as_array().unwrap();
        assert!(!candidates.is_empty() && candidates.len() <= 3, "{found}");
        let best = &candidates[0];
        let seconds = best["seconds"].as_f64().unwrap();
        assert!((4.0..=8.0).contains(&seconds), "{best}");
        assert!(best["text"].as_str().unwrap().ends_with('.'));
        assert!(best["score"].is_number() && best["reasons"].is_object());
        let (start, end) = (
            best["span"]["start"]["flicks"].as_i64().unwrap(),
            best["span"]["end"]["flicks"].as_i64().unwrap(),
        );
        assert_eq!(
            best["timeline_start"].as_i64().unwrap(),
            start + 2 * SECOND,
            "the clip starts 2 s into its timeline"
        );
        assert_eq!(best["timeline_end"].as_i64().unwrap(), end + 2 * SECOND);

        // The short: centred, captioned and titled, built in one step.
        let short = |follow: Option<&str>| Short {
            clip: clip.clone(),
            start,
            end,
            follow: follow.map(str::to_string),
            captions: true,
            title: Some("Best bit".into()),
            ..Short::default()
        };
        let followed = make_short(&workspace, &places, &short(Some("person"))).unwrap_err();
        missing(&followed, "pose", "shorts");
        let region = make_short(&workspace, &places, &short(Some("region"))).unwrap_err();
        assert!(region.message.contains("person or a subject"), "{region:?}");
        let backwards = make_short(
            &workspace,
            &places,
            &Short {
                end: start,
                ..short(None)
            },
        )
        .unwrap_err();
        assert_eq!(backwards.code, codes::INVALID_ARGUMENTS);
        let before = revision(&workspace);
        let made = make_short(&workspace, &places, &short(None)).unwrap();
        let report = &made["short"];
        assert_eq!(made["response"]["committed"], true);
        assert_eq!(made["response"]["revision"], before + 1, "one step");
        assert_eq!(report["centred"], Value::Null);
        // Two sentences of four words at least; the start moves to the nearest frame,
        // which may leave out a word that starts half a frame before it.
        assert!(report["caption_words"].as_u64().unwrap() >= 7, "{report}");
        assert!(report["name"].as_str().unwrap().starts_with("Short"));
        let duration = report["duration"].as_i64().unwrap();
        assert!(
            (duration - (end - start)).abs() <= SECOND / 15,
            "whole frames: {report}"
        );
        let timeline = report["timeline"].as_str().unwrap();
        let shown = workspace
            .query(
                "timeline.read",
                json!({ "timeline": timeline, "detail": "full" }),
            )
            .unwrap();
        assert_eq!(shown["format"]["width"], 1080);
        assert_eq!(shown["format"]["height"], 1920);
        let clips: Vec<&Value> = shown["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["clips"].as_array().unwrap())
            .collect();
        assert!(clips.iter().any(|c| c["id"] == report["clip"]), "{shown}");
        assert!(clips.iter().any(|c| c["kind"] == "caption"), "{shown}");
        assert!(clips.iter().any(|c| c["kind"] == "text"), "{shown}");
        let entry = last_edit(&workspace);
        assert_eq!(
            (entry["actor"].as_str(), entry["reason"].as_str()),
            (Some("human"), Some("Make a short"))
        );
        let stale = make_short(
            &workspace,
            &places,
            &Short {
                expected_revision: Some(before),
                ..short(None)
            },
        )
        .unwrap_err();
        assert_eq!(stale.code, codes::REVISION_CONFLICT);
        done(workspace, runtime, work);
    }

    /// True when the FFmpeg programs are here; in CI (`FOSSCUT_REQUIRE_FFMPEG`) they must be.
    fn ffmpeg_programs() -> bool {
        let runs = |program: &str| {
            std::process::Command::new(program)
                .arg("-version")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        let found = runs("ffmpeg") && runs("ffprobe");
        let required =
            std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
        assert!(
            found || !required,
            "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg or ffprobe is missing"
        );
        found
    }

    /// Writes 8 seconds of clicks at 120 beats a minute, the first at 0.37 s and every
    /// fourth louder, to `click.wav` in `work`.
    fn click_track(work: &Path) -> PathBuf {
        let status = std::process::Command::new("ffmpeg")
            .current_dir(work)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(
                "aevalsrc='(0.4+0.4*eq(mod(floor((t+0.13)/0.5),4),1))*sin(2*PI*2000*t)\
                 *exp(-mod(t+0.13,0.5)/0.0015)*gte(t,0.37)':s=44100:d=8",
            )
            .args(["-c:a", "pcm_s16le", "click.wav"])
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg writes the click track");
        work.join("click.wav")
    }

    #[test]
    fn beats_are_marked_and_cuts_moved_onto_them() {
        if !ffmpeg_programs() {
            eprintln!("skipped: ffmpeg or ffprobe is missing");
            return;
        }
        let (workspace, runtime, work, places) = opened("beats");
        let file = click_track(&work);
        let (bundle, _, _) = workspace.snapshot().unwrap();
        let args = fosscut_engine::import::media_add_args(&file, None, &bundle).unwrap();
        let music_asset = workspace
            .execute(Request::single("media.add", args))
            .unwrap()
            .results[0]["asset"]
            .as_str()
            .unwrap()
            .to_string();
        let music = place(&workspace, &music_asset, 0.0, 0.0, 8.0);
        // Three shots whose cuts (on frames at 30 per second) are near beats: 1.2 s near
        // 1.37 s, 2.5 s near 2.37 s.
        let video = video_asset(&workspace, 10);
        let first = place(&workspace, &video, 0.0, 0.0, 1.2);
        place(&workspace, &video, 1.2, 2.0, 1.3);
        place(&workspace, &video, 2.5, 4.0, 2.0);
        let view = workspace
            .query("timeline.read", json!({ "detail": "full" }))
            .unwrap();
        let track = view["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["clips"][0]["id"] == first.as_str())
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        let every = beat_markers(&workspace, &places, &music, Some(0), None).unwrap_err();
        assert!(every.message.contains("every 1 to 16"), "{every:?}");
        assert!(beat_markers(&workspace, &places, "nope", None, None).is_err());
        let not_video = beat_cuts(&workspace, &places, &music, &music, None, None).unwrap_err();
        assert_eq!(not_video.code, codes::INVALID_ARGUMENTS, "{not_video:?}");

        let before = revision(&workspace);
        let marked = beat_markers(&workspace, &places, &music, None, None).unwrap();
        let beats = &marked["beats"];
        let count = beats["markers"].as_u64().unwrap();
        assert!((14..=17).contains(&count), "{beats}");
        let tempo = beats["tempo_bpm"].as_f64().unwrap();
        assert!((tempo - 120.0).abs() < 2.0, "{beats}");
        assert_eq!(beats["existing"], 0);
        assert_eq!(marked["response"]["revision"], before + 1);
        assert_eq!(last_edit(&workspace)["reason"], "Beat markers");
        // Again: every beat has its marker, so nothing is added.
        let again = beat_markers(&workspace, &places, &music, None, None).unwrap();
        assert_eq!(again["beats"]["markers"], 0);
        assert_eq!(again["beats"]["existing"].as_u64().unwrap(), count);
        assert_eq!(again["response"], Value::Null);

        let cut = beat_cuts(&workspace, &places, &track, &music, None, None).unwrap();
        let cuts = &cut["cuts"];
        assert_eq!(
            (cuts["track"].as_str(), cuts["music"].as_str()),
            (Some(track.as_str()), Some(music.as_str()))
        );
        let snapped = cuts["snapped"].as_array().unwrap();
        assert_eq!(snapped.len(), 2, "{cuts}");
        let frame = SECOND / 30;
        for (moved, beat) in snapped.iter().zip([1.37, 2.37]) {
            let to = moved["to"].as_i64().unwrap() as f64 / SECOND as f64;
            assert!((to - beat).abs() <= 1.5 / 30.0, "{moved}");
            assert_ne!(moved["from"], moved["to"]);
        }
        assert!(cuts["unsnapped"].as_array().unwrap().is_empty(), "{cuts}");
        assert!(cuts["tolerance"].as_i64().unwrap() > frame && cuts.get("commands").is_none());
        assert_eq!(cut["response"]["committed"], true);
        assert_eq!(last_edit(&workspace)["reason"], "Cut to the beat");
        let starts: Vec<i64> = workspace
            .query("timeline.read", json!({ "detail": "full" }))
            .unwrap()["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == track.as_str())
            .unwrap()["clips"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["start"]["flicks"].as_i64().unwrap())
            .collect();
        assert_eq!(
            starts[1..],
            [
                snapped[0]["to"].as_i64().unwrap(),
                snapped[1]["to"].as_i64().unwrap()
            ]
        );
        // On the beats now: nothing moves, and nothing is applied.
        let still = beat_cuts(&workspace, &places, &track, &music, None, None).unwrap();
        assert_eq!(still["response"], Value::Null);
        done(workspace, runtime, work);
    }
}
