//! Caption commands (ADR-0021): page timed words into caption clips, edit a page, and give
//! a caption track its look from a preset and a style patch.

use std::collections::BTreeSet;

use fosscut_model::{
    CaptionContent, CaptionLine, CaptionStyle, CaptionWord, Clip, ClipContent, Id, IdKind, Project,
    TrackKind, Transform,
};
use fosscut_time::{Flicks, Rate, TimeInput, TimeReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::util::{
    ensure_free, ensure_unlocked, insert_sorted, locate_clip, timeline_id, timeline_mut,
    track_index,
};
use crate::presets::caption_preset;
use crate::{ApiError, Command, Context, codes};

/// A word to caption, in timeline time.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WordInput {
    /// The word as shown, with attached punctuation.
    pub text: String,
    /// When it is spoken on the timeline; kept exact, not snapped to frames.
    pub start: TimeInput,
    /// When it ends on the timeline.
    pub end: TimeInput,
    /// A keyword the `word.emphasis` animator styles.
    #[serde(default)]
    pub emphasis: bool,
    /// Start a new page at this word, whatever the paging limits say.
    #[serde(default)]
    pub new_page: bool,
    /// Start a new line at this word.
    #[serde(default)]
    pub new_line: bool,
}

/// How words are grouped into pages and lines (ADR-0021 item 6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Paging {
    /// Most words on a page.
    pub max_words: u32,
    /// Most lines on a page.
    pub max_lines: u32,
    /// Most characters on a line; by default as many as fit the safe width in the track's
    /// style, at most 18 on canvases taller than wide and 32 otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars_per_line: Option<u32>,
    /// A pause at least this long, in milliseconds, starts a new page.
    pub max_gap_ms: u32,
    /// How long a page stays after its last word, in milliseconds, if the next page
    /// does not start first.
    pub linger_ms: u32,
}

impl Default for Paging {
    fn default() -> Paging {
        Paging {
            max_words: 4,
            max_lines: 2,
            max_chars_per_line: None,
            max_gap_ms: 600,
            linger_ms: 300,
        }
    }
}

/// The default line length: as many characters as fit the safe width in `style`, from an
/// average glyph width (wider for capitals and the black weight), but no more than is
/// comfortable to read: 18 on canvases taller than wide, 32 otherwise.
fn default_chars(project: &Project, timeline: &Id, style: &CaptionStyle) -> usize {
    let Some(format) = project.timelines.get(timeline).map(|t| &t.format) else {
        return 32;
    };
    let (width, height) = (f64::from(format.width), f64::from(format.height));
    let [left, _, right, _] = fosscut_model::safe_margins(format.width, format.height);
    let average = if style.uppercase || style.weight == fosscut_model::FontWeight::Black {
        0.68
    } else {
        0.55
    };
    let fits = width * (1.0 - left - right) / (style.size.max(0.001) * height * average);
    let comfortable = if height > width { 18 } else { 32 };
    (fits.floor().max(1.0) as usize).clamp(8, comfortable)
}

/// A word with exact timeline times.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TimedWord {
    pub text: String,
    pub start: Flicks,
    pub end: Flicks,
    pub emphasis: bool,
    pub new_page: bool,
    pub new_line: bool,
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::CAPTIONS_INVALID_WORDS, message)
}

