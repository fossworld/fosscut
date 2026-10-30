//! Captions from transcripts and subtitle files (ADR-0021 item 7). `generate` maps the
//! cached transcript of each clip on a source track to timeline time; `import` reads SRT;
//! `export` writes SRT. Front ends hand the words to the core's `captions.add` with the
//! caller's safety-contract fields, so every write goes through the command framework.

use std::path::PathBuf;

use fosscut_model::{ClipContent, Id, Project, TrackKind};
use fosscut_time::{Flicks, Rate};
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;
use crate::analysis::{AnalysisRequest, Analyzer, Options, TranscriptOptions, cached};

/// What `generate` reads.
#[derive(Clone, Debug)]
pub struct GenerateRequest {
    /// The project bundle.
    pub bundle: PathBuf,
    /// Timeline; the primary timeline when `None`.
    pub timeline: Option<Id>,
    /// The track whose clips are captioned; the timeline's first video or audio track
    /// with sound when `None`.
    pub source_track: Option<Id>,
    /// The transcript options the media was analyzed with.
    pub transcript: TranscriptOptions,
    /// The analysis cache directory.
    pub cache: PathBuf,
    /// The models directory.
    pub models: PathBuf,
    /// Leave out words the transcript marks as filler candidates ("um", "uh").
    pub skip_fillers: bool,
    /// Song lyrics: without a source track, caption the first audio track with sound
    /// (the music) before any video track.
    pub lyrics: bool,
}

/// How lyrics are paged: a sung line per page, broken at breaths.
pub fn lyrics_paging() -> Value {
    json!({ "max_words": 8, "max_lines": 1, "max_gap_ms": 900, "linger_ms": 600 })
}

/// Words ready for `captions.add`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Generated {
    /// The timeline.
    pub timeline: Id,
    /// The track whose clips were captioned.
    pub source_track: Id,
    /// Clips captioned.
    pub clips: usize,
    /// Words in timeline order, as `captions.add` takes them.
    pub words: Vec<Value>,
}

/// A timeline time as a `captions.add` time input.
fn flicks(time: Flicks) -> Value {
    json!({ "flicks": time.get() })
}

/// The words of every clip on the source track, from cached transcripts, in timeline time.
/// A word belongs to a clip when it starts inside the part of the media the clip shows; it
/// ends no later than the clip.
pub fn generate(project: &Project, request: &GenerateRequest) -> Result<Generated, EngineError> {
    let timeline_id = request
        .timeline
        .clone()
        .unwrap_or_else(|| project.manifest.primary_timeline.clone());
    let timeline =
        project
            .timelines
            .get(&timeline_id)
            .ok_or_else(|| EngineError::TimelineNotFound {
                timeline: timeline_id.to_string(),
            })?;
    let with_sound = |kind: TrackKind| matches!(kind, TrackKind::Video | TrackKind::Audio);
    let has_audio = |asset: &Id| project.media.get(asset).is_some_and(|a| a.has_audio());
    let track = match &request.source_track {
        Some(id) => timeline
            .track(id)
            .ok_or_else(|| EngineError::InvalidArguments {
                message: format!("track {id} does not exist in {timeline_id}"),
            })?,
        None => {
            let candidate = |t: &&fosscut_model::Track| {
                with_sound(t.kind)
                    && t.clips
                        .iter()
                        .any(|c| c.content.asset().is_some_and(has_audio))
            };
            let music = request.lyrics.then(|| {
                timeline
                    .tracks
                    .iter()
                    .filter(|t| t.kind == TrackKind::Audio)
                    .find(candidate)
            });
            music
                .flatten()
                .or_else(|| timeline.tracks.iter().find(candidate))
        }
        .ok_or_else(|| EngineError::NoTranscript {
            assets: Vec::new(),
            message: format!("{timeline_id} has no clips with sound to caption"),
        })?,
    };
    let mut words = Vec::new();
    let mut missing = Vec::new();
    let mut clips = 0;
    for clip in &track.clips {
        let asset = match &clip.content {
            ClipContent::Video { asset, .. } | ClipContent::Audio { asset, .. }
                if has_audio(asset) =>
            {
                asset
            }
            _ => continue,
        };
        let analysis = AnalysisRequest {
            bundle: request.bundle.clone(),
            asset: asset.clone(),
            analyzers: vec![Analyzer::Transcript],
            options: Options {
                transcript: request.transcript.clone(),
                ..Options::default()
            },
            refresh: false,
            cache: request.cache.clone(),
            models: request.models.clone(),
        };
        // With no speech model there can be no transcript: report what is missing, and the
        // hint leads to transcribing, which asks for a model.
        let found = match cached(project, &analysis) {
            Err(EngineError::ModelNotInstalled { .. }) => None,
            other => other?,
        };
        let Some(transcript) = found.and_then(|r| r.results.transcript) else {
            if !missing.contains(&asset.to_string()) {
                missing.push(asset.to_string());
            }
            continue;
        };
        clips += 1;
        words.extend(clip_words(clip, &transcript, request.skip_fillers));
    }
    if !missing.is_empty() {
        return Err(EngineError::NoTranscript {
            message: format!("no cached transcript for {}", missing.join(", ")),
            assets: missing,
        });
    }
    if words.is_empty() {
        return Err(EngineError::NoTranscript {
            assets: Vec::new(),
            message: format!("the clips on {} contain no recognised words", track.id),
        });
    }
    Ok(Generated {
        timeline: timeline_id,
        source_track: track.id.clone(),
        clips,
        words,
    })
}

