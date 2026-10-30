//! Text clips (ADR-0027): add and restyle titles, save styles as project presets, and add
//! built-in title templates. Styles change through JSON merge patches over a preset or the
//! current style, and every style is checked (ranges, spans, known fonts) before it lands.

use fosscut_model::{
    Clip, ClipContent, Id, IdKind, Project, RevealMotion, RevealUnit, TextContent, TextPreset,
    TextReveal, TextSpan, TextStyle, TrackKind, Transform, text_issues, text_style_issues,
};
use fosscut_time::{Flicks, Rate, TimeInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ClipPlacement;
use super::caption::merge;
use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, find_or_create_track,
    insert_sorted, locate_clip, resolve, timeline_id, timeline_mut, track_index,
};
use crate::presets::{LineLook, text_style_preset, text_template};
use crate::{ApiError, Command, Context, codes};

const STYLE_FIELDS: &str = "Style fields: font, size, color, bold, italic, underline, case (as_written, upper, lower, title), align, letter_spacing, line_height, stroke {enabled, color, width}, box {enabled, color, padding, radius}, shadow {enabled, color, offset, blur}, glow {enabled, color, radius}; lengths are in em. capabilities lists the presets and fonts under text.";

/// How a text appears, as commands take it.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevealInput {
    /// What appears at a time: `letter` (a typewriter with `appear`), `word` or `line`.
    pub unit: RevealUnit,
    /// How each one appears: `appear` (at once), `fade` or `rise`; `appear` when omitted.
    #[serde(default)]
    pub motion: RevealMotion,
    /// Time from the clip's start until the whole text shows, with units, such as
    /// `{"seconds": 1}`.
    pub duration: TimeInput,
    /// Also leave the same way before the clip ends.
    #[serde(default)]
    pub exit: bool,
}

impl RevealInput {
    fn reveal(&self, rate: Rate) -> Result<TextReveal, ApiError> {
        let duration = resolve(&self.duration, rate)?;
        if !duration.is_positive() {
            return Err(invalid("a reveal needs a positive duration"));
        }
        Ok(TextReveal {
            unit: self.unit,
            motion: self.motion,
            duration,
            exit: self.exit,
        })
    }
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::TEXT_INVALID, message).with_hint(STYLE_FIELDS)
}

/// A style from a preset (project first, then built-in) or `base`, changed by `patch`.
fn styled(
    project: &Project,
    base: TextStyle,
    preset: Option<&str>,
    patch: Option<&Value>,
) -> Result<TextStyle, ApiError> {
    let start = match preset {
        Some(name) => match project
            .manifest
            .text_presets
            .iter()
            .find(|p| p.name == name)
        {
            Some(saved) => saved.style.clone(),
            None => text_style_preset(name)?,
        },
        None => base,
    };
    let Some(patch) = patch else {
        return Ok(start);
    };
    let mut value = serde_json::to_value(&start).unwrap_or_else(|_| json!({}));
    merge(&mut value, patch);
    serde_json::from_value(value).map_err(|e| invalid(format!("text style: {e}")))
}

/// Refuses a text that breaks the rules of ADR-0027, with the first problem as message.
fn checked(project: &Project, command: &str, content: &TextContent) -> Result<(), ApiError> {
    let issues = text_issues(project, content);
    let Some(first) = issues.first() else {
        return Ok(());
    };
    let code = if first.code == "fosscut.model.unknown_font" {
        codes::TEXT_UNKNOWN_FONT
    } else {
        codes::TEXT_INVALID
    };
    let hint = if code == codes::TEXT_UNKNOWN_FONT {
        "Import a .ttf or .otf file with media import, then name one of its families; media.list shows each font asset's families."
    } else {
        STYLE_FIELDS
    };
    Err(ApiError::new(code, format!("{command}: {}", first.message))
        .with_hint(hint)
        .with_details(serde_json::to_value(&issues).unwrap_or_default()))
}

/// Adds a text clip (a title or caption).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextAdd {
    /// The text; `\n` separates lines.
    pub text: String,
    /// Target text track; the first text track (created on top if needed) when omitted.
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Timeline position; appended after the track's last clip when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// Length; 3 seconds when omitted.
    #[serde(default)]
    pub duration: Option<TimeInput>,
    /// A text preset to start from: one saved in the project or a built-in one
    /// (`capabilities` lists them under text.presets).
    #[serde(default)]
    pub preset: Option<String>,
    /// The style, or changes to the preset's, as a JSON merge patch, for example
    /// `{"size": 0.1, "stroke": {"enabled": true}, "letter_spacing": 0.05}`.
    #[serde(default)]
    pub style: Option<Value>,
    /// Parts of the text styled differently, by character positions (`end` excluded).
    #[serde(default)]
    pub spans: Vec<TextSpan>,
    /// Show the text letter by letter, word by word or line by line.
    #[serde(default)]
    pub reveal: Option<RevealInput>,
    /// Placement; centred when omitted.
    #[serde(default)]
    pub transform: Option<Transform>,
}

