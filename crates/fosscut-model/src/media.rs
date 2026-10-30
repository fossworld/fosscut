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
    /// Folders of the media bin as paths such as `B-roll/Day 1`, sorted; each parent is
    /// listed too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
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
    /// A 3D color lookup table (`.cube`), used by the `color.lut` effect.
    Lut,
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
    /// The bin folder it is in; the top level when omitted. Organizing media never changes
    /// its identity (ADR-0004, item 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

/// True for a folder path of non-empty names separated by `/`, without surrounding spaces.
pub fn valid_folder(path: &str) -> bool {
    !path.is_empty()
        && path
            .split('/')
            .all(|name| !name.is_empty() && name.trim() == name)
}

/// The folder and its parents, `a/b/c` giving `a`, `a/b` and `a/b/c`.
pub fn folder_and_parents(path: &str) -> Vec<String> {
    let names: Vec<&str> = path.split('/').collect();
    (1..=names.len()).map(|n| names[..n].join("/")).collect()
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
    /// Size of a LUT asset's table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut: Option<LutInfo>,
}

/// A LUT file's table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LutInfo {
    /// Entries per axis, for example 33.
    pub size: u32,
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
    /// Display rotation from the container's display matrix in degrees counterclockwise,
    /// as FFmpeg reports it (for example -90 for portrait phone video).
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