/// Resolves words to exact timeline times and checks that they are non-empty and in order.
fn timed(words: &[WordInput], rate: Rate) -> Result<Vec<TimedWord>, ApiError> {
    if words.is_empty() {
        return Err(invalid("there are no words to caption"));
    }
    let mut out: Vec<TimedWord> = Vec::with_capacity(words.len());
    for (index, word) in words.iter().enumerate() {
        let start = word.start.resolve(rate)?;
        let end = word.end.resolve(rate)?;
        let text = word.text.trim().to_string();
        let problem = if text.is_empty() {
            Some("has no text")
        } else if start.is_negative() || end < start {
            Some("must start at zero or later and end after it starts")
        } else if out.last().is_some_and(|previous| start < previous.end) {
            Some("starts before the previous word ends")
        } else {
            None
        };
        if let Some(problem) = problem {
            return Err(
                invalid(format!("word {index} ({:?}) {problem}", word.text)).with_details(json!({
                    "word": index,
                    "start": TimeReport::new(start, rate),
                    "end": TimeReport::new(end, rate),
                })),
            );
        }
        out.push(TimedWord {
            text,
            start,
            end,
            emphasis: word.emphasis,
            new_page: word.new_page,
            new_line: word.new_line,
        });
    }
    Ok(out)
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

/// Whether a word ends a sentence, ignoring closing quotes and brackets.
fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', ')', ']', '\u{201d}', '\u{2019}'])
        .ends_with(['.', '!', '?', '\u{2026}'])
}

/// Groups words into pages of lines (ADR-0021 item 6). Every word lands on exactly one
/// page, in order; a word longer than a line gets a line of its own.
pub(crate) fn page_words(
    words: &[TimedWord],
    paging: &Paging,
    max_chars: usize,
) -> Vec<Vec<Vec<TimedWord>>> {
    let max_words = paging.max_words.max(1) as usize;
    let max_lines = paging.max_lines.max(1) as usize;
    let gap = Flicks::from_millis(i64::from(paging.max_gap_ms));
    let mut pages = Vec::new();
    let mut lines: Vec<Vec<TimedWord>> = Vec::new();
    let mut count = 0;
    let mut previous: Option<&TimedWord> = None;
    for word in words {
        let new_page = word.new_page
            || previous.is_some_and(|p| {
                ends_sentence(&p.text) || word.start - p.end >= gap || count >= max_words
            });
        if new_page && !lines.is_empty() {
            pages.push(std::mem::take(&mut lines));
            count = 0;
        }
        let fits = !word.new_line
            && lines.last().is_some_and(|line| {
                let used: usize = line.iter().map(|w| chars(&w.text) + 1).sum();
                used + chars(&word.text) <= max_chars
            });
        if !fits {
            if lines.len() >= max_lines {
                pages.push(std::mem::take(&mut lines));
                count = 0;
            }
            lines.push(Vec::new());
        }
        if let Some(line) = lines.last_mut() {
            line.push(word.clone());
        }
        count += 1;
        previous = Some(word);
    }
    if !lines.is_empty() {
        pages.push(lines);
    }
    pages
}

/// Wraps words into lines of at most `max_chars` characters.
fn wrap(words: Vec<CaptionWord>, max_chars: usize) -> Vec<CaptionLine> {
    let mut lines: Vec<CaptionLine> = Vec::new();
    let mut used = 0;
    for word in words {
        let length = chars(&word.text);
        match lines.last_mut() {
            Some(line) if used + 1 + length <= max_chars => {
                used += 1 + length;
                line.words.push(word);
            }
            _ => {
                used = length;
                lines.push(CaptionLine { words: vec![word] });
            }
        }
    }
    lines
}

fn floor_frame(time: Flicks, rate: Rate) -> Flicks {
    rate.frames_to_flicks(rate.frame_at(time))
}

fn ceil_frame(time: Flicks, rate: Rate) -> Flicks {
    let floor = floor_frame(time, rate);
    if floor == time {
        floor
    } else {
        rate.frames_to_flicks(rate.frame_at(time) + 1)
    }
}

/// A page laid out on the timeline: its frame-aligned range and its words relative to it.
pub(crate) struct LaidOut {
    pub start: Flicks,
    pub duration: Flicks,
    pub content: CaptionContent,
}

