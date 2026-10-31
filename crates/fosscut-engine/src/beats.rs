//! Editing to the beat of music (CC-199): the beats `fosscut-analysis` finds in a music
//! clip's sound (the `beats` analyzer, cached by content), mapped to the timeline through
//! the clip's speed.
//!
//! - [`markers`] returns `marker.add` calls at the beats (every one, every second or every
//!   fourth, counted from a downbeat when there is one) inside a music clip, or inside every
//!   clip of a timeline that plays a music asset;
//! - [`cut_to_beats`] returns the `clip.trim` and `clip.move` calls that move the cuts
//!   between the clips of a video track onto the nearest beats.
//!
//! Like the engine's other planners, these measure (or read the cache) and propose
//! commands; front ends apply them in one request, so the edit is one undoable step. The
//! cuts are planned by applying each candidate to a scratch copy of the project through the
//! command framework, so the calls returned are exactly those that succeed there, in order:
//! applied at the same revision they give the same timeline.

use std::path::PathBuf;

pub use fosscut_analysis::BeatOptions;
use fosscut_analysis::Beats;
use fosscut_core::{Request, Session};
use fosscut_model::{Clip, ClipContent, Id, MemoryStore, Project, Timeline, TrackKind};
use fosscut_time::Flicks;
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;
use crate::analysis::{AnalysisRequest, Analyzer, Options, analyze};

/// Where the beats are measured, and with what.
#[derive(Clone, Debug, PartialEq)]
pub struct BeatContext {
    /// The project bundle; relative media paths resolve against it.
    pub bundle: PathBuf,
    /// The analysis cache, normally [`crate::cache_dir`].
    pub cache: PathBuf,
    /// The models folder, normally [`crate::models_dir`].
    pub models: PathBuf,
    /// The `beats` analyzer's options.
    pub options: BeatOptions,
}

/// The music whose beats are marked.
#[derive(Clone, Debug, PartialEq)]
pub enum BeatSource {
    /// One clip with sound.
    Clip(Id),
    /// Every clip that plays this asset on `timeline` (the primary one when `None`).
    Asset {
        /// The music asset.
        asset: Id,
        /// The timeline.
        timeline: Option<Id>,
    },
}

/// Beat markers to add.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BeatMarkers {
    /// The timeline the markers go on.
    pub timeline: Id,
    /// The music's tempo in beats per minute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempo_bpm: Option<f64>,
    /// Beats in a bar, when downbeats were found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beats_per_bar: Option<u32>,
    /// How periodic the music is, from 0 to 1.
    pub confidence: f64,
    /// Timeline times of the markers, in order.
    pub at: Vec<Flicks>,
    /// Beats left out because a marker is already on their frame.
    pub existing: usize,
    /// `marker.add` calls, one per marker.
    pub commands: Vec<Value>,
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

/// The asset a clip plays sound from.
fn music_asset(project: &Project, clip: &Clip) -> Result<Id, EngineError> {
    match &clip.content {
        ClipContent::Video { asset, .. } | ClipContent::Audio { asset, .. } => {
            if project.media.get(asset).is_some_and(|a| a.has_audio()) {
                Ok(asset.clone())
            } else {
                Err(EngineError::NoAudio {
                    asset: asset.to_string(),
                })
            }
        }
        other => Err(invalid(format!(
            "{} is a {} clip; beats are found in clips with sound",
            clip.id,
            other.kind_name()
        ))),
    }
}

