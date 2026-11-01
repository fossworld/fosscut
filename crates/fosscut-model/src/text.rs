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
    /// How much each unit's motion overlaps the next one's, from 0 (one after another) to 1
    /// (all together); 0.5 when omitted. `appear` ignores it.
    #[serde(
        default = "default_overlap",
        skip_serializing_if = "is_default_overlap"
    )]
    pub overlap: f64,
    /// How each unit's motion runs; the motion's own easing when omitted (a cubic ease-out,
    /// `back` for `pop` and `bounce` for `drop`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub easing: Option<RevealEasing>,
    /// The order units arrive in; leaving reverses it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub order: RevealOrder,
    /// How units leave when `exit` is on; the arriving motion, reversed, when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_motion: Option<RevealMotion>,
}

/// Overlap of reveal units when a reveal does not say: each starts halfway through the
/// previous one's motion.
pub const DEFAULT_REVEAL_OVERLAP: f64 = 0.5;

fn default_overlap() -> f64 {
    DEFAULT_REVEAL_OVERLAP
}

fn is_default_overlap(value: &f64) -> bool {
    *value == DEFAULT_REVEAL_OVERLAP
}

impl Default for TextReveal {
    /// A one-letter-at-a-time typewriter that has not been given a duration yet.
    fn default() -> TextReveal {
        TextReveal {
            unit: RevealUnit::Letter,
            motion: RevealMotion::default(),
            duration: Flicks::ZERO,
            exit: false,
            overlap: DEFAULT_REVEAL_OVERLAP,
            easing: None,
            order: RevealOrder::default(),
            exit_motion: None,
        }
    }
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
    /// Fading in from a soft blur into focus.
    Blur,
    /// Growing from nothing past full size and settling (`back` easing).
    Pop,
    /// Shrinking from large into place while fading in.
    Zoom,
    /// Sliding in from the right towards the left while fading in.
    SlideLeft,
    /// Sliding in from the left towards the right while fading in.
    SlideRight,
    /// Falling into place from above with a bounce (`bounce` easing).
    Drop,
    /// Letters spread apart closing up to their spacing while fading in (tracking in).
    Collapse,
}

impl RevealMotion {
    /// The easing a motion uses when the reveal names none.
    pub fn natural_easing(self) -> RevealEasing {
        match self {
            RevealMotion::Pop => RevealEasing::Back,
            RevealMotion::Drop => RevealEasing::Bounce,
            _ => RevealEasing::EaseOut,
        }
    }

    /// How far, in em, units can stray from their place during this motion: the room a
    /// renderer leaves around the text. Motions that only fade need none, and a rise stays
    /// within the room below the baseline every line has.
    pub fn reach(self) -> f32 {
        match self {
            RevealMotion::Appear | RevealMotion::Fade | RevealMotion::Rise => 0.0,
            RevealMotion::Blur => 3.0 * REVEAL_BLUR,
            RevealMotion::Pop => 0.3,
            RevealMotion::Zoom => REVEAL_ZOOM,
            RevealMotion::SlideLeft | RevealMotion::SlideRight => REVEAL_SLIDE,
            RevealMotion::Drop => REVEAL_DROP,
            RevealMotion::Collapse => 0.0,
        }
    }

    /// How far units can move or grow sideways during this motion, as a fraction of their
    /// line's width: letters spreading apart, or a word or line growing around its centre
    /// (allowing for overshooting easings).
    pub fn sideways(self) -> f32 {
        match self {
            RevealMotion::Zoom => REVEAL_ZOOM / 2.0,
            RevealMotion::Pop => 0.2,
            RevealMotion::Collapse => REVEAL_SPREAD / 2.0,
            _ => 0.0,
        }
    }
}

/// Blur at the start of a `blur` reveal, in em (one standard deviation).
const REVEAL_BLUR: f32 = 0.2;
/// Extra size at the start of a `zoom` reveal: 0.8 starts at 180%.
const REVEAL_ZOOM: f32 = 0.8;
/// How far a `slide_left` or `slide_right` unit starts from its place, in em.
const REVEAL_SLIDE: f32 = 0.8;
/// How far above its place a `drop` unit starts, in em.
const REVEAL_DROP: f32 = 1.0;
/// How much wider than set the spacing starts in a `collapse`: 0.6 spreads each letter's
/// distance from its line's centre by 60%.
const REVEAL_SPREAD: f32 = 0.6;

