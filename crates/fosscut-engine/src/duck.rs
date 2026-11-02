//! Ducking (ADR-0034, measured edits): music made quieter while someone speaks. The speech
//! is read from each speech clip's newest cached transcript (its words), or, without one,
//! from its `silences` analysis (measured now and cached, since it needs no model): the
//! sound between the quiet ranges is speech. Speech is mapped to the timeline through each
//! clip's `source_in` and speed, pauses shorter than the gap are bridged, and every music
//! clip's `volume` gets keyframes that fall by `depth` over `attack` before speech starts
//! and rise back over `release` after it ends.
//!
//! Like the other planners, [`duck`] returns ordinary commands (`keyframe.set_all` on each
//! music clip's volume) that front ends apply in one request with the caller's revision,
//! dry run and request id, so the change is one undoable step.

use std::path::PathBuf;

use fosscut_analysis::{Analyzer, Transcript};
use fosscut_model::{Clip, ClipContent, Id, Project, Timeline};
use fosscut_time::Flicks;
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;
use crate::analysis::{AnalysisRequest, Options, analyze};

/// How deep the music goes under speech, in dB, by default.
pub const DEFAULT_DEPTH_DB: f64 = -12.0;
/// How long the music takes to go down before speech, by default: 0.2 s.
pub const DEFAULT_ATTACK: Flicks = Flicks::from_millis(200);
/// How long the music takes to come back after speech, by default: 0.5 s.
pub const DEFAULT_RELEASE: Flicks = Flicks::from_millis(500);
/// Pauses in the speech shorter than this keep the music down, by default: 0.8 s.
pub const DEFAULT_GAP: Flicks = Flicks::from_millis(800);

/// Where speech is read from.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechSource {
    /// The cached transcript when there is one, otherwise the silences.
    #[default]
    Auto,
    /// The newest cached transcript's words; fails when there is none.
    Transcript,
    /// The sound between quiet ranges of the `silences` analysis.
    Silences,
}

impl SpeechSource {
    /// The source called `name`: `auto`, `transcript` or `silences`.
    pub fn named(name: &str) -> Result<SpeechSource, EngineError> {
        match name {
            "auto" => Ok(SpeechSource::Auto),
            "transcript" => Ok(SpeechSource::Transcript),
            "silences" => Ok(SpeechSource::Silences),
            other => Err(invalid(format!(
                "duck: speech from {other:?}: use auto, transcript or silences"
            ))),
        }
    }
}

/// How the music ducks.
#[derive(Clone, Debug, PartialEq)]
pub struct DuckOptions {
    /// How far the music goes down under speech, in dB, from -60 to -1.
    pub depth_db: f64,
    /// How long it takes to go down, ending where speech starts; 0 to 5 s, rounded to
    /// whole frames, at least one.
    pub attack: Flicks,
    /// How long it takes to come back, starting where speech ends; 0 to 5 s, rounded to
    /// whole frames, at least one.
    pub release: Flicks,
    /// Pauses shorter than this (and never shorter than attack plus release) keep the music
    /// down; 0 to 10 s.
    pub gap: Flicks,
    /// Where speech is read from.
    pub source: SpeechSource,
}

impl Default for DuckOptions {
    fn default() -> DuckOptions {
        DuckOptions {
            depth_db: DEFAULT_DEPTH_DB,
            attack: DEFAULT_ATTACK,
            release: DEFAULT_RELEASE,
            gap: DEFAULT_GAP,
            source: SpeechSource::Auto,
        }
    }
}

/// Where analyses are read and cached.
#[derive(Clone, Debug, PartialEq)]
pub struct DuckContext {
    /// The project bundle; relative media paths resolve against it.
    pub bundle: PathBuf,
    /// The analysis cache, normally [`crate::cache_dir`].
    pub cache: PathBuf,
    /// The models folder, normally [`crate::models_dir`].
    pub models: PathBuf,
}

/// A stretch of timeline time.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Range {
    /// Where it starts.
    pub start: Flicks,
    /// Where it ends.
    pub end: Flicks,
}

/// Where one speech clip's speech came from.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpeechClip {
    /// The clip.
    pub clip: Id,
    /// `transcript` or `silences`.
    pub source: &'static str,
    /// How many stretches of speech it has on the timeline.
    pub ranges: usize,
}