/// A clip's transcript words in timeline time, as `captions.add` takes them. A word
/// belongs to the clip when it starts inside the part of the media the clip shows; it ends
/// no later than the clip. Words the recognizer timed as zero-length are kept with no
/// length, so none goes missing; fillers are left out with `skip_fillers`.
fn clip_words(
    clip: &fosscut_model::Clip,
    transcript: &fosscut_analysis::speech::Transcript,
    skip_fillers: bool,
) -> Vec<Value> {
    let fillers: std::collections::BTreeSet<usize> = if skip_fillers {
        transcript.fillers.iter().map(|f| f.word).collect()
    } else {
        Default::default()
    };
    let mut words = Vec::new();
    for (index, word) in transcript.words.iter().enumerate() {
        if fillers.contains(&index) {
            continue;
        }
        // Words map to the timeline through the clip's speed (ADR-0023); a reversed clip
        // plays a word's end first. Frozen clips are silent and caption nothing.
        let (Some(a), Some(b)) = (
            clip.offset_of_source(word.start.flicks),
            clip.offset_of_source(
                word.end
                    .flicks
                    .min(clip.content.source_in().unwrap_or_default() + clip.source_length()),
            ),
        ) else {
            continue;
        };
        let (from, to) = (a.min(b), a.max(b));
        if from >= clip.duration {
            continue;
        }
        words.push(json!({
            "text": word.text,
            "start": flicks(clip.start + from),
            "end": flicks(clip.start + to.min(clip.duration)),
        }));
    }
    words
}

/// One subtitle cue.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    /// When it appears, in milliseconds.
    pub start_ms: i64,
    /// When it disappears, in milliseconds.
    pub end_ms: i64,
    /// Its lines, without markup.
    pub lines: Vec<String>,
}

fn srt_error(line: usize, message: impl Into<String>) -> EngineError {
    EngineError::InvalidSrt {
        line,
        message: message.into(),
    }
}

/// `HH:MM:SS,mmm` (or with a full stop) in milliseconds.
fn srt_time(text: &str, line: usize) -> Result<i64, EngineError> {
    let text = text.trim();
    let (clock, millis) = text
        .split_once([',', '.'])
        .ok_or_else(|| srt_error(line, format!("{text:?} is not HH:MM:SS,mmm")))?;
    let parts: Vec<&str> = clock.split(':').collect();
    let number = |s: &str| {
        s.parse::<i64>()
            .ok()
            .filter(|n| *n >= 0)
            .ok_or_else(|| srt_error(line, format!("{text:?} is not HH:MM:SS,mmm")))
    };
    let [hours, minutes, seconds] = parts.as_slice() else {
        return Err(srt_error(line, format!("{text:?} is not HH:MM:SS,mmm")));
    };
    let (hours, minutes, seconds) = (number(hours)?, number(minutes)?, number(seconds)?);
    if millis.len() != 3 || minutes >= 60 || seconds >= 60 {
        return Err(srt_error(line, format!("{text:?} is not HH:MM:SS,mmm")));
    }
    Ok(((hours * 60 + minutes) * 60 + seconds) * 1000 + number(millis)?)
}

