//! Speech with word timings (ADR-0020): whisper.cpp recognizes 16 kHz mono audio, words are
//! assembled from its tokens with times from DTW alignment, their edges are trimmed to where
//! the audio is loud, and filler candidates and pauses are derived from the words.
//! Recognition needs the `speech` feature; the types and the word logic do not.

use fosscut_time::{FLICKS_PER_SECOND, Flicks};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Moment, Span};

/// The sample rate recognition reads.
pub const RATE: u32 = 16_000;

/// Options of the `transcript` analyzer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TranscriptOptions {
    /// Model name, for example `whisper-base.en`; the most capable installed speech model
    /// for the language when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Spoken language as a two-letter code such as `en`; detected when omitted, and always
    /// English with English-only (`.en`) models.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Gaps between words reported as pauses, in milliseconds (default 300).
    #[serde(default = "default_min_pause")]
    pub min_pause_ms: u32,
    /// Prompt the model to write fillers such as "um" instead of tidying them away
    /// (default true).
    #[serde(default = "default_true")]
    pub keep_fillers: bool,
}

fn default_min_pause() -> u32 {
    300
}

fn default_true() -> bool {
    true
}

impl Default for TranscriptOptions {
    fn default() -> TranscriptOptions {
        TranscriptOptions {
            model: None,
            language: None,
            min_pause_ms: default_min_pause(),
            keep_fillers: true,
        }
    }
}

/// A recognised word.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Word {
    /// The word as written, with attached punctuation.
    pub text: String,
    /// When it starts.
    pub start: Moment,
    /// When it ends.
    pub end: Moment,
    /// The model's confidence, from 0 to 1.
    pub probability: f64,
}

/// A stretch of speech the model recognised as a unit, usually a sentence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Segment {
    /// Its text.
    pub text: String,
    /// When it starts.
    pub start: Moment,
    /// When it ends.
    pub end: Moment,
    /// The model's confidence that this is not speech, from 0 to 1.
    pub no_speech_probability: f64,
}

/// A word that looks like a filler ("um", "uh").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Filler {
    /// Its index in `words`.
    pub word: usize,
    /// The word.
    pub text: String,
    /// When it starts.
    pub start: Moment,
    /// When it ends.
    pub end: Moment,
}

/// A transcript with word timings, filler candidates and pauses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Transcript {
    /// The model that recognised it.
    pub model: String,
    /// The spoken language, as a two-letter code.
    pub language: String,
    /// Sentences or phrases, in order.
    pub segments: Vec<Segment>,
    /// Every word, in order.
    pub words: Vec<Word>,
    /// Words that look like fillers; reported, never removed.
    pub fillers: Vec<Filler>,
    /// Gaps between words of at least `min_pause_ms`.
    pub pauses: Vec<Span>,
    /// The pause threshold used, in milliseconds.
    pub min_pause_ms: u32,
}

/// A token as the recognizer reports it: text and times in hundredths of a second.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    /// The token's text; a leading space starts a new word.
    pub text: String,
    /// When it starts, in hundredths of a second.
    pub start: i64,
    /// When it ends, in hundredths of a second.
    pub end: i64,
    /// The model's confidence.
    pub probability: f32,
}

/// A token as whisper.cpp aligns it, times in hundredths of a second.
#[derive(Clone, Debug, PartialEq)]
pub struct Aligned {
    /// The token's text.
    pub text: String,
    /// Start from whisper's timestamp heuristic.
    pub t0: i64,
    /// End from whisper's timestamp heuristic.
    pub t1: i64,
    /// The DTW alignment time, or -1 without DTW.
    pub dtw: i64,
    /// The model's confidence.
    pub probability: f32,
}

/// How much earlier than the alignment a token is placed, in hundredths of a second.
/// Measured on read speech and on words with known gaps, DTW times trail the audio by 60 to
/// 100 ms with every model size tried (ADR-0020, Validation).
pub const LEAD: i64 = 8;

/// Token times from whisper.cpp's DTW alignment. whisper.cpp stamps each token with the
/// moment the alignment moves on to the next token, so a token ends at its own stamp and
/// starts at the stamp of the token before it (the first at 0). Both are moved [`LEAD`]
/// earlier. Tokens without a DTW stamp keep whisper's heuristic `t0` and `t1`.
pub fn timed_tokens(aligned: &[Aligned]) -> Vec<Token> {
    let mut previous_end = 0;
    aligned
        .iter()
        .map(|token| {
            let (start, end) = if token.dtw >= 0 {
                ((previous_end - LEAD).max(0), (token.dtw - LEAD).max(0))
            } else {
                (token.t0, token.t1)
            };
            previous_end = if token.dtw >= 0 { token.dtw } else { token.t1 };
            Token {
                text: token.text.clone(),
                start,
                end: end.max(start),
                probability: token.probability,
            }
        })
        .collect()
}

