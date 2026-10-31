//! Translation with OPUS-MT (ADR-0033 item 6): English to Hungarian and Hungarian to
//! English, on the CPU, so the same texts always give the same translations.
//!
//! How a text is translated:
//!
//! 1. **Pieces.** The text is normalized (Unicode NFKC, control characters removed,
//!    whitespace collapsed) and cut into the source language's SentencePiece pieces with
//!    the model's own `source.spm`, by the most likely segmentation (unigram Viterbi).
//!    Pieces map to token ids through `vocab.json`; a piece it lacks is the unknown token.
//!    A text longer than [`SEGMENT_TOKENS`] is translated in segments of whole sentences
//!    (the models learned from single sentences), a sentence longer than
//!    [`MAX_SOURCE_TOKENS`] is cut between words, and the segments' translations are joined
//!    with spaces.
//! 2. **One segment at a time.** The encoder reads a segment and the merged decoder writes
//!    its translation token by token, reusing its attention cache. The 8-bit models
//!    quantize each run's activations as a whole, so in a batch of several segments a
//!    text's translation could change with the texts beside it (3 of 10 short sentences
//!    did in testing). By default a [`Translator`] therefore runs one segment at a time,
//!    and a text always gets the same translation; [`Translator::with_batch`] trades that
//!    for about twice the speed.
//! 3. **Greedy decoding.** Each step takes the most likely token, never the padding token,
//!    until the end token or [`max_new_tokens`] tokens; the pieces are joined and their
//!    word marks turned back into spaces.
//!
//! The pairs are the installed `translate` models' directions, `en>hu` and `hu>en`.
//!
//! The types, the errors and [`pairs`] exist in every build; translating needs the `onnx`
//! feature, which is on by default.

// Without the `onnx` feature only the types, errors and listings are used.
#![cfg_attr(not(feature = "onnx"), allow(dead_code))]

use std::collections::HashMap;
use std::path::Path;

use thiserror::Error;

use crate::models::{self, Manifest, ModelError};
use crate::onnx::OnnxError;

/// Source tokens up to which a text's sentences are translated together; a longer text is
/// translated in segments of whole sentences.
pub const SEGMENT_TOKENS: usize = 64;
/// The most source tokens in one segment; a longer sentence is cut between words.
pub const MAX_SOURCE_TOKENS: usize = 256;
/// The model's end token.
const END: i64 = 0;
/// The model's unknown token.
const UNKNOWN: i64 = 1;
/// The model's padding token, which also starts the decoder and is never generated.
const PAD: i64 = 62_521;

/// The most tokens written for a segment of `source` tokens: twice as many and ten more,
/// within the model's 512 positions.
pub fn max_new_tokens(source: usize) -> usize {
    (2 * source + 10).min(511)
}

/// Why a translation could not be made.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TranslateError {
    /// No translation model has this direction.
    #[error("there is no translation model for {pair}; the pairs are {}", pairs.join(", "))]
    UnsupportedPair {
        /// The requested direction.
        pair: String,
        /// The directions there are, such as `en>hu`.
        pairs: Vec<String>,
    },
    /// The model for the pair is not installed (or unknown).
    #[error(transparent)]
    Model(#[from] ModelError),
    /// ONNX Runtime is missing or could not load the model.
    #[error(transparent)]
    Onnx(#[from] OnnxError),
    /// The model failed while translating, or its files are damaged.
    #[error("translation failed: {0}")]
    Failed(String),
    /// This build has no translation (the `onnx` feature is off).
    #[error("this build of FossCut has no translation")]
    Unavailable,
}

impl TranslateError {
    /// The stable error code front ends report (ADR-0006).
    pub fn code(&self) -> &'static str {
        match self {
            TranslateError::UnsupportedPair { .. } => "fosscut.translate.unsupported_pair",
            TranslateError::Model(error) => crate::voice::model_code(error),
            TranslateError::Onnx(error) => error.code(),
            TranslateError::Failed(_) => "fosscut.translate.failed",
            TranslateError::Unavailable => "fosscut.translate.unavailable",
        }
    }
}