/// Text without markup such as `<i>` or `{\an8}`.
fn plain(line: &str) -> String {
    let mut out = String::new();
    let mut skip: Option<char> = None;
    for c in line.chars() {
        match (skip, c) {
            (None, '<') => skip = Some('>'),
            (None, '{') => skip = Some('}'),
            (Some(end), c) if c == end => skip = None,
            (Some(_), _) => {}
            (None, c) => out.push(c),
        }
    }
    out.trim().to_string()
}

/// Reads an SRT file: numbered cues with `start --> end` and text lines, separated by
/// blank lines. UTF-8, with or without a byte order mark, LF or CRLF.
pub fn parse_srt(text: &str) -> Result<Vec<Cue>, EngineError> {
    let text = text.trim_start_matches('\u{feff}');
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let mut cues = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        // An optional cue number, then the timing line.
        if !lines[i].contains("-->") {
            i += 1;
        }
        let timing = lines
            .get(i)
            .filter(|l| l.contains("-->"))
            .ok_or_else(|| srt_error(i + 1, "expected a timing line: start --> end"))?;
        let (start, end) = timing
            .split_once("-->")
            .ok_or_else(|| srt_error(i + 1, "expected start --> end"))?;
        // Position settings after the end time are ignored.
        let end = end.split_whitespace().next().unwrap_or("");
        let (start_ms, end_ms) = (srt_time(start, i + 1)?, srt_time(end, i + 1)?);
        if end_ms <= start_ms {
            return Err(srt_error(i + 1, "the cue ends before it starts"));
        }
        if cues
            .last()
            .is_some_and(|previous: &Cue| start_ms < previous.end_ms)
        {
            return Err(srt_error(i + 1, "the cue overlaps the previous one"));
        }
        i += 1;
        let mut text_lines = Vec::new();
        while i < lines.len() && !lines[i].trim().is_empty() {
            let line = plain(lines[i]);
            if !line.is_empty() {
                text_lines.push(line);
            }
            i += 1;
        }
        if text_lines.is_empty() {
            return Err(srt_error(i + 1, "the cue has no text"));
        }
        cues.push(Cue {
            start_ms,
            end_ms,
            lines: text_lines,
        });
    }
    if cues.is_empty() {
        return Err(srt_error(1, "the file has no cues"));
    }
    Ok(cues)
}

/// Words for `captions.add` from cues: one page per cue, lines kept, and the cue's time
/// (snapped to frames of `rate`, so a file exported from frame-aligned pages comes back to
/// the same frames) shared across its words by length.
pub fn cue_words(cues: &[Cue], rate: Rate) -> Vec<Value> {
    let mut words = Vec::new();
    for cue in cues {
        let start = rate.snap(Flicks::from_millis(cue.start_ms));
        let end = rate.snap(Flicks::from_millis(cue.end_ms)).max(start);
        let all: Vec<(usize, &str)> = cue
            .lines
            .iter()
            .enumerate()
            .flat_map(|(line, text)| text.split_whitespace().map(move |w| (line, w)))
            .collect();
        let total: usize = all.iter().map(|(_, w)| w.chars().count()).sum();
        let span = i128::from((end - start).get());
        let at = |count: usize| {
            Flicks::new(start.get() + (span * count as i128 / total.max(1) as i128) as i64)
        };
        let mut before = 0;
        let mut line_of_previous = None;
        for (index, (line, text)) in all.iter().enumerate() {
            let length = text.chars().count();
            words.push(json!({
                "text": text,
                "start": flicks(at(before)),
                "end": flicks(at(before + length)),
                "new_page": index == 0,
                "new_line": line_of_previous.is_some_and(|previous| previous != *line),
            }));
            before += length;
            line_of_previous = Some(*line);
        }
    }
    words
}

/// Paging that keeps imported cues exactly as they are: only their own breaks apply.
pub fn cue_paging() -> Value {
    json!({
        "max_words": 10_000,
        "max_lines": 1_000,
        "max_chars_per_line": 100_000,
        "max_gap_ms": u32::MAX,
        "linger_ms": 0,
    })
}

