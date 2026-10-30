use std::collections::BTreeMap;

use fosscut_time::{Flicks, Rate};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::animation::{KeyValue, Keyframe, evaluate};
use crate::caption::{CaptionContent, CaptionStyle};
use crate::effect::Effect;
use crate::{Id, is_default};

/// A timeline document (`timelines/<id>.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Timeline {
    /// Timeline id (`timeline_…`).
    pub id: Id,
    /// Timeline name.
    pub name: String,
    /// Canvas size, frame rate and audio sample rate.
    pub format: TimelineFormat,
    /// Tracks from bottom to top: later tracks draw over earlier ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<Track>,
}

impl Timeline {
    /// End of the last clip on any track.
    pub fn duration(&self) -> Flicks {
        self.tracks
            .iter()
            .map(Track::end)
            .max()
            .unwrap_or(Flicks::ZERO)
    }

    /// Finds a track by id.
    pub fn track(&self, id: &Id) -> Option<&Track> {
        self.tracks.iter().find(|t| &t.id == id)
    }

    /// Finds a track by id for modification.
    pub fn track_mut(&mut self, id: &Id) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| &t.id == id)
    }

    /// Finds a clip and the index of its track.
    pub fn find_clip(&self, id: &Id) -> Option<(usize, &Clip)> {
        self.tracks
            .iter()
            .enumerate()
            .find_map(|(index, track)| track.clips.iter().find(|c| &c.id == id).map(|c| (index, c)))
    }
}

/// Output format of a timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TimelineFormat {
    /// Canvas width in pixels.
    pub width: u32,
    /// Canvas height in pixels.
    pub height: u32,
    /// Frame rate; edits snap to its frames.
    pub rate: Rate,
    /// Audio sample rate in Hz.
    #[serde(
        default = "default_sample_rate",
        skip_serializing_if = "is_default_sample_rate"
    )]
    pub sample_rate: u32,
}

fn default_sample_rate() -> u32 {
    48_000
}

fn is_default_sample_rate(value: &u32) -> bool {
    *value == default_sample_rate()
}

/// What a track holds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// Video and image clips (their audio plays too).
    Video,
    /// Audio clips.
    Audio,
    /// Text clips: titles.
    Text,
    /// Caption clips: pages of timed words, with the look in the track (ADR-0021).
    Caption,
}

/// A track: clips ordered by start time that never overlap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Track {
    /// Track id (`track_…`).
    pub id: Id,
    /// What the track holds.
    pub kind: TrackKind,
    /// Optional display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Silences the track.
    #[serde(default, skip_serializing_if = "is_default")]
    pub muted: bool,
    /// Hides the track's pictures.
    #[serde(default, skip_serializing_if = "is_default")]
    pub hidden: bool,
    /// Prevents edits to the track.
    #[serde(default, skip_serializing_if = "is_default")]
    pub locked: bool,
    /// How the captions of a caption track look and move; defaults when omitted. Only
    /// caption tracks have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_style: Option<CaptionStyle>,
    /// Clips sorted by start time, without overlaps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<Clip>,
}

impl Track {
    /// End of the last clip, or zero for an empty track.
    pub fn end(&self) -> Flicks {
        self.clips.last().map(Clip::end).unwrap_or(Flicks::ZERO)
    }
}

/// A clip on a track. The content kind decides which fields apply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Clip {
    /// Clip id (`clip_…`).
    pub id: Id,
    /// Timeline position of the first frame.
    pub start: Flicks,
    /// Length on the timeline.
    pub duration: Flicks,
    /// Media or text content.
    #[serde(flatten)]
    pub content: ClipContent,
    /// Placement on the canvas; identity when omitted.
    #[serde(default, skip_serializing_if = "is_default")]
    pub transform: Transform,
    /// Edges of the picture removed; nothing when omitted.
    #[serde(default, skip_serializing_if = "is_default")]
    pub crop: Crop,
    /// Opacity from 0 to 1.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f64,
    /// How the picture combines with the layers below it (normal when omitted).
    #[serde(default, skip_serializing_if = "is_default")]
    pub blend: BlendMode,
    /// Linear audio gain (1 is unchanged, 0 is silent).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub volume: f64,
    /// Fades, pan and channels of the clip's sound; unchanged when omitted.
    #[serde(default, skip_serializing_if = "is_default")]
    pub audio: ClipAudio,
    /// False deactivates the clip: it keeps its place but is neither drawn nor heard.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub enabled: bool,
    /// Effects on the clip's picture, applied in order before placement. On an adjustment
    /// clip they apply to everything below it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Keyframes by property path (`transform.position`, `opacity`, ...), each list sorted
    /// by time with at most one keyframe per time. A property with keyframes takes its
    /// value from them, not from its field.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keyframes: BTreeMap<String, Vec<Keyframe>>,
}

