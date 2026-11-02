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

    /// Every font family text can use: the bundled one and those of the font assets, sorted.
    pub fn font_families(&self) -> Vec<String> {
        let mut families: Vec<String> = std::iter::once(crate::BUNDLED_FONT.to_string())
            .chain(
                self.assets
                    .iter()
                    .filter_map(|a| a.info.font.as_ref())
                    .flat_map(|f| f.families.iter().cloned()),
            )
            .collect();
        families.sort();
        families.dedup();
        families
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
    /// A font file (`.ttf`, `.otf`, `.ttc`) whose families text can use (ADR-0027).
    Font,
    /// A vector drawing (`.svg`, ADR-0036): shown as an image, drawn sharp at any size, and
    /// split into editable shape clips by `vector.explode`.
    Vector,
    /// A web composition (ADR-0039): an HTML page, with the files it loads, copied into the
    /// bundle's `web/` folder and rendered frame by frame in a headless browser. Video clips
    /// show it like a video; its size and frame rate are in `video`, its length in
    /// `duration`.
    Web,
    /// A 3D model (`.glb`, or `.gltf` with its buffers and textures, ADR-0040), copied
    /// into the bundle's `models/` folder by content and shown by 3D object clips.
    Model,
}

impl AssetKind {
    /// True for the kinds video clips show from a source position: video files and web
    /// compositions (ADR-0039).
    pub fn is_video(self) -> bool {
        matches!(self, AssetKind::Video | AssetKind::Web)
    }
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
    /// The families and faces in a font asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<FontInfo>,
    /// What a vector drawing holds; its size in pixels is in `video`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<VectorInfo>,
    /// What a 3D model holds (ADR-0040).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelInfo>,
}

/// What a 3D model file holds (ADR-0040), probed at import.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelInfo {
    /// Meshes its default scene draws.
    pub meshes: u32,
    /// Triangles drawn.
    pub triangles: u64,
    /// Materials.
    pub materials: u32,
    /// Images its materials use.
    pub textures: u32,
    /// The size of its bounds `[x, y, z]` in its own units (metres in glTF), y up.
    pub size: [f64; 3],
    /// The file's own note of the tool that wrote it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    /// The file's own copyright line, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
}

/// What a vector drawing (`.svg`) holds (ADR-0036).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VectorInfo {
    /// Its view box `[x, y, width, height]` in its own units.
    pub view_box: [f64; 4],
    /// How many path elements it draws, each a shape clip when exploded.
    pub paths: u32,
}

/// What a font file holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FontInfo {
    /// Family names, sorted, as text styles name them.
    pub families: Vec<String>,
    /// Number of faces (a collection holds several).
    pub faces: u32,
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
