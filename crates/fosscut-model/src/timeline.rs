use fosscut_time::{Flicks, Rate};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::caption::{CaptionContent, CaptionStyle};
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
    /// Opacity from 0 to 1.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f64,
    /// Linear audio gain (1 is unchanged, 0 is silent).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub volume: f64,
}

impl Clip {
    /// Timeline position just after the last frame.
    pub fn end(&self) -> Flicks {
        self.start + self.duration
    }
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
}

impl ClipContent {
    /// The referenced asset, if any.
    pub fn asset(&self) -> Option<&Id> {
        match self {
            ClipContent::Video { asset, .. }
            | ClipContent::Audio { asset, .. }
            | ClipContent::Image { asset } => Some(asset),
            ClipContent::Text(_) | ClipContent::Caption(_) => None,
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
    /// Clockwise rotation in degrees.
    #[serde(default, skip_serializing_if = "is_default")]
    pub rotation: f64,
}

impl Default for Transform {
    fn default() -> Transform {
        Transform {
            position: [0.0, 0.0],
            scale: 1.0,
            rotation: 0.0,
        }
    }
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