/// Levels in dBFS of consecutive 10 ms windows of 16 kHz audio.
pub fn levels(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(RATE as usize / 100)
        .map(|window| {
            let power = window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32;
            10.0 * power.max(1e-12).log10()
        })
        .collect()
}

/// Moves each word's start later and end earlier past quiet 10 ms windows, keeping at least
/// 30 ms of every word, and keeps words inside the audio. A window is quiet below both 10 dB
/// over the background (the 10th percentile level) and 30 dB under the loud parts (the 95th
/// percentile). Alignment smears words over the pauses around them; trimming makes pauses
/// between words match the silence or room tone.
pub fn trim_quiet(words: &mut [Word], levels: &[f32]) {
    let mut sorted = levels.to_vec();
    sorted.sort_by(f32::total_cmp);
    let (Some(&background), Some(&loud)) = (
        sorted.get(sorted.len() / 10),
        sorted.get(sorted.len() * 95 / 100),
    ) else {
        return;
    };
    let threshold = (background + 10.0).max(loud - 30.0);
    let window = FLICKS_PER_SECOND / 100;
    let quiet = |index: i64| {
        usize::try_from(index)
            .ok()
            .and_then(|i| levels.get(i))
            .is_none_or(|&level| level < threshold)
    };
    let duration = levels.len() as i64;
    for word in words.iter_mut() {
        let mut start = (word.start.flicks.get() / window).min(duration);
        let mut end = (word.end.flicks.get() / window).min(duration);
        if loud < -60.0 {
            // Too quiet to tell speech from silence; keep the alignment.
        } else {
            while start < end - 3 && quiet(start) {
                start += 1;
            }
            while end > start + 3 && quiet(end - 1) {
                end -= 1;
            }
        }
        word.start = Moment::new(centiseconds(start));
        word.end = Moment::new(centiseconds(end.max(start)));
    }
}

/// Hundredths of a second as flicks.
fn centiseconds(value: i64) -> Flicks {
    Flicks::new(value.max(0) * (FLICKS_PER_SECOND / 100))
}

/// Builds words from one segment's tokens. A token that begins with a space, or the first
/// token, starts a word; punctuation-only tokens attach to the word before them. A word
/// starts at its first token and ends at its last token's end, but never after the next
/// word starts (or `segment_end`), so gaps between words remain visible as pauses.
pub fn assemble_words(tokens: &[Token], segment_end: i64) -> Vec<Word> {
    struct Building {
        text: String,
        start: i64,
        end: i64,
        probabilities: Vec<f32>,
    }
    let mut words: Vec<Building> = Vec::new();
    for token in tokens {
        let starts_word = token.text.starts_with(' ') || words.is_empty();
        let punctuation = token.text.trim().chars().all(|c| c.is_ascii_punctuation());
        match words.last_mut() {
            Some(word) if !starts_word || (punctuation && !token.text.trim().is_empty()) => {
                word.text.push_str(token.text.trim_start());
                word.end = word.end.max(token.end);
                word.probabilities.push(token.probability);
            }
            _ if token.text.trim().is_empty() => {}
            _ => words.push(Building {
                text: token.text.trim().to_string(),
                start: token.start,
                end: token.end,
                probabilities: vec![token.probability],
            }),
        }
    }
    let starts: Vec<i64> = words.iter().map(|w| w.start).collect();
    words
        .into_iter()
        .enumerate()
        .map(|(i, word)| {
            let limit = starts.get(i + 1).copied().unwrap_or(segment_end);
            let end = if word.end > word.start {
                word.end.min(limit)
            } else {
                limit
            }
            .max(word.start);
            let probability = word
                .probabilities
                .iter()
                .map(|&p| f64::from(p))
                .sum::<f64>()
                / word.probabilities.len().max(1) as f64;
            Word {
                text: word.text,
                start: Moment::new(centiseconds(word.start)),
                end: Moment::new(centiseconds(end)),
                probability: (probability * 1000.0).round() / 1000.0,
            }
        })
        .collect()
}

/// The word in lowercase letters only, for matching.
pub fn bare(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphabetic())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Filler words by language (ADR-0020, item 5).
fn filler_list(language: &str) -> &'static [&'static str] {
    match language {
        "en" => &[
            "um", "uh", "erm", "er", "ah", "hmm", "mm", "uhm", "umm", "hm",
        ],
        _ => &[],
    }
}