impl Clip {
    /// A clip with default placement, sound and no keyframes.
    pub fn new(id: Id, start: Flicks, duration: Flicks, content: ClipContent) -> Clip {
        Clip {
            id,
            start,
            duration,
            content,
            transform: Transform::default(),
            crop: Crop::default(),
            opacity: 1.0,
            blend: BlendMode::default(),
            volume: 1.0,
            audio: ClipAudio::default(),
            enabled: true,
            effects: Vec::new(),
            keyframes: BTreeMap::new(),
        }
    }

    /// The effect with this id.
    pub fn effect(&self, id: &Id) -> Option<&Effect> {
        self.effects.iter().find(|e| &e.id == id)
    }

    /// Timeline position just after the last frame.
    pub fn end(&self) -> Flicks {
        self.start + self.duration
    }

    /// The value of an animatable property at `offset` from the clip's start: from its
    /// keyframes when it has any, otherwise `None`.
    pub fn keyed(&self, path: &str, offset: Flicks) -> Option<KeyValue> {
        self.keyframes.get(path).and_then(|k| evaluate(k, offset))
    }

    fn number_at(&self, path: &str, offset: Flicks, value: f64) -> f64 {
        self.keyed(path, offset)
            .and_then(KeyValue::number)
            .unwrap_or(value)
    }

    fn pair_at(&self, path: &str, offset: Flicks, value: [f64; 2]) -> [f64; 2] {
        self.keyed(path, offset)
            .and_then(KeyValue::pair)
            .unwrap_or(value)
    }

    /// The placement at `offset` from the clip's start, with keyframes applied.
    pub fn transform_at(&self, offset: Flicks) -> Transform {
        let t = &self.transform;
        Transform {
            position: self.pair_at("transform.position", offset, t.position),
            scale: self.number_at("transform.scale", offset, t.scale),
            stretch: self.pair_at("transform.stretch", offset, t.stretch),
            rotation: self.number_at("transform.rotation", offset, t.rotation),
            flip_horizontal: t.flip_horizontal,
            flip_vertical: t.flip_vertical,
        }
    }

    /// The crop at `offset`, with keyframes applied.
    pub fn crop_at(&self, offset: Flicks) -> Crop {
        let c = &self.crop;
        Crop {
            left: self.number_at("crop.left", offset, c.left),
            right: self.number_at("crop.right", offset, c.right),
            top: self.number_at("crop.top", offset, c.top),
            bottom: self.number_at("crop.bottom", offset, c.bottom),
        }
    }

    /// The opacity at `offset`, with keyframes applied, clamped to 0 to 1.
    pub fn opacity_at(&self, offset: Flicks) -> f64 {
        self.number_at("opacity", offset, self.opacity)
            .clamp(0.0, 1.0)
    }

    /// The linear gain at `offset`, with keyframes applied, never below 0.
    pub fn volume_at(&self, offset: Flicks) -> f64 {
        self.number_at("volume", offset, self.volume).max(0.0)
    }

    /// The pan at `offset`, with keyframes applied, clamped to -1 to 1.
    pub fn pan_at(&self, offset: Flicks) -> f64 {
        self.number_at("audio.pan", offset, self.audio.pan)
            .clamp(-1.0, 1.0)
    }

    /// Gain of the clip's fades at `offset`: rising from 0 over the fade in, falling to 0
    /// over the fade out, on a quarter sine (constant power), 1 in between. When the fades
    /// overlap, the lower gain wins.
    pub fn fade_at(&self, offset: Flicks) -> f64 {
        let ramp = |position: Flicks, length: Flicks| -> f64 {
            if !length.is_positive() {
                return 1.0;
            }
            let t = (position.get() as f64 / length.get() as f64).clamp(0.0, 1.0);
            (t * std::f64::consts::FRAC_PI_2).sin()
        };
        let fade_in = ramp(offset, self.audio.fade_in);
        let fade_out = ramp(self.duration - offset, self.audio.fade_out);
        fade_in.min(fade_out)
    }

    /// Shifts every keyframe by `delta`, as when the clip's start moves over its content
    /// (a trim of the start edge, or the right half of a split).
    pub fn shift_keyframes(&mut self, delta: Flicks) {
        for keys in self.keyframes.values_mut() {
            for key in keys {
                key.time = key.time + delta;
            }
        }
        for effect in &mut self.effects {
            effect.shift_keyframes(delta);
        }
    }
}

