//! Text clips (ADR-0027): one style for the whole text, spans that restyle parts of it, and
//! a reveal that shows it letter by letter, word by word or line by line. Fonts are the
//! bundled family or a font asset of the project, never a machine's installed fonts.

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::caption::{Stroke, TextBox};
use crate::is_default;
use crate::timeline::{Align, Color};

/// The font family every build has (SIL Open Font License).
pub const BUNDLED_FONT: &str = "Inter";

/// Text content: the text, its style, restyled parts and how it appears.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextContent {
    /// The text; `\n` separates lines.
    pub text: String,
    /// Appearance; defaults when omitted.
    #[serde(default, skip_serializing_if = "is_default")]
    pub style: TextStyle,
    /// Parts of the text styled differently, by character positions; later spans win where
    /// they overlap.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<TextSpan>,
    /// How the text appears at the clip's start (and leaves at its end).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal: Option<TextReveal>,
}

impl TextContent {
    /// Plain text with the default style.
    pub fn plain(text: impl Into<String>) -> TextContent {
        TextContent {
            text: text.into(),
            ..TextContent::default()
        }
    }

    /// Text with one style.
    pub fn styled(text: impl Into<String>, style: TextStyle) -> TextContent {
        TextContent {
            text: text.into(),
            style,
            ..TextContent::default()
        }
    }

    /// Every font family the content names, the style's first.
    pub fn fonts(&self) -> impl Iterator<Item = &str> {
        self.style
            .font
            .iter()
            .chain(self.spans.iter().filter_map(|s| s.font.as_ref()))
            .map(String::as_str)
    }
}

/// Text appearance. Lengths such as spacing, stroke and shadow are fractions of the font
/// size (em), so a style looks the same at every size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextStyle {
    /// Font family: `Inter` (bundled) or a family of one of the project's font assets;
    /// `Inter` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font size as a fraction of the canvas height.
    #[serde(default = "default_text_size")]
    pub size: f64,
    /// Fill colour.
    #[serde(default)]
    pub color: Color,
    /// Bold weight.
    #[serde(default, skip_serializing_if = "is_default")]
    pub bold: bool,
    /// Italic style.
    #[serde(default, skip_serializing_if = "is_default")]
    pub italic: bool,
    /// Underlined.
    #[serde(default, skip_serializing_if = "is_default")]
    pub underline: bool,
    /// Letter case shown; the text itself is kept as written.
    #[serde(default, skip_serializing_if = "is_default")]
    pub case: TextCase,
    /// Horizontal alignment of lines.
    #[serde(default, skip_serializing_if = "is_default")]
    pub align: Align,
    /// Extra space between letters in em, from -0.5 to 2; 0 when omitted.
    #[serde(default, skip_serializing_if = "is_default")]
    pub letter_spacing: f64,
    /// Distance between lines as a multiple of the font size, from 0.5 to 4; 1.2 when
    /// omitted.
    #[serde(
        default = "default_line_height",
        skip_serializing_if = "is_default_line_height"
    )]
    pub line_height: f64,
    /// Outline around the letters (off when omitted).
    #[serde(default = "Stroke::off", skip_serializing_if = "Stroke::is_off")]
    pub stroke: Stroke,
    /// Box behind each line (off when omitted).
    #[serde(rename = "box", default, skip_serializing_if = "is_default")]
    pub background: TextBox,
    /// Shadow under the letters (off when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub shadow: Shadow,
    /// Soft light around the letters (off when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub glow: Glow,
}

fn default_text_size() -> f64 {
    0.08
}

/// Line distance as a multiple of the font size when a style does not say.
pub const DEFAULT_LINE_HEIGHT: f64 = 1.2;

fn default_line_height() -> f64 {
    DEFAULT_LINE_HEIGHT
}

fn is_default_line_height(value: &f64) -> bool {
    *value == DEFAULT_LINE_HEIGHT
}

impl Default for TextStyle {
    fn default() -> TextStyle {
        TextStyle {
            font: None,
            size: default_text_size(),
            color: Color::default(),
            bold: false,
            italic: false,
            underline: false,
            case: TextCase::default(),
            align: Align::default(),
            letter_spacing: 0.0,
            line_height: DEFAULT_LINE_HEIGHT,
            stroke: Stroke::off(),
            background: TextBox::default(),
            shadow: Shadow::default(),
            glow: Glow::default(),
        }
    }
}

impl TextStyle {
    /// The font family, `Inter` when the style names none.
    pub fn family(&self) -> &str {
        self.font.as_deref().unwrap_or(BUNDLED_FONT)
    }
}

/// Letter case shown.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextCase {
    /// As written.
    #[default]
    AsWritten,
    /// ALL CAPITALS.
    Upper,
    /// all lower case.
    Lower,
    /// The First Letter Of Every Word In Capitals, the rest lower case.
    Title,
}