/// Every translation direction FossCut has a model for, such as `en>hu`.
pub fn pairs() -> Vec<&'static str> {
    models::MODELS
        .iter()
        .filter(|m| m.task == "translate")
        .map(|m| m.languages)
        .collect()
}

/// The model for `pair` (`en>hu` or `hu>en`), whether installed or not.
pub fn model_for(pair: &str) -> Result<&'static Manifest, TranslateError> {
    let wanted = pair.trim().to_lowercase();
    models::MODELS
        .iter()
        .find(|m| m.task == "translate" && m.languages == wanted)
        .ok_or_else(|| TranslateError::UnsupportedPair {
            pair: pair.to_string(),
            pairs: pairs().into_iter().map(String::from).collect(),
        })
}

/// Translates `texts` from one language to the other as `pair` says (`en>hu` or `hu>en`),
/// with the model installed in `models`; one translation per text, in order, and an empty
/// text stays empty. For many calls, open one [`Translator`] and reuse it.
pub fn translate(
    models: &Path,
    pair: &str,
    texts: &[String],
) -> Result<Vec<String>, TranslateError> {
    #[cfg(feature = "onnx")]
    {
        Translator::open(models, pair)?.translate(texts)
    }
    #[cfg(not(feature = "onnx"))]
    {
        let _ = (models, texts, model_for(pair)?);
        Err(TranslateError::Unavailable)
    }
}

#[cfg(feature = "onnx")]
pub use inference::Translator;

/// A SentencePiece unigram model: pieces with their log probabilities.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Unigram {
    scores: HashMap<String, f32>,
    longest: usize,
    unknown: f32,
}

impl Unigram {
    /// Reads a SentencePiece model file (a `ModelProto` message): its normal and
    /// user-defined pieces with their scores.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Unigram, String> {
        let mut scores = HashMap::new();
        let mut reader = Proto::new(bytes);
        while let Some((field, value)) = reader.field()? {
            let (1, Wire::Bytes(piece)) = (field, value) else {
                continue;
            };
            let mut text = None;
            let mut score = 0.0f32;
            let mut kind = 1u64;
            let mut inner = Proto::new(piece);
            while let Some((field, value)) = inner.field()? {
                match (field, value) {
                    (1, Wire::Bytes(b)) => {
                        text = Some(String::from_utf8(b.to_vec()).map_err(|e| e.to_string())?);
                    }
                    (2, Wire::Fixed32(bits)) => score = f32::from_bits(bits),
                    (3, Wire::Varint(v)) => kind = v,
                    _ => {}
                }
            }
            // 1 is a normal piece and 4 a user-defined one; unknown, control, unused and
            // byte pieces are never matched in text.
            if let Some(text) = text
                && matches!(kind, 1 | 4)
            {
                scores.insert(text, score);
            }
        }
        if scores.is_empty() {
            return Err("the SentencePiece model has no pieces".into());
        }
        let longest = scores.keys().map(|p| p.chars().count()).max().unwrap_or(1);
        // SentencePiece scores an unknown character 10 below the least likely piece.
        let lowest = scores.values().copied().fold(f32::MAX, f32::min);
        Ok(Unigram {
            scores,
            longest,
            unknown: lowest - 10.0,
        })
    }

    /// The most likely pieces for `text`, after [`normalize`]. A character no piece covers
    /// is a piece of its own, which the vocabulary maps to the unknown token.
    pub(crate) fn encode(&self, text: &str) -> Vec<String> {
        let normalized = normalize(text);
        if normalized.is_empty() {
            return Vec::new();
        }
        let chars: Vec<char> = format!("▁{}", normalized.replace(' ', "▁"))
            .chars()
            .collect();
        let n = chars.len();
        let mut best = vec![f32::NEG_INFINITY; n + 1];
        let mut from = vec![0usize; n + 1];
        best[0] = 0.0;
        for start in 0..n {
            if best[start] == f32::NEG_INFINITY {
                continue;
            }
            let mut piece = String::new();
            let mut matched_single = false;
            for end in start + 1..=n.min(start + self.longest) {
                piece.push(chars[end - 1]);
                if let Some(score) = self.scores.get(&piece) {
                    matched_single |= end == start + 1;
                    let total = best[start] + score;
                    if total > best[end] {
                        best[end] = total;
                        from[end] = start;
                    }
                }
            }
            if !matched_single {
                let total = best[start] + self.unknown;
                if total > best[start + 1] {
                    best[start + 1] = total;
                    from[start + 1] = start;
                }
            }
        }
        let mut pieces = Vec::new();
        let mut end = n;
        while end > 0 {
            let start = from[end];
            pieces.push(chars[start..end].iter().collect::<String>());
            end = start;
        }
        pieces.reverse();
        pieces
    }
}