fn yes() -> bool {
    true
}

fn is_yes(value: &bool) -> bool {
    *value
}

fn one() -> f64 {
    1.0
}

fn is_one(value: &f64) -> bool {
    *value == 1.0
}

/// The content of a clip, tagged by `kind`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClipContent {
    /// A section of a video asset.
    Video {
        /// The asset (`asset_…`).
        asset: Id,
        /// Source position of the clip's first frame.
        #[serde(default, skip_serializing_if = "is_default")]
        source_in: Flicks,
    },
    /// A section of an audio asset (or the audio of a video asset).
    Audio {
        /// The asset (`asset_…`).
        asset: Id,
        /// Source position of the clip's first sample.
        #[serde(default, skip_serializing_if = "is_default")]
        source_in: Flicks,
    },
    /// A still image held for the clip duration.
    Image {
        /// The asset (`asset_…`).
        asset: Id,
    },
    /// A text title.
    Text(TextContent),
    /// A caption page: lines of timed words (ADR-0021).
    Caption(CaptionContent),
    /// An adjustment layer: its effects apply to everything below it while it lasts.
    Adjustment,
}

impl ClipContent {
    /// The referenced asset, if any.
    pub fn asset(&self) -> Option<&Id> {
        match self {
            ClipContent::Video { asset, .. }
            | ClipContent::Audio { asset, .. }
            | ClipContent::Image { asset } => Some(asset),
            ClipContent::Text(_) | ClipContent::Caption(_) | ClipContent::Adjustment => None,
        }
    }

    /// Source in-point for time-based media.
    pub fn source_in(&self) -> Option<Flicks> {
        match self {
            ClipContent::Video { source_in, .. } | ClipContent::Audio { source_in, .. } => {
                Some(*source_in)
            }
            _ => None,
        }
    }

    /// The content kind as used in JSON (`video`, `audio`, `image`, `text`, `caption`).
    pub fn kind_name(&self) -> &'static str {
        match self {
            ClipContent::Video { .. } => "video",
            ClipContent::Audio { .. } => "audio",
            ClipContent::Image { .. } => "image",
            ClipContent::Text(_) => "text",
            ClipContent::Caption(_) => "caption",
            ClipContent::Adjustment => "adjustment",
        }
    }
}

/// Placement of a clip on the canvas.
///
/// At identity the picture is fitted inside the canvas and centred.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Transform {
    /// Offset of the clip centre from the canvas centre, as fractions of canvas width
    /// and height; x grows to the right, y grows downwards.
    #[serde(default, skip_serializing_if = "is_default")]
    pub position: [f64; 2],
    /// Uniform scale relative to the fitted size (1 when omitted).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub scale: f64,
    /// Extra width and height factors on top of `scale`, for non-uniform scaling
    /// (`[1, 1]` when omitted).
    #[serde(default = "ones", skip_serializing_if = "is_ones")]
    pub stretch: [f64; 2],
    /// Clockwise rotation in degrees.
    #[serde(default, skip_serializing_if = "is_default")]
    pub rotation: f64,
    /// Mirrors the picture left to right.
    #[serde(default, skip_serializing_if = "is_default")]
    pub flip_horizontal: bool,
    /// Mirrors the picture top to bottom.
    #[serde(default, skip_serializing_if = "is_default")]
    pub flip_vertical: bool,
}

impl Default for Transform {
    fn default() -> Transform {
        Transform {
            position: [0.0, 0.0],
            scale: 1.0,
            stretch: ones(),
            rotation: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
        }
    }
}

impl Transform {
    /// Width and height factors relative to the fitted size: `scale` times `stretch`.
    pub fn size_factors(&self) -> [f64; 2] {
        [self.scale * self.stretch[0], self.scale * self.stretch[1]]
    }
}

fn ones() -> [f64; 2] {
    [1.0, 1.0]
}

fn is_ones(value: &[f64; 2]) -> bool {
    *value == ones()
}

/// Edges removed from a clip's picture, as fractions of its width and height before any
/// flip. What remains keeps its size and place on the canvas.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Crop {
    /// Fraction of the width removed on the left.
    #[serde(default, skip_serializing_if = "is_default")]
    pub left: f64,
    /// Fraction of the width removed on the right.
    #[serde(default, skip_serializing_if = "is_default")]
    pub right: f64,
    /// Fraction of the height removed at the top.
    #[serde(default, skip_serializing_if = "is_default")]
    pub top: f64,
    /// Fraction of the height removed at the bottom.
    #[serde(default, skip_serializing_if = "is_default")]
    pub bottom: f64,
}

