//! Text to speech with Kokoro-82M (ADR-0033 item 5): English text in, 24 kHz mono samples
//! out, on the CPU, so the same text, voice and speed always give the same samples.
//!
//! How text becomes speech:
//!
//! 1. **Paragraphs and sentences.** Text is split into paragraphs at line breaks, each
//!    paragraph is pronounced, and its phonemes are cut into sentences after `.`, `!`, `?`
//!    and `…`. A sentence longer than the model's input ([`MAX_PHONEMES`]) is cut at
//!    clause punctuation, then between words.
//! 2. **Pronunciation.** Words are looked up in the CMU Pronouncing Dictionary, which is
//!    installed with the voice model, and its ARPAbet phonemes are written in the phoneme
//!    set Kokoro was trained on, with British rules on top for the British voices (the
//!    `pronounce` module documents the mapping). FossCut reads numbers, times, years,
//!    money and common abbreviations in words itself. A word the dictionary lacks is
//!    pronounced from an inflection or the parts of it the dictionary has (`FossCut` as
//!    `Foss` and `cut`, `moonbase` as `moon` and `base`), as letter names when it looks
//!    like an abbreviation (`QZX`, `U.S.`), and otherwise by spelling-to-sound rules for
//!    common English patterns (digraphs, vowel teams, a final silent `e`, `-tion`, `-ed`
//!    and so on). Letters with accents are read as their plain letters, and nothing makes
//!    the synthesis fail.
//! 3. **Synthesis.** Each piece runs through the model with the voice's style for its
//!    length. Silence at the edges of each piece is trimmed, and the pieces are joined
//!    with pauses: [`SENTENCE_PAUSE_MS`] after a sentence, [`CLAUSE_PAUSE_MS`] where a long
//!    sentence was cut, [`PARAGRAPH_PAUSE_MS`] between paragraphs.
//!
//! **Provenance.** The dictionary is the CMU Pronouncing Dictionary, Copyright (C)
//! 1993-2015 Carnegie Mellon University, under its BSD-2-Clause license, downloaded from a
//! pinned commit of Carnegie Mellon's repository and installed, with its license, as part
//! of the `kokoro-82m` model. The ARPAbet mapping, the British rules, number and
//! abbreviation reading and every fallback rule were written for FossCut from the phoneme
//! inventories; no other pronouncing dictionary, rule set or program output is used.
//!
//! Voices are the American (`af_`, `am_`) and British (`bf_`, `bm_`) English voices of the
//! `kokoro-82m` model; the accent picks the pronunciation. There is no voice for any other
//! language (ADR-0033: no cleanly licensed Hungarian voice exists), which callers check
//! with [`check_language`].
//!
//! The types, the errors, [`voices`] and pronunciation ([`Dictionary`], [`phrases`]) exist in
//! every build; synthesizing needs the `onnx` feature, which is on by default.

// Without the `onnx` feature only the types, errors and listings are used.
#![cfg_attr(not(feature = "onnx"), allow(dead_code))]

use std::path::Path;

use fosscut_time::{FLICKS_PER_SECOND, Flicks};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::models::{self, ModelError};
use crate::onnx::OnnxError;

/// The voice model.
pub const MODEL: &str = "kokoro-82m";
/// The sample rate of the speech.
pub const RATE: u32 = 24_000;
/// The most phonemes the model reads in one run: its context of 512 tokens holds them
/// between a start and an end token, and each voice has a style for every length up to it.
pub const MAX_PHONEMES: usize = 510;
/// The voice used when none is named.
pub const DEFAULT_VOICE: &str = "af_heart";
/// The slowest speed.
pub const MIN_SPEED: f32 = 0.5;
/// The fastest speed.
pub const MAX_SPEED: f32 = 2.0;
/// The pause after a sentence, in milliseconds.
pub const SENTENCE_PAUSE_MS: u32 = 300;
/// The pause where a sentence too long for one run was cut, in milliseconds.
pub const CLAUSE_PAUSE_MS: u32 = 120;
/// The pause between paragraphs, in milliseconds.
pub const PARAGRAPH_PAUSE_MS: u32 = 600;
/// Bumped whenever the same text, voice, speed and model can give different samples, so
/// callers that keep speech by request (the engine's `speech/` files) make it again.
/// (2: pronunciation from the CMU Pronouncing Dictionary.)
pub const VERSION: u32 = 2;

/// Floats in one style vector of a voice.
const STYLE: usize = 256;
/// Style vectors in a voice file, one per input length.
const STYLES: usize = MAX_PHONEMES;
/// Punctuation the model reads, which alone is nothing to say.
const PUNCTUATION: &str = ";:,.!?\u{2014}…\"()“” ";

/// The accent a voice speaks with, which picks its pronunciation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    /// American English (`a` voices).
    American,
    /// British English (`b` voices).
    British,
}

/// A voice's gender, as its author labelled it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Gender {
    /// `f` voices.
    Female,
    /// `m` voices.
    Male,
}

/// A voice of the voice model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceInfo {
    /// Its name, for example `af_heart`.
    pub name: String,
    /// The English it speaks.
    pub accent: Accent,
    /// Its gender.
    pub gender: Gender,
    /// Whether its file is installed in the models folder.
    pub installed: bool,
}

/// Synthesized speech.
#[derive(Clone, Debug, PartialEq)]
pub struct Speech {
    /// Mono samples from -1 to 1.
    pub samples: Vec<f32>,
    /// Samples per second: [`RATE`].
    pub rate: u32,
}

impl Speech {
    /// How long it lasts.
    pub fn duration(&self) -> Flicks {
        Flicks::new(
            (self.samples.len() as i128 * i128::from(FLICKS_PER_SECOND)
                / i128::from(self.rate.max(1))) as i64,
        )
    }

    /// How long it lasts, in seconds.
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / f64::from(self.rate.max(1))
    }
}

/// Why speech could not be made.
#[derive(Debug, Error, PartialEq)]
pub enum VoiceError {
    /// No voice has this name.
    #[error("there is no voice {name}; the voices are {}", voices.join(", "))]
    UnknownVoice {
        /// The requested name.
        name: String,
        /// The voices there are.
        voices: Vec<String>,
    },
    /// The text is not in English, the only language the voices speak.
    #[error(
        "the voices speak English only, not {language}; translate the text to English first (for example captions translate with hu>en)"
    )]
    LanguageUnavailable {
        /// The requested language, or a description of the text's script.
        language: String,
    },
    /// The text has nothing to say.
    #[error("the text has no words to speak")]
    EmptyText,
    /// The speed is outside [`MIN_SPEED`] to [`MAX_SPEED`].
    #[error("speed {speed} is outside {MIN_SPEED} to {MAX_SPEED}")]
    InvalidSpeed {
        /// The requested speed.
        speed: f32,
    },
    /// The voice model is unknown or not installed.
    #[error(transparent)]
    Model(#[from] ModelError),
    /// ONNX Runtime is missing or could not load the model.
    #[error(transparent)]
    Onnx(#[from] OnnxError),
    /// The model failed while speaking, or its files are damaged.
    #[error("speech synthesis failed: {0}")]
    Failed(String),
    /// This build has no voices (the `onnx` feature is off).
    #[error("this build of FossCut has no voices")]
    Unavailable,
}

impl VoiceError {
    /// The stable error code front ends report (ADR-0006).
    pub fn code(&self) -> &'static str {
        match self {
            VoiceError::UnknownVoice { .. } => "fosscut.voice.unknown_voice",
            VoiceError::LanguageUnavailable { .. } => "fosscut.voice.language_unavailable",
            VoiceError::EmptyText | VoiceError::InvalidSpeed { .. } => {
                "fosscut.request.invalid_arguments"
            }
            VoiceError::Model(error) => model_code(error),
            VoiceError::Onnx(error) => error.code(),
            VoiceError::Failed(_) => "fosscut.voice.synthesis_failed",
            VoiceError::Unavailable => "fosscut.voice.unavailable",
        }
    }
}

/// The error code of a model error, as the engine reports it.
pub(crate) fn model_code(error: &ModelError) -> &'static str {
    match error {
        ModelError::Unknown { .. } => "fosscut.model.unknown",
        ModelError::NotInstalled { .. } => "fosscut.model.not_installed",
        ModelError::ChecksumMismatch { .. } => "fosscut.model.checksum_mismatch",
        ModelError::DownloadFailed { .. } => "fosscut.model.download_failed",
        ModelError::Unsupported { .. } => "fosscut.model.unsupported_platform",
    }
}

/// Every voice of the voice model, American first, with whether it is installed in
/// `models`.
pub fn voices(models: &Path) -> Vec<VoiceInfo> {
    let Ok(model) = models::manifest(MODEL) else {
        return Vec::new();
    };
    model
        .files
        .iter()
        .filter_map(|file| {
            let name = file.name.strip_prefix("voices/")?.strip_suffix(".bin")?;
            let (accent, gender) = kind(name)?;
            let installed = std::fs::metadata(models::file_path(models, model, file.name))
                .is_ok_and(|m| m.len() == file.size);
            Some(VoiceInfo {
                name: name.to_string(),
                accent,
                gender,
                installed,
            })
        })
        .collect()
}

/// The accent and gender in a voice's name: `af_heart` is an American female voice.
fn kind(name: &str) -> Option<(Accent, Gender)> {
    let mut letters = name.chars();
    let accent = match letters.next()? {
        'a' => Accent::American,
        'b' => Accent::British,
        _ => return None,
    };
    let gender = match letters.next()? {
        'f' => Gender::Female,
        'm' => Gender::Male,
        _ => return None,
    };
    (letters.next() == Some('_')).then_some((accent, gender))
}

/// The voice called `name`, or [`VoiceError::UnknownVoice`] with the voices there are.
pub fn voice(name: &str) -> Result<VoiceInfo, VoiceError> {
    let all = voices(Path::new(""));
    all.iter()
        .find(|v| v.name == name)
        .cloned()
        .ok_or_else(|| VoiceError::UnknownVoice {
            name: name.to_string(),
            voices: all.iter().map(|v| v.name.clone()).collect(),
        })
}

/// Checks that `language` (`en`, `en-US`, `en_GB`, `english` and so on) is one the voices
/// speak, which is English only.
pub fn check_language(language: &str) -> Result<(), VoiceError> {
    let lower = language.trim().to_lowercase();
    let primary = lower.split(['-', '_']).next().unwrap_or_default();
    if matches!(primary, "en" | "eng" | "english") {
        Ok(())
    } else {
        Err(VoiceError::LanguageUnavailable {
            language: language.trim().to_string(),
        })
    }
}

/// Text that cannot be English: letters, none of them Latin, such as Cyrillic or Chinese.
/// Latin letters with accents pass; they are read as their plain letters.
fn check_script(text: &str) -> Result<(), VoiceError> {
    let letters = text.chars().filter(|c| c.is_alphabetic());
    let mut any = false;
    for letter in letters {
        if latin(letter) {
            return Ok(());
        }
        any = true;
    }
    if any {
        Err(VoiceError::LanguageUnavailable {
            language: "text in a script other than Latin".into(),
        })
    } else {
        Ok(())
    }
}

/// Whether `c` is a Latin letter: ASCII, Latin-1, or the Latin Extended blocks.
fn latin(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c as u32, 0xC0..=0x24F | 0x1E00..=0x1EFF)
}

/// A piece of text the model speaks in one run, and the pause after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phrase {
    /// Its phonemes, with spaces and punctuation, at most [`MAX_PHONEMES`] characters.
    pub phonemes: String,
    /// Silence after it, in milliseconds; 0 after the last.
    pub pause_ms: u32,
}

pub use pronounce::Dictionary;

