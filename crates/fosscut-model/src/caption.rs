//! Captions (ADR-0021): caption clips are pages of lines of timed words, and caption tracks
//! own the captions' look and animators. Word times are relative to the clip's start.

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::is_default;
use crate::timeline::{Align, Color};

/// One caption page: what is on screen at once.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CaptionContent {
    /// Lines from top to bottom.
    pub lines: Vec<CaptionLine>,
}

impl CaptionContent {
    /// Every word, line by line.
    pub fn words(&self) -> impl Iterator<Item = &CaptionWord> {
        self.lines.iter().flat_map(|line| line.words.iter())
    }

    /// The page's text: words joined by spaces, lines by `\n`.
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| {
                line.words
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// One line of a caption page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CaptionLine {
    /// Words from left to right.
    pub words: Vec<CaptionWord>,
}

/// A timed word.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CaptionWord {
    /// The word as shown, with attached punctuation.
    pub text: String,
    /// When it is spoken, from the clip's start.
    pub start: Flicks,
    /// When it ends, from the clip's start.
    pub end: Flicks,
    /// A keyword the `word.emphasis` animator styles.
    #[serde(default, skip_serializing_if = "is_default")]
    pub emphasis: bool,
}

/// How a caption track's captions look and move; every field has a default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionStyle {
    /// Font family; the bundled sans-serif when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font weight (bold when omitted).
    #[serde(default = "bold", skip_serializing_if = "is_bold")]
    pub weight: FontWeight,
    /// Italic style.
    #[serde(default, skip_serializing_if = "is_default")]
    pub italic: bool,
    /// Font size as a fraction of the canvas height (0.055 when omitted).
    #[serde(
        default = "default_caption_size",
        skip_serializing_if = "is_default_caption_size"
    )]
    pub size: f64,
    /// Text colour (white when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub color: Color,
    /// Horizontal alignment of lines.
    #[serde(default, skip_serializing_if = "is_default")]
    pub align: Align,
    /// Show words in capitals.
    #[serde(default, skip_serializing_if = "is_default")]
    pub uppercase: bool,
    /// Outline around the letters (black, on, when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub stroke: Stroke,
    /// Background box behind each line (off when omitted).
    #[serde(rename = "box", default, skip_serializing_if = "is_default")]
    pub background: CaptionBox,
    /// Where the page sits inside the safe area (bottom when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub placement: Placement,
    /// Animation, evaluated per frame from word times.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub animators: Vec<Animator>,
}

fn bold() -> FontWeight {
    FontWeight::Bold
}

fn is_bold(weight: &FontWeight) -> bool {
    *weight == FontWeight::Bold
}

fn default_caption_size() -> f64 {
    0.055
}

fn is_default_caption_size(size: &f64) -> bool {
    *size == default_caption_size()
}

impl Default for CaptionStyle {
    fn default() -> CaptionStyle {
        CaptionStyle {
            font: None,
            weight: FontWeight::Bold,
            italic: false,
            size: default_caption_size(),
            color: Color::default(),
            align: Align::default(),
            uppercase: false,
            stroke: Stroke::default(),
            background: CaptionBox::default(),
            placement: Placement::default(),
            animators: Vec::new(),
        }
    }
}

/// Font weights of the bundled fonts.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FontWeight {
    /// Regular (400).
    #[default]
    Regular,
    /// Bold (700).
    Bold,
    /// Extra bold (800).
    ExtraBold,
    /// Black (900).
    Black,
}

/// An outline around the letters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    /// Whether it is drawn.
    pub enabled: bool,
    /// Its colour (black when omitted).
    #[serde(default = "black", skip_serializing_if = "is_black")]
    pub color: Color,
    /// Its width as a fraction of the font size (0.12 when omitted).
    #[serde(
        default = "default_stroke_width",
        skip_serializing_if = "is_default_stroke_width"
    )]
    pub width: f64,
}

fn black() -> Color {
    Color::parse("#000000").unwrap_or_default()
}

fn is_black(color: &Color) -> bool {
    *color == black()
}

fn default_stroke_width() -> f64 {
    0.12
}