/// The words in `words` that look like fillers in `language`.
pub fn fillers(words: &[Word], language: &str) -> Vec<Filler> {
    let list = filler_list(language);
    words
        .iter()
        .enumerate()
        .filter(|(_, w)| list.contains(&bare(&w.text).as_str()))
        .map(|(index, w)| Filler {
            word: index,
            text: w.text.clone(),
            start: w.start.clone(),
            end: w.end.clone(),
        })
        .collect()
}

/// Gaps of at least `min_pause_ms` between the end of one word and the start of the next.
pub fn pauses(words: &[Word], min_pause_ms: u32) -> Vec<Span> {
    let min = Flicks::new(i64::from(min_pause_ms) * (FLICKS_PER_SECOND / 1000));
    words
        .windows(2)
        .filter(|pair| pair[1].start.flicks - pair[0].end.flicks >= min)
        .map(|pair| Span::new(pair[0].end.flicks, pair[1].start.flicks))
        .collect()
}

/// A prompt written with disfluencies, so the model keeps writing them (ADR-0020 item 5).
const DISFLUENT_PROMPT: &str = "Um, so, uh, I was thinking, hmm, that we, er, could start.";

#[cfg(feature = "speech")]
mod whisper {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::time::Duration;

    use whisper_rs::{
        DtwMode, DtwModelPreset, DtwParameters, FullParams, SamplingStrategy, WhisperContext,
        WhisperContextParameters,
    };

    use super::{Aligned, DISFLUENT_PROMPT, Segment, Transcript, TranscriptOptions};
    use crate::models::Manifest;
    use crate::{Moment, speech};

    /// Why recognition failed.
    #[derive(Debug, thiserror::Error)]
    pub enum SpeechError {
        /// whisper.cpp reported an error.
        #[error("speech recognition failed: {0}")]
        Whisper(String),
        /// The caller cancelled.
        #[error("cancelled")]
        Cancelled,
    }

    fn preset(model: &Manifest) -> Option<DtwModelPreset> {
        match model.name {
            "whisper-tiny.en" => Some(DtwModelPreset::TinyEn),
            "whisper-base.en" => Some(DtwModelPreset::BaseEn),
            "whisper-small" => Some(DtwModelPreset::Small),
            "whisper-large-v3-turbo" => Some(DtwModelPreset::LargeV3Turbo),
            _ => None,
        }
    }