/// The phrases `text` is spoken as with `accent`'s pronunciation from `dictionary`:
/// paragraphs at line breaks, sentences, and pieces of long sentences, each within the
/// model's input.
pub fn phrases(
    dictionary: &Dictionary,
    text: &str,
    accent: Accent,
) -> Result<Vec<Phrase>, VoiceError> {
    check_script(text)?;
    let mut phrases: Vec<Phrase> = Vec::new();
    for paragraph in text.lines().map(str::trim).filter(|p| !p.is_empty()) {
        let phonemes = pronounce::paragraph(dictionary, paragraph, accent);
        for sentence in sentences(&phonemes) {
            let pieces = pieces(&sentence, MAX_PHONEMES);
            let last = pieces.len().saturating_sub(1);
            for (index, piece) in pieces.into_iter().enumerate() {
                if !speakable(&piece) {
                    continue;
                }
                phrases.push(Phrase {
                    phonemes: piece,
                    pause_ms: if index == last {
                        SENTENCE_PAUSE_MS
                    } else {
                        CLAUSE_PAUSE_MS
                    },
                });
            }
        }
        if let Some(last) = phrases.last_mut() {
            last.pause_ms = PARAGRAPH_PAUSE_MS;
        }
    }
    if let Some(last) = phrases.last_mut() {
        last.pause_ms = 0;
    }
    Ok(phrases)
}

/// Whether `phonemes` hold anything but punctuation and spaces.
fn speakable(phonemes: &str) -> bool {
    phonemes.chars().any(|c| !PUNCTUATION.contains(c))
}

/// A paragraph's phonemes cut into sentences after `.`, `!`, `?` or `…` (with any closing
/// quotes or brackets) followed by a space.
fn sentences(phonemes: &str) -> Vec<String> {
    let chars: Vec<char> = phonemes.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        current.push(chars[i]);
        let ends = matches!(chars[i], '.' | '!' | '?' | '…');
        if ends {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], '.' | '!' | '?' | '…' | '"' | '”' | ')')
            {
                current.push(chars[j]);
                j += 1;
            }
            i = j;
            if i < chars.len() && chars[i] == ' ' {
                out.push(std::mem::take(&mut current).trim().to_string());
            }
            continue;
        }
        i += 1;
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out.retain(|s| !s.is_empty());
    out
}