/// Places pages one after another on frames: a page starts at the frame of its first word
/// and ends `linger` after its last word, but never after the next page starts, nor lingers
/// past `until` (the end of the timeline's other content), so captions do not lengthen it. Words are
/// stored relative to their page and kept inside it.
pub(crate) fn lay_out(
    pages: &[Vec<Vec<TimedWord>>],
    linger: Flicks,
    until: Option<Flicks>,
    rate: Rate,
) -> Vec<LaidOut> {
    let frame = rate.frame_duration();
    let mut out: Vec<LaidOut> = Vec::with_capacity(pages.len());
    let mut previous_end = Flicks::ZERO;
    for (index, page) in pages.iter().enumerate() {
        let first = page.iter().flatten().next();
        let last = page.iter().flatten().last();
        let (Some(first), Some(last)) = (first, last) else {
            continue;
        };
        // A page starts on the frame that holds its first word, so every word keeps its
        // exact time inside the page.
        let start = floor_frame(first.start, rate).max(previous_end);
        let mut end = ceil_frame(last.end + linger, rate);
        if let Some(until) = until {
            end = end.min(until.max(ceil_frame(last.end, rate)));
        }
        if let Some(next) = pages.get(index + 1).and_then(|p| p.iter().flatten().next()) {
            // The next page starts on the frame of its first word. Word starts matter most
            // (highlights begin there), so when words meet inside a frame the end of this
            // page's last word is cut to the frame instead.
            end = end.min(floor_frame(next.start, rate));
        }
        let end = end.max(start + frame);
        let duration = end - start;
        let relative = |time: Flicks| (time - start).max(Flicks::ZERO).min(duration);
        let lines = page
            .iter()
            .map(|line| CaptionLine {
                words: line
                    .iter()
                    .map(|word| CaptionWord {
                        text: word.text.clone(),
                        start: relative(word.start),
                        end: relative(word.end),
                        emphasis: word.emphasis,
                    })
                    .collect(),
            })
            .collect();
        out.push(LaidOut {
            start,
            duration,
            content: CaptionContent { lines },
        });
        previous_end = end;
    }
    out
}

/// Splits a caption page at `offset` from its start: each word goes to the side that holds
/// most of it. `None` when one side would have no words.
pub(crate) fn split_caption(
    caption: &CaptionContent,
    offset: Flicks,
) -> Option<(CaptionContent, CaptionContent)> {
    let mut left = Vec::new();
    let mut right = Vec::new();
    for line in &caption.lines {
        let (before, after): (Vec<&CaptionWord>, Vec<&CaptionWord>) =
            line.words.iter().partition(|w| {
                i128::from(w.start.get()) + i128::from(w.end.get()) <= 2 * i128::from(offset.get())
            });
        if !before.is_empty() {
            left.push(CaptionLine {
                words: before
                    .into_iter()
                    .map(|w| CaptionWord {
                        end: w.end.min(offset),
                        ..w.clone()
                    })
                    .collect(),
            });
        }
        if !after.is_empty() {
            right.push(CaptionLine {
                words: after
                    .into_iter()
                    .map(|w| CaptionWord {
                        start: (w.start - offset).max(Flicks::ZERO),
                        end: w.end - offset,
                        ..w.clone()
                    })
                    .collect(),
            });
        }
    }
    (!left.is_empty() && !right.is_empty()).then_some((
        CaptionContent { lines: left },
        CaptionContent { lines: right },
    ))
}

/// The part of a caption page between `from` and `to` (from its start), as a page of its
/// own starting at `from`: words whose middle lies inside are kept, cut to the window and
/// shifted. `None` when no word is left.
pub(crate) fn window_caption(
    caption: &CaptionContent,
    from: Flicks,
    to: Flicks,
) -> Option<CaptionContent> {
    let inside = |w: &CaptionWord| {
        let middle = i128::from(w.start.get()) + i128::from(w.end.get());
        middle >= 2 * i128::from(from.get()) && middle < 2 * i128::from(to.get())
    };
    let lines: Vec<CaptionLine> = caption
        .lines
        .iter()
        .map(|line| CaptionLine {
            words: line
                .words
                .iter()
                .filter(|w| inside(w))
                .map(|w| CaptionWord {
                    start: w.start.max(from).min(to) - from,
                    end: w.end.max(from).min(to) - from,
                    ..w.clone()
                })
                .collect(),
        })
        .filter(|line| !line.words.is_empty())
        .collect();
    (!lines.is_empty()).then_some(CaptionContent { lines })
}

