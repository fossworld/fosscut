//! Built-in caption presets (ADR-0021 item 4), text style presets and title templates
//! (ADR-0027): data documents written for FossCut. A preset's values are copied into the
//! project, so projects never depend on them.

use fosscut_model::{
    Animator, CaptionStyle, Color, PageFade, PagePop, RevealMotion, RevealUnit, TextSpan,
    TextStyle, WordEmphasis, WordHighlight, WordKaraoke, WordPop, WordReveal,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{ApiError, codes};

const SOURCES: [&str; 4] = [
    include_str!("../presets/captions/bold-pop.json"),
    include_str!("../presets/captions/highlight.json"),
    include_str!("../presets/captions/karaoke.json"),
    include_str!("../presets/captions/boxed.json"),
];

const TEXT_STYLES: &str = include_str!("../presets/text/styles.json");
const TITLE_TEMPLATES: &str = include_str!("../presets/text/templates.json");

/// A built-in text style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextStylePreset {
    /// Stable name, for example `neon`.
    pub name: String,
    /// What it looks like and what it suits.
    pub description: String,
    /// The style it gives a text clip.
    pub style: TextStyle,
}

/// A built-in title: lines with their own look, a style, a place and a reveal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TitleTemplate {
    /// Stable name, for example `lower-third`.
    pub name: String,
    /// What it shows and when to use it.
    pub description: String,
    /// Its lines, top to bottom.
    pub lines: Vec<TemplateLine>,
    /// The style of the whole title.
    pub style: TextStyle,
    /// Where it sits, as a clip's `transform.position`.
    pub position: [f64; 2],
    /// How it appears.
    #[serde(default)]
    pub reveal: Option<TemplateReveal>,
}

/// One line of a title template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateLine {
    /// Text shown when the caller gives none.
    pub placeholder: String,
    /// How the line differs from the title's style.
    #[serde(default)]
    pub span: LineLook,
}

/// How a template line differs from its title's style.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LineLook {
    /// Fill colour.
    #[serde(default)]
    pub color: Option<Color>,
    /// Bold weight.
    #[serde(default)]
    pub bold: Option<bool>,
    /// Italic style.
    #[serde(default)]
    pub italic: Option<bool>,
    /// Underlined.
    #[serde(default)]
    pub underline: Option<bool>,
    /// Font family.
    #[serde(default)]
    pub font: Option<String>,
    /// Font size as a fraction of the canvas height.
    #[serde(default)]
    pub size: Option<f64>,
}

impl LineLook {
    /// The look as a span over characters `start` to `end`.
    pub fn at(&self, start: usize, end: usize) -> TextSpan {
        TextSpan {
            start,
            end,
            color: self.color.clone(),
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            font: self.font.clone(),
            size: self.size,
        }
    }
}

/// How a template's title appears.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateReveal {
    /// What appears at a time.
    pub unit: RevealUnit,
    /// How each one appears.
    #[serde(default)]
    pub motion: RevealMotion,
    /// Until the whole title shows, in milliseconds.
    pub duration_ms: i64,
    /// Also leave the same way.
    #[serde(default)]
    pub exit: bool,
}

/// Every built-in text style preset.
pub fn text_style_presets() -> Vec<TextStylePreset> {
    serde_json::from_str(TEXT_STYLES).unwrap_or_default()
}

/// The style of the built-in text preset called `name`.
pub fn text_style_preset(name: &str) -> Result<TextStyle, ApiError> {
    let presets = text_style_presets();
    presets
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.style.clone())
        .ok_or_else(|| {
            let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
            ApiError::new(
                codes::TEXT_UNKNOWN_PRESET,
                format!("there is no text preset {name}"),
            )
            .with_hint(format!(
                "Built-in presets: {}; text.preset_save adds the project's own.",
                names.join(", ")
            ))
        })
}

/// Every built-in title template.
pub fn text_templates() -> Vec<TitleTemplate> {
    serde_json::from_str(TITLE_TEMPLATES).unwrap_or_default()
}