fn is_default_stroke_width(width: &f64) -> bool {
    *width == default_stroke_width()
}

impl Default for Stroke {
    fn default() -> Stroke {
        Stroke {
            enabled: true,
            color: black(),
            width: default_stroke_width(),
        }
    }
}

/// A box behind each line of text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionBox {
    /// Whether it is drawn.
    pub enabled: bool,
    /// Its colour (black at 70% when omitted).
    #[serde(default = "shade", skip_serializing_if = "is_shade")]
    pub color: Color,
    /// Space around the text as a fraction of the font size (0.25 when omitted).
    #[serde(
        default = "default_padding",
        skip_serializing_if = "is_default_padding"
    )]
    pub padding: f64,
    /// Corner radius as a fraction of the box height (0.3 when omitted).
    #[serde(default = "default_radius", skip_serializing_if = "is_default_radius")]
    pub radius: f64,
}

fn shade() -> Color {
    Color::parse("#000000B3").unwrap_or_default()
}

fn is_shade(color: &Color) -> bool {
    *color == shade()
}

fn default_padding() -> f64 {
    0.25
}

fn is_default_padding(value: &f64) -> bool {
    *value == default_padding()
}

fn default_radius() -> f64 {
    0.3
}

fn is_default_radius(value: &f64) -> bool {
    *value == default_radius()
}

impl Default for CaptionBox {
    fn default() -> CaptionBox {
        CaptionBox {
            enabled: false,
            color: shade(),
            padding: default_padding(),
            radius: default_radius(),
        }
    }
}

/// Where a caption page sits inside the canvas's safe area (ADR-0021 item 5).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// At the top of the safe area.
    Top,
    /// Centred.
    Middle,
    /// At the bottom of the safe area.
    #[default]
    Bottom,
}

/// The safe area of a canvas as fractions of its width and height: left, top, right and
/// bottom margins (ADR-0021 item 5). Platforms draw their own controls over the bottom and
/// sides of tall videos.
pub fn safe_margins(width: u32, height: u32) -> [f64; 4] {
    if height > width {
        [0.06, 0.10, 0.06, 0.22]
    } else {
        [0.05, 0.05, 0.05, 0.05]
    }
}

/// A caption animator (ADR-0021 item 3): its type, version, switch and parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum Animator {
    /// The page fades in and out.
    #[serde(rename = "page.fade")]
    PageFade(Animated<PageFade>),
    /// The page scales in.
    #[serde(rename = "page.pop")]
    PagePop(Animated<PagePop>),
    /// Each word appears when it is spoken.
    #[serde(rename = "word.reveal")]
    WordReveal(Animated<WordReveal>),
    /// The spoken word changes colour, gains a box or grows.
    #[serde(rename = "word.highlight")]
    WordHighlight(Animated<WordHighlight>),
    /// Each word pops up and back as it is spoken.
    #[serde(rename = "word.pop")]
    WordPop(Animated<WordPop>),
    /// Each word fills from left to right while it is spoken.
    #[serde(rename = "word.karaoke")]
    WordKaraoke(Animated<WordKaraoke>),
    /// Words marked `emphasis` stand out.
    #[serde(rename = "word.emphasis")]
    WordEmphasis(Animated<WordEmphasis>),
}