impl Command for TextAdd {
    const NAME: &'static str = "text.add";
    const SUMMARY: &'static str = "Add a text clip: a style or preset (outline, shadow, boxes, glow, spacing, case, underline, project fonts), spans that restyle parts, and a reveal (typewriter, word by word, line by line). Never overwrites: fails with fosscut.clip.overlap if the range is taken.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        if self.text.trim().is_empty() {
            return Err(ApiError::new(
                codes::TEXT_EMPTY,
                "text.add: text must not be empty",
            ));
        }
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let duration = match &self.duration {
            Some(duration) => resolve(duration, rate)?,
            None => rate.snap(Flicks::from_seconds(3).unwrap_or(Flicks::ZERO)),
        };
        let style = styled(
            ctx.project,
            TextStyle::default(),
            self.preset.as_deref(),
            self.style.as_ref(),
        )?;
        let content = TextContent {
            text: self.text,
            style,
            spans: self.spans,
            reveal: self.reveal.as_ref().map(|r| r.reveal(rate)).transpose()?,
        };
        checked(ctx.project, Self::NAME, &content)?;
        place_text(
            ctx,
            &timeline,
            self.track.as_ref(),
            self.start.as_ref(),
            duration,
            content,
            self.transform.unwrap_or_default(),
        )
    }
}

/// Puts a text clip on a text track, refusing taken ranges.
fn place_text(
    ctx: &mut Context<'_>,
    timeline: &Id,
    track: Option<&Id>,
    start: Option<&TimeInput>,
    duration: Flicks,
    content: TextContent,
    transform: Transform,
) -> Result<ClipPlacement, ApiError> {
    let rate = ctx.project.timelines[timeline].format.rate;
    let track = match track {
        Some(track) => track.clone(),
        None => find_or_create_track(ctx.project, timeline, TrackKind::Text)?,
    };
    let id = ctx.project.allocate_id(IdKind::Clip);
    let timeline_doc = timeline_mut(ctx.project, timeline)?;
    let index = track_index(timeline_doc, &track)?;
    let track_doc = &mut timeline_doc.tracks[index];
    let content = ClipContent::Text(Box::new(content));
    ensure_unlocked(track_doc)?;
    ensure_kind(track_doc, &content)?;
    let start = match start {
        Some(start) => resolve(start, rate)?,
        None => track_doc.end(),
    };
    ensure_valid_range(start, duration)?;
    ensure_free(track_doc, start, start + duration, None, rate)?;
    let clip = Clip {
        transform,
        ..Clip::new(id, start, duration, content)
    };
    let placement = ClipPlacement::of(&clip, &track, rate);
    insert_sorted(track_doc, clip);
    Ok(placement)
}

/// Changes a text clip's text, style, spans or reveal.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextSet {
    /// The text clip.
    pub clip: Id,
    /// New text; spans that no longer fit are dropped.
    #[serde(default)]
    pub text: Option<String>,
    /// A preset whose style replaces the current one before `style` applies.
    #[serde(default)]
    pub preset: Option<String>,
    /// Changes to the style as a JSON merge patch; `null` resets a field to its default.
    #[serde(default)]
    pub style: Option<Value>,
    /// Replaces the spans; `[]` removes them.
    #[serde(default)]
    pub spans: Option<Vec<TextSpan>>,
    /// A new reveal, or `null` to show the text at once.
    #[serde(default, deserialize_with = "present")]
    #[schemars(with = "Option<RevealInput>")]
    pub reveal: Option<Option<RevealInput>>,
}

/// Tells a field set to `null` (`Some(None)`) from one left out (`None`).
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A text clip's content after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TextChanged {
    /// The clip.
    pub clip: Id,
    /// Its text, style, spans and reveal now; defaults are omitted.
    pub content: TextContent,
}