impl TextCase {
    /// How each character of `text` is shown, one string per character, so positions in
    /// the written text map to the shown text even when a letter becomes two ("ß" to "SS").
    pub fn shown(self, text: &str) -> Vec<String> {
        let mut word_start = true;
        text.chars()
            .map(|c| {
                let upper = match self {
                    TextCase::AsWritten => None,
                    TextCase::Upper => Some(true),
                    TextCase::Lower => Some(false),
                    TextCase::Title => Some(word_start),
                };
                word_start = c.is_whitespace();
                match upper {
                    None => c.to_string(),
                    Some(true) => c.to_uppercase().collect(),
                    Some(false) => c.to_lowercase().collect(),
                }
            })
            .collect()
    }
}

/// Part of a text styled differently. Positions count characters (Unicode scalar values)
/// of the written text from 0; `end` is not included.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextSpan {
    /// First character.
    pub start: usize,
    /// The character after the last one.
    pub end: usize,
    /// Fill colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Bold weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    /// Italic style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// Underlined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    /// Font family, as the style's `font`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font size as a fraction of the canvas height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
}

/// A shadow cast by the letters (and their outline).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Shadow {
    /// Whether it is drawn.
    pub enabled: bool,
    /// Its colour (black at 70% when omitted).
    #[serde(default = "shadow_color", skip_serializing_if = "is_shadow_color")]
    pub color: Color,
    /// How far it falls, right and down, in em, each from -1 to 1 ([0.06, 0.06] when
    /// omitted).
    #[serde(
        default = "default_shadow_offset",
        skip_serializing_if = "is_default_shadow_offset"
    )]
    pub offset: [f64; 2],
    /// How soft it is in em, from 0 (sharp) to 1 (0.08 when omitted).
    #[serde(default = "default_blur", skip_serializing_if = "is_default_blur")]
    pub blur: f64,
}

fn shadow_color() -> Color {
    Color::parse("#000000B3").unwrap_or_default()
}

fn is_shadow_color(color: &Color) -> bool {
    *color == shadow_color()
}

fn default_shadow_offset() -> [f64; 2] {
    [0.06, 0.06]
}

fn is_default_shadow_offset(value: &[f64; 2]) -> bool {
    *value == default_shadow_offset()
}

fn default_blur() -> f64 {
    0.08
}

fn is_default_blur(value: &f64) -> bool {
    *value == default_blur()
}

impl Default for Shadow {
    fn default() -> Shadow {
        Shadow {
            enabled: false,
            color: shadow_color(),
            offset: default_shadow_offset(),
            blur: default_blur(),
        }
    }
}

/// Soft light around the letters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Glow {
    /// Whether it is drawn.
    pub enabled: bool,
    /// Its colour; its alpha sets the strength (white when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub color: Color,
    /// How far it reaches in em, from 0 to 2 (0.3 when omitted).
    #[serde(default = "default_glow", skip_serializing_if = "is_default_glow")]
    pub radius: f64,
}

fn default_glow() -> f64 {
    0.3
}

fn is_default_glow(value: &f64) -> bool {
    *value == default_glow()
}

impl Default for Glow {
    fn default() -> Glow {
        Glow {
            enabled: false,
            color: Color::default(),
            radius: default_glow(),
        }
    }
}

/// How a text appears: its letters, words or lines one after another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextReveal {
    /// What appears at a time.
    pub unit: RevealUnit,
    /// How each one appears.
    #[serde(default, skip_serializing_if = "is_default")]
    pub motion: RevealMotion,
    /// Time from the clip's start until the whole text is shown.
    pub duration: Flicks,
    /// Also leave the same way, in reverse order, over the same time before the clip ends.
    #[serde(default, skip_serializing_if = "is_default")]
    pub exit: bool,
}

/// What a reveal shows at a time.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevealUnit {
    /// One letter after another, as typed.
    Letter,
    /// One word after another.
    Word,
    /// One line after another.
    Line,
}

/// How each unit of a reveal appears.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevealMotion {
    /// At once (a typewriter for letters).
    #[default]
    Appear,
    /// Fading in.
    Fade,
    /// Fading in while rising into place.
    Rise,
}

impl TextReveal {
    /// The look of unit `index` of `count` at `time` from the clip's start, for a clip
    /// `duration` long: opacity from 0 to 1 and how far below its place it still is, in em.
    pub fn unit_at(
        &self,
        index: usize,
        count: usize,
        time: Flicks,
        duration: Flicks,
    ) -> (f32, f32) {
        let count = count.max(1);
        let span = self.duration.get().max(1) as f64;
        // Units start one step apart; fades and rises take two steps, so the last one
        // finishes exactly at the reveal's end.
        let steps = match self.motion {
            RevealMotion::Appear => count as f64,
            RevealMotion::Fade | RevealMotion::Rise => count as f64 + 1.0,
        };
        let step = span / steps;
        let progress_at = |t: f64, i: usize| match self.motion {
            RevealMotion::Appear => {
                if t >= i as f64 * step {
                    1.0
                } else {
                    0.0
                }
            }
            RevealMotion::Fade | RevealMotion::Rise => {
                ((t - i as f64 * step) / (2.0 * step)).clamp(0.0, 1.0)
            }
        };
        let mut progress = progress_at(time.get() as f64, index);
        if self.exit {
            // Leaving mirrors arriving in time, so the last unit to arrive leaves first, as
            // typing backwards.
            let left = (duration - time).get() as f64;
            progress = progress.min(progress_at(left, index));
        }
        let eased = 1.0 - (1.0 - progress).powi(3);
        let rise = match self.motion {
            RevealMotion::Rise => (1.0 - eased) * 0.35,
            _ => 0.0,
        };
        (eased as f32, rise as f32)
    }
}

