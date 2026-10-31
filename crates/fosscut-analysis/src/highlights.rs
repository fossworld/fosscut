//! Candidate short clips in long footage: stretches of whole sentences, scored from cached
//! measurements, each with the numbers behind its score. FossCut measures; which candidate
//! makes a good short is the user's agent's call, so every reason is reported for it to
//! choose or rank again (ADR-0012).
//!
//! **Sentences** are runs of transcript words ending at a word that ends with `.`, `!`,
//! `?` or `…` (closing quotes and brackets aside); a transcript without such punctuation is
//! split where the recognizer's segments start. Sentences are found in the whole transcript,
//! so one that a clip's range cuts into is left out. A candidate is one or more consecutive
//! sentences lasting from `min_seconds` to `max_seconds`, from just before its first word
//! (at most 0.15 s, never into the word before) to just after its last (at most 0.3 s, never
//! into the next word), so it never starts or ends inside a word.
//!
//! **Scores**, each from 0 to 1, are combined by `weights` over the measurements that are
//! there:
//!
//! - `density`: words per second next to the footage's own rate: 0.5 at the same rate, 1 at
//!   one and a half times it, 0 at half;
//! - `pauses`: fewer is better: 1 without pauses between words (gaps of 0.3 s or more), 0
//!   when they fill a quarter of the candidate;
//! - `loudness` (with `levels`): half from the loudest window's height over the footage's
//!   median level (1 at 6 LU above), half from the standard deviation of its windows (1 at
//!   4 LU);
//! - `scenes` (with `scenes`): shot cuts per minute, 1 at 12 or more;
//! - `self_contained`: the silence before its first word and after its last, each 1 at
//!   0.6 s or more (or with nothing said on that side), averaged: a candidate that starts
//!   after a pause and ends before one stands on its own.
//!
//! The best candidates are kept greedily, each sharing at most `max_overlap` of the shorter
//! one's length with those kept before it.

use fosscut_time::{FLICKS_PER_SECOND, Flicks};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Span;
use crate::levels::Levels;
use crate::scenes::Scenes;
use crate::speech::{Transcript, Word};

/// The gap between words that counts as a pause, in seconds.
const PAUSE: f64 = 0.3;
/// How much of the silence before the first word a candidate takes, in seconds.
const LEAD_IN: f64 = 0.15;
/// How much of the silence after the last word a candidate takes, in seconds.
const TAIL: f64 = 0.3;

/// How much each score counts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HighlightWeights {
    /// Words per second (default 0.25).
    pub density: f64,
    /// Few pauses (default 0.2).
    pub pauses: f64,
    /// Loudness peaks and variation (default 0.2).
    pub loudness: f64,
    /// Shot cuts (default 0.15).
    pub scenes: f64,
    /// Silence before and after (default 0.2).
    pub self_contained: f64,
}

impl Default for HighlightWeights {
    fn default() -> HighlightWeights {
        HighlightWeights {
            density: 0.25,
            pauses: 0.2,
            loudness: 0.2,
            scenes: 0.15,
            self_contained: 0.2,
        }
    }
}

/// Options of the candidate search.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HighlightOptions {
    /// Shortest candidate, in seconds, 1 to 600 (default 15).
    pub min_seconds: f64,
    /// Longest candidate, in seconds, from `min_seconds` to 600 (default 60).
    pub max_seconds: f64,
    /// Most candidates returned, 1 to 100 (default 10).
    pub count: u32,
    /// The share of the shorter of two candidates they may have in common, 0 to 1
    /// (default 0: candidates do not overlap).
    pub max_overlap: f64,
    /// How much each score counts; negative weights count as 0.
    pub weights: HighlightWeights,
}

impl Default for HighlightOptions {
    fn default() -> HighlightOptions {
        HighlightOptions {
            min_seconds: 15.0,
            max_seconds: 60.0,
            count: 10,
            max_overlap: 0.0,
            weights: HighlightWeights::default(),
        }
    }
}

impl HighlightOptions {
    /// The options as they are used, every value inside its range.
    pub fn clamped(&self) -> HighlightOptions {
        let d = HighlightOptions::default();
        let finite = |v: f64, default: f64| if v.is_finite() { v } else { default };
        let weight = |v: f64| finite(v, 0.0).max(0.0);
        let min_seconds = finite(self.min_seconds, d.min_seconds).clamp(1.0, 600.0);
        HighlightOptions {
            min_seconds,
            max_seconds: finite(self.max_seconds, d.max_seconds).clamp(min_seconds, 600.0),
            count: self.count.clamp(1, 100),
            max_overlap: finite(self.max_overlap, d.max_overlap).clamp(0.0, 1.0),
            weights: HighlightWeights {
                density: weight(self.weights.density),
                pauses: weight(self.weights.pauses),
                loudness: weight(self.weights.loudness),
                scenes: weight(self.weights.scenes),
                self_contained: weight(self.weights.self_contained),
            },
        }
    }
}

