use fosscut_time::{Flicks, Rate};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Id, is_default};

/// The media table: one immutable record per source file (`media.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Media {
    /// Imported assets in import order.
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Media {
    /// Finds an asset by id.
    pub fn get(&self, id: &Id) -> Option<&Asset> {
        self.assets.iter().find(|a| &a.id == id)
    }
}

/// What kind of media an asset is.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    /// Moving picture, possibly with audio.
    Video,
    /// Sound only.
    Audio,
    /// A still image.
    Image,
}

/// A source media file referenced by clips. Splitting a clip never copies an asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Asset {
    /// Asset id (`asset_…`).
    pub id: Id,
    /// Media kind.
    pub kind: AssetKind,
    /// Display name, usually the file name.
    pub name: String,
    /// Where to find the file.
    pub path: PathHints,
    /// Content identity used to relink moved files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<ContentIdentity>,
    /// Stream information recorded at import.
    #[serde(default, skip_serializing_if = "is_default")]
    pub info: MediaInfo,
}

impl Asset {
    /// True when the asset has an audio stream.
    pub fn has_audio(&self) -> bool {
        self.kind == AssetKind::Audio || self.info.audio.is_some()
    }
}

/// Path hints for a source file. Relative paths are relative to the project bundle.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PathHints {
    /// Path relative to the `.fosscut` bundle directory, with `/` separators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative: Option<String>,
    /// Absolute path at import time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute: Option<String>,
}

/// Identity of a file's content: its size and a BLAKE3 hash of its first and last MiB.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContentIdentity {
    /// File size in bytes.
    pub size: u64,
    /// Hex BLAKE3 digest of the size, the first MiB and the last MiB of the file.
    pub sample_blake3: String,
}

/// Stream information probed at import.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MediaInfo {
    /// Playable duration; absent for still images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Flicks>,
    /// Container format name, for example `mov,mp4,m4a,3gp,3g2,mj2`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    /// Primary video stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoInfo>,
    /// Primary audio stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioInfo>,
}

/// Video stream properties.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VideoInfo {
    /// Coded width in pixels.
    pub width: u32,
    /// Coded height in pixels.
    pub height: u32,
    /// Frame rate, when constant and known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<Rate>,
    /// Codec name, for example `h264`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    /// Display rotation in degrees from container metadata.
    #[serde(default, skip_serializing_if = "is_default")]
    pub rotation: i32,
}

/// Audio stream properties.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AudioInfo {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u16,
    /// Codec name, for example `aac`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
}