impl Animator {
    /// Every animator type with its current version.
    pub const TYPES: [(&'static str, u32); 7] = [
        ("page.fade", 1),
        ("page.pop", 1),
        ("word.reveal", 1),
        ("word.highlight", 1),
        ("word.pop", 1),
        ("word.karaoke", 1),
        ("word.emphasis", 1),
    ];

    /// The type name, for example `word.highlight`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Animator::PageFade(_) => "page.fade",
            Animator::PagePop(_) => "page.pop",
            Animator::WordReveal(_) => "word.reveal",
            Animator::WordHighlight(_) => "word.highlight",
            Animator::WordPop(_) => "word.pop",
            Animator::WordKaraoke(_) => "word.karaoke",
            Animator::WordEmphasis(_) => "word.emphasis",
        }
    }

    /// The stored version.
    pub fn version(&self) -> u32 {
        match self {
            Animator::PageFade(a) => a.version,
            Animator::PagePop(a) => a.version,
            Animator::WordReveal(a) => a.version,
            Animator::WordHighlight(a) => a.version,
            Animator::WordPop(a) => a.version,
            Animator::WordKaraoke(a) => a.version,
            Animator::WordEmphasis(a) => a.version,
        }
    }

    /// Whether it is applied.
    pub fn enabled(&self) -> bool {
        match self {
            Animator::PageFade(a) => a.enabled,
            Animator::PagePop(a) => a.enabled,
            Animator::WordReveal(a) => a.enabled,
            Animator::WordHighlight(a) => a.enabled,
            Animator::WordPop(a) => a.enabled,
            Animator::WordKaraoke(a) => a.enabled,
            Animator::WordEmphasis(a) => a.enabled,
        }
    }
}

/// An animator's version, switch and parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Animated<P: Default + PartialEq> {
    /// Parameter version; 1 when omitted.
    #[serde(default = "first_version")]
    pub version: u32,
    /// Whether it is applied; on when omitted.
    #[serde(default = "on")]
    pub enabled: bool,
    /// Parameters; defaults when omitted.
    #[serde(default, skip_serializing_if = "is_default")]
    pub params: P,
}

fn first_version() -> u32 {
    1
}

fn on() -> bool {
    true
}

impl<P: Default + PartialEq> Default for Animated<P> {
    fn default() -> Animated<P> {
        Animated {
            version: 1,
            enabled: true,
            params: P::default(),
        }
    }
}

/// Parameters of `page.fade`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct PageFade {
    /// Fade-in length in milliseconds.
    pub in_ms: u32,
    /// Fade-out length in milliseconds.
    pub out_ms: u32,
}

impl Default for PageFade {
    fn default() -> PageFade {
        PageFade {
            in_ms: 120,
            out_ms: 120,
        }
    }
}

/// Parameters of `page.pop`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct PagePop {
    /// Scale the page starts from.
    pub from: f64,
    /// Length in milliseconds.
    pub duration_ms: u32,
}

impl Default for PagePop {
    fn default() -> PagePop {
        PagePop {
            from: 0.8,
            duration_ms: 150,
        }
    }
}

/// Parameters of `word.reveal`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct WordReveal {
    /// Fade-in length of each word in milliseconds; 0 shows it at once.
    pub fade_ms: u32,
}

impl Default for WordReveal {
    fn default() -> WordReveal {
        WordReveal { fade_ms: 80 }
    }
}

/// Parameters of `word.highlight`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct WordHighlight {
    /// Colour of the spoken word.
    pub color: Color,
    /// Colour of a box behind the spoken word; none when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub box_color: Option<Color>,
    /// Scale of the spoken word.
    pub scale: f64,
}

impl Default for WordHighlight {
    fn default() -> WordHighlight {
        WordHighlight {
            color: yellow(),
            box_color: None,
            scale: 1.0,
        }
    }
}

/// Parameters of `word.pop`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct WordPop {
    /// Largest scale, reached halfway.
    pub scale: f64,
    /// Length in milliseconds from the word's start.
    pub duration_ms: u32,
}

impl Default for WordPop {
    fn default() -> WordPop {
        WordPop {
            scale: 1.2,
            duration_ms: 200,
        }
    }
}

/// Parameters of `word.karaoke`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct WordKaraoke {
    /// Colour that fills each word while it is spoken.
    pub color: Color,
}

impl Default for WordKaraoke {
    fn default() -> WordKaraoke {
        WordKaraoke { color: yellow() }
    }
}

/// Parameters of `word.emphasis`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct WordEmphasis {
    /// Colour of emphasised words.
    pub color: Color,
    /// Scale of emphasised words.
    pub scale: f64,
}

impl Default for WordEmphasis {
    fn default() -> WordEmphasis {
        WordEmphasis {
            color: yellow(),
            scale: 1.1,
        }
    }
}

fn yellow() -> Color {
    Color::parse("#FFE14D").unwrap_or_default()
}