fn srt_clock(time: Flicks) -> String {
    let ms = (i128::from(time.get()) * 1000 + i128::from(fosscut_time::FLICKS_PER_SECOND) / 2)
        / i128::from(fosscut_time::FLICKS_PER_SECOND);
    let ms = ms.max(0) as i64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// Which text of bilingual captions a subtitle file holds.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum SrtText {
    /// The words as shown.
    #[default]
    Original,
    /// Each page's translation, or its words where it has none.
    Translation,
    /// The words, then the translation line under them.
    Both,
}

impl SrtText {
    /// The choice named `original`, `translation` or `both`.
    pub fn named(name: &str) -> Result<SrtText, EngineError> {
        match name {
            "original" => Ok(SrtText::Original),
            "translation" => Ok(SrtText::Translation),
            "both" => Ok(SrtText::Both),
            other => Err(EngineError::InvalidArguments {
                message: format!(
                    "subtitle text must be original, translation or both, not {other:?}"
                ),
            }),
        }
    }
}

/// The caption pages of a caption track as SRT: one cue per page, times rounded to the
/// millisecond, lines kept, with the pages' translations as `text` says.
pub fn export_srt(
    project: &Project,
    timeline: Option<&Id>,
    track: Option<&Id>,
    text: SrtText,
) -> Result<String, EngineError> {
    let timeline_id = timeline
        .cloned()
        .unwrap_or_else(|| project.manifest.primary_timeline.clone());
    let doc = project
        .timelines
        .get(&timeline_id)
        .ok_or_else(|| EngineError::TimelineNotFound {
            timeline: timeline_id.to_string(),
        })?;
    let track = match track {
        Some(id) => doc.track(id).filter(|t| t.kind == TrackKind::Caption),
        None => doc.tracks.iter().find(|t| t.kind == TrackKind::Caption),
    }
    .ok_or_else(|| EngineError::NoCaptions {
        timeline: timeline_id.to_string(),
    })?;
    let mut out = String::new();
    let mut number = 0;
    for clip in &track.clips {
        let ClipContent::Caption(caption) = &clip.content else {
            continue;
        };
        number += 1;
        let words = caption.text();
        let shown = match (text, &caption.translation) {
            (SrtText::Translation, Some(translation)) => translation.clone(),
            (SrtText::Both, Some(translation)) => format!("{words}\n{translation}"),
            _ => words,
        };
        out.push_str(&format!(
            "{number}\n{} --> {}\n{shown}\n\n",
            srt_clock(clip.start),
            srt_clock(clip.end()),
        ));
    }
    if number == 0 {
        return Err(EngineError::NoCaptions {
            timeline: timeline_id.to_string(),
        });
    }
    Ok(out)
}