/// Applies a JSON merge patch (RFC 7396) to `target`.
fn merge(target: &mut Value, patch: &Value) {
    match patch {
        Value::Object(fields) => {
            if !target.is_object() {
                *target = json!({});
            }
            if let Value::Object(object) = target {
                for (key, value) in fields {
                    if value.is_null() {
                        object.remove(key);
                    } else {
                        merge(object.entry(key.clone()).or_insert(Value::Null), value);
                    }
                }
            }
        }
        other => *target = other.clone(),
    }
}

/// `base`, replaced by a preset when one is named, then changed by a merge patch.
fn styled(
    base: CaptionStyle,
    preset: Option<&str>,
    patch: Option<&Value>,
) -> Result<CaptionStyle, ApiError> {
    let start = match preset {
        Some(name) => caption_preset(name)?,
        None => base,
    };
    let Some(patch) = patch else {
        return Ok(start);
    };
    let mut value = serde_json::to_value(&start).unwrap_or_else(|_| json!({}));
    merge(&mut value, patch);
    serde_json::from_value(value).map_err(|e| {
        ApiError::new(codes::INVALID_ARGUMENTS, format!("caption style: {e}")).with_hint(
            "Style fields: font, weight, italic, size, color, align, uppercase, stroke, box, placement, animators; capabilities lists the animators.",
        )
    })
}

/// Stores a style sparsely: defaults are omitted.
fn store_style(style: CaptionStyle) -> Option<CaptionStyle> {
    (style != CaptionStyle::default()).then_some(style)
}

/// A caption page as commands report it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CaptionPage {
    /// Clip id.
    pub clip: Id,
    /// First frame on the timeline.
    pub start: TimeReport,
    /// Position just after the last frame.
    pub end: TimeReport,
    /// The page's text; lines are separated by `\n`.
    pub text: String,
}

impl CaptionPage {
    fn of(clip: &Clip, rate: Rate) -> CaptionPage {
        CaptionPage {
            clip: clip.id.clone(),
            start: TimeReport::new(clip.start, rate),
            end: TimeReport::new(clip.end(), rate),
            text: match &clip.content {
                ClipContent::Caption(caption) => caption.text(),
                _ => String::new(),
            },
        }
    }
}

/// Pages timed words into caption clips on a caption track.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionsAdd {
    /// Words in timeline order with timeline times, for example
    /// `[{"text": "Hello", "start": {"seconds": "1.2"}, "end": {"seconds": "1.5"}}]`.
    pub words: Vec<WordInput>,
    /// Target caption track; the first caption track (created on top if needed) when
    /// omitted.
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// How words are grouped into pages and lines.
    #[serde(default)]
    pub paging: Paging,
    /// A built-in look for the track (`capabilities` lists them), applied before `style`.
    #[serde(default)]
    pub preset: Option<String>,
    /// Changes to the track's look as a JSON merge patch, for example
    /// `{"size": 0.07, "placement": "middle"}`.
    #[serde(default)]
    pub style: Option<Value>,
    /// Remove the track's existing captions first; destructive.
    #[serde(default)]
    pub replace: bool,
}

/// Result of `captions.add`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CaptionsAdded {
    /// The caption track.
    pub track: Id,
    /// The timeline.
    pub timeline: Id,
    /// Number of words captioned.
    pub words: usize,
    /// The pages, in order.
    pub pages: Vec<CaptionPage>,
    /// Captions removed first because of `replace`.
    pub removed: usize,
}

impl Command for CaptionsAdd {
    const NAME: &'static str = "captions.add";
    const SUMMARY: &'static str = "Caption timed words: page them into caption clips (a few words per page, split at sentence ends and pauses) on a caption track, optionally styled with a preset. Never overwrites unless replace is set.";
    type Output = CaptionsAdded;