/// What candidates are scored from, all in the media's own time.
#[derive(Clone, Copy, Debug)]
pub struct HighlightInput<'a> {
    /// The transcript: sentences, words and pauses.
    pub transcript: &'a Transcript,
    /// Loudness over time, if measured.
    pub levels: Option<&'a Levels>,
    /// Shot cuts, if measured.
    pub scenes: Option<&'a Scenes>,
    /// Only candidates inside this part of the media (a clip's part of it).
    pub range: Option<(Flicks, Flicks)>,
}

/// Each score of a candidate, from 0 to 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Scores {
    /// From words per second.
    pub density: f64,
    /// From the time in pauses.
    pub pauses: f64,
    /// From loudness peaks and variation, when levels were measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness: Option<f64>,
    /// From shot cuts, when scenes were measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenes: Option<f64>,
    /// From the silence before and after.
    pub self_contained: f64,
}

/// The measurements behind a candidate's score.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Reasons {
    /// Words per second.
    pub words_per_second: f64,
    /// The footage's own words per second, for comparison.
    pub typical_words_per_second: f64,
    /// The share of the candidate in pauses between words.
    pub pause_share: f64,
    /// The longest pause inside it, in seconds.
    pub longest_pause_seconds: f64,
    /// Silence before its first word, in seconds; absent when nothing is said before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_before_seconds: Option<f64>,
    /// Silence after its last word, in seconds; absent when nothing is said after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_after_seconds: Option<f64>,
    /// How far its loudest window is above the footage's median level, in LU.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness_peak_lu: Option<f64>,
    /// The standard deviation of its windows' loudness, in LU.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness_variation_lu: Option<f64>,
    /// Shot cuts inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_cuts: Option<u32>,
    /// Shot cuts per minute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuts_per_minute: Option<f64>,
    /// Each score.
    pub scores: Scores,
}

/// A candidate short.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Highlight {
    /// Where it starts and ends, from the start of the file.
    pub span: Span,
    /// Its length in seconds, to the millisecond.
    pub seconds: f64,
    /// Its score from 0 to 1, to 0.001.
    pub score: f64,
    /// What is said.
    pub text: String,
    /// Sentences in it.
    pub sentences: u32,
    /// Words in it.
    pub words: u32,
    /// The measurements behind the score.
    pub reasons: Reasons,
}

/// The candidates, best first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Highlights {
    /// Sentences found.
    pub sentences: u32,
    /// Candidates of an allowed length that were scored.
    pub considered: u32,
    /// The best, best first.
    pub candidates: Vec<Highlight>,
}

fn seconds(time: Flicks) -> f64 {
    time.get() as f64 / FLICKS_PER_SECOND as f64
}

fn flicks(seconds: f64) -> Flicks {
    Flicks::new((seconds * FLICKS_PER_SECOND as f64).round() as i64)
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// Whether `text` ends a sentence.
fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .trim_end_matches(['"', '\'', ')', ']', '\u{201d}', '\u{2019}', '\u{bb}'])
        .ends_with(['.', '!', '?', '\u{2026}'])
}

/// The sentences of `words` as ranges of word indices (last included): split after words
/// that end one, or, without any such word, where `segments` start.
pub fn sentences(words: &[Word], transcript: &Transcript) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    if words.is_empty() {
        return found;
    }
    let punctuated = words.iter().any(|w| ends_sentence(&w.text));
    let starts: Vec<Flicks> = transcript.segments.iter().map(|s| s.start.flicks).collect();
    let mut first = 0;
    for i in 0..words.len() {
        let last = i + 1 == words.len();
        let boundary = if punctuated {
            ends_sentence(&words[i].text)
        } else {
            // A segment starts between this word and the next.
            words.get(i + 1).is_some_and(|next| {
                starts
                    .iter()
                    .any(|&s| s > words[i].start.flicks && s <= next.start.flicks)
            })
        };
        if boundary || last {
            found.push((first, i));
            first = i + 1;
        }
    }
    found
}