impl Command for TextSet {
    const NAME: &'static str = "text.set";
    const SUMMARY: &'static str = "Change a text clip: its text, a preset, a style patch (font, size, colour, bold, italic, underline, case, alignment, letter and line spacing, outline, box, shadow, glow), its spans or its reveal.";
    type Output = TextChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<TextChanged, ApiError> {
        if self.text.is_none()
            && self.preset.is_none()
            && self.style.is_none()
            && self.spans.is_none()
            && self.reveal.is_none()
        {
            return Err(invalid(
                "text.set: give text, preset, style, spans or reveal",
            ));
        }
        let location = locate_clip(ctx.project, &self.clip)?;
        let rate = ctx.project.timelines[&location.timeline].format.rate;
        let clip =
            &ctx.project.timelines[&location.timeline].tracks[location.track].clips[location.clip];
        let ClipContent::Text(current) = &clip.content else {
            return Err(ApiError::new(
                codes::TEXT_NOT_TEXT,
                format!(
                    "text.set: {} is a {} clip, not a text clip",
                    self.clip,
                    clip.content.kind_name()
                ),
            ));
        };
        let mut content = current.clone();
        if let Some(text) = self.text {
            if text.trim().is_empty() {
                return Err(ApiError::new(
                    codes::TEXT_EMPTY,
                    "text.set: text must not be empty",
                ));
            }
            let length = text.chars().count();
            content.spans.retain(|s| s.end <= length);
            content.text = text;
        }
        content.style = styled(
            ctx.project,
            content.style,
            self.preset.as_deref(),
            self.style.as_ref(),
        )?;
        if let Some(spans) = self.spans {
            content.spans = spans;
        }
        if let Some(reveal) = self.reveal {
            content.reveal = reveal.map(|r| r.reveal(rate)).transpose()?;
        }
        checked(ctx.project, Self::NAME, &content)?;
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let track = &mut timeline.tracks[location.track];
        ensure_unlocked(track)?;
        track.clips[location.clip].content = ClipContent::Text(content.clone());
        Ok(TextChanged {
            clip: self.clip,
            content: *content,
        })
    }
}

/// Saves a text style in the project under a name, for `preset` in `text.add` and
/// `text.set`.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextPresetSave {
    /// The preset's name; a built-in preset of the same name is hidden by it.
    pub name: String,
    /// The text clip whose style to save.
    #[serde(default)]
    pub clip: Option<Id>,
    /// Or the style itself, as a merge patch over the default style.
    #[serde(default)]
    pub style: Option<Value>,
    /// Replace a saved preset of this name instead of failing.
    #[serde(default)]
    pub replace: bool,
}

/// The project's saved text presets after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TextPresets {
    /// Every saved preset, sorted by name.
    pub presets: Vec<TextPreset>,
}

impl Command for TextPresetSave {
    const NAME: &'static str = "text.preset_save";
    const SUMMARY: &'static str = "Save a text clip's style (or a given style) as a named preset of the project, to reuse with preset in text.add and text.set.";
    type Output = TextPresets;

    fn apply(self, ctx: &mut Context<'_>) -> Result<TextPresets, ApiError> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(invalid("text.preset_save: the preset needs a name"));
        }
        let style = match (&self.clip, &self.style) {
            (Some(clip), None) => {
                let location = locate_clip(ctx.project, clip)?;
                match &ctx.project.timelines[&location.timeline].tracks[location.track].clips
                    [location.clip]
                    .content
                {
                    ClipContent::Text(text) => text.style.clone(),
                    other => {
                        return Err(ApiError::new(
                            codes::TEXT_NOT_TEXT,
                            format!(
                                "text.preset_save: {clip} is a {} clip, not a text clip",
                                other.kind_name()
                            ),
                        ));
                    }
                }
            }
            (None, Some(patch)) => styled(ctx.project, TextStyle::default(), None, Some(patch))?,
            _ => {
                return Err(invalid("text.preset_save: give either a clip or a style"));
            }
        };
        if let Some(first) = text_style_issues(ctx.project, &style).first() {
            return Err(invalid(format!("text.preset_save: {}", first.message)));
        }
        let presets = &mut ctx.project.manifest.text_presets;
        match presets.iter().position(|p| p.name == name) {
            Some(index) if self.replace => presets[index].style = style,
            Some(_) => {
                return Err(ApiError::new(
                    codes::TEXT_PRESET_EXISTS,
                    format!("text.preset_save: the project already has a preset {name:?}"),
                )
                .with_hint("Pass replace: true to overwrite it, or choose another name."));
            }
            None => {
                presets.push(TextPreset { name, style });
                presets.sort_by(|a, b| a.name.cmp(&b.name));
            }
        }
        Ok(TextPresets {
            presets: presets.clone(),
        })
    }
}