/// How a reveal unit's motion runs from 0 to 1 (Robert Penner's easing equations and CSS
/// cubic Bezier curves).
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevealEasing {
    /// Constant speed.
    Linear,
    /// Starts slowly (cubic).
    EaseIn,
    /// Slows to a stop (cubic): the default for most motions.
    EaseOut,
    /// Starts and ends slowly (cubic).
    EaseInOut,
    /// Overshoots by about 10% and settles back.
    Back,
    /// Bounces against its end a few times.
    Bounce,
    /// Springs past its end and rings out.
    Elastic,
    /// A cubic Bezier `[x1, y1, x2, y2]` as in CSS; x1 and x2 from 0 to 1, y may overshoot.
    Bezier([f64; 4]),
}

impl RevealEasing {
    /// The fraction of the motion done at fraction `p` of its time; may overshoot 1.
    pub fn apply(self, p: f64) -> f64 {
        let p = p.clamp(0.0, 1.0);
        match self {
            RevealEasing::Linear => p,
            RevealEasing::EaseIn => p * p * p,
            RevealEasing::EaseOut => 1.0 - (1.0 - p).powi(3),
            RevealEasing::EaseInOut => {
                if p < 0.5 {
                    4.0 * p * p * p
                } else {
                    1.0 - (2.0 - 2.0 * p).powi(3) / 2.0
                }
            }
            RevealEasing::Back => {
                const C1: f64 = 1.70158;
                let u = p - 1.0;
                1.0 + (C1 + 1.0) * u * u * u + C1 * u * u
            }
            RevealEasing::Bounce => bounce_out(p),
            RevealEasing::Elastic => {
                if p <= 0.0 || p >= 1.0 {
                    p
                } else {
                    let c4 = std::f64::consts::TAU / 3.0;
                    2f64.powf(-10.0 * p) * ((10.0 * p - 0.75) * c4).sin() + 1.0
                }
            }
            RevealEasing::Bezier([x1, y1, x2, y2]) => {
                crate::Interpolation::Bezier([x1, y1, x2, y2]).progress(p)
            }
        }
    }
}

/// Penner's bounce ease-out: four arcs, each lower than the last.
fn bounce_out(p: f64) -> f64 {
    const N: f64 = 7.5625;
    const D: f64 = 2.75;
    if p < 1.0 / D {
        N * p * p
    } else if p < 2.0 / D {
        let q = p - 1.5 / D;
        N * q * q + 0.75
    } else if p < 2.5 / D {
        let q = p - 2.25 / D;
        N * q * q + 0.9375
    } else {
        let q = p - 2.625 / D;
        N * q * q + 0.984_375
    }
}

/// The order units of a reveal arrive in.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevealOrder {
    /// First to last, as read.
    #[default]
    Forward,
    /// Last to first.
    Backward,
    /// From the middle outwards; units as far from the middle arrive together.
    Center,
    /// From both ends inwards.
    Edges,
    /// A shuffled order, the same on every render.
    Random,
}

impl RevealOrder {
    /// When unit `index` of `count` starts, in steps from the first unit's start, and the
    /// last step any unit starts on.
    fn slot(self, index: usize, count: usize) -> (f64, f64) {
        let last = count.saturating_sub(1) as f64;
        let i = index as f64;
        let middle = last / 2.0;
        match self {
            RevealOrder::Forward => (i, last),
            RevealOrder::Backward => (last - i, last),
            RevealOrder::Center => {
                let first = middle.fract();
                ((i - middle).abs() - first, middle - first)
            }
            RevealOrder::Edges => {
                let first = middle.fract();
                (middle - (i - middle).abs(), middle - first)
            }
            RevealOrder::Random => {
                // The unit's rank among the units by an integer hash of their positions.
                let key = |k: usize| (shuffle_hash(k as u64), k);
                let mine = key(index);
                let rank = (0..count).filter(|&k| key(k) < mine).count();
                (rank as f64, last)
            }
        }
    }
}