/// The first `cues` cues of an SRT text, so a caller without file access can check what was
/// written.
pub fn srt_preview(srt: &str, cues: usize) -> String {
    srt.split("\n\n")
        .filter(|cue| !cue.trim().is_empty())
        .take(cues)
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_files_parse_with_markup_crlf_and_settings() {
        let text = "\u{feff}1\r\n00:00:01,200 --> 00:00:02,500 X1:0\r\n<i>Hello</i> there\r\n{\\an8}second line\r\n\r\n2\r\n00:00:03.000 --> 00:00:04,000\r\nBye\r\n";
        let cues = parse_srt(text).unwrap();
        assert_eq!(
            cues,
            [
                Cue {
                    start_ms: 1200,
                    end_ms: 2500,
                    lines: vec!["Hello there".into(), "second line".into()],
                },
                Cue {
                    start_ms: 3000,
                    end_ms: 4000,
                    lines: vec!["Bye".into()],
                },
            ]
        );
        // Without cue numbers too.
        assert_eq!(
            parse_srt("00:00:00,000 --> 00:00:01,000\nHi\n").unwrap()[0].lines,
            ["Hi"]
        );
    }

    #[test]
    fn broken_srt_files_name_the_line() {
        for (text, line) in [
            ("", 1),
            ("1\nnot a time\nHi\n", 2),
            ("1\n00:00:02,000 --> 00:00:01,000\nHi\n", 2),
            ("1\n00:00:01,000 --> 00:00:02,000\n\n", 3),
            ("1\n00:61:00,000 --> 00:62:00,000\nHi\n", 2),
            ("1\n00:00:01,00 --> 00:00:02,000\nHi\n", 2),
            ("1\n00:00:01 --> 00:00:02,000\nHi\n", 2),
            ("1\n0:0 --> 0:1\nHi\n", 2),
            (
                "1\n00:00:01,000 --> 00:00:03,000\nA\n\n2\n00:00:02,000 --> 00:00:04,000\nB\n",
                6,
            ),
        ] {
            match parse_srt(text) {
                Err(EngineError::InvalidSrt { line: found, .. }) => {
                    assert_eq!(found, line, "{text:?}")
                }
                other => panic!("{text:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn cues_become_pages_of_words_spread_by_length() {
        let cues = [Cue {
            start_ms: 1000,
            end_ms: 2000,
            lines: vec!["ab cd".into(), "efgh".into()],
        }];
        let words = cue_words(&cues, Rate::FPS_30);
        let ms = |v: &Value| v["flicks"].as_i64().unwrap() / 705_600;
        let shown: Vec<(&str, i64, i64, bool, bool)> = words
            .iter()
            .map(|w| {
                (
                    w["text"].as_str().unwrap(),
                    ms(&w["start"]),
                    ms(&w["end"]),
                    w["new_page"].as_bool().unwrap(),
                    w["new_line"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("ab", 1000, 1250, true, false),
                ("cd", 1250, 1500, false, false),
                ("efgh", 1500, 2000, false, true),
            ]
        );
        assert_eq!(cue_paging()["linger_ms"], 0);
        assert_eq!(srt_clock(Flicks::from_millis(3_723_004)), "01:02:03,004");
        assert_eq!(srt_clock(Rate::FPS_30.frames_to_flicks(37)), "00:00:01,233");
        assert_eq!(plain("<b>x</b> {y}z"), "x z");
        let srt = "1\na\nA\n\n2\nb\nB\n\n3\nc\nC\n\n";
        assert_eq!(srt_preview(srt, 2), "1\na\nA\n\n2\nb\nB");
        assert_eq!(srt_preview(srt, 9), "1\na\nA\n\n2\nb\nB\n\n3\nc\nC");
    }

    #[test]
    fn clip_words_keep_zero_length_words_and_skip_fillers_when_asked() {
        use fosscut_analysis::Moment;
        use fosscut_analysis::speech::{Filler, Transcript, Word};
        use fosscut_model::{Clip, IdKind};
        let at = |ms: i64| Moment::new(Flicks::from_millis(ms));
        let word = |text: &str, from: i64, to: i64| Word {
            text: text.into(),
            start: at(from),
            end: at(to),
            probability: 0.9,
        };
        let transcript = Transcript {
            model: "test".into(),
            language: "en".into(),
            segments: Vec::new(),
            words: vec![
                word("Well", 1000, 1300),
                word("um", 1300, 1600),
                word("ride", 1600, 1600),
                word("along", 1600, 2000),
                word("later", 9000, 9500),
            ],
            fillers: vec![Filler {
                word: 1,
                text: "um".into(),
                start: at(1300),
                end: at(1600),
            }],
            pauses: Vec::new(),
            min_pause_ms: 300,
        };
        // A clip showing the media from 1 s for 2 s, placed at 10 s.
        let clip = Clip::new(
            Id::new(IdKind::Clip, 1),
            Flicks::from_seconds(10).unwrap(),
            Flicks::from_seconds(2).unwrap(),
            ClipContent::Audio {
                asset: Id::new(IdKind::Asset, 2),
                source_in: Flicks::from_seconds(1).unwrap(),
            },
        );
        let texts = |words: &[Value]| -> Vec<String> {
            words
                .iter()
                .map(|w| w["text"].as_str().unwrap().to_string())
                .collect()
        };
        let all = clip_words(&clip, &transcript, false);
        assert_eq!(texts(&all), ["Well", "um", "ride", "along"]);
        assert_eq!(all[2]["start"], all[2]["end"], "kept with no length");
        assert_eq!(
            all[0]["start"]["flicks"],
            Flicks::from_seconds(10).unwrap().get()
        );
        let tidy = clip_words(&clip, &transcript, true);
        assert_eq!(texts(&tidy), ["Well", "ride", "along"]);
    }
}