impl Crop {
    /// The kept part `[left, top, right, bottom]` as fractions from the top-left corner,
    /// each edge clamped to 0 to 1 and never past the opposite edge.
    pub fn kept(&self) -> [f64; 4] {
        let edge = |v: f64| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let left = edge(self.left);
        let top = edge(self.top);
        let right = (1.0 - edge(self.right)).max(left);
        let bottom = (1.0 - edge(self.bottom)).max(top);
        [left, top, right, bottom]
    }
}

/// How a clip's picture combines with the layers below it. Every mode except `normal`
/// works on display-encoded (sRGB) values, as designers expect, and then mixes by the
/// clip's opacity (ADR-0011, item 4).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    /// Covers what is below.
    #[default]
    Normal,
    /// Darkens: below times the picture.
    Multiply,
    /// Lightens: the inverse of multiplying the inverses.
    Screen,
    /// Multiply in the shadows of what is below and screen in its highlights.
    Overlay,
    /// The darker of the two, per channel.
    Darken,
    /// The lighter of the two, per channel.
    Lighten,
    /// Brightens what is below to reflect the picture.
    ColorDodge,
    /// Darkens what is below to reflect the picture.
    ColorBurn,
    /// Overlay with the roles swapped: the picture decides multiply or screen.
    HardLight,
    /// A gentler hard light.
    SoftLight,
    /// Sum minus one: darkens more than multiply.
    LinearBurn,
    /// Sum (linear dodge), clipped at white.
    Add,
    /// Absolute difference.
    Difference,
    /// Like difference with lower contrast.
    Exclusion,
}

/// How a clip's sound is shaped: fades, pan and channels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClipAudio {
    /// Fade in from silence over this long at the clip's start.
    #[serde(default, skip_serializing_if = "is_default")]
    pub fade_in: Flicks,
    /// Fade out to silence over this long at the clip's end.
    #[serde(default, skip_serializing_if = "is_default")]
    pub fade_out: Flicks,
    /// Stereo balance: -1 left, 0 centre (both sides unchanged), 1 right.
    #[serde(default, skip_serializing_if = "is_default")]
    pub pan: f64,
    /// Which source channels play.
    #[serde(default, skip_serializing_if = "is_default")]
    pub channels: ChannelMode,
}

/// Which channels of a clip's sound play where (fill channel).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMode {
    /// As recorded.
    #[default]
    Stereo,
    /// The left channel fills both.
    Left,
    /// The right channel fills both.
    Right,
    /// Both channels mixed to one, played on both.
    Mono,
}

/// Text content with a single style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextContent {
    /// The text; `\n` separates lines.
    pub text: String,
    /// Appearance; defaults when omitted.
    #[serde(default, skip_serializing_if = "is_default")]
    pub style: TextStyle,
}

/// Text appearance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextStyle {
    /// Font family; the default sans-serif font when omitted.
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
    /// Horizontal alignment of lines.
    #[serde(default, skip_serializing_if = "is_default")]
    pub align: Align,
}

fn default_text_size() -> f64 {
    0.08
}

impl Default for TextStyle {
    fn default() -> TextStyle {
        TextStyle {
            font: None,
            size: default_text_size(),
            color: Color::default(),
            bold: false,
            italic: false,
            align: Align::default(),
        }
    }
}

/// Horizontal text alignment.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    /// Left aligned.
    Left,
    /// Centred.
    #[default]
    Center,
    /// Right aligned.
    Right,
}

/// An sRGB colour as `#RRGGBB` or `#RRGGBBAA`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color(String);

impl Color {
    /// Parses `#RRGGBB` or `#RRGGBBAA` (case-insensitive), normalising to uppercase.
    pub fn parse(text: &str) -> Result<Color, String> {
        let hex = text.strip_prefix('#').unwrap_or("");
        if (hex.len() == 6 || hex.len() == 8) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Color(format!("#{}", hex.to_ascii_uppercase())))
        } else {
            Err(format!(
                "invalid colour {text:?}: expected #RRGGBB or #RRGGBBAA"
            ))
        }
    }

    /// The colour string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Color {
    fn default() -> Color {
        Color("#FFFFFF".to_string())
    }
}

impl TryFrom<String> for Color {
    type Error = String;
    fn try_from(value: String) -> Result<Color, String> {
        Color::parse(&value)
    }
}

impl From<Color> for String {
    fn from(color: Color) -> String {
        color.0
    }
}

impl JsonSchema for Color {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{8})$",
            "description": "sRGB colour #RRGGBB or #RRGGBBAA."
        })
    }
}