/// One music clip's ducking.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DuckedClip {
    /// The clip.
    pub clip: Id,
    /// Its volume away from speech.
    pub level: f64,
    /// Its volume under speech.
    pub ducked: f64,
    /// The keyframes set.
    pub keys: usize,
}

/// What [`duck`] found, with the commands that apply it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DuckPlan {
    /// The timeline.
    pub timeline: Id,
    /// The depth, attack, release and gap used: dB and seconds.
    pub depth_db: f64,
    /// See `depth_db`.
    pub attack_seconds: f64,
    /// See `depth_db`.
    pub release_seconds: f64,
    /// See `depth_db`.
    pub gap_seconds: f64,
    /// Each speech clip and where its speech came from.
    pub speech_clips: Vec<SpeechClip>,
    /// The speech on the timeline, short pauses bridged, in order.
    pub speech: Vec<Range>,
    /// The music clips ducked.
    pub clips: Vec<DuckedClip>,
    /// Music clips no speech overlaps, left as they are.
    pub untouched: Vec<Id>,
    /// `keyframe.set_all` calls on the music clips' `volume`.
    pub commands: Vec<Value>,
}

fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::InvalidArguments {
        message: message.into(),
    }
}

/// The timeline and clip of `id`.
fn find<'a>(project: &'a Project, id: &Id) -> Result<(&'a Timeline, &'a Clip), EngineError> {
    project
        .timelines
        .values()
        .find_map(|t| {
            t.tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|c| &c.id == id)
                .map(|c| (t, c))
        })
        .ok_or_else(|| invalid(format!("clip {id} does not exist")))
}

/// The asset a clip plays sound from, when it does.
fn sound_asset<'a>(project: &Project, clip: &'a Clip) -> Option<&'a Id> {
    match &clip.content {
        ClipContent::Video { asset, .. } | ClipContent::Audio { asset, .. } => project
            .media
            .get(asset)
            .is_some_and(|a| a.has_audio())
            .then_some(asset),
        _ => None,
    }
}

/// A clip's volume away from any ducking: its loudest volume keyframe, or its volume.
pub fn level(clip: &Clip) -> f64 {
    match clip.keyframes.get("volume") {
        Some(keys) if !keys.is_empty() => keys
            .iter()
            .filter_map(|k| k.value.number())
            .fold(0.0, f64::max),
        _ => clip.volume,
    }
}

fn seconds(time: Flicks) -> f64 {
    time.as_seconds_f64()
}