/// The beats of `asset`'s sound: from the cache, or measured now.
fn measure(
    project: &Project,
    context: &BeatContext,
    asset: &Id,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<Beats, EngineError> {
    let report = analyze(
        project,
        &AnalysisRequest {
            bundle: context.bundle.clone(),
            asset: asset.clone(),
            analyzers: vec![Analyzer::Beats],
            options: Options {
                beats: context.options.clone(),
                ..Options::default()
            },
            refresh: false,
            cache: context.cache.clone(),
            models: context.models.clone(),
        },
        progress,
    )?;
    let beats = report.results.beats.unwrap_or_else(|| Beats {
        tempo_bpm: None,
        confidence: 0.0,
        beats_per_bar: None,
        downbeat_confidence: None,
        beats: Vec::new(),
    });
    if beats.beats.is_empty() {
        return Err(EngineError::NoBeats {
            asset: asset.to_string(),
        });
    }
    Ok(beats)
}

/// The indices of the beats kept with `every`: counted from the first downbeat, or from the
/// first beat.
fn kept(beats: &Beats, every: u32) -> Vec<usize> {
    let every = every.max(1) as usize;
    let phase = beats.beats.iter().position(|b| b.downbeat).unwrap_or(0) % every;
    (0..beats.beats.len())
        .filter(|i| i % every == phase)
        .collect()
}

/// Where the kept beats fall on the timeline inside `clip`, on frames, with whether each is
/// a downbeat.
fn on_timeline(
    clip: &Clip,
    timeline: &Timeline,
    beats: &Beats,
    keep: &[usize],
) -> Vec<(Flicks, bool)> {
    let rate = timeline.format.rate;
    let mut found: Vec<(Flicks, bool)> = keep
        .iter()
        .filter_map(|&i| {
            let beat = &beats.beats[i];
            let offset = clip.offset_of_source(beat.time.flicks)?;
            let at = rate.snap(clip.start + offset);
            (at >= clip.start && at < clip.end()).then_some((at, beat.downbeat))
        })
        .collect();
    found.sort();
    found.dedup_by(|a, b| a.0 == b.0);
    found
}

fn check_every(every: u32) -> Result<(), EngineError> {
    if (1..=16).contains(&every) {
        Ok(())
    } else {
        Err(invalid(format!(
            "every is {every}; mark every 1 to 16 beats (usually 1, 2 or 4)"
        )))
    }
}

/// Measures the beats of `source` (unless cached) and returns the `marker.add` calls that
/// mark every `every`-th beat it plays on the timeline (1 to 16; counted from the first
/// downbeat when the music has them, so 4 marks bars). Markers are named `downbeat` or
/// `beat`; frames that already have a marker are left alone.
pub fn markers(
    project: &Project,
    context: &BeatContext,
    source: &BeatSource,
    every: u32,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<BeatMarkers, EngineError> {
    check_every(every)?;
    let (timeline, clips, asset): (&Timeline, Vec<&Clip>, Id) = match source {
        BeatSource::Clip(id) => {
            let (timeline, clip) = find(project, id)?;
            (timeline, vec![clip], music_asset(project, clip)?)
        }
        BeatSource::Asset { asset, timeline } => {
            let id = timeline
                .clone()
                .unwrap_or_else(|| project.manifest.primary_timeline.clone());
            let doc = project
                .timelines
                .get(&id)
                .ok_or_else(|| EngineError::TimelineNotFound {
                    timeline: id.to_string(),
                })?;
            if project.media.get(asset).is_none() {
                return Err(EngineError::AssetNotFound {
                    asset: asset.to_string(),
                });
            }
            let clips: Vec<&Clip> = doc
                .tracks
                .iter()
                .flat_map(|t| &t.clips)
                .filter(|c| {
                    matches!(&c.content, ClipContent::Video { asset: a, .. } | ClipContent::Audio { asset: a, .. } if a == asset)
                })
                .collect();
            if clips.is_empty() {
                return Err(invalid(format!("no clip on {id} plays {asset}")));
            }
            (doc, clips, asset.clone())
        }
    };
    let beats = measure(project, context, &asset, progress)?;
    let keep = kept(&beats, every);
    let mut at: Vec<(Flicks, bool)> = clips
        .iter()
        .flat_map(|clip| on_timeline(clip, timeline, &beats, &keep))
        .collect();
    at.sort();
    at.dedup_by(|a, b| a.0 == b.0);
    let before = at.len();
    let rate = timeline.format.rate;
    at.retain(|(time, _)| {
        !timeline
            .markers
            .iter()
            .any(|m| rate.nearest_frame(m.at) == rate.nearest_frame(*time))
    });
    let commands = at
        .iter()
        .map(|(time, downbeat)| {
            json!({
                "command": "marker.add",
                "args": {
                    "at": { "flicks": time.get() },
                    "name": if *downbeat { "downbeat" } else { "beat" },
                    "timeline": timeline.id,
                },
            })
        })
        .collect();
    Ok(BeatMarkers {
        timeline: timeline.id.clone(),
        tempo_bpm: beats.tempo_bpm,
        beats_per_bar: beats.beats_per_bar,
        confidence: beats.confidence,
        existing: before - at.len(),
        at: at.into_iter().map(|(t, _)| t).collect(),
        commands,
    })
}

/// How [`cut_to_beats`] moves cuts.
#[derive(Clone, Debug, PartialEq)]
pub struct CutOptions {
    /// Only every `every`-th beat counts (1 to 16), counted from the first downbeat.
    pub every: u32,
    /// How far a cut may move; half the spacing of the beats that count when `None`, so
    /// every cut reaches its nearest one.
    pub tolerance: Option<Flicks>,
    /// The shortest a clip may become.
    pub min_length: Flicks,
}

impl Default for CutOptions {
    fn default() -> CutOptions {
        CutOptions {
            every: 1,
            tolerance: None,
            min_length: Flicks::from_millis(250),
        }
    }
}

/// A cut moved onto a beat.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SnappedCut {
    /// The clip before the cut.
    pub before: Id,
    /// The clip after it.
    pub after: Id,
    /// Where the cut was.
    pub from: Flicks,
    /// The beat it is on now.
    pub to: Flicks,
}

/// A cut left where it was.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnsnappedCut {
    /// The clip before it, unless a gap is before it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Id>,
    /// The clip after it.
    pub after: Id,
    /// Where it is.
    pub at: Flicks,
    /// Why it stays: no beat within the tolerance, or the error the nearest beats gave
    /// (such as `fosscut.clip.source_range` when a clip's media ends too soon).
    pub reason: String,
}