    fn apply(self, ctx: &mut Context<'_>) -> Result<CaptionsAdded, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let words = timed(&self.words, rate)?;
        // Captions follow the rest of the edit; they never make the timeline longer.
        let until = ctx.project.timelines[&timeline]
            .tracks
            .iter()
            .filter(|t| t.kind != TrackKind::Caption)
            .map(fosscut_model::Track::end)
            .max()
            .filter(|end| end.is_positive());
        let track_id = match &self.track {
            Some(track) => track.clone(),
            None => match ctx.project.timelines[&timeline]
                .tracks
                .iter()
                .find(|t| t.kind == TrackKind::Caption)
            {
                Some(track) => track.id.clone(),
                None => {
                    let id = ctx.project.allocate_id(IdKind::Track);
                    timeline_mut(ctx.project, &timeline)?
                        .tracks
                        .push(fosscut_model::Track {
                            id: id.clone(),
                            kind: TrackKind::Caption,
                            name: Some("Captions".into()),
                            muted: false,
                            hidden: false,
                            locked: false,
                            caption_style: None,
                            clips: Vec::new(),
                        });
                    id
                }
            },
        };
        let index = track_index(&ctx.project.timelines[&timeline], &track_id)?;
        let track = &ctx.project.timelines[&timeline].tracks[index];
        ensure_unlocked(track)?;
        if track.kind != TrackKind::Caption {
            return Err(ApiError::new(
                codes::TRACK_KIND_MISMATCH,
                format!(
                    "captions need a caption track, but {} is a {:?} track",
                    track.id, track.kind
                ),
            ));
        }
        let existing = track.clips.len();
        let base = track.caption_style.clone().unwrap_or_default();
        let style = if self.preset.is_some() || self.style.is_some() {
            Some(styled(
                base.clone(),
                self.preset.as_deref(),
                self.style.as_ref(),
            )?)
        } else {
            None
        };
        if self.replace && existing > 0 {
            ctx.require_destructive(
                "captions.add with replace",
                json!({ "track": track_id, "clips": existing }),
            )?;
        }
        let max_chars = self.paging.max_chars_per_line.map_or_else(
            || default_chars(ctx.project, &timeline, style.as_ref().unwrap_or(&base)),
            |n| n.max(1) as usize,
        );
        let pages = lay_out(
            &page_words(&words, &self.paging, max_chars),
            Flicks::from_millis(i64::from(self.paging.linger_ms)),
            until,
            rate,
        );
        let ids: Vec<Id> = pages
            .iter()
            .map(|_| ctx.project.allocate_id(IdKind::Clip))
            .collect();
        let track = &mut timeline_mut(ctx.project, &timeline)?.tracks[index];
        let removed = if self.replace {
            std::mem::take(&mut track.clips).len()
        } else {
            0
        };
        if let Some(style) = style {
            track.caption_style = store_style(style);
        }
        let mut reports = Vec::with_capacity(pages.len());
        for (page, id) in pages.into_iter().zip(ids) {
            ensure_free(track, page.start, page.start + page.duration, None, rate)?;
            let clip = Clip {
                id,
                start: page.start,
                duration: page.duration,
                content: ClipContent::Caption(page.content),
                transform: Transform::default(),
                opacity: 1.0,
                volume: 1.0,
            };
            reports.push(CaptionPage::of(&clip, rate));
            insert_sorted(track, clip);
        }
        Ok(CaptionsAdded {
            track: track_id,
            timeline,
            words: words.len(),
            pages: reports,
            removed,
        })
    }
}

/// Changes the words of one caption page.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionsEdit {
    /// The caption clip.
    pub clip: Id,
    /// New timed words in timeline time, inside the clip; replaces the page's words.
    #[serde(default)]
    pub words: Option<Vec<WordInput>>,
    /// New wording for the page, `\n` between lines; the page's time is shared across the
    /// new words by their length. Use instead of `words`.
    #[serde(default)]
    pub text: Option<String>,
    /// Indexes of the words (counting from 0 over the whole page) to emphasise; every
    /// other word loses its emphasis.
    #[serde(default)]
    pub emphasis: Option<Vec<usize>>,
}

impl Command for CaptionsEdit {
    const NAME: &'static str = "captions.edit";
    const SUMMARY: &'static str = "Change one caption page: new timed words, new wording that keeps the page's timing, or which words are emphasised.";
    type Output = CaptionPage;