/// `sentence` in pieces of at most `limit` characters: whole if it fits, otherwise cut
/// after clause punctuation (a comma, semicolon, colon or dash) and packed, and a clause that is still too
/// long cut between words, or within a word as a last resort.
fn pieces(sentence: &str, limit: usize) -> Vec<String> {
    let length = |s: &str| s.chars().count();
    if length(sentence) <= limit {
        return vec![sentence.to_string()];
    }
    let mut clauses = Vec::new();
    let mut current = String::new();
    for word in sentence.split(' ').filter(|w| !w.is_empty()) {
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
        if word.ends_with([',', ';', ':', '\u{2014}']) {
            clauses.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        clauses.push(current);
    }
    let mut units = Vec::new();
    for clause in clauses {
        if length(&clause) <= limit {
            units.push(clause);
            continue;
        }
        for word in clause.split(' ') {
            if length(word) <= limit {
                units.push(word.to_string());
            } else {
                let chars: Vec<char> = word.chars().collect();
                units.extend(chars.chunks(limit).map(|c| c.iter().collect::<String>()));
            }
        }
    }
    // Pack units into pieces, a space between them.
    let mut out: Vec<String> = Vec::new();
    for unit in units {
        match out.last_mut() {
            Some(last) if length(last) + 1 + length(&unit) <= limit => {
                last.push(' ');
                last.push_str(&unit);
            }
            _ => out.push(unit),
        }
    }
    out
}

/// Trims silence (below about -54 dBFS) from both ends of `samples`, keeping 20 ms before
/// the first and after the last sound so soft onsets and endings stay whole.
fn trim(samples: &[f32]) -> &[f32] {
    const THRESHOLD: f32 = 0.002;
    let margin = (RATE / 50) as usize;
    let first = samples.iter().position(|s| s.abs() > THRESHOLD);
    let last = samples.iter().rposition(|s| s.abs() > THRESHOLD);
    match (first, last) {
        (Some(first), Some(last)) => {
            &samples[first.saturating_sub(margin)..(last + 1 + margin).min(samples.len())]
        }
        _ => &[],
    }
}

/// Checks a speed and keeps it as given.
fn checked_speed(speed: f32) -> Result<f32, VoiceError> {
    if speed.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(&speed) {
        Ok(speed)
    } else {
        Err(VoiceError::InvalidSpeed { speed })
    }
}

#[cfg(feature = "onnx")]
pub use synthesis::Synthesizer;

/// Synthesizes `text` with `voice` at `speed` (1 is the voice's own pace), with the voice
/// model in `models`. For many requests, open one [`Synthesizer`] and reuse it.
pub fn synthesize(
    models: &Path,
    text: &str,
    voice: &str,
    speed: f32,
) -> Result<Speech, VoiceError> {
    #[cfg(feature = "onnx")]
    {
        Synthesizer::open(models)?.speak(text, voice, speed)
    }
    #[cfg(not(feature = "onnx"))]
    {
        let _ = (models, text, voice, checked_speed(speed)?);
        Err(VoiceError::Unavailable)
    }
}

/// Kokoro on ONNX Runtime.
#[cfg(feature = "onnx")]
mod synthesis {
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;

    use ort::session::SessionInputValue;
    use ort::value::Tensor;

    use super::{
        Dictionary, MAX_PHONEMES, MODEL, Phrase, RATE, STYLE, STYLES, Speech, VoiceError,
        checked_speed, phrases, trim,
    };
    use crate::models::{self, ModelError};
    use crate::onnx::{self, Device, Session, SessionOptions};

    /// The voice model loaded once, for many requests.
    pub struct Synthesizer {
        session: Session,
        dictionary: Arc<Dictionary>,
        vocabulary: HashMap<char, i64>,
        models: std::path::PathBuf,
        styles: HashMap<String, Vec<f32>>,
    }

    impl std::fmt::Debug for Synthesizer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Synthesizer")
                .field("models", &self.models)
                .field("voices_loaded", &self.styles.len())
                .finish_non_exhaustive()
        }
    }

    fn failed(message: impl std::fmt::Display) -> VoiceError {
        VoiceError::Failed(message.to_string())
    }

    impl Synthesizer {
        /// Loads the voice model from `models` into a CPU session, which gives the same
        /// samples on every run, with its pronouncing dictionary.
        pub fn open(models: &Path) -> Result<Synthesizer, VoiceError> {
            let model = models::manifest(MODEL)?;
            if !models::is_installed(models, model) {
                return Err(ModelError::NotInstalled { what: MODEL.into() }.into());
            }
            let dictionary = Dictionary::open(models)?;
            let tokenizer = std::fs::read(models::file_path(models, model, "tokenizer.json"))
                .map_err(|e| failed(format!("cannot read the tokenizer: {e}")))?;
            let tokenizer: serde_json::Value = serde_json::from_slice(&tokenizer)
                .map_err(|e| failed(format!("the tokenizer is damaged: {e}")))?;
            let vocabulary: HashMap<char, i64> = tokenizer["model"]["vocab"]
                .as_object()
                .map(|vocab| {
                    vocab
                        .iter()
                        .filter_map(|(symbol, id)| {
                            let mut chars = symbol.chars();
                            match (chars.next(), chars.next(), id.as_i64()) {
                                (Some(c), None, Some(id)) => Some((c, id)),
                                _ => None,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            if vocabulary.is_empty() {
                return Err(failed("the tokenizer has no vocabulary"));
            }
            let session = onnx::session(
                &models::file_path(models, model, "model.onnx"),
                &SessionOptions::new(models).on(Device::Cpu),
            )?;
            Ok(Synthesizer {
                session,
                dictionary,
                vocabulary,
                models: models.to_path_buf(),
                styles: HashMap::new(),
            })
        }

        /// Speaks `text` with `voice` at `speed`.
        pub fn speak(&mut self, text: &str, voice: &str, speed: f32) -> Result<Speech, VoiceError> {
            let speed = checked_speed(speed)?;
            let info = super::voice(voice)?;
            let phrases = phrases(&self.dictionary, text, info.accent)?;
            if phrases.is_empty() {
                return Err(VoiceError::EmptyText);
            }
            self.load(voice)?;
            let mut samples: Vec<f32> = Vec::new();
            for phrase in &phrases {
                let spoken = self.run(phrase, voice, speed)?;
                samples.extend_from_slice(trim(&spoken));
                samples.extend(std::iter::repeat_n(
                    0.0,
                    (u64::from(phrase.pause_ms) * u64::from(RATE) / 1000) as usize,
                ));
            }
            if samples.iter().all(|s| *s == 0.0) {
                return Err(failed("the model returned silence"));
            }
            // Keep peaks inside full scale; the same samples always scale the same way.
            let peak = samples.iter().fold(0.0f32, |a, s| a.max(s.abs()));
            if peak > 0.99 {
                let gain = 0.99 / peak;
                samples.iter_mut().for_each(|s| *s *= gain);
            }
            Ok(Speech {
                samples,
                rate: RATE,
            })
        }

        /// Reads `voice`'s styles once: 510 vectors of 256 floats, one per input length.
        fn load(&mut self, voice: &str) -> Result<(), VoiceError> {
            if self.styles.contains_key(voice) {
                return Ok(());
            }
            let model = models::manifest(MODEL)?;
            let name = format!("voices/{voice}.bin");
            let bytes =
                std::fs::read(models::file_path(&self.models, model, &name)).map_err(|_| {
                    ModelError::NotInstalled {
                        what: format!("{MODEL}/{name}"),
                    }
                })?;
            if bytes.len() != STYLES * STYLE * 4 {
                return Err(failed(format!(
                    "{name} has {} bytes, not {}",
                    bytes.len(),
                    STYLES * STYLE * 4
                )));
            }
            let styles = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            self.styles.insert(voice.to_string(), styles);
            Ok(())
        }

        /// One run of the model: the phrase's phonemes as token ids between start and end
        /// tokens, the voice's style for that many phonemes, and the speed.
        fn run(
            &mut self,
            phrase: &Phrase,
            voice: &str,
            speed: f32,
        ) -> Result<Vec<f32>, VoiceError> {
            let mut ids = vec![0i64];
            ids.extend(
                phrase
                    .phonemes
                    .chars()
                    .filter_map(|c| self.vocabulary.get(&c).copied())
                    .take(MAX_PHONEMES),
            );
            ids.push(0);
            let row = (ids.len() - 2).min(STYLES - 1);
            let styles = self
                .styles
                .get(voice)
                .ok_or_else(|| failed(format!("{voice} is not loaded")))?;
            let style = styles[row * STYLE..(row + 1) * STYLE].to_vec();
            let count = ids.len();
            let inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> = vec![
                (
                    "input_ids".into(),
                    Tensor::from_array((vec![1usize, count], ids))
                        .map_err(failed)?
                        .into(),
                ),
                (
                    "style".into(),
                    Tensor::from_array((vec![1usize, STYLE], style))
                        .map_err(failed)?
                        .into(),
                ),
                (
                    "speed".into(),
                    Tensor::from_array((vec![1usize], vec![speed]))
                        .map_err(failed)?
                        .into(),
                ),
            ];
            let outputs = self.session.run(inputs).map_err(failed)?;
            let (_, waveform) = outputs["waveform"]
                .try_extract_tensor::<f32>()
                .map_err(failed)?;
            if waveform.iter().any(|s| !s.is_finite()) {
                return Err(failed("the model returned samples that are not numbers"));
            }
            Ok(waveform.to_vec())
        }
    }
}

/// Pronunciation: the CMU Pronouncing Dictionary read into the voice model's phonemes, and
/// FossCut's own rules for numbers, abbreviations and the words the dictionary lacks.
///
/// **From ARPAbet to the model's phonemes.** The dictionary writes 39 ARPAbet phonemes,
/// vowels with stress 0 (none), 1 (primary) or 2 (secondary). Kokoro reads IPA-style
/// symbols with single letters for diphthongs and the stress mark before the stressed
/// vowel, so `HH AH0 L OW1` is `həlˈO`. American English:
///
/// | ARPAbet | US | | ARPAbet | US | | ARPAbet | US |
/// |---|---|---|---|---|---|---|---|---|
/// | AA | ɑ | | EY | A | | OY | Y |
/// | AE | æ | | IH | ɪ (ᵻ in a final -ed, -es) | | UH | ʊ |
/// | AH0 | ə (ᵊl before a final l) | | IY | i | | UW | u |
/// | AH1, AH2 | ʌ | | OW | O | | ER0 | əɹ |
/// | AO | ɔ | | AW | W | | ER1, ER2 | ɜɹ |
/// | AY | I | | EH | ɛ | | | |
///
/// Consonants: B b, CH ʧ, D d, DH ð, F f, G ɡ, HH h, JH ʤ, K k, L l, M m, N n, NG ŋ, P p,
/// R ɹ, S s, SH ʃ, T t (T, the flap, between a vowel or r and an unstressed vowel), TH θ,
/// V v, W w, Y j, Z z, ZH ʒ.
///
/// **British English** applies simple rules on top of the American pronunciation:
/// - **Non-rhotic r:** an r with no vowel after it is dropped and colours the vowel before
///   it: AA R ɑː, AO R and OW R ɔː, EH R and EY R ɛː, IH R and IY R ɪə, UH R and UW R ʊə,
///   AY R Iə, AW R Wə, ER1 ɜː and ER0 ə.
/// - **Long vowels:** stressed IY iː and UW uː, AO ɔː, and OW Q.
/// - **Lot:** AA is ɒ in words spelled with o (or wa, qua), ɑː otherwise (father); AO is ɒ
///   in words spelled with o that have no ou, oa, au or aw (dog, long), ɔː otherwise.
/// - **Trap and bath:** AE is a, and ɑː before f, s or th with no vowel after them
///   (after, fast, path).
/// - **No flap:** t stays t.
///
/// **Stress.** Where a word has several primary stresses (spelled abbreviations such as
/// `fbi`), all but the last become secondary; a final IY with secondary stress after a
/// primary is unstressed (`thirty`). Function words (articles, prepositions,
/// conjunctions, forms of be and have, possessives) lose their stress, and personal
/// pronouns keep only secondary stress. `the` is ðə, or ði before a vowel.
///
/// **Choosing a pronunciation.** The dictionary's first is used: no part-of-speech tagger
/// chooses between heteronyms such as `read` or `live`.
///
/// **Words the dictionary lacks:** an inflection of a word it has (-s, -es, -'s, -ed,
/// -ing), the parts of hyphenated words and of words that change case (`FossCut` as
/// `Foss` and `Cut`), two dictionary words joined (`moonbase`), letter names for
/// abbreviations (capitals of two or three letters in mixed-case text, dotted ones such as
/// `U.S.`, and words without vowels), and otherwise [`letters_to_sounds`]. Letters with
/// accents are read as their plain letters.
///
/// **Numbers** are read in words: cardinals, ordinals (31st), decimals, times (3:45),
/// years (1999, 2026), decades (1990s), money ($12.50, £3, €5), percentages and signs.
/// **Abbreviations** such as Dr., Mr., St., etc., e.g. and the months are written out, and
/// `&`, `+`, `=`, `@` and `%` are said.
mod pronounce {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, PoisonError};

    use super::{Accent, MODEL, VoiceError};
    use crate::models::{self, ModelError};

    /// The dictionary in the voice model's folder.
    pub(super) const FILE: &str = "cmudict/cmudict.dict";

    /// An ARPAbet phoneme.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Arpa {
        Aa,
        Ae,
        Ah,
        Ao,
        Aw,
        Ay,
        B,
        Ch,
        D,
        Dh,
        Eh,
        Er,
        Ey,
        F,
        G,
        Hh,
        Ih,
        Iy,
        Jh,
        K,
        L,
        M,
        N,
        Ng,
        Ow,
        Oy,
        P,
        R,
        S,
        Sh,
        T,
        Th,
        Uh,
        Uw,
        V,
        W,
        Y,
        Z,
        Zh,
    }

    impl Arpa {
        fn parse(symbol: &str) -> Option<Arpa> {
            use Arpa::*;
            Some(match symbol {
                "AA" => Aa,
                "AE" => Ae,
                "AH" => Ah,
                "AO" => Ao,
                "AW" => Aw,
                "AY" => Ay,
                "B" => B,
                "CH" => Ch,
                "D" => D,
                "DH" => Dh,
                "EH" => Eh,
                "ER" => Er,
                "EY" => Ey,
                "F" => F,
                "G" => G,
                "HH" => Hh,
                "IH" => Ih,
                "IY" => Iy,
                "JH" => Jh,
                "K" => K,
                "L" => L,
                "M" => M,
                "N" => N,
                "NG" => Ng,
                "OW" => Ow,
                "OY" => Oy,
                "P" => P,
                "R" => R,
                "S" => S,
                "SH" => Sh,
                "T" => T,
                "TH" => Th,
                "UH" => Uh,
                "UW" => Uw,
                "V" => V,
                "W" => W,
                "Y" => Y,
                "Z" => Z,
                "ZH" => Zh,
                _ => return None,
            })
        }

        fn vowel(self) -> bool {
            use Arpa::*;
            matches!(
                self,
                Aa | Ae | Ah | Ao | Aw | Ay | Eh | Er | Ey | Ih | Iy | Ow | Oy | Uh | Uw
            )
        }

        /// A consonant's symbol (the same in both accents).
        fn consonant(self) -> &'static str {
            use Arpa::*;
            match self {
                B => "b",
                Ch => "ʧ",
                D => "d",
                Dh => "ð",
                F => "f",
                G => "ɡ",
                Hh => "h",
                Jh => "ʤ",
                K => "k",
                L => "l",
                M => "m",
                N => "n",
                Ng => "ŋ",
                P => "p",
                R => "ɹ",
                S => "s",
                Sh => "ʃ",
                T => "t",
                Th => "θ",
                V => "v",
                W => "w",
                Y => "j",
                Z => "z",
                Zh => "ʒ",
                _ => "",
            }
        }
    }

    /// A phoneme with its stress, which only vowels have.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Phone {
        arpa: Arpa,
        stress: Option<u8>,
    }

    impl Phone {
        fn parse(symbol: &str) -> Option<Phone> {
            let (name, stress) = match symbol.as_bytes().last()? {
                digit @ b'0'..=b'2' => (&symbol[..symbol.len() - 1], Some(digit - b'0')),
                _ => (symbol, None),
            };
            let arpa = Arpa::parse(name)?;
            (arpa.vowel() == stress.is_some()).then_some(Phone { arpa, stress })
        }

        fn consonant(arpa: Arpa) -> Phone {
            Phone { arpa, stress: None }
        }

        fn unstressed(arpa: Arpa) -> Phone {
            Phone {
                arpa,
                stress: Some(0),
            }
        }
    }

    /// An English pronouncing dictionary in the CMU Pronouncing Dictionary's format.
    pub struct Dictionary {
        words: HashMap<String, Vec<Box<[Phone]>>>,
    }

    impl std::fmt::Debug for Dictionary {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Dictionary")
                .field("words", &self.words.len())
                .finish()
        }
    }

    impl Dictionary {
        /// Reads a dictionary: on each line a word (alternates numbered `word(2)`) and its
        /// ARPAbet phonemes with stress digits; `#` starts a comment. Lines that do not
        /// parse are skipped.
        pub fn parse(text: &str) -> Dictionary {
            let mut words: HashMap<String, Vec<Box<[Phone]>>> = HashMap::new();
            for line in text.lines() {
                let line = line.split('#').next().unwrap_or_default();
                let mut fields = line.split_whitespace();
                let Some(word) = fields.next() else {
                    continue;
                };
                let word = word.split_once('(').map_or(word, |(w, _)| w).to_lowercase();
                let phones: Option<Vec<Phone>> = fields.map(Phone::parse).collect();
                if let Some(phones) = phones.filter(|p| !p.is_empty()) {
                    words
                        .entry(word)
                        .or_default()
                        .push(phones.into_boxed_slice());
                }
            }
            Dictionary { words }
        }

        /// The voice model's dictionary in `models`, read once per process.
        pub fn open(models: &Path) -> Result<Arc<Dictionary>, VoiceError> {
            static LOADED: Mutex<Vec<(PathBuf, Arc<Dictionary>)>> = Mutex::new(Vec::new());
            let model = models::manifest(MODEL)?;
            let path = models::file_path(models, model, FILE);
            let mut loaded = LOADED.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((_, dictionary)) = loaded.iter().find(|(p, _)| *p == path) {
                return Ok(Arc::clone(dictionary));
            }
            let text = std::fs::read_to_string(&path).map_err(|_| ModelError::NotInstalled {
                what: format!("{MODEL}/{FILE}"),
            })?;
            let dictionary = Arc::new(Dictionary::parse(&text));
            if dictionary.is_empty() {
                return Err(VoiceError::Failed(format!(
                    "{} has no pronunciations",
                    path.display()
                )));
            }
            loaded.push((path, Arc::clone(&dictionary)));
            Ok(dictionary)
        }

        /// Words with at least one pronunciation.
        pub fn len(&self) -> usize {
            self.words.len()
        }

        /// Whether it has no words.
        pub fn is_empty(&self) -> bool {
            self.words.is_empty()
        }

        /// The first pronunciation of `word` (lowercase).
        fn first(&self, word: &str) -> Option<&[Phone]> {
            self.words.get(word).and_then(|p| p.first()).map(|p| &p[..])
        }

        /// The phonemes of `word` (lowercase) in `accent`, if the dictionary has it.
        pub fn phonemes(&self, word: &str, accent: Accent) -> Option<String> {
            self.first(word)
                .map(|phones| render(phones, accent, word, stress_of(word)))
        }
    }

    /// How a word is stressed.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Stress {
        /// As the dictionary says.
        Full,
        /// Not at all: function words.
        None,
        /// Secondary at most: personal pronouns.
        Light,
    }

    /// Words said without stress in running speech.
    const FUNCTION_WORDS: &[&str] = &[
        "a", "an", "and", "am", "are", "as", "at", "be", "been", "but", "by", "do", "does", "for",
        "from", "had", "has", "have", "her", "his", "in", "into", "is", "its", "my", "nor", "of",
        "on", "onto", "or", "our", "than", "their", "to", "was", "were", "with", "your",
    ];

    /// Pronouns said with secondary stress at most.
    const PRONOUNS: &[&str] = &[
        "i", "me", "you", "he", "him", "she", "it", "we", "us", "they", "them", "it's", "i'm",
        "you're", "we're", "they're",
    ];

    fn stress_of(word: &str) -> Stress {
        if FUNCTION_WORDS.contains(&word) {
            Stress::None
        } else if PRONOUNS.contains(&word) {
            Stress::Light
        } else {
            Stress::Full
        }
    }

    /// `phones` in the voice model's phonemes for `accent`; `spelling` (lowercase) guides
    /// the British vowels.
    fn render(phones: &[Phone], accent: Accent, spelling: &str, stress: Stress) -> String {
        use Arpa::*;
        let british = accent == Accent::British;
        let n = phones.len();
        let mut stresses: Vec<Option<u8>> = phones.iter().map(|p| p.stress).collect();
        let primaries: Vec<usize> = (0..n).filter(|&i| stresses[i] == Some(1)).collect();
        if let Some((_, rest)) = primaries.split_last() {
            for &i in rest {
                stresses[i] = Some(2);
            }
        }
        if let Some(last) = (0..n).rev().find(|&i| phones[i].arpa.vowel())
            && phones[last].arpa == Iy
            && stresses[last] == Some(2)
            && stresses[..last].contains(&Some(1))
        {
            stresses[last] = Some(0);
        }
        for s in stresses.iter_mut().filter(|s| s.is_some()) {
            *s = match (stress, *s) {
                (Stress::None, _) => Some(0),
                (Stress::Light, Some(1)) => Some(2),
                (_, s) => s,
            };
        }
        let vowel_at = |i: usize| i < n && phones[i].arpa.vowel();
        let mut out = String::new();
        let mut i = 0;
        while i < n {
            let phone = phones[i];
            let s = stresses[i];
            if !phone.arpa.vowel() {
                // The American flap: t between a vowel (or a vowel and r) and an unstressed
                // vowel.
                let after_vowel = i > 0
                    && (vowel_at(i - 1) || (phones[i - 1].arpa == R && i > 1 && vowel_at(i - 2)));
                let flap = !british
                    && phone.arpa == T
                    && after_vowel
                    && vowel_at(i + 1)
                    && stresses[i + 1] == Some(0);
                out.push_str(if flap { "T" } else { phone.arpa.consonant() });
                i += 1;
                continue;
            }
            let mark = match s {
                Some(1) => "ˈ",
                Some(2) => "ˌ",
                _ => "",
            };
            let next = phones.get(i + 1).map(|p| p.arpa);
            // A syllabic l: an unstressed schwa before an l that ends the syllable.
            if phone.arpa == Ah && s == Some(0) && i > 0 && next == Some(L) && !vowel_at(i + 2) {
                out.push_str("ᵊl");
                i += 2;
                continue;
            }
            // The reduced vowel of a final -ed or -es (not the vowel of "is").
            if phone.arpa == Ih
                && s == Some(0)
                && i > 0
                && i + 2 == n
                && matches!(next, Some(D | Z))
            {
                out.push_str(if british { "ɪ" } else { "ᵻ" });
                i += 1;
                continue;
            }
            let r_coda = next == Some(R) && !vowel_at(i + 2);
            let (sound, skip) = if british {
                british_vowel(
                    phone.arpa,
                    s,
                    r_coda,
                    next,
                    vowel_at(i + 2),
                    vowel_at(i + 1),
                    spelling,
                )
            } else {
                (american_vowel(phone.arpa, s), 0)
            };
            out.push_str(mark);
            out.push_str(sound);
            i += 1 + skip;
        }
        out
    }

    fn american_vowel(arpa: Arpa, stress: Option<u8>) -> &'static str {
        use Arpa::*;
        match (arpa, stress) {
            (Aa, _) => "ɑ",
            (Ae, _) => "æ",
            (Ah, Some(0)) => "ə",
            (Ah, _) => "ʌ",
            (Ao, _) => "ɔ",
            (Aw, _) => "W",
            (Ay, _) => "I",
            (Eh, _) => "ɛ",
            (Er, Some(0)) => "əɹ",
            (Er, _) => "ɜɹ",
            (Ey, _) => "A",
            (Ih, _) => "ɪ",
            (Iy, _) => "i",
            (Ow, _) => "O",
            (Oy, _) => "Y",
            (Uh, _) => "ʊ",
            _ => "u",
        }
    }

    /// A British vowel, and how many following phonemes it takes in (1 for an r it
    /// absorbs).
    fn british_vowel(
        arpa: Arpa,
        stress: Option<u8>,
        r_coda: bool,
        next: Option<Arpa>,
        vowel_after_next: bool,
        next_is_vowel: bool,
        spelling: &str,
    ) -> (&'static str, usize) {
        use Arpa::*;
        if r_coda {
            let coloured = match arpa {
                Aa => "ɑː",
                Ao | Ow => "ɔː",
                Eh | Ey | Ae => "ɛː",
                Ih | Iy => "ɪə",
                Uh | Uw => "ʊə",
                Ay => "Iə",
                Aw => "Wə",
                Ah if stress == Some(0) => "ə",
                Ah => "ʌ",
                Er if stress == Some(0) => "ə",
                Er => "ɜː",
                Oy => "Y",
                _ => "",
            };
            if !coloured.is_empty() {
                return (coloured, 1);
            }
        }
        let lot_o = spelling.contains('o') || spelling.contains("wa") || spelling.contains("qua");
        let thought = ["ou", "oa", "au", "aw"]
            .iter()
            .any(|t| spelling.contains(t));
        let sound = match (arpa, stress) {
            (Aa, _) if lot_o => "ɒ",
            (Aa, _) => "ɑː",
            (Ae, _) if matches!(next, Some(F | S | Th)) && !vowel_after_next => "ɑː",
            (Ae, _) => "a",
            (Ah, Some(0)) => "ə",
            (Ah, _) => "ʌ",
            (Ao, _) if spelling.contains('o') && !thought && next != Some(R) => "ɒ",
            (Ao, _) => "ɔː",
            (Aw, _) => "W",
            (Ay, _) => "I",
            (Eh, _) => "ɛ",
            // Before a vowel the r of er is said.
            (Er, Some(0)) if next_is_vowel => "əɹ",
            (Er, Some(0)) => "ə",
            (Er, _) if next_is_vowel => "ɜːɹ",
            (Er, _) => "ɜː",
            (Ey, _) => "A",
            (Ih, _) => "ɪ",
            (Iy, Some(0)) => "i",
            (Iy, _) => "iː",
            (Ow, _) => "Q",
            (Oy, _) => "Y",
            (Uh, _) => "ʊ",
            (_, Some(0)) => "u",
            _ => "uː",
        };
        (sound, 0)
    }

    /// The pronunciation of an inflected form of a dictionary word: -'s, -s', -s, -es,
    /// -ies, -ed, -d and -ing, with a doubled consonant or a dropped e restored.
    fn inflected(dictionary: &Dictionary, word: &str) -> Option<Vec<Phone>> {
        use Arpa::*;
        let base =
            |stem: &str| -> Option<Vec<Phone>> { dictionary.first(stem).map(<[Phone]>::to_vec) };
        let plural = |mut phones: Vec<Phone>| {
            match phones.last().map(|p| p.arpa) {
                Some(S | Z | Sh | Zh | Ch | Jh) => {
                    phones.extend([Phone::unstressed(Ih), Phone::consonant(Z)]);
                }
                Some(P | T | K | F | Th) => phones.push(Phone::consonant(S)),
                _ => phones.push(Phone::consonant(Z)),
            }
            phones
        };
        let past = |mut phones: Vec<Phone>| {
            match phones.last().map(|p| p.arpa) {
                Some(T | D) => phones.extend([Phone::unstressed(Ih), Phone::consonant(D)]),
                Some(P | K | F | Th | S | Sh | Ch) => phones.push(Phone::consonant(T)),
                _ => phones.push(Phone::consonant(D)),
            }
            phones
        };
        let undoubled = |stem: &str| -> Option<Vec<Phone>> {
            let bytes = stem.as_bytes();
            (bytes.len() >= 3 && bytes[bytes.len() - 1] == bytes[bytes.len() - 2])
                .then(|| base(&stem[..stem.len() - 1]))
                .flatten()
        };
        if let Some(stem) = word.strip_suffix("'s") {
            return base(stem).map(plural);
        }
        if let Some(stem) = word.strip_suffix("s'") {
            return base(&format!("{stem}s")).or_else(|| base(stem).map(plural));
        }
        if word.len() < 4 || !word.is_ascii() {
            return None;
        }
        if let Some(stem) = word.strip_suffix("ies") {
            return base(&format!("{stem}y")).map(plural);
        }
        if let Some(stem) = word.strip_suffix("es")
            && let Some(phones) = base(stem)
        {
            return Some(plural(phones));
        }
        if let Some(stem) = word.strip_suffix('s')
            && !stem.ends_with('s')
        {
            return base(stem).map(plural);
        }
        if let Some(stem) = word.strip_suffix("ed") {
            return base(stem)
                .or_else(|| base(&format!("{stem}e")))
                .or_else(|| undoubled(stem))
                .map(past);
        }
        if let Some(stem) = word.strip_suffix("ing") {
            let mut phones = base(stem)
                .or_else(|| base(&format!("{stem}e")))
                .or_else(|| undoubled(stem))?;
            phones.extend([Phone::unstressed(Ih), Phone::consonant(Ng)]);
            return Some(phones);
        }
        None
    }

    /// A piece of a paragraph to say.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) enum Item {
        /// A word, as written.
        Word(String),
        /// Letters to say by name.
        Spell(String),
        /// Punctuation the model reads.
        Mark(char),
    }

    /// The phonemes of one paragraph.
    pub(super) fn paragraph(dictionary: &Dictionary, text: &str, accent: Accent) -> String {
        let items = items(text);
        let mixed = text.chars().any(char::is_lowercase);
        let mut out = Output::default();
        for (index, item) in items.iter().enumerate() {
            match item {
                Item::Word(word) => {
                    // Whether the next word starts with a vowel, for "the".
                    let vowel = items[index + 1..]
                        .iter()
                        .find_map(|i| match i {
                            Item::Word(next) if spelled(next, mixed) => Some(name_vowel(next)),
                            Item::Word(next) => {
                                let next = next.to_lowercase();
                                Some(match dictionary.first(&next) {
                                    Some(phones) => phones[0].arpa.vowel(),
                                    None => next.starts_with(['a', 'e', 'i', 'o', 'u']),
                                })
                            }
                            Item::Spell(letters) => Some(name_vowel(letters)),
                            Item::Mark(_) => None,
                        })
                        .unwrap_or(false);
                    out.word(&word_sounds(dictionary, word, vowel, accent, mixed));
                }
                Item::Spell(letters) => out.word(&spell(&fold(letters), accent)),
                Item::Mark(mark) => out.mark(*mark),
            }
        }
        out.text
    }

    /// The phonemes of a paragraph as they are put together: words separated by spaces,
    /// closing punctuation against the word before it, opening punctuation against the
    /// word after it, and a dash between spaces.
    #[derive(Default)]
    struct Output {
        text: String,
        opened: bool,
        quoted: bool,
    }

    impl Output {
        fn word(&mut self, phonemes: &str) {
            if phonemes.is_empty() {
                return;
            }
            if !self.text.is_empty() && !self.opened {
                self.text.push(' ');
            }
            self.text.push_str(phonemes);
            self.opened = false;
        }

        fn mark(&mut self, mark: char) {
            let opening = match mark {
                '(' | '“' => true,
                '"' => {
                    self.quoted = !self.quoted;
                    self.quoted
                }
                _ => false,
            };
            if opening {
                if !self.text.is_empty() && !self.opened {
                    self.text.push(' ');
                }
                self.text.push(mark);
                self.opened = true;
            } else if mark == '\u{2014}' {
                self.text.push(' ');
                self.text.push(mark);
                self.opened = false;
            } else {
                self.text.push(mark);
            }
        }
    }

    /// Words and marks of `text`: numbers in words, abbreviations written out, symbols
    /// said, other characters dropped.
    pub(super) fn items(text: &str) -> Vec<Item> {
        let chars: Vec<char> = text
            .chars()
            .map(|c| match c {
                '’' | '‘' | 'ʼ' => '\'',
                '–' | '―' => '\u{2014}',
                _ => c,
            })
            .collect();
        let n = chars.len();
        let at = |j: usize| chars.get(j).copied();
        let letter = |j: usize| at(j).is_some_and(char::is_alphabetic);
        let digit = |j: usize| at(j).is_some_and(|c| c.is_ascii_digit());
        // The next character that is not a space, from `j`.
        let visible = |j: usize| {
            chars[j.min(n)..]
                .iter()
                .copied()
                .find(|c| !c.is_whitespace())
        };
        let sentence_follows =
            |j: usize| visible(j).is_none_or(|c| c.is_uppercase() || !c.is_alphanumeric());
        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            let c = chars[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            let signed = c == '-' && digit(i + 1) && (i == 0 || chars[i - 1].is_whitespace());
            let money = matches!(c, '$' | '£' | '€') && digit(i + 1);
            if c.is_ascii_digit() || signed || money {
                let start = i;
                i += 1;
                while i < n
                    && (chars[i].is_ascii_digit()
                        || (matches!(chars[i], ',' | '.' | ':') && digit(i + 1)))
                {
                    i += 1;
                }
                while letter(i) {
                    i += 1;
                }
                if at(i) == Some('%') {
                    i += 1;
                }
                let raw: String = chars[start..i].iter().collect();
                out.extend(number_items(&raw));
                continue;
            }
            if c.is_alphabetic() {
                // Letters with dots: U.S., e.g., a.m.
                if at(i + 1) == Some('.') && letter(i + 2) && !letter(i + 3) {
                    let mut letters = String::from(c);
                    let mut k = i + 1;
                    while at(k) == Some('.') && letter(k + 1) && !letter(k + 2) {
                        letters.push(chars[k + 1]);
                        k += 2;
                    }
                    let dot = at(k) == Some('.');
                    if dot {
                        k += 1;
                    }
                    i = k;
                    match letters.to_lowercase().as_str() {
                        "eg" => out.extend(words(&["for", "example"])),
                        "ie" => out.extend(words(&["that", "is"])),
                        _ => {
                            out.push(Item::Spell(letters));
                            if dot && sentence_follows(i) {
                                out.push(Item::Mark('.'));
                            }
                        }
                    }
                    continue;
                }
                let start = i;
                while letter(i) || (matches!(at(i), Some('\'' | '-')) && letter(i + 1)) {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                // The apostrophe of a plural possessive (the dogs' bowls).
                if at(i) == Some('\'') && word.ends_with(['s', 'S']) {
                    i += 1;
                }
                if at(i) == Some('.')
                    && let Some((expansion, ends)) = abbreviation(&word, visible(i + 1))
                {
                    i += 1;
                    out.extend(words(expansion));
                    if ends && sentence_follows(i) {
                        out.push(Item::Mark('.'));
                    }
                    continue;
                }
                out.push(Item::Word(word));
                continue;
            }
            i += 1;
            match c {
                '.' | ',' | '!' | '?' | ';' | ':' | '…' | '(' | ')' | '"' | '“' | '”'
                | '\u{2014}' => out.push(Item::Mark(c)),
                '[' | '{' => out.push(Item::Mark('(')),
                ']' | '}' => out.push(Item::Mark(')')),
                // A spaced hyphen or a double one is a dash.
                '-' if at(i) == Some('-') || at(i).is_none_or(char::is_whitespace) => {
                    while at(i) == Some('-') {
                        i += 1;
                    }
                    out.push(Item::Mark('\u{2014}'));
                }
                '&' => out.push(Item::Word("and".into())),
                '+' => out.push(Item::Word("plus".into())),
                '=' => out.push(Item::Word("equals".into())),
                '@' => out.push(Item::Word("at".into())),
                '%' => out.push(Item::Word("percent".into())),
                '#' if digit(i) => out.push(Item::Word("number".into())),
                _ => {}
            }
        }
        out
    }

    fn words(words: &[&str]) -> Vec<Item> {
        words.iter().map(|w| Item::Word((*w).to_string())).collect()
    }

    /// What a word followed by a full stop abbreviates, if it is one FossCut knows, and
    /// whether the stop may also end a sentence. Titles and months need their capital;
    /// `No.` is a number only before a digit; `St.` is saint before a name and street
    /// otherwise.
    fn abbreviation(word: &str, next: Option<char>) -> Option<(&'static [&'static str], bool)> {
        let capital = word.chars().next().is_some_and(char::is_uppercase);
        let name_follows = next.is_some_and(char::is_uppercase);
        let found: (&'static [&'static str], bool) = match word.to_lowercase().as_str() {
            "dr" if capital => (&["doctor"], false),
            "mr" if capital => (&["mister"], false),
            "mrs" if capital => (&["missus"], false),
            "ms" if capital => (&["miz"], false),
            "prof" if capital => (&["professor"], false),
            "st" if capital && name_follows => (&["saint"], false),
            "st" if capital => (&["street"], true),
            "jr" if capital => (&["junior"], true),
            "sr" if capital => (&["senior"], true),
            "inc" if capital => (&["incorporated"], true),
            "ltd" if capital => (&["limited"], true),
            "dept" if capital => (&["department"], true),
            "jan" if capital => (&["january"], true),
            "feb" if capital => (&["february"], true),
            "mar" if capital => (&["march"], true),
            "apr" if capital => (&["april"], true),
            "jun" if capital => (&["june"], true),
            "jul" if capital => (&["july"], true),
            "aug" if capital => (&["august"], true),
            "sep" | "sept" if capital => (&["september"], true),
            "oct" if capital => (&["october"], true),
            "nov" if capital => (&["november"], true),
            "dec" if capital => (&["december"], true),
            "vs" => (&["versus"], false),
            "etc" => (&["et", "cetera"], true),
            "approx" => (&["approximately"], false),
            "no" if next.is_some_and(|c| c.is_ascii_digit()) => (&["number"], false),
            _ => return None,
        };
        Some(found)
    }

    /// Whether `word` is said letter by letter: two capitals in mixed-case text.
    fn spelled(word: &str, mixed: bool) -> bool {
        let letters = word.chars().filter(|c| c.is_alphabetic()).count();
        let capitals = word.chars().all(|c| !c.is_alphabetic() || c.is_uppercase());
        mixed && capitals && letters == 2 && !word.contains('\'')
    }

    /// Whether the name of the first of `letters` starts with a vowel (an, ef, aitch...).
    fn name_vowel(letters: &str) -> bool {
        letters
            .to_lowercase()
            .starts_with(['a', 'e', 'f', 'h', 'i', 'l', 'm', 'n', 'o', 'r', 's', 'x'])
    }

    /// The phonemes of one word: `the` by whether a vowel follows, two capitals spelled
    /// in mixed-case text (US, AI; longer abbreviations the dictionary has are spelled
    /// in it already), then the dictionary, the parts of a hyphenated word, an
    /// inflection, and [`unknown`].
    fn word_sounds(
        dictionary: &Dictionary,
        word: &str,
        vowel_next: bool,
        accent: Accent,
        mixed: bool,
    ) -> String {
        let lower = word.to_lowercase();
        if lower == "the" {
            return if vowel_next { "ði" } else { "ðə" }.into();
        }
        if spelled(word, mixed) {
            return spell(&fold(word), accent);
        }
        if let Some(phonemes) = dictionary.phonemes(&lower, accent) {
            return phonemes;
        }
        if word.contains('-') {
            return word
                .split('-')
                .filter(|part| !part.is_empty())
                .enumerate()
                .map(|(index, part)| {
                    let sounds = word_sounds(dictionary, part, false, accent, mixed);
                    if index == 0 {
                        sounds
                    } else {
                        sounds.replace('ˈ', "ˌ")
                    }
                })
                .collect();
        }
        if let Some(phones) = inflected(dictionary, &lower) {
            return render(&phones, accent, &lower, Stress::Full);
        }
        unknown(dictionary, word, accent)
    }

    /// A word the dictionary does not have: each run of letters (split also where a
    /// lowercase letter meets a capital) from the dictionary if it has that part,
    /// otherwise spelled if it looks like an abbreviation, split into two dictionary words
    /// if it is a compound, or read by [`letters_to_sounds`]. Stress after the first part
    /// becomes secondary.
    fn unknown(dictionary: &Dictionary, text: &str, accent: Accent) -> String {
        let mut out = String::new();
        for (index, part) in letter_runs(text).iter().enumerate() {
            let sounds = part_sounds(dictionary, part, accent);
            if index == 0 {
                out.push_str(&sounds);
            } else {
                out.push_str(&sounds.replace('ˈ', "ˌ"));
            }
        }
        out
    }

    fn known(dictionary: &Dictionary, word: &str, accent: Accent) -> Option<String> {
        dictionary.phonemes(word, accent).or_else(|| {
            inflected(dictionary, word).map(|phones| render(&phones, accent, word, Stress::Full))
        })
    }

    fn part_sounds(dictionary: &Dictionary, part: &str, accent: Accent) -> String {
        if let Some(phonemes) = known(dictionary, &part.to_lowercase(), accent) {
            return phonemes;
        }
        let plain = fold(part);
        if plain.is_empty() {
            return String::new();
        }
        if let Some(phonemes) = known(dictionary, &plain, accent) {
            return phonemes;
        }
        let capitals = part.chars().all(|c| !c.is_alphabetic() || c.is_uppercase());
        if !plain.contains(['a', 'e', 'i', 'o', 'u', 'y']) || (capitals && plain.len() <= 4) {
            return spell(&plain, accent);
        }
        if let Some(phonemes) = compound(dictionary, &plain, accent) {
            return phonemes;
        }
        letters_to_sounds(&plain, accent == Accent::British)
    }

    /// Letter names, with secondary stress on all but the last.
    pub(super) fn spell(plain: &str, accent: Accent) -> String {
        let british = accent == Accent::British;
        let names: Vec<&str> = plain.chars().map(|c| letter_name(c, british)).collect();
        let last = names.len().saturating_sub(1);
        names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                if i == last {
                    (*name).to_string()
                } else {
                    name.replace('ˈ', "ˌ")
                }
            })
            .collect()
    }

    /// A word made of two dictionary words of at least three letters each, split where
    /// the two are most even.
    fn compound(dictionary: &Dictionary, plain: &str, accent: Accent) -> Option<String> {
        let length = plain.len();
        if length < 6 {
            return None;
        }
        (3..=length - 3)
            .filter_map(|at| {
                let first = dictionary.phonemes(&plain[..at], accent)?;
                let second = dictionary.phonemes(&plain[at..], accent)?;
                Some((at.min(length - at), first + &second.replace('ˈ', "ˌ")))
            })
            .max_by_key(|(even, _)| *even)
            .map(|(_, phonemes)| phonemes)
    }

    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];

    /// `n` in words: one hundred twenty three, two million five.
    pub(super) fn cardinal(n: u64) -> Vec<String> {
        fn below_thousand(n: u64, out: &mut Vec<String>) {
            if n >= 100 {
                out.push(ONES[(n / 100) as usize].into());
                out.push("hundred".into());
            }
            let rest = n % 100;
            if rest >= 20 {
                out.push(TENS[(rest / 10) as usize].into());
                if !rest.is_multiple_of(10) {
                    out.push(ONES[(rest % 10) as usize].into());
                }
            } else if rest > 0 || n == 0 {
                out.push(ONES[rest as usize].into());
            }
        }
        if n == 0 {
            return vec!["zero".into()];
        }
        let mut out = Vec::new();
        let mut rest = n;
        for (scale, name) in [
            (1_000_000_000_000_000_000u64, "quintillion"),
            (1_000_000_000_000_000, "quadrillion"),
            (1_000_000_000_000, "trillion"),
            (1_000_000_000, "billion"),
            (1_000_000, "million"),
            (1_000, "thousand"),
        ] {
            if rest >= scale {
                below_thousand(rest / scale, &mut out);
                out.push(name.into());
                rest %= scale;
            }
        }
        if rest > 0 {
            below_thousand(rest, &mut out);
        }
        out
    }

    /// Words with the last made ordinal: one is first, twenty is twentieth.
    fn ordinal(mut words: Vec<String>) -> Vec<String> {
        if let Some(last) = words.last_mut() {
            *last = match last.as_str() {
                "one" => "first".into(),
                "two" => "second".into(),
                "three" => "third".into(),
                "five" => "fifth".into(),
                "eight" => "eighth".into(),
                "nine" => "ninth".into(),
                "twelve" => "twelfth".into(),
                word if word.ends_with('y') => format!("{}ieth", &word[..word.len() - 1]),
                word => format!("{word}th"),
            };
        }
        words
    }

    /// Words with the last made plural, for decades: nineties, eighties.
    fn plural(mut words: Vec<String>) -> Vec<String> {
        if let Some(last) = words.last_mut() {
            *last = if let Some(stem) = last.strip_suffix('y') {
                format!("{stem}ies")
            } else if last.ends_with('x') {
                format!("{last}es")
            } else {
                format!("{last}s")
            };
        }
        words
    }

    /// A year in words: nineteen ninety nine, twenty twenty six, two thousand five,
    /// nineteen hundred, nineteen oh five.
    fn year(n: u64) -> Vec<String> {
        if (2000..2010).contains(&n) || n.is_multiple_of(1000) {
            return cardinal(n);
        }
        let mut out = cardinal(n / 100);
        match n % 100 {
            0 => out.push("hundred".into()),
            small @ 1..=9 => {
                out.push("oh".into());
                out.push(ONES[small as usize].into());
            }
            rest => out.extend(cardinal(rest)),
        }
        out
    }

    /// Digits one by one.
    fn digits(text: &str) -> Vec<String> {
        text.chars()
            .filter_map(|c| c.to_digit(10))
            .map(|d| ONES[d as usize].to_string())
            .collect()
    }

    /// A number as written, in words: `12`, `1,000`, `3.14`, `3:45`, `31st`, `1990s`,
    /// `2026`, `$12.50`, `£3`, `15%`, `-4`; letters after it that are not an ordinal or
    /// plural ending (`4K`) are spelled.
    pub(super) fn number_items(raw: &str) -> Vec<Item> {
        let mut text = raw;
        let mut out: Vec<String> = Vec::new();
        if let Some(rest) = text.strip_prefix('-') {
            out.push("minus".into());
            text = rest;
        }
        let currency = match text.chars().next() {
            Some('$') => Some(["dollar", "dollars", "cent", "cents"]),
            Some('£') => Some(["pound", "pounds", "penny", "pence"]),
            Some('€') => Some(["euro", "euros", "cent", "cents"]),
            _ => None,
        };
        if currency.is_some() {
            text = &text[text.chars().next().map_or(0, char::len_utf8)..];
        }
        let percent = text.ends_with('%');
        let text = text.trim_end_matches('%');
        let body_end = text
            .char_indices()
            .find(|(_, c)| c.is_alphabetic())
            .map_or(text.len(), |(i, _)| i);
        let (body, suffix) = text.split_at(body_end);
        let lower = suffix.to_lowercase();
        let ordinal_suffix = matches!(lower.as_str(), "st" | "nd" | "rd" | "th");
        let plural_suffix = lower == "s";
        let whole = |digits_text: &str| -> Option<u64> {
            let plain: String = digits_text.chars().filter(|c| *c != ',').collect();
            plain.parse().ok()
        };
        let mut spoken: Vec<String> = if let Some((hours, minutes)) = body.split_once(':') {
            let mut words = whole(hours).map(cardinal).unwrap_or_default();
            match minutes.parse::<u64>() {
                Ok(0) => words.push("o'clock".into()),
                Ok(m @ 1..=9) => {
                    words.push("oh".into());
                    words.push(ONES[m as usize].into());
                }
                Ok(m) => words.extend(cardinal(m)),
                Err(_) => words.extend(digits(minutes)),
            }
            words
        } else if let Some([one, many, small_one, small_many]) = currency {
            let (units, hundredths) = body.split_once('.').unwrap_or((body, ""));
            let units = whole(units).unwrap_or(0);
            let cents: u64 = format!("{hundredths:0<2}")
                .get(..2)
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
            let mut words = Vec::new();
            if units > 0 || cents == 0 {
                words.extend(cardinal(units));
                words.push(if units == 1 { one } else { many }.into());
            }
            if cents > 0 {
                if units > 0 {
                    words.push("and".into());
                }
                words.extend(cardinal(cents));
                words.push(if cents == 1 { small_one } else { small_many }.into());
            }
            words
        } else if let Some((units, fraction)) = body.split_once('.') {
            let mut words = whole(units).map(cardinal).unwrap_or_else(|| digits(units));
            words.push("point".into());
            words.extend(digits(fraction));
            words
        } else {
            let plain: String = body.chars().filter(|c| *c != ',').collect();
            match plain.parse::<u64>() {
                _ if plain.len() > 18 || (plain.len() > 1 && plain.starts_with('0')) => {
                    digits(&plain)
                }
                Ok(n) if ordinal_suffix => ordinal(cardinal(n)),
                Ok(n)
                    if !body.contains(',')
                        && plain.len() == 4
                        && (1100..2100).contains(&n)
                        && currency.is_none()
                        && !percent =>
                {
                    year(n)
                }
                Ok(n) => cardinal(n),
                Err(_) => digits(&plain),
            }
        };
        if plural_suffix {
            spoken = plural(spoken);
        }
        out.extend(spoken);
        if percent {
            out.push("percent".into());
        }
        let mut items: Vec<Item> = out.into_iter().map(Item::Word).collect();
        if !suffix.is_empty() && !ordinal_suffix && !plural_suffix {
            items.push(Item::Spell(suffix.to_string()));
        }
        items
    }

    /// Runs of letters in `text`, split where a lowercase letter meets a capital
    /// (`FossCut` is `Foss` and `Cut`).
    pub(super) fn letter_runs(text: &str) -> Vec<String> {
        let mut runs: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut previous_lower = false;
        for c in text.chars() {
            if !c.is_alphabetic() {
                // An apostrophe stays inside a word (don't, FossCut's).
                if matches!(c, '\'' | '’') && !current.is_empty() {
                    current.push('\'');
                } else if !current.is_empty() {
                    runs.push(std::mem::take(&mut current));
                }
                previous_lower = false;
                continue;
            }
            if c.is_uppercase() && previous_lower && !current.is_empty() {
                runs.push(std::mem::take(&mut current));
            }
            current.push(c);
            previous_lower = c.is_lowercase();
        }
        if !current.is_empty() {
            runs.push(current);
        }
        runs
    }

    /// Lowercase ASCII letters: accents removed (`ö` is `o`, `ß` is `ss`), anything else
    /// dropped.
    pub(super) fn fold(word: &str) -> String {
        let mut out = String::new();
        for c in word.chars().flat_map(char::to_lowercase) {
            let plain = match c {
                'a'..='z' => c,
                'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
                'æ' => {
                    out.push_str("ae");
                    continue;
                }
                'ç' | 'ć' | 'č' => 'c',
                'ď' | 'đ' => 'd',
                'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
                'ì'..='ï' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
                'ñ' | 'ń' | 'ň' => 'n',
                'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
                'œ' => {
                    out.push_str("oe");
                    continue;
                }
                'ř' => 'r',
                'ś' | 'š' | 'ş' => 's',
                'ß' => {
                    out.push_str("ss");
                    continue;
                }
                'ť' | 'ţ' => 't',
                'ù'..='ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
                'ý' | 'ÿ' => 'y',
                'ź' | 'ż' | 'ž' => 'z',
                'ł' => 'l',
                _ => continue,
            };
            out.push(plain);
        }
        out
    }

    /// An English letter's name in the voice model's phonemes, for letters the dictionary
    /// does not name.
    fn letter_name(c: char, british: bool) -> &'static str {
        match (c, british) {
            ('a', _) => "ˈA",
            ('b', false) => "bˈi",
            ('b', true) => "bˈiː",
            ('c', false) => "sˈi",
            ('c', true) => "sˈiː",
            ('d', false) => "dˈi",
            ('d', true) => "dˈiː",
            ('e', false) => "ˈi",
            ('e', true) => "ˈiː",
            ('f', _) => "ˈɛf",
            ('g', false) => "ʤˈi",
            ('g', true) => "ʤˈiː",
            ('h', _) => "ˈAʧ",
            ('i', _) => "ˈI",
            ('j', _) => "ʤˈA",
            ('k', _) => "kˈA",
            ('l', _) => "ˈɛl",
            ('m', _) => "ˈɛm",
            ('n', _) => "ˈɛn",
            ('o', false) => "ˈO",
            ('o', true) => "ˈQ",
            ('p', false) => "pˈi",
            ('p', true) => "pˈiː",
            ('q', false) => "kjˈu",
            ('q', true) => "kjˈuː",
            ('r', false) => "ˈɑɹ",
            ('r', true) => "ˈɑː",
            ('s', _) => "ˈɛs",
            ('t', false) => "tˈi",
            ('t', true) => "tˈiː",
            ('u', false) => "jˈu",
            ('u', true) => "jˈuː",
            ('v', false) => "vˈi",
            ('v', true) => "vˈiː",
            ('w', false) => "dˈʌbᵊlju",
            ('w', true) => "dˈʌbᵊljuː",
            ('x', _) => "ˈɛks",
            ('y', _) => "wˈI",
            ('z', false) => "zˈi",
            ('z', true) => "zˈɛd",
            _ => "",
        }
    }

    /// What a sound is, for stress.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Kind {
        /// A consonant.
        Consonant,
        /// A full vowel, which can take stress.
        Vowel,
        /// A reduced vowel, which never does (ə, ᵊ, ᵻ).
        Weak,
    }

    /// Reads a word of lowercase ASCII letters by spelling-to-sound rules written for
    /// FossCut: the common patterns of English spelling, primary stress on the first full
    /// vowel, and an unstressed `er` reduced. It is a fallback for names and invented words,
    /// so it aims at a plausible reading, not the right one.
    pub(super) fn letters_to_sounds(word: &str, british: bool) -> String {
        let w = word.as_bytes();
        let n = w.len();
        let vowel = |i: usize| i < n && matches!(w[i], b'a' | b'e' | b'i' | b'o' | b'u' | b'y');
        let consonant = |i: usize| i < n && w[i].is_ascii_alphabetic() && !vowel(i);
        let has_vowel_before =
            |i: usize| (0..i).any(|j| matches!(w[j], b'a' | b'e' | b'i' | b'o' | b'u' | b'y'));
        let at = |i: usize, s: &str| word[i..].starts_with(s);
        let ends = |i: usize, s: &str| i + s.len() == n && at(i, s);
        let mut units: Vec<(&'static str, Kind)> = Vec::new();
        let mut silent = vec![false; n];
        let mut i = 0;
        use Kind::{Consonant as C, Vowel as V, Weak as W};
        while i < n {
            if silent[i] {
                i += 1;
                continue;
            }
            let c = w[i];
            let next = if i + 1 < n { w[i + 1] } else { 0 };
            // Endings and syllables spelled the same way wherever they appear.
            let rule: Option<(usize, &[(&'static str, Kind)])> = if at(i, "tion") {
                Some((4, &[("ʃ", C), ("ə", W), ("n", C)]))
            } else if at(i, "ssion") {
                Some((5, &[("ʃ", C), ("ə", W), ("n", C)]))
            } else if at(i, "sion") {
                Some((4, &[("ʒ", C), ("ə", W), ("n", C)]))
            } else if at(i, "cian") {
                Some((4, &[("ʃ", C), ("ə", W), ("n", C)]))
            } else if at(i, "tial") || at(i, "cial") {
                Some((4, &[("ʃ", C), ("ᵊl", W)]))
            } else if ends(i, "ture") {
                Some((4, &[("ʧ", C), ("əɹ", W)]))
            } else if ends(i, "ous") && i > 0 {
                Some((3, &[("ə", W), ("s", C)]))
            } else if ends(i, "le") && i > 0 && consonant(i - 1) {
                Some((2, &[("ᵊl", W)]))
            } else if ends(i, "ed") && n > 3 && consonant(i - 1) {
                let before = w[i - 1];
                if matches!(before, b't' | b'd') {
                    Some((2, &[("ᵻ", W), ("d", C)]))
                } else if matches!(before, b'p' | b'k' | b'f' | b's' | b'x' | b'c' | b'h') {
                    Some((2, &[("t", C)]))
                } else {
                    Some((2, &[("d", C)]))
                }
            } else if ends(i, "es")
                && n > 3
                && (matches!(w[i - 1], b's' | b'x' | b'z')
                    || word[..i].ends_with("ch")
                    || word[..i].ends_with("sh"))
            {
                Some((2, &[("ᵻ", W), ("z", C)]))
            } else {
                None
            };
            if let Some((length, sounds)) = rule {
                units.extend_from_slice(sounds);
                i += length;
                continue;
            }
            // Vowels and vowel teams.
            if vowel(i) && !(c == b'y' && (i == 0 || vowel(i + 1))) {
                let r_after = |k: usize| i + k < n && w[i + k] == b'r' && !vowel(i + k + 1);
                let team: Option<(usize, &'static str, Kind)> = if at(i, "eau") {
                    Some((3, "O", V))
                } else if at(i, "eigh") {
                    Some((4, "A", V))
                } else if at(i, "igh") {
                    Some((3, "I", V))
                } else if at(i, "ough") || at(i, "augh") {
                    Some((4, "ɔ", V))
                } else if at(i, "air") || ends(i, "are") {
                    Some((3, "ɛɹ", V))
                } else if at(i, "ear") || at(i, "eer") {
                    Some((3, "ɪɹ", V))
                } else if at(i, "our") || at(i, "oor") {
                    Some((3, "ɔɹ", V))
                } else if c == b'a' && r_after(1) {
                    Some((2, "ɑɹ", V))
                } else if c == b'o' && r_after(1) {
                    Some((2, "ɔɹ", V))
                } else if matches!(c, b'e' | b'i' | b'u' | b'y') && r_after(1) {
                    Some((2, "ɜɹ", V))
                } else if at(i, "ee") || at(i, "ea") {
                    Some((2, "i", V))
                } else if at(i, "ai") || at(i, "ay") {
                    Some((2, "A", V))
                } else if at(i, "oa") || ends(i, "oe") {
                    Some((2, "O", V))
                } else if at(i, "oo") {
                    Some((2, "u", V))
                } else if ends(i, "ow") {
                    Some((2, "O", V))
                } else if at(i, "ou") || at(i, "ow") {
                    Some((2, "W", V))
                } else if at(i, "oi") || at(i, "oy") {
                    Some((2, "Y", V))
                } else if at(i, "au") || at(i, "aw") {
                    Some((2, "ɔ", V))
                } else if at(i, "ew") || at(i, "ue") || at(i, "ui") {
                    Some((2, "u", V))
                } else if ends(i, "ie") && n <= 4 {
                    Some((2, "I", V))
                } else if at(i, "ie") {
                    Some((2, "i", V))
                } else if ends(i, "ey") {
                    Some((2, "i", W))
                } else if at(i, "ei") || at(i, "ey") {
                    Some((2, "A", V))
                } else {
                    None
                };
                if at(i, "eu") {
                    units.extend_from_slice(&[("j", C), ("u", V)]);
                    i += 2;
                    continue;
                }
                if let Some((length, sound, kind)) = team {
                    units.push((sound, kind));
                    i += length;
                    continue;
                }
                // A vowel, one consonant and a final e (or -es, -ed): the vowel says its
                // name and the e is silent.
                let magic = consonant(i + 1)
                    && !matches!(next, b'w' | b'x' | b'y')
                    && i + 2 < n
                    && w[i + 2] == b'e'
                    && (i + 3 == n || (i + 4 == n && matches!(w[i + 3], b's' | b'd')));
                // The same long vowel before -tion and -sion (nation, motion) and before
                // one consonant and a final -le (table, noble).
                let open = (c != b'i' && (at(i + 1, "tion") || at(i + 1, "sion")))
                    || (consonant(i + 1) && (ends(i + 2, "le") || ends(i + 2, "les")));
                if magic || open {
                    if magic {
                        silent[i + 2] = true;
                    }
                    let sound = match c {
                        b'a' => "A",
                        b'e' => "i",
                        b'i' | b'y' => "I",
                        b'o' => "O",
                        _ => "u",
                    };
                    units.push((sound, V));
                    i += 1;
                    continue;
                }
                let last = i + 1 == n;
                let earlier = has_vowel_before(i);
                let sound: Option<(&'static str, Kind)> = match (c, last) {
                    (b'e', true) if earlier => None,
                    (b'e', true) => Some(("i", V)),
                    (b'a', true) if earlier => Some(("ə", W)),
                    (b'o', true) => Some(("O", V)),
                    (b'i', true) => Some(("i", V)),
                    (b'u', true) => Some(("u", V)),
                    (b'y', true) if earlier => Some(("i", W)),
                    (b'y', true) => Some(("I", V)),
                    (b'a', _) => Some(("æ", V)),
                    (b'e', _) => Some(("ɛ", V)),
                    (b'i' | b'y', _) => Some(("ɪ", V)),
                    (b'o', _) => Some(("ɑ", V)),
                    _ => Some(("ʌ", V)),
                };
                if let Some(sound) = sound {
                    units.push(sound);
                }
                i += 1;
                continue;
            }
            // Consonants.
            let start = i == 0;
            let (length, sounds): (usize, &[(&'static str, Kind)]) = if start && at(i, "kn") {
                (2, &[("n", C)])
            } else if start && at(i, "wr") {
                (2, &[("ɹ", C)])
            } else if start && (at(i, "gn") || at(i, "pn")) {
                (2, &[("n", C)])
            } else if start && at(i, "ps") {
                (2, &[("s", C)])
            } else if start && c == b'x' {
                (1, &[("z", C)])
            } else if at(i, "tch") {
                (3, &[("ʧ", C)])
            } else if at(i, "sch") {
                (3, &[("s", C), ("k", C)])
            } else if at(i, "ch") {
                (2, &[("ʧ", C)])
            } else if at(i, "sh") {
                (2, &[("ʃ", C)])
            } else if at(i, "th") {
                (2, &[("θ", C)])
            } else if at(i, "ph") {
                (2, &[("f", C)])
            } else if at(i, "wh") {
                (2, &[("w", C)])
            } else if at(i, "ck") {
                (2, &[("k", C)])
            } else if at(i, "ng") {
                (2, &[("ŋ", C)])
            } else if at(i, "nk") {
                (2, &[("ŋ", C), ("k", C)])
            } else if at(i, "qu") {
                (2, &[("k", C), ("w", C)])
            } else if at(i, "dg") {
                (2, &[("ʤ", C)])
            } else if at(i, "gh") {
                if start { (2, &[("ɡ", C)]) } else { (2, &[]) }
            } else if c == next && c != b'c' {
                // A doubled consonant sounds once.
                (2, single(c, i, w, &silent))
            } else {
                (1, single(c, i, w, &silent))
            };
            units.extend_from_slice(sounds);
            i += length;
        }
        // Primary stress before the first full vowel; an unstressed er is reduced.
        let stressed = units
            .iter()
            .position(|(_, kind)| *kind == Kind::Vowel)
            .or_else(|| units.iter().position(|(_, kind)| *kind == Kind::Weak));
        let mut out = String::new();
        for (index, (sound, kind)) in units.iter().enumerate() {
            let sound = if Some(index) != stressed && *sound == "ɜɹ" {
                "əɹ"
            } else {
                sound
            };
            if Some(index) == stressed {
                out.push('ˈ');
            }
            // A reduced final y stays short in British English too.
            out.push_str(if british && *kind != Kind::Weak {
                british_sound(sound)
            } else {
                sound
            });
        }
        out
    }

    /// One consonant letter's sound, from its neighbours: a soft c and g before e, i or
    /// y; s voiced between vowels and at the end after a voiced sound (the letter before
    /// a silent e counts as the sound before).
    fn single(c: u8, i: usize, w: &[u8], silent: &[bool]) -> &'static [(&'static str, Kind)] {
        use Kind::Consonant as C;
        let n = w.len();
        let before = match i.checked_sub(1) {
            Some(j) if silent[j] => j.checked_sub(1).map(|k| w[k]),
            Some(j) => Some(w[j]),
            None => None,
        };
        let vowel = |j: usize| j < n && matches!(w[j], b'a' | b'e' | b'i' | b'o' | b'u' | b'y');
        let soft = i + 1 < n && matches!(w[i + 1], b'e' | b'i' | b'y');
        match c {
            b'c' if soft => &[("s", C)],
            b'c' | b'k' | b'q' => &[("k", C)],
            b'g' if soft => &[("ʤ", C)],
            b'g' => &[("ɡ", C)],
            b'j' => &[("ʤ", C)],
            b'r' => &[("ɹ", C)],
            b'x' => &[("k", C), ("s", C)],
            b'y' => &[("j", C)],
            b'h' if vowel(i + 1) => &[("h", C)],
            b'h' => &[],
            b's' if i > 0 && vowel(i - 1) && vowel(i + 1) => &[("z", C)],
            b's' if i + 1 == n
                && matches!(
                    before,
                    Some(
                        b'a' | b'e'
                            | b'i'
                            | b'o'
                            | b'u'
                            | b'y'
                            | b'b'
                            | b'd'
                            | b'g'
                            | b'l'
                            | b'm'
                            | b'n'
                            | b'r'
                            | b'v'
                            | b'w'
                    )
                ) =>
            {
                &[("z", C)]
            }
            b's' => &[("s", C)],
            b'b' => &[("b", C)],
            b'd' => &[("d", C)],
            b'f' => &[("f", C)],
            b'l' => &[("l", C)],
            b'm' => &[("m", C)],
            b'n' => &[("n", C)],
            b'p' => &[("p", C)],
            b't' => &[("t", C)],
            b'v' => &[("v", C)],
            b'w' => &[("w", C)],
            b'z' => &[("z", C)],
            _ => &[],
        }
    }

    /// The British sound for an American one, by the same rules as for dictionary words:
    /// no r after vowels, long vowels marked, the rounded short o and the open short a.
    fn british_sound(sound: &'static str) -> &'static str {
        match sound {
            "O" => "Q",
            "æ" => "a",
            "ɑ" => "ɒ",
            "ɑɹ" => "ɑː",
            "ɔɹ" => "ɔː",
            "ɜɹ" => "ɜː",
            "əɹ" => "ə",
            "ɛɹ" => "ɛː",
            "ɪɹ" => "ɪə",
            "i" => "iː",
            "u" => "uː",
            "ɔ" => "ɔː",
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_come_from_the_manifest_with_accent_and_gender() {
        let missing = std::env::temp_dir().join(format!("fosscut-voices-{}", std::process::id()));
        let all = voices(&missing);
        assert_eq!(all.len(), 28);
        assert!(all.iter().all(|v| !v.installed));
        let heart = all.iter().find(|v| v.name == DEFAULT_VOICE).unwrap();
        assert_eq!(
            (heart.accent, heart.gender),
            (Accent::American, Gender::Female)
        );
        let george = voice("bm_george").unwrap();
        assert_eq!(
            (george.accent, george.gender),
            (Accent::British, Gender::Male)
        );
        let american = all.iter().filter(|v| v.accent == Accent::American).count();
        assert_eq!(american, 20);
        let error = voice("hu_anna").unwrap_err();
        assert_eq!(error.code(), "fosscut.voice.unknown_voice");
        assert!(error.to_string().contains("af_heart"), "{error}");
        assert_eq!(kind("xf_test"), None);
        assert_eq!(kind("af"), None);
        assert_eq!(
            serde_json::to_value(george).unwrap(),
            serde_json::json!({ "name": "bm_george", "accent": "british", "gender": "male", "installed": false })
        );
    }

    #[test]
    fn only_english_is_spoken() {
        for language in ["en", "en-US", "EN_gb", "english", " eng "] {
            assert_eq!(check_language(language), Ok(()), "{language}");
        }
        for language in ["hu", "hu-HU", "de", "", "enx"] {
            let error = check_language(language).unwrap_err();
            assert_eq!(error.code(), "fosscut.voice.language_unavailable");
        }
        assert_eq!(check_script("Köszönöm szépen"), Ok(()));
        assert_eq!(check_script("12:30 !!"), Ok(()));
        assert_eq!(
            check_script("Привет, мир").unwrap_err().code(),
            "fosscut.voice.language_unavailable"
        );
    }

    #[test]
    fn errors_have_codes() {
        assert_eq!(
            VoiceError::EmptyText.code(),
            "fosscut.request.invalid_arguments"
        );
        assert_eq!(
            VoiceError::InvalidSpeed { speed: 3.0 }.to_string(),
            "speed 3 is outside 0.5 to 2"
        );
        assert_eq!(
            VoiceError::from(ModelError::NotInstalled { what: MODEL.into() }).code(),
            "fosscut.model.not_installed"
        );
        assert_eq!(
            VoiceError::Onnx(OnnxError::RuntimeMissing { tried: Vec::new() }).code(),
            "fosscut.model.runtime_missing"
        );
        assert_eq!(
            VoiceError::Failed("x".into()).code(),
            "fosscut.voice.synthesis_failed"
        );
        assert_eq!(VoiceError::Unavailable.code(), "fosscut.voice.unavailable");
        for (error, code) in [
            (
                ModelError::Unknown { name: "x".into() },
                "fosscut.model.unknown",
            ),
            (
                ModelError::ChecksumMismatch {
                    name: "x".into(),
                    expected: "a".into(),
                    actual: "b".into(),
                },
                "fosscut.model.checksum_mismatch",
            ),
            (
                ModelError::DownloadFailed {
                    name: "x".into(),
                    message: "m".into(),
                },
                "fosscut.model.download_failed",
            ),
            (
                ModelError::Unsupported {
                    name: "x".into(),
                    platform: "p".into(),
                },
                "fosscut.model.unsupported_platform",
            ),
        ] {
            assert_eq!(model_code(&error), code);
        }
        assert_eq!(checked_speed(1.0), Ok(1.0));
        for speed in [0.4, 2.5, f32::NAN, f32::INFINITY] {
            assert!(checked_speed(speed).is_err(), "{speed}");
        }
    }

    #[test]
    fn sentences_are_cut_after_their_punctuation() {
        assert_eq!(
            sentences("wˈAt... wˌʌt?! jˈɛs \u{2014} nˈO; mˈAbi: ˌOkˈA"),
            ["wˈAt...", "wˌʌt?!", "jˈɛs \u{2014} nˈO; mˈAbi: ˌOkˈA"]
        );
        assert_eq!(sentences("“hˈI.” bˈI."), ["“hˈI.”", "bˈI."]);
        assert_eq!(sentences("θɹˈi.fˈIv"), ["θɹˈi.fˈIv"], "no space, no cut");
        assert!(sentences("  ").is_empty());
    }

    #[test]
    fn long_sentences_are_cut_at_clauses_then_words() {
        let sentence = "ab cd, ef gh; ij kl mn op";
        assert_eq!(pieces(sentence, 100), [sentence]);
        for limit in [4, 6, 9, 12, 15] {
            let cut = pieces(sentence, limit);
            assert!(
                cut.iter().all(|p| p.chars().count() <= limit),
                "{limit}: {cut:?}"
            );
            assert_eq!(
                cut.join(" ").replace(' ', ""),
                sentence.replace(' ', ""),
                "nothing lost at {limit}"
            );
        }
        let cut = pieces("ab cd, ef gh; ij", 8);
        assert_eq!(cut, ["ab cd,", "ef gh;", "ij"]);
        let long = "x".repeat(25);
        assert_eq!(pieces(&long, 10).len(), 3);
        assert!(speakable("hˈI.") && !speakable(" ... !"));
    }

    #[test]
    fn silence_is_trimmed_with_a_margin() {
        let mut samples = vec![0.0001f32; 2400];
        samples.extend(std::iter::repeat_n(0.5, 100));
        samples.extend(vec![0.0f32; 2400]);
        let trimmed = trim(&samples);
        assert_eq!(trimmed.len(), 100 + 2 * 480);
        assert!(trim(&[0.0; 100]).is_empty());
        let speech = Speech {
            samples: vec![0.0; 36_000],
            rate: RATE,
        };
        assert_eq!(speech.seconds(), 1.5);
        assert_eq!(speech.duration(), Flicks::from_millis(1500));
    }

    #[test]
    fn spelling_rules_read_invented_words() {
        use pronounce::letters_to_sounds as read;
        for (word, expected) in [
            ("fosscut", "fˈɑskʌt"),
            ("zorblax", "zˈɔɹblæks"),
            ("blorptastic", "blˈɔɹptæstɪk"),
            ("lake", "lˈAk"),
            ("bike", "bˈIk"),
            ("hopes", "hˈOps"),
            ("station", "stˈAʃən"),
            ("motion", "mˈOʃən"),
            ("mission", "mˈɪʃən"),
            ("knight", "nˈIt"),
            ("phone", "fˈOn"),
            ("cell", "sˈɛl"),
            ("gem", "ʤˈɛm"),
            ("boxes", "bˈɑksᵻz"),
            ("jumped", "ʤˈʌmpt"),
            ("wanted", "wˈæntᵻd"),
            ("table", "tˈAbᵊl"),
            ("happy", "hˈæpi"),
            ("my", "mˈI"),
            ("rose", "ɹˈOz"),
            ("thinker", "θˈɪŋkəɹ"),
            ("noble", "nˈObᵊl"),
        ] {
            assert_eq!(read(word, false), expected, "{word}");
        }
        assert_eq!(read("fosscut", true), "fˈɒskʌt");
        assert_eq!(read("zorblax", true), "zˈɔːblaks");
        assert_eq!(read("rose", true), "ɹˈQz");
        assert_eq!(read("happy", true), "hˈapi", "a short final y");
        assert_eq!(read("thinker", true), "θˈɪŋkə");
        assert_eq!(read("", false), "");
        assert_eq!(pronounce::fold("Köszönöm"), "koszonom");
        assert_eq!(pronounce::fold("Straße-Œuvre!"), "strasseoeuvre");
        assert_eq!(
            pronounce::letter_runs("FossCut's 3D-iPhone"),
            ["Foss", "Cut's", "D", "i", "Phone"]
        );
        assert_eq!(pronounce::letter_runs("FossCut"), ["Foss", "Cut"]);
        assert_eq!(pronounce::letter_runs("don't-stop"), ["don't", "stop"]);
    }

    /// A few CMU Pronouncing Dictionary lines.
    fn dictionary() -> Dictionary {
        Dictionary::parse(
            "the DH AH0\nthe(2) DH AH1\nquick K W IH1 K\nbrown B R AW1 N\nfox F AA1 K S\n\
             over OW1 V ER0\nwater W AO1 T ER0\nthere DH EH1 R\nvery V EH1 R IY0\n\
             father F AA1 DH ER0\nbath B AE1 TH\ncat K AE1 T\ndog D AO1 G\nlaw L AO1\n\
             bottle B AA1 T AH0 L\nwanted W AO1 N T IH0 D\nfbi EH1 F B IY1 AY1\n\
             thirty TH ER1 D IY2\nmoon M UW1 N\nbase B EY1 S\napple AE1 P AH0 L\n\
             and AH0 N D\nand(2) AE1 N D\nit IH1 T\nis IH1 Z\nfoss F AA1 S\ncut K AH1 T\n\
             book B UH1 K\ne IY1\nclip K L IH1 P\nedit EH1 D AH0 T\ndollars D AA1 L ER0 Z\n\
             twelve T W EH1 L V\nfifty F IH1 F T IY0\ncents S EH1 N T S\n\
             doctor D AA1 K T ER0\nsmith S M IH1 TH\nbroken( B R OW1 K AH0 N # a comment\n\
             bad XX1 Y\n",
        )
    }

    #[test]
    fn the_dictionary_reads_alternates_and_comments() {
        let words = dictionary();
        assert_eq!(
            words.len(),
            36,
            "the line with an unknown phoneme is skipped"
        );
        assert_eq!(words.phonemes("the", Accent::American).unwrap(), "ðə");
        assert_eq!(
            words.phonemes("broken", Accent::American).unwrap(),
            "bɹˈOkən"
        );
        assert!(words.phonemes("bad", Accent::American).is_none());
        assert!(Dictionary::parse("").is_empty());
        let missing = std::env::temp_dir().join(format!("fosscut-cmudict-{}", std::process::id()));
        assert_eq!(
            Dictionary::open(&missing).unwrap_err().code(),
            "fosscut.model.not_installed"
        );
    }

    #[test]
    fn arpabet_becomes_the_models_phonemes_in_both_accents() {
        let words = dictionary();
        let say = |word: &str, accent| words.phonemes(word, accent).unwrap();
        use Accent::{American as Us, British as Gb};
        for (word, american, british) in [
            ("quick", "kwˈɪk", "kwˈɪk"),
            ("brown", "bɹˈWn", "bɹˈWn"),
            ("over", "ˈOvəɹ", "ˈQvə"),
            ("water", "wˈɔTəɹ", "wˈɔːtə"),
            ("there", "ðˈɛɹ", "ðˈɛː"),
            ("very", "vˈɛɹi", "vˈɛɹi"),
            ("fox", "fˈɑks", "fˈɒks"),
            ("father", "fˈɑðəɹ", "fˈɑːðə"),
            ("bath", "bˈæθ", "bˈɑːθ"),
            ("cat", "kˈæt", "kˈat"),
            ("dog", "dˈɔɡ", "dˈɒɡ"),
            ("law", "lˈɔ", "lˈɔː"),
            ("bottle", "bˈɑTᵊl", "bˈɒtᵊl"),
            ("wanted", "wˈɔntᵻd", "wˈɔːntɪd"),
            ("fbi", "ˌɛfbˌiˈI", "ˌɛfbˌiːˈI"),
            ("thirty", "θˈɜɹdi", "θˈɜːdi"),
            ("moon", "mˈun", "mˈuːn"),
            ("and", "ənd", "ənd"),
            ("it", "ˌɪt", "ˌɪt"),
        ] {
            assert_eq!(say(word, Us), american, "{word} in American English");
            assert_eq!(say(word, Gb), british, "{word} in British English");
        }
    }

    #[test]
    fn paragraphs_read_numbers_abbreviations_and_unknown_words() {
        let words = dictionary();
        let say = |text: &str| pronounce::paragraph(&words, text, Accent::American);
        assert_eq!(say("The quick brown fox."), "ðə kwˈɪk bɹˈWn fˈɑks.");
        assert_eq!(say("the apple"), "ði ˈæpᵊl", "the before a vowel");
        assert_eq!(
            say("it is"),
            "ˌɪt ɪz",
            "pronouns light, function words unstressed"
        );
        assert_eq!(say("Dr. Smith"), "dˈɑktəɹ smˈɪθ");
        // Inflections, case changes and compounds of dictionary words.
        assert_eq!(say("clips"), "klˈɪps");
        assert_eq!(say("edits"), "ˈɛdəts");
        assert_eq!(say("FossCut"), "fˈɑskˌʌt");
        assert_eq!(say("moonbase"), "mˈunbˌAs");
        assert_eq!(say("e-book"), "ˈibˌʊk");
        // Abbreviations are spelled; other unknown words read by rule.
        assert_eq!(say("the FBI and the US"), "ði ˌɛfbˌiˈI ənd ðə jˌuˈɛs");
        assert_eq!(say("qzx"), "kjˌuzˌiˈɛks");
        assert_eq!(say("zorblax"), "zˈɔɹblæks");
        // Punctuation is kept for pauses: closing marks against the word, a dash spaced.
        assert_eq!(say("(fox) - \"cat\"; dog!"), "(fˈɑks) — \"kˈæt\"; dˈɔɡ!");
        assert_eq!(say("bath, and..."), "bˈæθ, ənd...");
        let items = pronounce::items("$12.50 at 3:45 on Oct. 31st, e.g. in the U.S. Then 15% & 4K");
        let texts: Vec<String> = items
            .iter()
            .map(|i| match i {
                pronounce::Item::Word(w) => w.clone(),
                pronounce::Item::Spell(s) => format!("<{s}>"),
                pronounce::Item::Mark(m) => m.to_string(),
            })
            .collect();
        assert_eq!(
            texts.join(" "),
            "twelve dollars and fifty cents at three forty five on october thirty first , \
             for example in the <US> . Then fifteen percent and four <K>"
        );
    }

    #[test]
    fn numbers_are_read_in_words() {
        let read = |raw: &str| -> String {
            pronounce::number_items(raw)
                .into_iter()
                .map(|i| match i {
                    pronounce::Item::Word(w) => w,
                    pronounce::Item::Spell(s) => format!("<{s}>"),
                    pronounce::Item::Mark(m) => m.to_string(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        for (raw, words) in [
            ("0", "zero"),
            ("7", "seven"),
            ("15", "fifteen"),
            ("42", "forty two"),
            ("100", "one hundred"),
            ("1,234", "one thousand two hundred thirty four"),
            ("1000000", "one million"),
            ("2026", "twenty twenty six"),
            ("1999", "nineteen ninety nine"),
            ("1905", "nineteen oh five"),
            ("1900", "nineteen hundred"),
            ("2000", "two thousand"),
            ("2007", "two thousand seven"),
            ("2100", "two thousand one hundred"),
            ("1990s", "nineteen nineties"),
            ("80s", "eighties"),
            ("1st", "first"),
            ("22nd", "twenty second"),
            ("103rd", "one hundred third"),
            ("12th", "twelfth"),
            ("40th", "fortieth"),
            ("3.14", "three point one four"),
            ("3:45", "three forty five"),
            ("3:05", "three oh five"),
            ("11:00", "eleven o'clock"),
            ("$1", "one dollar"),
            ("$12.50", "twelve dollars and fifty cents"),
            ("$0.99", "ninety nine cents"),
            ("£3", "three pounds"),
            ("€5.01", "five euros and one cent"),
            ("15%", "fifteen percent"),
            ("-4", "minus four"),
            ("007", "zero zero seven"),
            ("4K", "four <K>"),
        ] {
            assert_eq!(read(raw), words, "{raw}");
        }
        assert_eq!(
            pronounce::cardinal(u64::MAX).join(" "),
            "eighteen quintillion four hundred forty six quadrillion seven hundred forty four \
             trillion seventy three billion seven hundred nine million five hundred fifty one \
             thousand six hundred fifteen"
        );
    }
}