/// A 64-bit integer hash (the SplitMix64 finalizer), so a shuffled order is the same on
/// every machine.
fn shuffle_hash(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// How one unit of a reveal looks at a moment, relative to its place. Lengths are in em.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RevealLook {
    /// Opacity, 0 to 1.
    pub alpha: f32,
    /// How far below its place it is (negative: above).
    pub rise: f32,
    /// How far right of its place it is (negative: left).
    pub shift: f32,
    /// Its size relative to its set size, around the unit's centre.
    pub scale: f32,
    /// Blur (one standard deviation).
    pub blur: f32,
    /// How much further from its line's centre each letter is, as a fraction of its
    /// distance (0 in place).
    pub spread: f32,
}

impl RevealLook {
    /// Fully shown, in place.
    pub const SHOWN: RevealLook = RevealLook {
        alpha: 1.0,
        rise: 0.0,
        shift: 0.0,
        scale: 1.0,
        blur: 0.0,
        spread: 0.0,
    };

    /// A unit of `motion` at progress `p` (0 hidden, 1 in place), eased by `easing`.
    fn of(motion: RevealMotion, p: f64, easing: RevealEasing) -> RevealLook {
        if p >= 1.0 {
            return RevealLook::SHOWN;
        }
        let e = easing.apply(p) as f32;
        let away = 1.0 - e;
        // Motions that move become opaque by halfway, whatever their easing.
        let quick = (1.0 - (1.0 - (2.0 * p).min(1.0)).powi(3)) as f32;
        let mut look = RevealLook {
            alpha: e.clamp(0.0, 1.0),
            ..RevealLook::SHOWN
        };
        match motion {
            RevealMotion::Appear | RevealMotion::Fade => {}
            RevealMotion::Rise => look.rise = away * 0.35,
            RevealMotion::Blur => look.blur = away.max(0.0) * REVEAL_BLUR,
            RevealMotion::Pop => {
                look.alpha = quick;
                look.scale = e.max(0.0);
            }
            RevealMotion::Zoom => look.scale = 1.0 + REVEAL_ZOOM * away,
            RevealMotion::SlideLeft => {
                look.alpha = quick;
                look.shift = REVEAL_SLIDE * away;
            }
            RevealMotion::SlideRight => {
                look.alpha = quick;
                look.shift = -REVEAL_SLIDE * away;
            }
            RevealMotion::Drop => {
                look.alpha = quick;
                look.rise = -REVEAL_DROP * away;
            }
            RevealMotion::Collapse => look.spread = REVEAL_SPREAD * away,
        }
        look
    }
}

impl TextReveal {
    /// The look of unit `index` of `count` at `time` from the clip's start, for a clip
    /// `duration` long: opacity from 0 to 1 and how far below its place it still is, in em.
    /// [`TextReveal::look_at`] gives the whole look.
    pub fn unit_at(
        &self,
        index: usize,
        count: usize,
        time: Flicks,
        duration: Flicks,
    ) -> (f32, f32) {
        let look = self.look_at(index, count, time, duration);
        (look.alpha, look.rise)
    }

    /// The look of unit `index` of `count` at `time` from the clip's start, for a clip
    /// `duration` long.
    ///
    /// Units start in [`TextReveal::order`], one step apart; each moves for `1 / (1 -
    /// overlap)` steps (two by default), so the last finishes exactly at the reveal's end.
    /// `appear` shows each unit at once on its step. Leaving mirrors arriving in time, so
    /// the last unit to arrive leaves first, as typing backwards.
    pub fn look_at(
        &self,
        index: usize,
        count: usize,
        time: Flicks,
        duration: Flicks,
    ) -> RevealLook {
        let count = count.max(1);
        let span = self.duration.get().max(1) as f64;
        let (slot, last) = self.order.slot(index.min(count - 1), count);
        let progress_at = |t: f64, motion: RevealMotion| {
            if motion == RevealMotion::Appear {
                let step = span / (last + 1.0);
                return if t >= slot * step { 1.0 } else { 0.0 };
            }
            let overlap = self.overlap.clamp(0.0, 1.0);
            let own = span / (1.0 + last * (1.0 - overlap));
            let step = own * (1.0 - overlap);
            ((t - slot * step) / own).clamp(0.0, 1.0)
        };
        let arriving = progress_at(time.get() as f64, self.motion);
        let easing = |motion: RevealMotion| self.easing.unwrap_or(motion.natural_easing());
        if self.exit {
            let leaving_motion = self.exit_motion.unwrap_or(self.motion);
            let leaving = progress_at((duration - time).get() as f64, leaving_motion);
            if leaving < arriving {
                return RevealLook::of(leaving_motion, leaving, easing(leaving_motion));
            }
        }
        RevealLook::of(self.motion, arriving, easing(self.motion))
    }
}