/// A named text style saved in the project for reuse (`text.preset_save`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextPreset {
    /// Its name, unique in the project.
    pub name: String,
    /// The style it applies.
    pub style: TextStyle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases_map_every_written_character() {
        assert_eq!(TextCase::Upper.shown("straße").concat(), "STRASSE");
        assert_eq!(TextCase::Upper.shown("straße").len(), 6);
        assert_eq!(TextCase::Lower.shown("Hello World").concat(), "hello world");
        assert_eq!(
            TextCase::Title.shown("the QUICK  brown\nfox").concat(),
            "The Quick  Brown\nFox"
        );
        assert_eq!(TextCase::AsWritten.shown("MiXeD").concat(), "MiXeD");
    }

    #[test]
    fn reveals_show_units_in_order_and_finish_on_time() {
        let second = Flicks::from_seconds(1).unwrap_or_default();
        let clip = Flicks::from_seconds(4).unwrap_or_default();
        let typewriter = TextReveal {
            unit: RevealUnit::Letter,
            motion: RevealMotion::Appear,
            duration: second,
            exit: false,
        };
        let shown = |r: &TextReveal, t: Flicks| {
            (0..4)
                .map(|i| r.unit_at(i, 4, t, clip).0)
                .collect::<Vec<_>>()
        };
        assert_eq!(shown(&typewriter, Flicks::ZERO), vec![1.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            shown(&typewriter, Flicks::from_millis(600)),
            vec![1.0, 1.0, 1.0, 0.0]
        );
        assert_eq!(shown(&typewriter, second), vec![1.0; 4]);

        let rise = TextReveal {
            motion: RevealMotion::Rise,
            exit: true,
            ..typewriter
        };
        let (alpha, below) = rise.unit_at(0, 4, Flicks::ZERO, clip);
        assert_eq!((alpha, below), (0.0, 0.35));
        assert_eq!(rise.unit_at(3, 4, second, clip), (1.0, 0.0));
        let half = rise.unit_at(2, 4, Flicks::from_millis(500), clip).0;
        assert!(half > 0.0 && half < 1.0, "{half}");
        // At the end, everything has left; the first unit to leave is the last that came.
        assert_eq!(rise.unit_at(0, 4, clip, clip).0, 0.0);
        let leaving = clip - Flicks::from_millis(500);
        assert!(rise.unit_at(3, 4, leaving, clip).0 < rise.unit_at(0, 4, leaving, clip).0);
    }

    #[test]
    fn defaults_are_omitted_and_round_trip() {
        let plain = TextContent::plain("Hi");
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            serde_json::json!({"text": "Hi"})
        );
        let styled = TextContent {
            text: "Big news".into(),
            style: TextStyle {
                underline: true,
                case: TextCase::Upper,
                letter_spacing: 0.1,
                line_height: 1.5,
                stroke: Stroke {
                    enabled: true,
                    ..Stroke::default()
                },
                shadow: Shadow {
                    enabled: true,
                    ..Shadow::default()
                },
                glow: Glow {
                    enabled: true,
                    color: Color::parse("#FF00FF").unwrap(),
                    radius: 0.5,
                },
                ..TextStyle::default()
            },
            spans: vec![TextSpan {
                start: 4,
                end: 8,
                color: Color::parse("#FFCC00").ok(),
                bold: Some(true),
                italic: None,
                underline: None,
                font: None,
                size: None,
            }],
            reveal: Some(TextReveal {
                unit: RevealUnit::Word,
                motion: RevealMotion::Fade,
                duration: Flicks::from_millis(800),
                exit: false,
            }),
        };
        let json = serde_json::to_value(&styled).unwrap();
        assert_eq!(
            json["style"]["stroke"],
            serde_json::json!({"enabled": true})
        );
        assert_eq!(
            json["style"]["shadow"],
            serde_json::json!({"enabled": true})
        );
        assert_eq!(json["style"]["case"], "upper");
        assert!(json["style"].get("box").is_none());
        let back: TextContent = serde_json::from_value(json).unwrap();
        assert_eq!(back, styled);
        assert_eq!(styled.fonts().count(), 0);
        assert_eq!(styled.style.family(), BUNDLED_FONT);
    }
}