    fn apply(self, ctx: &mut Context<'_>) -> Result<CaptionPage, ApiError> {
        if self.words.is_some() && self.text.is_some() {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "captions.edit: give words or text, not both",
            ));
        }
        if self.words.is_none() && self.text.is_none() && self.emphasis.is_none() {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "captions.edit: give words, text or emphasis",
            ));
        }
        let location = locate_clip(ctx.project, &self.clip)?;
        let style = ctx.project.timelines[&location.timeline].tracks[location.track]
            .caption_style
            .clone()
            .unwrap_or_default();
        let max_chars = default_chars(ctx.project, &location.timeline, &style);
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let rate = timeline.format.rate;
        let track = &mut timeline.tracks[location.track];
        ensure_unlocked(track)?;
        let clip = &mut track.clips[location.clip];
        let (clip_start, duration) = (clip.start, clip.duration);
        let ClipContent::Caption(caption) = &mut clip.content else {
            return Err(ApiError::new(
                codes::TRACK_KIND_MISMATCH,
                format!("{} is not a caption clip", self.clip),
            ));
        };
        if let Some(words) = &self.words {
            let words = timed(words, rate)?;
            if words
                .iter()
                .any(|w| w.start < clip_start || w.end > clip_start + duration)
            {
                return Err(invalid(format!(
                    "words must lie inside {} ({} to {})",
                    self.clip,
                    TimeReport::new(clip_start, rate).timecode,
                    TimeReport::new(clip_start + duration, rate).timecode
                )));
            }
            let relative = words
                .into_iter()
                .map(|w| CaptionWord {
                    text: w.text,
                    start: w.start - clip_start,
                    end: w.end - clip_start,
                    emphasis: w.emphasis,
                })
                .collect();
            caption.lines = wrap(relative, max_chars);
        }
        if let Some(text) = &self.text {
            caption.lines = reword(caption, text, max_chars)?;
        }
        if let Some(emphasis) = &self.emphasis {
            let count = caption.words().count();
            if let Some(bad) = emphasis.iter().find(|&&i| i >= count) {
                return Err(invalid(format!(
                    "word {bad} does not exist; the page has {count} words"
                )));
            }
            let marked: BTreeSet<usize> = emphasis.iter().copied().collect();
            let mut index = 0;
            for line in &mut caption.lines {
                for word in &mut line.words {
                    word.emphasis = marked.contains(&index);
                    index += 1;
                }
            }
        }
        Ok(CaptionPage::of(clip, rate))
    }
}

/// New wording over the span of the page's current words, shared by character count.
fn reword(
    caption: &CaptionContent,
    text: &str,
    max_chars: usize,
) -> Result<Vec<CaptionLine>, ApiError> {
    let span_start = caption.words().next().map_or(Flicks::ZERO, |w| w.start);
    let span_end = caption.words().last().map_or(Flicks::ZERO, |w| w.end);
    let explicit: Vec<Vec<&str>> = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .filter(|words| !words.is_empty())
        .collect();
    let total: usize = explicit.iter().flatten().map(|w| chars(w)).sum();
    if total == 0 {
        return Err(invalid("the new text has no words"));
    }
    let span = i128::from(span_end.get() - span_start.get());
    let at = |count: usize| {
        Flicks::new(span_start.get() + (span * count as i128 / total as i128) as i64)
    };
    let mut before = 0;
    let mut lines = Vec::new();
    let several = explicit.len() > 1;
    let mut flat = Vec::new();
    for line in explicit {
        let mut words = Vec::new();
        for word in line {
            let length = chars(word);
            words.push(CaptionWord {
                text: word.to_string(),
                start: at(before),
                end: at(before + length),
                emphasis: false,
            });
            before += length;
        }
        if several {
            lines.push(CaptionLine { words });
        } else {
            flat = words;
        }
    }
    Ok(if several {
        lines
    } else {
        wrap(flat, max_chars)
    })
}