fn round(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// Plans keyframes that duck `music` clips under the speech of `speech` clips (every other
/// clip with sound on the music's timeline when empty).
pub fn duck(
    project: &Project,
    context: &DuckContext,
    music: &[Id],
    speech: &[Id],
    options: &DuckOptions,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<DuckPlan, EngineError> {
    let five = Flicks::from_millis(5_000);
    if !(options.depth_db.is_finite() && (-60.0..=-1.0).contains(&options.depth_db)) {
        return Err(invalid(format!(
            "duck: depth {} dB is outside -60 to -1",
            options.depth_db
        )));
    }
    for (name, value, most) in [
        ("attack", options.attack, five),
        ("release", options.release, five),
        ("gap", options.gap, Flicks::from_millis(10_000)),
    ] {
        if value.is_negative() || value > most {
            return Err(invalid(format!(
                "duck: {name} {} s is outside 0 to {} s",
                seconds(value),
                seconds(most)
            )));
        }
    }
    let Some(first) = music.first() else {
        return Err(invalid("duck: name the music clips to duck"));
    };
    let (timeline, _) = find(project, first)?;
    let mut music_clips = Vec::with_capacity(music.len());
    for id in music {
        let (on, clip) = find(project, id)?;
        if on.id != timeline.id {
            return Err(invalid(format!(
                "duck: {id} is on {}, not {} with {first}",
                on.id, timeline.id
            )));
        }
        if sound_asset(project, clip).is_none()
            && !matches!(clip.content, ClipContent::Timeline { .. })
        {
            return Err(invalid(format!(
                "duck: {id} is a {} clip without sound",
                clip.content.kind_name()
            )));
        }
        music_clips.push(clip);
    }
    let speech_clips: Vec<&Clip> = if speech.is_empty() {
        timeline
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter(|c| !music.contains(&c.id) && sound_asset(project, c).is_some())
            .collect()
    } else {
        let mut found = Vec::with_capacity(speech.len());
        for id in speech {
            let (on, clip) = find(project, id)?;
            if music.contains(id) {
                return Err(invalid(format!("duck: {id} is both music and speech")));
            }
            if on.id != timeline.id {
                return Err(invalid(format!(
                    "duck: {id} is on {}, not {} with the music",
                    on.id, timeline.id
                )));
            }
            if sound_asset(project, clip).is_none() {
                return Err(invalid(format!(
                    "duck: {id} is a {} clip without sound to listen to",
                    clip.content.kind_name()
                )));
            }
            found.push(clip);
        }
        found
    };
    if speech_clips.is_empty() {
        return Err(invalid(format!(
            "duck: {} has no clips with sound besides the music",
            timeline.id
        )));
    }

    // Speech on the timeline, clip by clip.
    let mut ranges = Vec::new();
    let mut reports = Vec::with_capacity(speech_clips.len());
    for clip in speech_clips {
        let Some(asset) = sound_asset(project, clip) else {
            continue;
        };
        let (source, spans) = speech_spans(project, context, asset, options.source, progress)?;
        let before = ranges.len();
        ranges.extend(spans.into_iter().filter_map(|span| on_timeline(clip, span)));
        reports.push(SpeechClip {
            clip: clip.id.clone(),
            source,
            ranges: ranges.len() - before,
        });
    }
    // Keyframes fall on frames: so do the speech's edges and the ramps, which last at least
    // a frame, so every key keeps its value.
    let rate = timeline.format.rate;
    let frame = rate.frame_duration();
    let attack = rate.snap(options.attack).max(frame);
    let release = rate.snap(options.release).max(frame);
    let ranges = ranges
        .into_iter()
        .map(|r| Range {
            start: rate.snap(r.start),
            end: rate.snap(r.end),
        })
        .filter(|r| r.start < r.end)
        .collect();
    let bridge = options.gap.max(attack + release);
    let speech = merge(ranges, bridge);

    let factor = 10f64.powf(options.depth_db / 20.0);
    let mut clips = Vec::new();
    let mut untouched = Vec::new();
    let mut commands = Vec::new();
    for clip in music_clips {
        let base = level(clip);
        let low = base * factor;
        let keys = envelope(clip, &speech, (base, low), (attack, release), rate);
        if keys.is_empty() {
            untouched.push(clip.id.clone());
            continue;
        }
        let calls: Vec<Value> = keys
            .iter()
            .map(|(offset, value)| {
                json!({ "offset": { "flicks": offset.get() }, "value": round(*value) })
            })
            .collect();
        commands.push(json!({
            "command": "keyframe.set_all",
            "args": { "clip": clip.id, "property": "volume", "keys": calls },
        }));
        clips.push(DuckedClip {
            clip: clip.id.clone(),
            level: round(base),
            ducked: round(low),
            keys: keys.len(),
        });
    }
    Ok(DuckPlan {
        timeline: timeline.id.clone(),
        depth_db: options.depth_db,
        attack_seconds: seconds(attack),
        release_seconds: seconds(release),
        gap_seconds: seconds(bridge),
        speech_clips: reports,
        speech,
        clips,
        untouched,
        commands,
    })
}

/// Speech in `asset`'s own time: its transcript's words, or the sound between silences.
fn speech_spans(
    project: &Project,
    context: &DuckContext,
    asset: &Id,
    source: SpeechSource,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<(&'static str, Vec<Range>), EngineError> {
    if source != SpeechSource::Silences {
        let transcript = crate::measured::newest::<Transcript>(
            project,
            &context.bundle,
            &context.cache,
            asset,
            Analyzer::Transcript,
            &|_| true,
        )?;
        match transcript {
            Some((transcript, _)) => {
                let words = transcript
                    .words
                    .iter()
                    .map(|w| Range {
                        start: w.start.flicks,
                        // Words timed as zero-length still count for a moment.
                        end: w.end.flicks.max(w.start.flicks + Flicks::from_millis(50)),
                    })
                    .collect();
                return Ok(("transcript", words));
            }
            None if source == SpeechSource::Transcript => {
                return Err(crate::measured::missing(
                    asset,
                    Analyzer::Transcript,
                    json!({}),
                    "duck",
                ));
            }
            None => {}
        }
    }
    let report = analyze(
        project,
        &AnalysisRequest {
            bundle: context.bundle.clone(),
            asset: asset.clone(),
            analyzers: vec![Analyzer::Silences],
            options: Options::default(),
            refresh: false,
            cache: context.cache.clone(),
            models: context.models.clone(),
        },
        progress,
    )?;
    let quiet = report
        .results
        .silences
        .map(|s| s.ranges)
        .unwrap_or_default();
    let duration = project
        .media
        .get(asset)
        .and_then(|a| a.info.duration)
        .unwrap_or_default();
    let mut spans = Vec::new();
    let mut at = Flicks::ZERO;
    for range in quiet {
        if range.start.flicks > at {
            spans.push(Range {
                start: at,
                end: range.start.flicks,
            });
        }
        at = at.max(range.end.flicks);
    }
    if duration > at {
        spans.push(Range {
            start: at,
            end: duration,
        });
    }
    Ok(("silences", spans))
}

/// A span of `clip`'s media on the timeline, through its speed; `None` when the clip does
/// not play it.
fn on_timeline(clip: &Clip, span: Range) -> Option<Range> {
    let source_in = clip.content.source_in()?;
    let source_end = source_in + clip.source_length();
    let (from, to) = (span.start.max(source_in), span.end.min(source_end));
    if from >= to {
        return None;
    }
    let (a, b) = (clip.offset_of_source(from)?, clip.offset_of_source(to)?);
    let (a, b) = (a.min(b), a.max(b).min(clip.duration));
    (a < b).then(|| Range {
        start: clip.start + a,
        end: clip.start + b,
    })
}

/// `ranges` sorted, with those closer than `bridge` joined.
fn merge(mut ranges: Vec<Range>, bridge: Flicks) -> Vec<Range> {
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start - last.end < bridge => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// The volume keyframes of `clip` (offsets from its start, on frames of `rate`, as
/// keyframes are) that hold `base`, fall to `low` over `attack` before each speech range
/// and rise back over `release` after it. Ranges are at least attack plus release apart, so
/// the ramps never overlap. Empty when no range reaches the clip.
fn envelope(
    clip: &Clip,
    speech: &[Range],
    (base, low): (f64, f64),
    (attack, release): (Flicks, Flicks),
    rate: fosscut_time::Rate,
) -> Vec<(Flicks, f64)> {
    let (start, end) = (clip.start, clip.end());
    let touching: Vec<&Range> = speech
        .iter()
        .filter(|r| r.start - attack < end && r.end + release > start)
        .collect();
    if touching.is_empty() {
        return Vec::new();
    }
    let value = |t: Flicks| -> f64 {
        for r in &touching {
            if t >= r.start && t <= r.end {
                return low;
            }
            if t < r.start && t > r.start - attack {
                let x = (r.start - t).get() as f64 / attack.get().max(1) as f64;
                return low + (base - low) * x;
            }
            if t > r.end && t < r.end + release {
                let x = (t - r.end).get() as f64 / release.get().max(1) as f64;
                return low + (base - low) * x;
            }
        }
        base
    };
    let mut times = vec![start, end];
    for r in &touching {
        times.extend([r.start - attack, r.start, r.end, r.end + release]);
    }
    let mut times: Vec<Flicks> = times
        .into_iter()
        .map(|t| start + rate.snap(t - start))
        .filter(|t| *t >= start && *t <= end)
        .collect();
    times.sort();
    times.dedup();
    let mut keys: Vec<(Flicks, f64)> = times.into_iter().map(|t| (t - start, value(t))).collect();
    // Values hold before the first key and after the last, and between equal keys: keep
    // only the keys where the volume turns.
    let mut kept: Vec<(Flicks, f64)> = Vec::with_capacity(keys.len());
    for (index, key) in keys.iter().enumerate() {
        let previous = kept.last().map(|k| k.1);
        let next = keys.get(index + 1).map(|k| k.1);
        let flat_before = previous.is_none_or(|p| (p - key.1).abs() < 1e-9);
        let flat_after = next.is_none_or(|n| (n - key.1).abs() < 1e-9);
        if flat_before && flat_after && !(previous.is_none() && next.is_none()) {
            continue;
        }
        kept.push(*key);
    }
    keys = kept;
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_model::{IdKind, Keyframe};
    use fosscut_time::Rate;

    fn ms(n: i64) -> Flicks {
        Flicks::from_millis(n)
    }

    fn range(a: i64, b: i64) -> Range {
        Range {
            start: ms(a),
            end: ms(b),
        }
    }

    fn music(start: i64, length: i64) -> Clip {
        Clip::new(
            Id::new(IdKind::Clip, 9),
            ms(start),
            ms(length),
            ClipContent::Audio {
                asset: Id::new(IdKind::Asset, 1),
                source_in: Flicks::ZERO,
            },
        )
    }

    #[test]
    fn close_speech_joins_and_far_speech_stays_apart() {
        let merged = merge(
            vec![range(3000, 4000), range(0, 1000), range(1500, 2000)],
            ms(600),
        );
        assert_eq!(merged, vec![range(0, 2000), range(3000, 4000)]);
        assert_eq!(merge(Vec::new(), ms(600)), Vec::new());
    }

    #[test]
    fn the_envelope_ramps_down_before_speech_and_back_after() {
        let clip = music(0, 10_000);
        let keys = envelope(
            &clip,
            &[range(2000, 4000), range(6000, 7000)],
            (0.8, 0.2),
            (ms(200), ms(500)),
            Rate::FPS_30,
        );
        let expected = [
            (1800, 0.8),
            (2000, 0.2),
            (4000, 0.2),
            (4500, 0.8),
            (5800, 0.8),
            (6000, 0.2),
            (7000, 0.2),
            (7500, 0.8),
        ];
        assert_eq!(keys.len(), expected.len(), "{keys:?}");
        for ((offset, value), (t, v)) in keys.iter().zip(expected) {
            assert_eq!(*offset, ms(t));
            assert!((value - v).abs() < 1e-9);
        }
        // Speech running into the clip's start: it starts ducked (held from its first key),
        // rising after.
        let late = music(3000, 5000);
        let keys = envelope(
            &late,
            &[range(2000, 4000)],
            (1.0, 0.25),
            (ms(200), ms(500)),
            Rate::FPS_30,
        );
        assert_eq!(keys, vec![(ms(1000), 0.25), (ms(1500), 1.0)]);
        // A ramp cut by the clip's end ends the clip partway.
        let short = music(0, 1900);
        let keys = envelope(
            &short,
            &[range(2000, 4000)],
            (1.0, 0.0),
            (ms(200), ms(500)),
            Rate::FPS_30,
        );
        assert_eq!(keys.len(), 2);
        assert!((keys[1].1 - 0.5).abs() < 1e-9, "{keys:?}");
        // No speech near the clip: nothing to do.
        let far = envelope(
            &music(20_000, 1000),
            &[range(0, 1000)],
            (1.0, 0.25),
            (ms(200), ms(500)),
            Rate::FPS_30,
        );
        assert!(far.is_empty());
        // Times fall on frames, and two that share one become one key.
        let snapped = envelope(
            &music(0, 2000),
            &[range(1010, 1020)],
            (1.0, 0.5),
            (ms(5), ms(5)),
            Rate::FPS_30,
        );
        assert!(snapped.iter().all(|(t, _)| Rate::FPS_30.snap(*t) == *t));
        assert!(snapped.windows(2).all(|w| w[0].0 < w[1].0), "{snapped:?}");
    }

    #[test]
    fn levels_come_from_the_loudest_volume_key() {
        let mut clip = music(0, 1000);
        clip.volume = 0.5;
        assert_eq!(level(&clip), 0.5);
        clip.keyframes.insert(
            "volume".into(),
            vec![
                Keyframe {
                    time: Flicks::ZERO,
                    value: fosscut_model::KeyValue::Number(0.7),
                    interpolation: Default::default(),
                },
                Keyframe {
                    time: ms(500),
                    value: fosscut_model::KeyValue::Number(0.2),
                    interpolation: Default::default(),
                },
            ],
        );
        assert_eq!(level(&clip), 0.7);
        assert_eq!(
            SpeechSource::named("silences").unwrap(),
            SpeechSource::Silences
        );
        assert_eq!(
            SpeechSource::named("music").unwrap_err().code(),
            "fosscut.request.invalid_arguments"
        );
    }
}