    /// Recognises `samples` (16 kHz mono) with the model at `path`, calling `progress` with a
    /// percentage while it runs; returning `false` from it cancels.
    pub fn transcribe(
        path: &Path,
        model: &Manifest,
        samples: &[f32],
        options: &TranscriptOptions,
        progress: &mut dyn FnMut(u64) -> bool,
    ) -> Result<Transcript, SpeechError> {
        let failed = |e: whisper_rs::WhisperError| SpeechError::Whisper(e.to_string());
        // whisper.cpp and GGML log to stderr; without a logging backend the hooks drop it.
        whisper_rs::install_logging_hooks();
        let mut context_params = WhisperContextParameters::default();
        if let Some(model_preset) = preset(model) {
            context_params.dtw_parameters = DtwParameters {
                mode: DtwMode::ModelPreset { model_preset },
                ..DtwParameters::default()
            };
        }
        let context = WhisperContext::new_with_params(path, context_params).map_err(failed)?;
        let mut state = context.create_state().map_err(failed)?;

        let percent = Arc::new(AtomicI32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        // English-only models cannot detect a language; detection would only guess wrong.
        let language = match (&options.language, model.languages) {
            (Some(language), _) => language.clone(),
            (None, "en") => "en".into(),
            (None, _) => "auto".into(),
        };
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&language));
        params.set_token_timestamps(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        if options.keep_fillers {
            params.set_initial_prompt(DISFLUENT_PROMPT);
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
        params.set_n_threads(threads as i32);
        let reported = Arc::clone(&percent);
        params.set_progress_callback_safe(move |p: i32| reported.store(p, Ordering::Relaxed));
        let stop = Arc::clone(&cancel);
        // whisper-rs 0.16 reads the abort closure's user data as the closure's own type but
        // stores it boxed as `Box<dyn FnMut() -> bool>`; passing that type makes the two agree.
        let abort: Box<dyn FnMut() -> bool> = Box::new(move || stop.load(Ordering::Relaxed));
        params.set_abort_callback_safe(abort);

        // whisper.cpp's callbacks must outlive the call, so recognition runs on its own
        // thread while this one relays progress and cancellation.
        let outcome = std::thread::scope(|scope| {
            let worker = scope.spawn(|| state.full(params, samples));
            while !worker.is_finished() {
                if !progress(percent.load(Ordering::Relaxed).clamp(0, 100) as u64) {
                    cancel.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            worker.join()
        });
        if cancel.load(Ordering::Relaxed) {
            return Err(SpeechError::Cancelled);
        }
        outcome
            .map_err(|_| SpeechError::Whisper("the recognizer stopped".into()))?
            .map_err(failed)?;

        let detected = whisper_rs::get_lang_str(state.full_lang_id_from_state())
            .unwrap_or("en")
            .to_string();
        // Tokens of every segment in order, since each token's start comes from the one before.
        let eot = context.token_eot();
        let mut aligned = Vec::new();
        let mut bounds = Vec::new();
        for segment in state.as_iter() {
            let first = aligned.len();
            aligned.extend(
                (0..segment.n_tokens())
                    .filter_map(|i| segment.get_token(i))
                    .filter(|token| token.token_id() < eot)
                    .map(|token| {
                        let data = token.token_data();
                        Aligned {
                            text: token
                                .to_str_lossy()
                                .map(|t| t.into_owned())
                                .unwrap_or_default(),
                            t0: data.t0,
                            t1: data.t1,
                            dtw: data.t_dtw,
                            probability: data.p,
                        }
                    }),
            );
            bounds.push((
                segment
                    .to_str_lossy()
                    .map(|t| t.trim().to_string())
                    .unwrap_or_default(),
                (segment.start_timestamp(), segment.end_timestamp()),
                f64::from(segment.no_speech_probability()),
                first..aligned.len(),
            ));
        }
        let tokens = speech::timed_tokens(&aligned);
        let levels = speech::levels(samples);
        let duration = levels.len() as i64;
        let mut segments = Vec::new();
        let mut words = Vec::new();
        for (text, (start, end), no_speech_probability, range) in bounds {
            let mut own = speech::assemble_words(&tokens[range], end.min(duration));
            speech::trim_quiet(&mut own, &levels);
            let (start, end) = match (own.first(), own.last()) {
                (Some(first), Some(last)) => (first.start.clone(), last.end.clone()),
                _ => (
                    Moment::new(speech::centiseconds(start.min(duration))),
                    Moment::new(speech::centiseconds(end.min(duration))),
                ),
            };
            words.extend(own);
            segments.push(Segment {
                text,
                start,
                end,
                no_speech_probability,
            });
        }
        Ok(Transcript {
            model: model.name.into(),
            fillers: speech::fillers(&words, &detected),
            pauses: speech::pauses(&words, options.min_pause_ms),
            language: detected,
            segments,
            words,
            min_pause_ms: options.min_pause_ms,
        })
    }
}

#[cfg(feature = "speech")]
pub use whisper::{SpeechError, transcribe};

#[cfg(test)]
mod tests {
    use super::*;

    fn token(text: &str, start: i64, end: i64) -> Token {
        Token {
            text: text.into(),
            start,
            end,
            probability: 0.9,
        }
    }

    #[test]
    fn words_are_built_from_tokens() {
        let tokens = [
            token(" Some", 42, 70),
            token("times", 70, 94),
            token(" he", 94, 101),
            token(" would", 101, 117),
            token(",", 117, 118),
            token(" um", 140, 160),
            token(".", 160, 161),
        ];
        let words = assemble_words(&tokens, 170);
        let shown: Vec<(&str, &str, &str)> = words
            .iter()
            .map(|w| {
                (
                    w.text.as_str(),
                    w.start.seconds.as_str(),
                    w.end.seconds.as_str(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("Sometimes", "0.42", "0.94"),
                ("he", "0.94", "1.01"),
                ("would,", "1.01", "1.18"),
                ("um.", "1.4", "1.61")
            ]
        );
        assert_eq!(words[0].probability, 0.9);
        assert!(assemble_words(&[], 100).is_empty());
        assert!(assemble_words(&[token(" ", 0, 1)], 1).is_empty());
    }

    #[test]
    fn fillers_and_pauses_are_found() {
        let tokens = [
            token(" So", 0, 20),
            token(" um", 20, 40),
            token(",", 40, 41),
            token(" this", 100, 120),
            token(" Uh", 125, 140),
            token(" works", 140, 170),
        ];
        let words = assemble_words(&tokens, 180);
        let candidates = fillers(&words, "en");
        let found: Vec<(usize, &str)> = candidates
            .iter()
            .map(|f| (f.word, f.text.as_str()))
            .collect();
        assert_eq!(found, [(1, "um,"), (3, "Uh")]);
        assert!(
            fillers(&words, "de").is_empty(),
            "no list for this language"
        );
        assert_eq!(bare("Hmm..."), "hmm");
        // "um," ends at 0.41 s and "this" starts at 1 s: one pause of 0.59 s.
        let gaps = pauses(&words, 300);
        assert_eq!(gaps.len(), 1);
        assert_eq!(
            (gaps[0].start.seconds.as_str(), gaps[0].end.seconds.as_str()),
            ("0.41", "1")
        );
        assert!(pauses(&words, 700).is_empty());
        let defaults: TranscriptOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, TranscriptOptions::default());
        assert!(defaults.keep_fillers);
    }

    fn aligned(text: &str, t0: i64, t1: i64, dtw: i64) -> Aligned {
        Aligned {
            text: text.into(),
            t0,
            t1,
            dtw,
            probability: 0.8,
        }
    }

    #[test]
    fn tokens_end_at_their_alignment_and_start_at_the_one_before() {
        let tokens = timed_tokens(&[
            aligned(" Some", 10, 40, 60),
            aligned("times", 40, 65, 82),
            aligned(" he", 67, 82, 110),
            aligned(" late", 120, 140, -1),
            aligned(" again", 140, 150, 5),
        ]);
        let times: Vec<(i64, i64)> = tokens.iter().map(|t| (t.start, t.end)).collect();
        assert_eq!(
            times,
            [
                // The first starts at 0, each later one where the one before ended, all
                // moved LEAD (80 ms) earlier and never before 0.
                (0, 52),
                (52, 74),
                (74, 102),
                // Without DTW, whisper's own times.
                (120, 140),
                // A stamp before the previous end still gives an empty, ordered token.
                (132, 132),
            ]
        );
        assert_eq!(tokens[0].text, " Some");
        assert_eq!(tokens[0].probability, 0.8);
    }

    /// Levels: 2 s of background at `floor` dB with speech at `speech` dB in the windows
    /// listed, in hundredths of a second.
    fn levels_with(floor: f32, speech: f32, loud: &[(usize, usize)]) -> Vec<f32> {
        let mut levels = vec![floor; 200];
        for &(from, to) in loud {
            levels[from..to].fill(speech);
        }
        levels
    }

    fn word(start: i64, end: i64) -> Word {
        Word {
            text: "w".into(),
            start: Moment::new(centiseconds(start)),
            end: Moment::new(centiseconds(end)),
            probability: 1.0,
        }
    }

    fn spans(words: &[Word]) -> Vec<(&str, &str)> {
        words
            .iter()
            .map(|w| (w.start.seconds.as_str(), w.end.seconds.as_str()))
            .collect()
    }

    #[test]
    fn levels_are_measured_in_ten_millisecond_windows() {
        let mut samples = vec![0.5_f32; 160];
        samples.extend(vec![0.0; 160]);
        samples.extend(vec![1.0; 80]);
        let measured = levels(&samples);
        assert_eq!(measured.len(), 3);
        assert!((measured[0] + 6.02).abs() < 0.01, "{measured:?}");
        assert_eq!(measured[1], -120.0, "silence bottoms out");
        assert!(measured[2].abs() < 0.01, "a short last window counts");
    }

    #[test]
    fn quiet_edges_are_trimmed_from_words() {
        // Speech at 0.5-0.8 s and 1.2-1.5 s over room tone; the alignment smears both words
        // over the pause between them.
        let levels = levels_with(-55.0, -20.0, &[(50, 80), (120, 150)]);
        let mut words = vec![word(40, 100), word(100, 160), word(185, 230)];
        trim_quiet(&mut words, &levels);
        assert_eq!(
            spans(&words),
            [("0.5", "0.8"), ("1.2", "1.5"), ("1.97", "2")],
            "a word in silence keeps its last 30 ms, and none passes the end of the audio"
        );

        // Room tone 10 dB under the threshold rule still counts as quiet; quiet speech above
        // it does not.
        let levels = levels_with(-45.0, -30.0, &[(50, 80)]);
        let mut words = vec![word(30, 100)];
        trim_quiet(&mut words, &levels);
        assert_eq!(spans(&words), [("0.5", "0.8")]);

        // Audio too quiet to tell speech from silence keeps the alignment.
        let levels = vec![-90.0; 200];
        let mut words = vec![word(30, 100)];
        trim_quiet(&mut words, &levels);
        assert_eq!(spans(&words), [("0.3", "1")]);
        trim_quiet(&mut words, &[]);
        assert_eq!(
            spans(&words),
            [("0.3", "1")],
            "no audio, nothing to measure"
        );
    }
}