/// Gives a caption track its look.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionsStyle {
    /// The caption track; the timeline's first caption track when omitted.
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// A built-in look (`capabilities` lists them); replaces the current look before
    /// `style` is applied.
    #[serde(default)]
    pub preset: Option<String>,
    /// Changes as a JSON merge patch over the preset or the current look, for example
    /// `{"size": 0.07, "stroke": {"enabled": false}}`; `animators` replaces the whole list.
    #[serde(default)]
    pub style: Option<Value>,
}

/// Result of `captions.style`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CaptionsStyled {
    /// The caption track.
    pub track: Id,
    /// Its look now; fields at their defaults are omitted, as in the project.
    pub style: CaptionStyle,
}

impl Command for CaptionsStyle {
    const NAME: &'static str = "captions.style";
    const SUMMARY: &'static str = "Set how a caption track's captions look and move: a built-in preset, a style patch, or both.";
    type Output = CaptionsStyled;

    fn apply(self, ctx: &mut Context<'_>) -> Result<CaptionsStyled, ApiError> {
        if self.preset.is_none() && self.style.is_none() {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "captions.style: give a preset, a style, or both",
            ));
        }
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let doc = timeline_mut(ctx.project, &timeline)?;
        let index = match &self.track {
            Some(track) => track_index(doc, track)?,
            None => doc
                .tracks
                .iter()
                .position(|t| t.kind == TrackKind::Caption)
                .ok_or_else(|| {
                    ApiError::new(
                        codes::TRACK_NOT_FOUND,
                        format!("timeline {timeline} has no caption track"),
                    )
                    .with_hint("Add captions first with captions.add or the captions tool.")
                })?,
        };
        let track = &mut doc.tracks[index];
        ensure_unlocked(track)?;
        if track.kind != TrackKind::Caption {
            return Err(ApiError::new(
                codes::TRACK_KIND_MISMATCH,
                format!(
                    "{} is a {:?} track, not a caption track",
                    track.id, track.kind
                ),
            ));
        }
        let base = track.caption_style.clone().unwrap_or_default();
        let style = styled(base, self.preset.as_deref(), self.style.as_ref())?;
        track.caption_style = store_style(style.clone());
        Ok(CaptionsStyled {
            track: track.id.clone(),
            style,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start_ms: i64, end_ms: i64) -> TimedWord {
        TimedWord {
            text: text.into(),
            start: Flicks::from_millis(start_ms),
            end: Flicks::from_millis(end_ms),
            emphasis: false,
            new_page: false,
            new_line: false,
        }
    }

    fn texts(pages: &[Vec<Vec<TimedWord>>]) -> Vec<Vec<String>> {
        pages
            .iter()
            .map(|page| {
                page.iter()
                    .map(|line| {
                        line.iter()
                            .map(|w| w.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn pages_break_at_sentences_pauses_and_limits() {
        let words = [
            word("So", 0, 200),
            word("this", 200, 400),
            word("works.", 400, 800),
            word("Then", 900, 1100),
            word("we", 1100, 1200),
            word("wait", 1200, 1500),
            word("here", 2300, 2600),
            word("a", 2600, 2700),
            word("b", 2700, 2800),
            word("c", 2800, 2900),
            word("d", 2900, 3000),
            word("e", 3000, 3100),
        ];
        let pages = page_words(&words, &Paging::default(), 18);
        assert_eq!(
            texts(&pages),
            [
                vec!["So this works."],
                vec!["Then we wait"],
                vec!["here a b c"],
                vec!["d e"],
            ]
        );
        // Short lines wrap into several lines, up to max_lines per page.
        let long = [
            word("extraordinary", 0, 100),
            word("circumstances", 100, 200),
            word("demand", 200, 300),
        ];
        let narrow = Paging {
            max_lines: 2,
            ..Paging::default()
        };
        assert_eq!(
            texts(&page_words(&long, &narrow, 14)),
            [vec!["extraordinary", "circumstances"], vec!["demand"]]
        );
        assert!(ends_sentence("done?\u{201d}") && !ends_sentence("Dr"));

        // Explicit breaks win over the limits.
        let mut forced = [
            word("one", 0, 100),
            word("two", 100, 200),
            word("three", 200, 300),
        ];
        forced[1].new_line = true;
        forced[2].new_page = true;
        assert_eq!(
            texts(&page_words(&forced, &Paging::default(), 40)),
            [vec!["one", "two"], vec!["three"]]
        );
    }

    #[test]
    fn pages_sit_on_frames_without_overlapping() {
        let rate = Rate::FPS_30;
        let words = [
            word("One.", 10, 300),
            word("Two.", 310, 600),
            word("Three", 2000, 2400),
        ];
        let laid = lay_out(
            &page_words(&words, &Paging::default(), 18),
            Flicks::from_millis(300),
            None,
            rate,
        );
        assert_eq!(laid.len(), 3);
        for pair in laid.windows(2) {
            assert!(pair[0].start + pair[0].duration <= pair[1].start);
        }
        for page in &laid {
            assert!(rate.is_on_frame(page.start) && rate.is_on_frame(page.duration));
            for w in page.content.words() {
                assert!(Flicks::ZERO <= w.start && w.start <= w.end && w.end <= page.duration);
            }
        }
        // The first page ends where the second starts; the last lingers 300 ms.
        assert_eq!(laid[0].start + laid[0].duration, laid[1].start);
        assert_eq!(
            laid[2].start + laid[2].duration,
            ceil_frame(Flicks::from_millis(2700), rate)
        );
        // Nothing lingers past the end of the rest of the timeline (here 2.5 s).
        let capped = lay_out(
            &page_words(&words, &Paging::default(), 18),
            Flicks::from_millis(300),
            Some(Flicks::from_millis(2500)),
            rate,
        );
        assert_eq!(
            capped[2].start + capped[2].duration,
            Flicks::from_millis(2500)
        );
        // Unless a word itself is spoken later.
        let late = lay_out(
            &page_words(&words, &Paging::default(), 18),
            Flicks::from_millis(300),
            Some(Flicks::from_millis(1000)),
            rate,
        );
        assert_eq!(
            late[2].start + late[2].duration,
            ceil_frame(Flicks::from_millis(2400), rate)
        );
        // Words keep exact times relative to their page.
        assert_eq!(
            laid[2].content.lines[0].words[0].start,
            Flicks::from_millis(2000) - laid[2].start
        );
    }

    #[test]
    fn splits_keep_every_word_on_the_side_that_holds_most_of_it() {
        let ms = Flicks::from_millis;
        let caption = CaptionContent {
            lines: vec![CaptionLine {
                words: vec![
                    CaptionWord {
                        text: "left".into(),
                        start: ms(0),
                        end: ms(400),
                        emphasis: true,
                    },
                    CaptionWord {
                        text: "middle".into(),
                        start: ms(400),
                        end: ms(1000),
                        emphasis: false,
                    },
                ],
            }],
        };
        let (left, right) = split_caption(&caption, ms(600)).unwrap();
        assert_eq!(left.text(), "left");
        assert_eq!(right.text(), "middle");
        assert!(left.lines[0].words[0].emphasis);
        assert_eq!(right.lines[0].words[0].start, Flicks::ZERO);
        assert_eq!(right.lines[0].words[0].end, ms(400));
        assert!(split_caption(&caption, ms(100)).is_none(), "no words left");
    }

    #[test]
    fn merge_patches_replace_lists_and_remove_nulls() {
        let mut value = json!({ "size": 0.05, "stroke": { "enabled": true, "width": 0.1 }, "animators": [1, 2] });
        merge(
            &mut value,
            &json!({ "stroke": { "width": null, "color": "#FF0000" }, "animators": [3], "uppercase": true }),
        );
        assert_eq!(
            value,
            json!({ "size": 0.05, "stroke": { "enabled": true, "color": "#FF0000" }, "animators": [3], "uppercase": true })
        );
        merge(&mut value, &json!(7));
        assert_eq!(value, json!(7));
    }
}