/// What is wrong with a reveal's overlap and easing, if anything (its duration is checked
/// with the text).
pub(crate) fn reveal_problems(reveal: &TextReveal) -> Vec<&'static str> {
    let mut problems = Vec::new();
    if !(reveal.overlap.is_finite() && (0.0..=1.0).contains(&reveal.overlap)) {
        problems.push("a reveal's overlap must be from 0 to 1");
    }
    if let Some(RevealEasing::Bezier(points)) = reveal.easing
        && !(points.iter().all(|p| p.is_finite())
            && (0.0..=1.0).contains(&points[0])
            && (0.0..=1.0).contains(&points[2]))
    {
        problems.push("a reveal's Bezier easing needs finite points with x1 and x2 from 0 to 1");
    }
    problems
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
            ..TextReveal::default()
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

    fn reveal(motion: RevealMotion) -> TextReveal {
        TextReveal {
            unit: RevealUnit::Word,
            motion,
            duration: Flicks::from_seconds(1).unwrap_or_default(),
            ..TextReveal::default()
        }
    }

    fn ms(value: i64) -> Flicks {
        Flicks::from_millis(value)
    }

    #[test]
    fn overlap_spaces_units_and_the_last_ends_on_time() {
        let clip = ms(4000);
        // Three units with no overlap: one after another, each a third of the reveal.
        let apart = TextReveal {
            overlap: 0.0,
            easing: Some(RevealEasing::Linear),
            ..reveal(RevealMotion::Fade)
        };
        let alpha = |r: &TextReveal, i: usize, t: i64| r.look_at(i, 3, ms(t), clip).alpha;
        assert!((alpha(&apart, 0, 166) - 0.5).abs() < 0.01);
        assert_eq!(alpha(&apart, 1, 333), 0.0, "the second waits for the first");
        assert!((alpha(&apart, 1, 500) - 0.5).abs() < 0.01);
        assert_eq!(alpha(&apart, 2, 1000), 1.0);
        // Full overlap: all together.
        let together = TextReveal {
            overlap: 1.0,
            ..apart.clone()
        };
        assert_eq!(alpha(&together, 0, 500), alpha(&together, 2, 500));
        assert!((alpha(&together, 2, 500) - 0.5).abs() < 1e-6);
        // The default overlap keeps the timing reveals always had: two steps per unit.
        let classic = reveal(RevealMotion::Fade);
        assert_eq!(classic.overlap, DEFAULT_REVEAL_OVERLAP);
        let p: f64 = (500.0 - 250.0) / 500.0;
        let eased = 1.0 - (1.0 - p).powi(3);
        assert!((f64::from(alpha(&classic, 1, 500)) - eased).abs() < 1e-6);
        for overlap in [0.0, 0.3, 0.5, 0.9, 1.0] {
            let r = TextReveal {
                overlap,
                ..reveal(RevealMotion::Rise)
            };
            for i in 0..5 {
                assert_eq!(
                    r.look_at(i, 5, ms(1000), clip),
                    RevealLook::SHOWN,
                    "{overlap}"
                );
                assert_eq!(r.look_at(i, 5, ms(0), clip).alpha, 0.0);
            }
        }
    }

    #[test]
    fn orders_choose_which_unit_comes_first() {
        let clip = ms(4000);
        let first_shown = |order: RevealOrder, count: usize| -> Vec<usize> {
            let r = TextReveal {
                order,
                ..reveal(RevealMotion::Appear)
            };
            (0..count)
                .filter(|&i| r.look_at(i, count, ms(0), clip).alpha > 0.0)
                .collect()
        };
        assert_eq!(first_shown(RevealOrder::Forward, 5), [0]);
        assert_eq!(first_shown(RevealOrder::Backward, 5), [4]);
        assert_eq!(first_shown(RevealOrder::Center, 5), [2]);
        assert_eq!(first_shown(RevealOrder::Center, 4), [1, 2]);
        assert_eq!(first_shown(RevealOrder::Edges, 5), [0, 4]);
        let random = first_shown(RevealOrder::Random, 8);
        assert_eq!(random.len(), 1);
        assert_eq!(
            random,
            first_shown(RevealOrder::Random, 8),
            "the same every time"
        );
        // A shuffled order is a permutation: each unit has its own step.
        let r = TextReveal {
            order: RevealOrder::Random,
            ..reveal(RevealMotion::Appear)
        };
        let mut steps: Vec<_> = (0..8)
            .map(|i| (0..8).find(|&s| r.look_at(i, 8, ms(s * 125), clip).alpha > 0.0))
            .collect();
        steps.sort_unstable();
        steps.dedup();
        assert_eq!(steps.len(), 8, "{steps:?}");
        // Center and edges finish on time even when fewer steps are needed.
        for order in [RevealOrder::Center, RevealOrder::Edges] {
            let r = TextReveal {
                order,
                ..reveal(RevealMotion::Fade)
            };
            for i in 0..6 {
                assert_eq!(r.look_at(i, 6, ms(1000), clip).alpha, 1.0);
            }
        }
    }

    #[test]
    fn motions_move_scale_blur_and_spread_units() {
        let clip = ms(4000);
        let start = |motion| reveal(motion).look_at(0, 1, ms(0), clip);
        let middle = |motion| reveal(motion).look_at(0, 1, ms(400), clip);
        assert!(start(RevealMotion::Blur).blur > 0.19 && middle(RevealMotion::Blur).blur > 0.0);
        assert!((start(RevealMotion::Zoom).scale - 1.8).abs() < 1e-6);
        assert!(start(RevealMotion::SlideLeft).shift > 0.7, "from the right");
        assert!(
            start(RevealMotion::SlideRight).shift < -0.7,
            "from the left"
        );
        assert!(start(RevealMotion::Drop).rise < -0.9, "from above");
        assert!(start(RevealMotion::Collapse).spread > 0.5);
        assert!(start(RevealMotion::Pop).scale.abs() < 1e-6, "from nothing");
        // Pop overshoots (back) and drop bounces (bounce) without being asked.
        let pop = reveal(RevealMotion::Pop);
        let peak = (0..100)
            .map(|t| pop.look_at(0, 1, ms(t * 10), clip).scale)
            .fold(0.0f32, f32::max);
        assert!(peak > 1.05, "{peak}");
        let drop = reveal(RevealMotion::Drop);
        let heights: Vec<f32> = (0..=100)
            .map(|t| drop.look_at(0, 1, ms(t * 10), clip).rise)
            .collect();
        let landings = heights
            .windows(3)
            .filter(|w| w[1] > w[0] && w[1] > w[2])
            .count();
        assert!(landings >= 2, "bounces: {landings}");
        // Moving motions are opaque by halfway; a fade follows its easing.
        let half = reveal(RevealMotion::SlideLeft).look_at(0, 1, ms(500), clip);
        assert_eq!(half.alpha, 1.0);
        assert!(middle(RevealMotion::Fade).alpha < 1.0);
        // Every motion ends in place, and reports room for where it strays.
        for motion in [
            RevealMotion::Appear,
            RevealMotion::Fade,
            RevealMotion::Rise,
            RevealMotion::Blur,
            RevealMotion::Pop,
            RevealMotion::Zoom,
            RevealMotion::SlideLeft,
            RevealMotion::SlideRight,
            RevealMotion::Drop,
            RevealMotion::Collapse,
        ] {
            let done = reveal(motion).look_at(0, 1, ms(1000), clip);
            assert!((done.scale - 1.0).abs() < 1e-6, "{motion:?}");
            assert_eq!(
                (done.alpha, done.rise, done.shift, done.blur, done.spread),
                (1.0, 0.0, 0.0, 0.0, 0.0),
                "{motion:?}"
            );
            // A rise stays within the room below the baseline that every line has.
            let s = start(motion);
            let strays = s.rise.abs().max(s.shift.abs()).max(3.0 * s.blur);
            assert!(
                motion == RevealMotion::Rise || motion.reach() >= strays,
                "{motion:?}"
            );
        }
    }

    #[test]
    fn easings_run_from_start_to_end() {
        for easing in [
            RevealEasing::Linear,
            RevealEasing::EaseIn,
            RevealEasing::EaseOut,
            RevealEasing::EaseInOut,
            RevealEasing::Back,
            RevealEasing::Bounce,
            RevealEasing::Elastic,
            RevealEasing::Bezier([0.2, 0.0, 0.2, 1.0]),
        ] {
            assert!(easing.apply(0.0).abs() < 1e-9, "{easing:?}");
            assert!((easing.apply(1.0) - 1.0).abs() < 1e-9, "{easing:?}");
        }
        assert!(RevealEasing::EaseIn.apply(0.5) < 0.5 && RevealEasing::EaseOut.apply(0.5) > 0.5);
        assert!((RevealEasing::EaseInOut.apply(0.5) - 0.5).abs() < 1e-9);
        assert!((0..100).any(|i| RevealEasing::Back.apply(f64::from(i) / 100.0) > 1.05));
        assert!((0..100).any(|i| RevealEasing::Elastic.apply(f64::from(i) / 100.0) > 1.1));
        assert!((0..100).all(|i| RevealEasing::Bounce.apply(f64::from(i) / 100.0) <= 1.0));
    }

    #[test]
    fn exits_can_leave_another_way() {
        let clip = ms(4000);
        let leave = TextReveal {
            exit: true,
            exit_motion: Some(RevealMotion::Drop),
            ..reveal(RevealMotion::Fade)
        };
        let arriving = leave.look_at(0, 2, ms(300), clip);
        assert_eq!(
            (arriving.rise, arriving.shift),
            (0.0, 0.0),
            "arrives fading"
        );
        let leaving = leave.look_at(1, 2, ms(3600), clip);
        assert!(leaving.rise < 0.0, "leaves upwards, the way a drop arrives");
        assert!(leaving.alpha < 1.0);
        assert_eq!(
            leave.look_at(0, 2, clip, clip).alpha,
            0.0,
            "gone at the end"
        );
        assert_eq!(leave.look_at(0, 2, ms(2000), clip), RevealLook::SHOWN);
        // The last to arrive leaves first.
        assert!(
            leave.look_at(1, 2, ms(3500), clip).alpha < leave.look_at(0, 2, ms(3500), clip).alpha
        );
        // Unit tuples stay as they were for callers that only need opacity and rise.
        assert_eq!(
            leave.unit_at(1, 2, ms(3600), clip),
            (leaving.alpha, leaving.rise)
        );
    }

    #[test]
    fn new_reveal_fields_are_sparse_and_round_trip() {
        let plain = reveal(RevealMotion::Fade);
        let json = serde_json::to_value(&plain).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"unit": "word", "motion": "fade", "duration": ms(1000)})
        );
        let rich = TextReveal {
            overlap: 0.8,
            easing: Some(RevealEasing::Bezier([0.3, 1.4, 0.6, 1.0])),
            order: RevealOrder::Center,
            exit: true,
            exit_motion: Some(RevealMotion::Blur),
            ..reveal(RevealMotion::Collapse)
        };
        let json = serde_json::to_value(&rich).unwrap();
        assert_eq!(json["motion"], "collapse");
        assert_eq!(json["easing"]["bezier"][1], 1.4);
        assert_eq!(json["order"], "center");
        assert_eq!(json["exit_motion"], "blur");
        let back: TextReveal = serde_json::from_value(json).unwrap();
        assert_eq!(back, rich);
        let old: TextReveal = serde_json::from_str(
            r#"{"unit": "line", "motion": "rise", "duration": 705600000, "exit": true}"#,
        )
        .unwrap();
        assert_eq!(old.overlap, DEFAULT_REVEAL_OVERLAP);
        assert_eq!(
            (old.easing, old.order, old.exit_motion),
            (None, RevealOrder::Forward, None)
        );
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
                ..TextReveal::default()
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