/// Removes a saved text preset. Clips styled with it keep their style.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextPresetRemove {
    /// The preset's name.
    pub name: String,
}

impl Command for TextPresetRemove {
    const NAME: &'static str = "text.preset_remove";
    const SUMMARY: &'static str =
        "Remove a text preset saved in the project; clips styled with it keep their style.";
    type Output = TextPresets;

    fn apply(self, ctx: &mut Context<'_>) -> Result<TextPresets, ApiError> {
        let presets = &mut ctx.project.manifest.text_presets;
        let Some(index) = presets.iter().position(|p| p.name == self.name) else {
            let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
            return Err(ApiError::new(
                codes::TEXT_UNKNOWN_PRESET,
                format!(
                    "text.preset_remove: the project has no preset {:?}",
                    self.name
                ),
            )
            .with_hint(if names.is_empty() {
                "The project has no saved presets.".to_string()
            } else {
                format!("Saved presets: {}.", names.join(", "))
            }));
        };
        presets.remove(index);
        Ok(TextPresets {
            presets: presets.clone(),
        })
    }
}

/// Adds a built-in title template as one text clip: its lines styled and placed as the
/// template says, with the given texts.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextTemplate {
    /// The template (`capabilities` lists them under text.templates).
    pub template: String,
    /// The text of each line, in order. With none, the template's placeholders show; once
    /// you give any, the lines you leave out (and empty strings) are dropped.
    #[serde(default)]
    pub lines: Vec<String>,
    /// Target text track; the first text track (created on top if needed) when omitted.
    #[serde(default)]
    pub track: Option<Id>,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Timeline position; appended after the track's last clip when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
    /// Length; 4 seconds when omitted.
    #[serde(default)]
    pub duration: Option<TimeInput>,
    /// Changes to the template's style as a JSON merge patch.
    #[serde(default)]
    pub style: Option<Value>,
}

impl Command for TextTemplate {
    const NAME: &'static str = "text.template";
    const SUMMARY: &'static str = "Add a title template (lower-third, headline, chapter, quote, callout, end-card) as a styled, animated text clip with your lines.";
    type Output = ClipPlacement;

    fn apply(self, ctx: &mut Context<'_>) -> Result<ClipPlacement, ApiError> {
        let template = text_template(&self.template)?;
        if self.lines.len() > template.lines.len() {
            return Err(invalid(format!(
                "text.template: {} has {} lines, not {}",
                template.name,
                template.lines.len(),
                self.lines.len()
            )));
        }
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let duration = match &self.duration {
            Some(duration) => resolve(duration, rate)?,
            None => rate.snap(Flicks::from_seconds(4).unwrap_or(Flicks::ZERO)),
        };
        // Lines joined by `\n`, each restyled by its span.
        let mut text = String::new();
        let mut spans = Vec::new();
        // Lines you give replace the placeholders; once you give any, the lines you leave
        // out are dropped rather than showing a placeholder you did not write.
        let blank = String::new();
        for (index, line) in template.lines.iter().enumerate() {
            let words = match self.lines.get(index) {
                Some(given) => given,
                None if self.lines.is_empty() => &line.placeholder,
                None => &blank,
            };
            if words.trim().is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push('\n');
            }
            let start = text.chars().count();
            text.push_str(words);
            let end = text.chars().count();
            if line.span != LineLook::default() {
                spans.push(line.span.at(start, end));
            }
        }
        if text.trim().is_empty() {
            return Err(ApiError::new(
                codes::TEXT_EMPTY,
                "text.template: every line is empty",
            ));
        }
        let style = styled(ctx.project, template.style, None, self.style.as_ref())?;
        // The reveal never takes longer than half the clip, so the text rests in between.
        let reveal = template.reveal.map(|r| TextReveal {
            unit: r.unit,
            motion: r.motion,
            duration: Flicks::from_millis(r.duration_ms).min(Flicks::new(duration.get() / 2)),
            exit: r.exit,
        });
        let content = TextContent {
            text,
            style,
            spans,
            reveal: reveal.filter(|r| r.duration.is_positive()),
        };
        checked(ctx.project, Self::NAME, &content)?;
        let transform = Transform {
            position: template.position,
            ..Transform::default()
        };
        place_text(
            ctx,
            &timeline,
            self.track.as_ref(),
            self.start.as_ref(),
            duration,
            content,
            transform,
        )
    }
}