/// Cuts moved onto beats, and the calls that move them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BeatCuts {
    /// The video track.
    pub track: Id,
    /// The music clip.
    pub music: Id,
    /// The music's tempo in beats per minute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempo_bpm: Option<f64>,
    /// Beats that counted, inside the music clip.
    pub beats: usize,
    /// The tolerance used.
    pub tolerance: Flicks,
    /// Cuts moved, and clips after gaps moved, onto beats; a cut already on a beat is
    /// listed with `from` equal to `to`.
    pub snapped: Vec<SnappedCut>,
    /// Cuts and clips that stay.
    pub unsnapped: Vec<UnsnappedCut>,
    /// `clip.trim` and `clip.move` calls, in the order they apply.
    pub commands: Vec<Value>,
}

/// The clip `id` in a scratch project.
fn current<'a>(project: &'a Project, id: &Id) -> Option<&'a Clip> {
    project
        .timelines
        .values()
        .flat_map(|t| &t.tracks)
        .flat_map(|t| &t.clips)
        .find(|c| &c.id == id)
}

fn trim(clip: &Id, edge: &str, at: Flicks) -> Value {
    json!({ "command": "clip.trim", "args": { "clip": clip, edge: { "flicks": at.get() } } })
}

/// Measures the beats of `music` (unless cached) and returns the calls that move each cut
/// between consecutive clips of `track` onto the nearest beat within the tolerance: the
/// clip before ends there and the one after starts there (a roll edit, so the edit keeps
/// its length and order and nothing overlaps). A clip after a gap, with a gap or nothing
/// after it, moves so it starts on a beat. Cuts whose nearest beats would need more media
/// than a clip has, or leave a clip shorter than `min_length`, try the next nearest beat
/// and otherwise stay, with the reason.
pub fn cut_to_beats(
    project: &Project,
    context: &BeatContext,
    track: &Id,
    music: &Id,
    options: &CutOptions,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<BeatCuts, EngineError> {
    check_every(options.every)?;
    let (timeline, music_clip) = find(project, music)?;
    let doc = timeline.track(track).ok_or_else(|| {
        invalid(format!(
            "track {track} is not on {}, where {music} is",
            timeline.id
        ))
    })?;
    if doc.kind != TrackKind::Video {
        return Err(invalid(format!(
            "{track} is not a video track; cut_to_beats moves the cuts of a video track"
        )));
    }
    let asset = music_asset(project, music_clip)?;
    let beats = measure(project, context, &asset, progress)?;
    let keep = kept(&beats, options.every);
    let points: Vec<Flicks> = on_timeline(music_clip, timeline, &beats, &keep)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    if points.is_empty() {
        return Err(EngineError::NoBeats {
            asset: asset.to_string(),
        });
    }
    let tolerance = match options.tolerance {
        Some(t) if t.is_negative() => {
            return Err(invalid("cut_to_beats: the tolerance cannot be negative"));
        }
        Some(t) => t,
        None => {
            let mut gaps: Vec<i64> = points.windows(2).map(|w| (w[1] - w[0]).get()).collect();
            gaps.sort_unstable();
            Flicks::new(gaps.get(gaps.len() / 2).map_or(i64::MAX / 4, |g| g / 2))
        }
    };
    let min_length = options
        .min_length
        .max(timeline.format.rate.frame_duration());
    let mut ids: Vec<(Flicks, Id)> = doc.clips.iter().map(|c| (c.start, c.id.clone())).collect();
    ids.sort();
    let mut scratch = Session::create(MemoryStore::new(), project.clone()).map_err(|e| {
        invalid(format!(
            "cut_to_beats: the project cannot be planned on: {}",
            e.message
        ))
    })?;
    let mut snapped = Vec::new();
    let mut unsnapped = Vec::new();
    let mut commands = Vec::new();
    let nearest = |at: Flicks, low: Flicks, high: Flicks| -> Vec<Flicks> {
        let mut near: Vec<Flicks> = points
            .iter()
            .copied()
            .filter(|&b| (b - at).get().abs() <= tolerance.get() && b >= low && b <= high)
            .collect();
        near.sort_by_key(|&b| ((b - at).get().abs(), b));
        near
    };
    for k in 1..ids.len() {
        let state = scratch.project();
        let (Some(a), Some(b)) = (current(state, &ids[k - 1].1), current(state, &ids[k].1)) else {
            continue;
        };
        let (a, b) = (a.clone(), b.clone());
        if a.end() == b.start {
            let cut = b.start;
            if points.contains(&cut) {
                snapped.push(SnappedCut {
                    before: a.id.clone(),
                    after: b.id.clone(),
                    from: cut,
                    to: cut,
                });
                continue;
            }
            let candidates = nearest(cut, a.start + min_length, b.end() - min_length);
            let mut reason = "no beat within the tolerance".to_string();
            let mut done = false;
            for beat in candidates {
                let calls = if beat > cut {
                    [trim(&b.id, "start", beat), trim(&a.id, "end", beat)]
                } else {
                    [trim(&a.id, "end", beat), trim(&b.id, "start", beat)]
                };
                match apply(&mut scratch, &calls) {
                    Ok(()) => {
                        commands.extend(calls);
                        snapped.push(SnappedCut {
                            before: a.id.clone(),
                            after: b.id.clone(),
                            from: cut,
                            to: beat,
                        });
                        done = true;
                        break;
                    }
                    Err(code) => reason = code,
                }
            }
            if !done {
                unsnapped.push(UnsnappedCut {
                    before: Some(a.id.clone()),
                    after: b.id.clone(),
                    at: cut,
                    reason,
                });
            }
        } else if a.end() < b.start {
            // A clip after a gap moves whole, unless a cut follows it, which it would break.
            let next = ids
                .get(k + 1)
                .and_then(|(_, id)| current(scratch.project(), id))
                .map(|c| c.start);
            if next == Some(b.end()) {
                continue;
            }
            if points.contains(&b.start) {
                continue;
            }
            let room = next.map_or(Flicks::new(i64::MAX / 4), |n| n - b.duration);
            let candidates = nearest(b.start, a.end(), room);
            let mut reason = "no beat within the tolerance".to_string();
            let mut done = false;
            for beat in candidates {
                let calls = [json!({
                    "command": "clip.move",
                    "args": { "clip": b.id, "start": { "flicks": beat.get() } },
                })];
                match apply(&mut scratch, &calls) {
                    Ok(()) => {
                        commands.extend(calls);
                        snapped.push(SnappedCut {
                            before: a.id.clone(),
                            after: b.id.clone(),
                            from: b.start,
                            to: beat,
                        });
                        done = true;
                        break;
                    }
                    Err(code) => reason = code,
                }
            }
            if !done {
                unsnapped.push(UnsnappedCut {
                    before: None,
                    after: b.id.clone(),
                    at: b.start,
                    reason,
                });
            }
        }
    }
    Ok(BeatCuts {
        track: track.clone(),
        music: music.clone(),
        tempo_bpm: beats.tempo_bpm,
        beats: points.len(),
        tolerance,
        snapped,
        unsnapped,
        commands,
    })
}

/// Applies `calls` to the scratch session in one request, as a front end would without
/// `allow_destructive`; the error code when they fail.
pub(crate) fn apply(scratch: &mut Session<MemoryStore>, calls: &[Value]) -> Result<(), String> {
    let request: Request = serde_json::from_value(json!({
        "commands": calls,
        "actor": "planner",
    }))
    .map_err(|e| format!("fosscut.request.invalid: {e}"))?;
    scratch
        .execute(request)
        .map(|_| ())
        .map_err(|e| e.code.to_string())
}

#[cfg(test)]
mod tests {
    use fosscut_analysis::{Beat, Moment};

    use super::*;

    fn beats(downbeat_at: Option<usize>, n: usize) -> Beats {
        Beats {
            tempo_bpm: Some(120.0),
            confidence: 0.9,
            beats_per_bar: downbeat_at.map(|_| 4),
            downbeat_confidence: None,
            beats: (0..n)
                .map(|i| Beat {
                    time: Moment::new(Flicks::from_millis(500 * i as i64)),
                    strength: 1.0,
                    downbeat: downbeat_at.is_some_and(|d| i % 4 == d),
                })
                .collect(),
        }
    }

    #[test]
    fn every_counts_from_the_first_downbeat() {
        assert_eq!(kept(&beats(None, 9), 4), vec![0, 4, 8]);
        assert_eq!(kept(&beats(Some(1), 9), 4), vec![1, 5]);
        assert_eq!(kept(&beats(Some(3), 9), 2), vec![1, 3, 5, 7]);
        assert_eq!(kept(&beats(None, 3), 1), vec![0, 1, 2]);
        assert!(check_every(0).is_err() && check_every(17).is_err() && check_every(4).is_ok());
    }
}