/// The built-in title template called `name`.
pub fn text_template(name: &str) -> Result<TitleTemplate, ApiError> {
    let templates = text_templates();
    templates
        .iter()
        .find(|t| t.name == name)
        .cloned()
        .ok_or_else(|| {
            let names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
            ApiError::new(
                codes::TEXT_UNKNOWN_TEMPLATE,
                format!("there is no title template {name}"),
            )
            .with_hint(format!("Templates: {}.", names.join(", ")))
        })
}

/// A built-in caption look.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionPreset {
    /// Stable name, for example `highlight`.
    pub name: String,
    /// What it looks like and what it suits.
    pub description: String,
    /// The look it gives a caption track.
    pub style: CaptionStyle,
}

/// Every built-in caption preset. The documents are compiled in and checked by tests.
pub fn caption_presets() -> Vec<CaptionPreset> {
    SOURCES
        .iter()
        .filter_map(|source| serde_json::from_str(source).ok())
        .collect()
}

/// The style of the preset called `name`.
pub fn caption_preset(name: &str) -> Result<CaptionStyle, ApiError> {
    let presets = caption_presets();
    presets
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.style.clone())
        .ok_or_else(|| {
            let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
            ApiError::new(
                codes::CAPTIONS_UNKNOWN_PRESET,
                format!("there is no caption preset {name}"),
            )
            .with_hint(format!("Presets: {}.", names.join(", ")))
        })
}

/// Every caption animator with its version, what it does and, unless `brief`, the JSON
/// Schema of its parameters (ADR-0021 item 3).
pub fn caption_animators(brief: bool) -> Vec<Value> {
    let schema = |value: schemars::Schema| serde_json::to_value(value).unwrap_or_default();
    let all = [
        (
            "page.fade",
            "The page fades in and out.",
            schema(schemars::schema_for!(PageFade)),
        ),
        (
            "page.pop",
            "The page scales in from a smaller size.",
            schema(schemars::schema_for!(PagePop)),
        ),
        (
            "word.reveal",
            "Each word appears when it is spoken.",
            schema(schemars::schema_for!(WordReveal)),
        ),
        (
            "word.highlight",
            "The spoken word changes colour, gains a box or grows.",
            schema(schemars::schema_for!(WordHighlight)),
        ),
        (
            "word.pop",
            "Each word pops up and back as it is spoken.",
            schema(schemars::schema_for!(WordPop)),
        ),
        (
            "word.karaoke",
            "Each word fills with a colour from left to right while it is spoken.",
            schema(schemars::schema_for!(WordKaraoke)),
        ),
        (
            "word.emphasis",
            "Words marked emphasis (captions.edit) stand out in colour and size.",
            schema(schemars::schema_for!(WordEmphasis)),
        ),
    ];
    all.into_iter()
        .map(|(name, summary, params)| {
            let version = Animator::TYPES
                .iter()
                .find(|(n, _)| *n == name)
                .map_or(1, |(_, v)| *v);
            if brief {
                json!({ "type": name, "version": version, "summary": summary })
            } else {
                json!({ "type": name, "version": version, "summary": summary, "params_schema": params })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_parses_and_is_distinct() {
        let presets = caption_presets();
        let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["bold-pop", "highlight", "karaoke", "boxed"]);
        for preset in &presets {
            assert!(preset.description.ends_with('.'));
            assert!(!preset.style.animators.is_empty(), "{}", preset.name);
            assert!(preset.style.animators.iter().all(Animator::enabled));
        }
        for pair in presets.windows(2) {
            assert_ne!(pair[0].style, pair[1].style);
        }
        assert_eq!(caption_preset("boxed").unwrap(), presets[3].style);
        let unknown = caption_preset("neon").unwrap_err();
        assert_eq!(unknown.code, codes::CAPTIONS_UNKNOWN_PRESET);
        assert!(unknown.hint.unwrap().contains("karaoke"));
    }

    #[test]
    fn every_animator_type_is_described() {
        let full = caption_animators(false);
        assert_eq!(full.len(), Animator::TYPES.len());
        for (entry, (name, version)) in full.iter().zip(Animator::TYPES) {
            assert_eq!(entry["type"], name);
            assert_eq!(entry["version"], version);
            assert!(entry["params_schema"]["properties"].is_object(), "{name}");
        }
        assert!(caption_animators(true)[0].get("params_schema").is_none());
    }
}