/// Text as the tokenizer reads it: Unicode NFKC, control characters removed, runs of
/// whitespace as one space, trimmed.
pub(crate) fn normalize(text: &str) -> String {
    #[cfg(feature = "onnx")]
    let text: String = unicode_normalization::UnicodeNormalization::nfkc(text).collect();
    #[cfg(not(feature = "onnx"))]
    let text = text.to_string();
    text.chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Token ids for `pieces` from `vocabulary` (runs of unknown pieces as one unknown token),
/// in segments that each end with the end token: sentences (cut after `.`, `!`, `?` or `…`)
/// packed together up to `pack` tokens, and a sentence longer than `limit` cut at word
/// starts.
pub(crate) fn segments(
    pieces: &[String],
    vocabulary: &HashMap<String, i64>,
    pack: usize,
    limit: usize,
) -> Vec<Vec<i64>> {
    let mut ids: Vec<(i64, &str)> = Vec::new();
    for piece in pieces {
        let id = vocabulary.get(piece).copied().unwrap_or(UNKNOWN);
        if id == UNKNOWN && ids.last().is_some_and(|(last, _)| *last == UNKNOWN) {
            continue;
        }
        ids.push((id, piece));
    }
    let mut sentences: Vec<&[(i64, &str)]> = Vec::new();
    let mut start = 0;
    for (i, (_, piece)) in ids.iter().enumerate() {
        let word_follows = ids.get(i + 1).is_none_or(|(_, next)| next.starts_with('▁'));
        if piece.ends_with(['.', '!', '?', '…']) && word_follows {
            sentences.push(&ids[start..=i]);
            start = i + 1;
        }
    }
    if start < ids.len() {
        sentences.push(&ids[start..]);
    }
    let limit = limit.max(1);
    let mut units: Vec<&[(i64, &str)]> = Vec::new();
    for sentence in sentences {
        let mut rest = sentence;
        while rest.len() > limit {
            let cut = rest[..limit]
                .iter()
                .rposition(|(_, p)| p.starts_with('▁'))
                .filter(|&i| i > 0)
                .unwrap_or(limit);
            units.push(&rest[..cut]);
            rest = &rest[cut..];
        }
        if !rest.is_empty() {
            units.push(rest);
        }
    }
    let mut out: Vec<Vec<i64>> = Vec::new();
    for unit in units {
        let unit_ids = unit.iter().map(|(id, _)| *id);
        match out.last_mut() {
            Some(last) if last.len() + unit.len() <= pack => last.extend(unit_ids),
            _ => out.push(unit_ids.collect()),
        }
    }
    for segment in &mut out {
        segment.push(END);
    }
    out
}

/// Pieces back to text: joined, word marks as spaces, whitespace collapsed.
pub(crate) fn detokenize<'a>(pieces: impl IntoIterator<Item = &'a str>) -> String {
    pieces
        .into_iter()
        .collect::<String>()
        .replace('▁', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A protobuf value.
enum Wire<'a> {
    Varint(u64),
    Fixed64,
    Bytes(&'a [u8]),
    Fixed32(u32),
}

/// Reads protobuf fields one after another.
struct Proto<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Proto<'a> {
    fn new(bytes: &'a [u8]) -> Proto<'a> {
        Proto { bytes, at: 0 }
    }

    fn varint(&mut self) -> Result<u64, String> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self
                .bytes
                .get(self.at)
                .ok_or("the SentencePiece model ends inside a number")?;
            self.at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err("a number in the SentencePiece model is too long".into())
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(count)
            .filter(|&end| end <= self.bytes.len())
            .ok_or("the SentencePiece model ends inside a field")?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    /// The next field number and value, or `None` at the end.
    fn field(&mut self) -> Result<Option<(u64, Wire<'a>)>, String> {
        if self.at >= self.bytes.len() {
            return Ok(None);
        }
        let key = self.varint()?;
        let value = match key & 7 {
            0 => Wire::Varint(self.varint()?),
            1 => {
                self.take(8)?;
                Wire::Fixed64
            }
            2 => {
                let length = usize::try_from(self.varint()?).map_err(|e| e.to_string())?;
                Wire::Bytes(self.take(length)?)
            }
            5 => {
                let b = self.take(4)?;
                Wire::Fixed32(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            other => return Err(format!("the SentencePiece model has wire type {other}")),
        };
        Ok(Some((key >> 3, value)))
    }
}

/// OPUS-MT on ONNX Runtime.
#[cfg(feature = "onnx")]
mod inference {
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::path::Path;

    use ort::session::SessionInputValue;
    use ort::value::{DynValue, Tensor};

    use super::{
        END, MAX_SOURCE_TOKENS, PAD, SEGMENT_TOKENS, TranslateError, UNKNOWN, Unigram, detokenize,
        max_new_tokens, model_for, segments,
    };
    use crate::models::{self, ModelError};
    use crate::onnx::{self, Device, Session, SessionOptions};

    /// Decoder layers, attention heads and the size of each head.
    const LAYERS: usize = 6;
    const HEADS: usize = 8;
    const HEAD: usize = 64;

    fn failed(message: impl std::fmt::Display) -> TranslateError {
        TranslateError::Failed(message.to_string())
    }

    /// A translation model loaded once, for many texts.
    pub struct Translator {
        pair: String,
        encoder: Session,
        decoder: Session,
        source: Unigram,
        vocabulary: HashMap<String, i64>,
        pieces: Vec<String>,
        cache: Vec<String>,
        rows: usize,
    }

    impl std::fmt::Debug for Translator {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Translator")
                .field("pair", &self.pair)
                .finish_non_exhaustive()
        }
    }

    impl Translator {
        /// Loads the model for `pair` from `models` into CPU sessions, which give the same
        /// results on every run.
        pub fn open(models: &Path, pair: &str) -> Result<Translator, TranslateError> {
            let model = model_for(pair)?;
            if !models::is_installed(models, model) {
                return Err(ModelError::NotInstalled {
                    what: model.name.into(),
                }
                .into());
            }
            let read = |name: &str| {
                std::fs::read(models::file_path(models, model, name))
                    .map_err(|e| failed(format!("cannot read {}/{name}: {e}", model.name)))
            };
            let source = Unigram::parse(&read("source.spm")?)
                .map_err(|e| failed(format!("{}/source.spm: {e}", model.name)))?;
            let vocabulary: HashMap<String, i64> = serde_json::from_slice(&read("vocab.json")?)
                .map_err(|e| failed(format!("{}/vocab.json is damaged: {e}", model.name)))?;
            let size = vocabulary.values().copied().max().unwrap_or(-1) + 1;
            if size <= PAD {
                return Err(failed(format!(
                    "{}/vocab.json has {size} tokens, not {}",
                    model.name,
                    PAD + 1
                )));
            }
            let mut pieces = vec![String::new(); size as usize];
            for (piece, &id) in &vocabulary {
                if let Some(slot) = usize::try_from(id).ok().and_then(|i| pieces.get_mut(i)) {
                    *slot = piece.clone();
                }
            }
            let options = SessionOptions::new(models).on(Device::Cpu);
            let encoder =
                onnx::session(&models::file_path(models, model, "encoder.onnx"), &options)?;
            let decoder =
                onnx::session(&models::file_path(models, model, "decoder.onnx"), &options)?;
            let mut cache = Vec::new();
            for layer in 0..LAYERS {
                for part in ["decoder", "encoder"] {
                    for kind in ["key", "value"] {
                        cache.push(format!("past_key_values.{layer}.{part}.{kind}"));
                    }
                }
            }
            Ok(Translator {
                pair: model.languages.to_string(),
                encoder,
                decoder,
                source,
                vocabulary,
                pieces,
                cache,
                rows: 1,
            })
        }

        /// The same translator running `rows` segments at a time (at least 1): faster, but
        /// a text's translation can then depend on the texts translated with it, because
        /// the 8-bit models quantize each run as a whole. The same texts in the same order
        /// still give the same translations.
        pub fn with_batch(self, rows: usize) -> Translator {
            Translator {
                rows: rows.max(1),
                ..self
            }
        }

        /// The direction it translates, such as `en>hu`.
        pub fn pair(&self) -> &str {
            &self.pair
        }

        /// Translates `texts`, one translation per text, in order; an empty text stays
        /// empty.
        pub fn translate(&mut self, texts: &[String]) -> Result<Vec<String>, TranslateError> {
            // Every text's segments, then all segments sorted by length into batches (of one
            // by default).
            let mut jobs: Vec<(usize, Vec<i64>)> = Vec::new();
            for (index, text) in texts.iter().enumerate() {
                let pieces = self.source.encode(text);
                for segment in
                    segments(&pieces, &self.vocabulary, SEGMENT_TOKENS, MAX_SOURCE_TOKENS)
                {
                    jobs.push((index, segment));
                }
            }
            let mut order: Vec<usize> = (0..jobs.len()).collect();
            order.sort_by_key(|&j| jobs[j].1.len());
            let mut written: Vec<Vec<i64>> = vec![Vec::new(); jobs.len()];
            for batch in order.chunks(self.rows) {
                let sources: Vec<&[i64]> = batch.iter().map(|&j| jobs[j].1.as_slice()).collect();
                for (slot, tokens) in batch.iter().zip(self.batch(&sources)?) {
                    written[*slot] = tokens;
                }
            }
            let mut out: Vec<Vec<String>> = vec![Vec::new(); texts.len()];
            for ((index, _), tokens) in jobs.iter().zip(&written) {
                let text = detokenize(
                    tokens
                        .iter()
                        .filter(|&&t| t != END && t != PAD && t != UNKNOWN)
                        .filter_map(|&t| usize::try_from(t).ok().and_then(|t| self.pieces.get(t)))
                        .map(String::as_str),
                );
                if !text.is_empty() {
                    out[*index].push(text);
                }
            }
            Ok(out.into_iter().map(|parts| parts.join(" ")).collect())
        }

        /// Greedy decoding of a batch of source segments (each ending with the end token):
        /// the tokens written for each, without the start token, up to its end token.
        fn batch(&mut self, sources: &[&[i64]]) -> Result<Vec<Vec<i64>>, TranslateError> {
            let rows = sources.len();
            let width = sources.iter().map(|s| s.len()).max().unwrap_or(0);
            if rows == 0 || width == 0 {
                return Ok(vec![Vec::new(); rows]);
            }
            let mut ids = Vec::with_capacity(rows * width);
            let mut mask = Vec::with_capacity(rows * width);
            for source in sources {
                ids.extend_from_slice(source);
                ids.extend(std::iter::repeat_n(PAD, width - source.len()));
                mask.extend(std::iter::repeat_n(1i64, source.len()));
                mask.extend(std::iter::repeat_n(0i64, width - source.len()));
            }
            let mask = Tensor::from_array((vec![rows, width], mask)).map_err(failed)?;
            let hidden: DynValue = {
                let inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> = vec![
                    (
                        "input_ids".into(),
                        Tensor::from_array((vec![rows, width], ids))
                            .map_err(failed)?
                            .into(),
                    ),
                    ("attention_mask".into(), (&mask).into()),
                ];
                let mut outputs = self.encoder.run(inputs).map_err(failed)?;
                outputs
                    .remove("last_hidden_state")
                    .ok_or_else(|| failed("the encoder has no last_hidden_state"))?
            };
            let empty = || -> Result<DynValue, TranslateError> {
                Ok(
                    Tensor::from_array((vec![rows, HEADS, 0, HEAD], Vec::<f32>::new()))
                        .map_err(failed)?
                        .into_dyn(),
                )
            };
            let mut past: Vec<DynValue> = (0..self.cache.len())
                .map(|_| empty())
                .collect::<Result<_, _>>()?;
            let limit = sources
                .iter()
                .map(|s| max_new_tokens(s.len()))
                .max()
                .unwrap_or(1);
            let mut written: Vec<Vec<i64>> = vec![Vec::new(); rows];
            let mut done = vec![false; rows];
            let mut last = vec![PAD; rows];
            for step in 0..limit {
                let mut inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> = vec![
                    ("encoder_attention_mask".into(), (&mask).into()),
                    (
                        "input_ids".into(),
                        Tensor::from_array((vec![rows, 1], last.clone()))
                            .map_err(failed)?
                            .into(),
                    ),
                    ("encoder_hidden_states".into(), (&hidden).into()),
                    (
                        "use_cache_branch".into(),
                        Tensor::from_array((vec![1usize], vec![step > 0]))
                            .map_err(failed)?
                            .into(),
                    ),
                ];
                for (name, value) in self.cache.iter().zip(&past) {
                    inputs.push((Cow::Owned(name.clone()), value.into()));
                }
                let mut outputs = self.decoder.run(inputs).map_err(failed)?;
                {
                    let (shape, logits) = outputs["logits"]
                        .try_extract_tensor::<f32>()
                        .map_err(failed)?;
                    let vocabulary = shape.last().copied().unwrap_or(0) as usize;
                    if vocabulary <= PAD as usize || logits.len() < rows * vocabulary {
                        return Err(failed(format!("unexpected logits of shape {shape:?}")));
                    }
                    for row in 0..rows {
                        if done[row] {
                            last[row] = PAD;
                            continue;
                        }
                        // The padding token is never written.
                        let scores = &logits[row * vocabulary..row * vocabulary + PAD as usize];
                        let next = scores
                            .iter()
                            .enumerate()
                            .fold((0usize, f32::NEG_INFINITY), |best, (i, &s)| {
                                if s > best.1 { (i, s) } else { best }
                            })
                            .0 as i64;
                        last[row] = next;
                        if next == END {
                            done[row] = true;
                        } else {
                            written[row].push(next);
                        }
                    }
                }
                // Self-attention keys and values grow every step; cross-attention ones come
                // from the first step only (later steps return empty placeholders).
                for (name, slot) in self.cache.iter().zip(past.iter_mut()) {
                    if step == 0 || name.contains(".decoder.") {
                        let present = name.replace("past_key_values", "present");
                        *slot = outputs
                            .remove(&present)
                            .ok_or_else(|| failed(format!("the decoder has no {present}")))?;
                    }
                }
                if done.iter().all(|d| *d) {
                    break;
                }
            }
            Ok(written)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A protobuf field: number, wire type and payload.
    fn field(number: u64, wire: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![(number << 3 | wire) as u8];
        if wire == 2 {
            out.push(payload.len() as u8);
        }
        out.extend_from_slice(payload);
        out
    }

    fn piece(text: &str, score: f32, kind: u64) -> Vec<u8> {
        let mut body = field(1, 2, text.as_bytes());
        body.extend(field(2, 5, &score.to_le_bytes()));
        body.extend(field(3, 0, &[kind as u8]));
        field(1, 2, &body)
    }

    fn model() -> Unigram {
        let mut bytes = Vec::new();
        for (text, score) in [
            ("▁hello", -1.0),
            ("▁hell", -2.0),
            ("o", -2.0),
            ("▁", -3.0),
            ("▁wor", -3.0),
            ("ld", -2.5),
            ("▁world", -6.0),
            (".", -1.5),
        ] {
            bytes.extend(piece(text, score, 1));
        }
        bytes.extend(piece("<unk>", 0.0, 2));
        bytes.extend(piece("</s>", 0.0, 3));
        // A trainer spec (field 2) that is skipped.
        bytes.extend(field(2, 2, &[8, 1]));
        Unigram::parse(&bytes).unwrap()
    }

    #[test]
    fn sentencepiece_models_parse_and_segment_by_likelihood() {
        let unigram = model();
        assert_eq!(unigram.longest, 6);
        assert_eq!(unigram.unknown, -16.0);
        assert!(
            !unigram.scores.contains_key("<unk>"),
            "unknown and control pieces skipped"
        );
        // "▁wor" + "ld" (-5.5) beats "▁world" (-6).
        assert_eq!(
            unigram.encode("  hello\u{3000}world. "),
            ["▁hello", "▁wor", "ld", "."]
        );
        // A character no piece covers stands alone.
        assert_eq!(unigram.encode("hello€"), ["▁hello", "€"]);
        assert!(unigram.encode(" \n ").is_empty());
        assert!(Unigram::parse(&[]).is_err());
        assert!(Unigram::parse(&[0x0a, 0x05, 0x0a]).is_err(), "truncated");
        assert!(Unigram::parse(&[0x0f]).is_err(), "wire type 7");
        assert_eq!(normalize("ﬁne\u{0007}  tune\t"), "fine tune");
    }

    #[test]
    fn long_texts_become_segments_at_sentence_ends() {
        let vocabulary: HashMap<String, i64> = [("▁a", 5), ("▁b.", 6), ("▁c", 7), ("d", 8)]
            .into_iter()
            .map(|(p, i)| (p.to_string(), i))
            .collect();
        let pieces: Vec<String> = ["▁a", "▁b.", "▁c", "d", "▁a", "x", "y", "▁c"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            segments(&pieces, &vocabulary, 100, 100),
            [vec![5, 6, 7, 8, 5, UNKNOWN, 7, END]],
            "unknown runs are one token"
        );
        // Sentences packed up to 4 tokens; the second sentence (5 tokens) cut at a word.
        assert_eq!(
            segments(&pieces, &vocabulary, 4, 4),
            [vec![5, 6, 7, 8, END], vec![5, UNKNOWN, 7, END]]
        );
        assert_eq!(
            segments(&pieces, &vocabulary, 3, 4),
            [vec![5, 6, END], vec![7, 8, END], vec![5, UNKNOWN, 7, END]]
        );
        // A sentence end only where a word follows.
        let dotted: Vec<String> = ["▁b.", "d", "▁a"].map(String::from).to_vec();
        assert_eq!(segments(&dotted, &vocabulary, 1, 10), [vec![6, 8, 5, END]]);
        assert!(segments(&[], &vocabulary, 4, 4).is_empty());
        assert_eq!(detokenize(["▁Thank", "▁you", "."]), "Thank you.");
        assert_eq!(max_new_tokens(5), 20);
        assert_eq!(max_new_tokens(400), 511);
    }

    #[test]
    fn pairs_come_from_the_translation_models() {
        assert_eq!(pairs(), ["hu>en", "en>hu"]);
        assert_eq!(model_for("en>hu").unwrap().name, "opus-mt-en-hu");
        assert_eq!(model_for(" HU>EN ").unwrap().name, "opus-mt-hu-en");
        let error = model_for("en>de").unwrap_err();
        assert_eq!(error.code(), "fosscut.translate.unsupported_pair");
        assert_eq!(
            error.to_string(),
            "there is no translation model for en>de; the pairs are hu>en, en>hu"
        );
        assert_eq!(
            TranslateError::from(ModelError::NotInstalled {
                what: "opus-mt-en-hu".into()
            })
            .code(),
            "fosscut.model.not_installed"
        );
        assert_eq!(
            TranslateError::Failed("x".into()).code(),
            "fosscut.translate.failed"
        );
        assert_eq!(
            TranslateError::Unavailable.code(),
            "fosscut.translate.unavailable"
        );
        assert_eq!(
            TranslateError::Onnx(OnnxError::LoadFailed {
                model: "m".into(),
                message: "x".into()
            })
            .code(),
            "fosscut.model.load_failed"
        );
        let missing =
            std::env::temp_dir().join(format!("fosscut-translate-{}", std::process::id()));
        let error = translate(&missing, "en>hu", &["Hi".into()]).unwrap_err();
        let expected = if cfg!(feature = "onnx") {
            "fosscut.model.not_installed"
        } else {
            "fosscut.translate.unavailable"
        };
        assert_eq!(error.code(), expected, "{error}");
        assert_eq!(
            translate(&missing, "xx>yy", &[]).unwrap_err().code(),
            "fosscut.translate.unsupported_pair"
        );
    }
}