/// The candidate shorts of `input`; see the [module documentation](self).
pub fn candidates(input: &HighlightInput<'_>, options: &HighlightOptions) -> Highlights {
    let options = options.clamped();
    let range = input.range.unwrap_or((Flicks::ZERO, Flicks::new(i64::MAX)));
    let words = &input.transcript.words;
    // Sentences of the whole transcript, so one the range cuts into is left out.
    let units: Vec<(usize, usize)> = sentences(words, input.transcript)
        .into_iter()
        .filter(|&(from, to)| {
            words[from].start.flicks >= range.0 && words[to].end.flicks <= range.1
        })
        .collect();
    let empty = Highlights {
        sentences: units.len() as u32,
        considered: 0,
        candidates: Vec::new(),
    };
    let (Some(&(first, _)), Some(&(_, last))) = (units.first(), units.last()) else {
        return empty;
    };
    let start_of = |i: usize| seconds(words[i].start.flicks);
    let end_of = |i: usize| seconds(words[i].end.flicks);
    let (range_start, range_end) = (seconds(range.0), seconds(range.1));
    let speaking = (end_of(last) - start_of(first)).max(1e-3);
    let typical = (last - first + 1) as f64 / speaking;
    // The silence after each word, to the next.
    let gaps: Vec<f64> = (0..words.len())
        .map(|i| {
            words.get(i + 1).map_or(0.0, |next| {
                (seconds(next.start.flicks) - end_of(i)).max(0.0)
            })
        })
        .collect();
    let median_level = input.levels.and_then(|levels| {
        let mut all: Vec<f64> = levels
            .between(start_of(first), end_of(last))
            .into_iter()
            .flatten()
            .collect();
        (!all.is_empty()).then(|| {
            all.sort_by(f64::total_cmp);
            all[all.len() / 2]
        })
    });
    let w = &options.weights;
    let mut scored: Vec<Highlight> = Vec::new();
    for (i, &(from, _)) in units.iter().enumerate() {
        for (j, &(_, to)) in units.iter().enumerate().skip(i) {
            // Nothing said before the first word or after the last counts as silence.
            let before = (from > 0).then(|| gaps[from - 1]);
            let after = (to + 1 < words.len()).then(|| gaps[to]);
            let start = (start_of(from) - LEAD_IN.min(before.unwrap_or(LEAD_IN))).max(range_start);
            let end = (end_of(to) + TAIL.min(after.unwrap_or(TAIL))).min(range_end);
            let length = end - start;
            if length > options.max_seconds {
                break;
            }
            if length < options.min_seconds {
                continue;
            }
            let count = to - from + 1;
            let pauses: Vec<f64> = gaps[from..to]
                .iter()
                .copied()
                .filter(|g| *g >= PAUSE)
                .collect();
            let pause_share = pauses.iter().sum::<f64>() / length;
            let longest = pauses.iter().copied().fold(0.0, f64::max);
            let wps = count as f64 / length;
            let density = (wps / typical.max(1e-6) - 0.5).clamp(0.0, 1.0);
            let pause_score = 1.0 - (pause_share / 0.25).clamp(0.0, 1.0);
            let separated = |gap: Option<f64>| gap.map_or(1.0, |g| (g / 0.6).min(1.0));
            let contained = 0.5 * (separated(before) + separated(after));
            let loudness = input.levels.zip(median_level).and_then(|(levels, median)| {
                let values: Vec<f64> = levels.between(start, end).into_iter().flatten().collect();
                if values.is_empty() {
                    return None;
                }
                let n = values.len() as f64;
                let mean = values.iter().sum::<f64>() / n;
                let deviation = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
                let peak = values.iter().copied().fold(f64::MIN, f64::max) - median;
                Some((peak, deviation))
            });
            let cuts = input.scenes.map(|scenes| {
                scenes
                    .cuts
                    .iter()
                    .filter(|c| {
                        let t = seconds(c.time.flicks);
                        t > start && t < end
                    })
                    .count() as u32
            });
            let loudness_score = loudness
                .map(|(peak, dev)| 0.5 * ((peak / 6.0).clamp(0.0, 1.0) + (dev / 4.0).min(1.0)));
            let per_minute = cuts.map(|c| f64::from(c) * 60.0 / length);
            let scene_score = per_minute.map(|p| (p / 12.0).min(1.0));
            let mut total =
                w.density * density + w.pauses * pause_score + w.self_contained * contained;
            let mut weight = w.density + w.pauses + w.self_contained;
            if let Some(s) = loudness_score {
                total += w.loudness * s;
                weight += w.loudness;
            }
            if let Some(s) = scene_score {
                total += w.scenes * s;
                weight += w.scenes;
            }
            let score = if weight > 0.0 { total / weight } else { 0.0 };
            let text = words[from..=to]
                .iter()
                .map(|w| w.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            scored.push(Highlight {
                span: Span::new(flicks(start), flicks(end)),
                seconds: round(length, 3),
                score: round(score, 3),
                text,
                sentences: (j - i + 1) as u32,
                words: count as u32,
                reasons: Reasons {
                    words_per_second: round(wps, 2),
                    typical_words_per_second: round(typical, 2),
                    pause_share: round(pause_share, 3),
                    longest_pause_seconds: round(longest, 3),
                    gap_before_seconds: before.map(|g| round(g, 3)),
                    gap_after_seconds: after.map(|g| round(g, 3)),
                    loudness_peak_lu: loudness.map(|l| round(l.0, 1)),
                    loudness_variation_lu: loudness.map(|l| round(l.1, 1)),
                    scene_cuts: cuts,
                    cuts_per_minute: per_minute.map(|p| round(p, 1)),
                    scores: Scores {
                        density: round(density, 3),
                        pauses: round(pause_score, 3),
                        loudness: loudness_score.map(|s| round(s, 3)),
                        scenes: scene_score.map(|s| round(s, 3)),
                        self_contained: round(contained, 3),
                    },
                },
            });
        }
    }
    let considered = scored.len() as u32;
    scored.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.span.start.flicks.cmp(&b.span.start.flicks))
            .then(a.span.end.flicks.cmp(&b.span.end.flicks))
    });
    let mut kept: Vec<Highlight> = Vec::new();
    for candidate in scored {
        if kept.len() >= options.count as usize {
            break;
        }
        let fits = kept.iter().all(|k| {
            let (a0, a1) = (k.span.start.flicks.get(), k.span.end.flicks.get());
            let (b0, b1) = (
                candidate.span.start.flicks.get(),
                candidate.span.end.flicks.get(),
            );
            let shared = (a1.min(b1) - a0.max(b0)).max(0) as f64;
            let shorter = (a1 - a0).min(b1 - b0).max(1) as f64;
            shared <= options.max_overlap * shorter
        });
        if fits {
            kept.push(candidate);
        }
    }
    Highlights {
        sentences: units.len() as u32,
        considered,
        candidates: kept,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Moment;
    use crate::scenes::Cut;
    use crate::speech::Segment;

    fn at(seconds: f64) -> Moment {
        Moment::new(flicks(seconds))
    }

    /// A transcript of `sentences`, each `(earliest start, words, seconds per word)`, with
    /// `gap` seconds between them.
    fn transcript(sentences: &[(f64, usize, f64)], gap: f64) -> Transcript {
        let mut words = Vec::new();
        let mut segments = Vec::new();
        let mut t = 0.0f64;
        for (n, &(start, count, per_word)) in sentences.iter().enumerate() {
            t = t.max(start);
            let first = t;
            for k in 0..count {
                let end = t + per_word * 0.8;
                let text = if k + 1 == count {
                    format!("w{n}x{k}.")
                } else {
                    format!("w{n}x{k}")
                };
                words.push(Word {
                    text,
                    start: at(t),
                    end: at(end),
                    probability: 0.9,
                });
                t += per_word;
            }
            segments.push(Segment {
                text: String::new(),
                start: at(first),
                end: at(t),
                no_speech_probability: 0.0,
            });
            t += gap;
        }
        Transcript {
            model: "test".into(),
            language: "en".into(),
            segments,
            words,
            fillers: Vec::new(),
            pauses: Vec::new(),
            min_pause_ms: 300,
        }
    }

    #[test]
    fn candidates_are_whole_sentences_of_an_allowed_length() {
        // Ten sentences of 20 words at 0.4 s a word (8 s each) with 1 s between them.
        let spoken = transcript(&[(0.0, 20, 0.4); 10], 1.0);
        let found = candidates(
            &HighlightInput {
                transcript: &spoken,
                levels: None,
                scenes: None,
                range: None,
            },
            &HighlightOptions::default(),
        );
        assert_eq!(found.sentences, 10);
        assert!(!found.candidates.is_empty());
        for candidate in &found.candidates {
            assert!(candidate.seconds >= 15.0 && candidate.seconds <= 60.0);
            let (start, end) = (candidate.span.start.flicks, candidate.span.end.flicks);
            // Never inside a word.
            for word in &spoken.words {
                let inside = |t: Flicks| t > word.start.flicks && t < word.end.flicks;
                assert!(!inside(start) && !inside(end), "{}", candidate.text);
            }
            assert!(candidate.text.ends_with('.'));
            assert!(candidate.reasons.scores.loudness.is_none());
            assert!(
                candidate
                    .reasons
                    .gap_before_seconds
                    .is_none_or(|g| g >= 0.99)
            );
        }
        // Non-overlapping by default.
        let mut spans: Vec<(i64, i64)> = found
            .candidates
            .iter()
            .map(|c| (c.span.start.flicks.get(), c.span.end.flicks.get()))
            .collect();
        spans.sort_unstable();
        assert!(spans.windows(2).all(|w| w[0].1 <= w[1].0));
    }

    #[test]
    fn dense_loud_cut_up_stretches_score_higher_and_reasons_explain_it() {
        // Slow sentences, then fast ones from 60 s.
        let mut plan = vec![(0.0, 10, 0.8); 6];
        plan.extend(vec![(60.0, 30, 0.3); 6]);
        let spoken = transcript(&plan, 0.8);
        let levels = Levels {
            window_ms: 1000,
            lufs: (0..200)
                .map(|s| Some(if s >= 60 && s % 5 == 0 { -14.0 } else { -24.0 }))
                .collect(),
        };
        let scenes = Scenes {
            threshold: 0.35,
            min_shot_ms: 500,
            frames: 6000,
            cuts: (0..30)
                .map(|k| Cut {
                    frame: 0,
                    time: at(62.0 + 3.0 * k as f64),
                    score: 1.0,
                })
                .collect(),
        };
        let found = candidates(
            &HighlightInput {
                transcript: &spoken,
                levels: Some(&levels),
                scenes: Some(&scenes),
                range: None,
            },
            &HighlightOptions {
                count: 3,
                ..HighlightOptions::default()
            },
        );
        assert_eq!(found.candidates.len(), 3);
        let best = &found.candidates[0];
        assert!(seconds(best.span.start.flicks) >= 59.0, "{best:?}");
        let reasons = &best.reasons;
        assert!(reasons.words_per_second > reasons.typical_words_per_second);
        assert!(reasons.loudness_peak_lu.unwrap() >= 9.9);
        assert!(reasons.scene_cuts.unwrap() >= 4);
        assert!(
            found
                .candidates
                .windows(2)
                .all(|w| w[0].score >= w[1].score)
        );
        // Only the slow part: a range keeps candidates inside it.
        let early = candidates(
            &HighlightInput {
                transcript: &spoken,
                levels: Some(&levels),
                scenes: Some(&scenes),
                range: Some((Flicks::ZERO, flicks(58.0))),
            },
            &HighlightOptions::default(),
        );
        assert!(!early.candidates.is_empty());
        assert!(
            early
                .candidates
                .iter()
                .all(|c| c.span.end.flicks <= flicks(58.0))
        );
        // Weights re-rank: with only pauses counting, the score is the pause score.
        let paused = candidates(
            &HighlightInput {
                transcript: &spoken,
                levels: None,
                scenes: None,
                range: None,
            },
            &HighlightOptions {
                weights: HighlightWeights {
                    density: 0.0,
                    pauses: 1.0,
                    loudness: 0.0,
                    scenes: 0.0,
                    self_contained: 0.0,
                },
                ..HighlightOptions::default()
            },
        );
        assert!(!paused.candidates.is_empty());
        assert!(
            paused
                .candidates
                .iter()
                .all(|c| (c.score - c.reasons.scores.pauses).abs() < 1e-9)
        );
    }

    #[test]
    fn short_or_unpunctuated_speech() {
        let short = transcript(&[(0.0, 5, 0.4)], 1.0);
        let found = candidates(
            &HighlightInput {
                transcript: &short,
                levels: None,
                scenes: None,
                range: None,
            },
            &HighlightOptions::default(),
        );
        assert_eq!(found.considered, 0);
        assert!(found.candidates.is_empty());
        // Without punctuation, segments split the sentences.
        let mut plain = transcript(&[(0.0, 10, 0.5); 4], 0.5);
        for word in &mut plain.words {
            word.text = word.text.trim_end_matches('.').to_string();
        }
        assert_eq!(sentences(&plain.words, &plain).len(), 4);
        assert!(ends_sentence("end.\u{201d}") && ends_sentence("why?)") && !ends_sentence("so,"));
        let wild = HighlightOptions {
            min_seconds: 0.0,
            max_seconds: 1e9,
            count: 0,
            max_overlap: 3.0,
            weights: HighlightWeights {
                density: -1.0,
                ..HighlightWeights::default()
            },
        }
        .clamped();
        assert_eq!(
            (
                wild.min_seconds,
                wild.max_seconds,
                wild.count,
                wild.max_overlap
            ),
            (1.0, 600.0, 1, 1.0)
        );
        assert_eq!(wild.weights.density, 0.0);
    }
}
