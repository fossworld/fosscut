//! FossCut project documents (ADR-0004).
//!
//! A project is a directory bundle `Name.fosscut/`:
//!
//! ```text
//! project.json          manifest: format version, id, name, revision, timelines
//! media.json            asset table, one record per source file
//! timelines/<id>.json   one document per timeline
//! history/ops.jsonl     append-only operation log (written by fosscut-core)
//! ```
//!
//! Documents are sparse (defaults are omitted), references are typed, and nothing
//! machine-specific is stored besides path hints for media. All file access goes through
//! [`ProjectStore`], so the model builds for `wasm32-unknown-unknown` and can be backed
//! by browser storage or a server later.

#![forbid(unsafe_code)]

mod animation;
mod bundle;
mod caption;
mod effect;
mod error;
mod id;
mod media;
mod project;
mod store;
mod timeline;
mod validate;

pub use animation::{
    Applies, BLEND_MODES, CHANNEL_MODES, Interpolation, KeyValue, Keyframe, Property, PropertyKind,
    clip_properties, clip_property, evaluate,
};
pub use bundle::{DocumentKey, MANIFEST_PATH, MEDIA_PATH, OPS_LOG_PATH, Project, TIMELINES_DIR};
pub use caption::{
    Animated, Animator, CaptionBox, CaptionContent, CaptionLine, CaptionStyle, CaptionWord,
    FontWeight, PageFade, PagePop, Placement, Stroke, WordEmphasis, WordHighlight, WordKaraoke,
    WordPop, WordReveal, safe_margins,
};
pub use effect::{
    AssetRef, Effect, EffectCategory, EffectSpec, HSL_BANDS, HSL_PARAMS, ParamDefault, ParamKind,
    ParamSpec, ParamValue, effect_path, effect_type, effect_types,
};
pub use error::ModelError;
pub use id::{Id, IdKind};
pub use media::{
    Asset, AssetKind, AudioInfo, ContentIdentity, LutInfo, Media, MediaInfo, PathHints, VideoInfo,
};
pub use project::{Manifest, TimelineEntry};
pub use store::{DirStore, MemoryStore, ProjectStore, StoreError};
pub use timeline::{
    Align, BlendMode, ChannelMode, Clip, ClipAudio, ClipContent, Color, Crop, TextContent,
    TextStyle, Timeline, TimelineFormat, Track, TrackKind, Transform,
};
pub use validate::{Issue, validate};

/// Current project format version (semantic version, ADR-0004). 0.2.0 added caption
/// tracks and caption clips (ADR-0021); 0.3.0 added keyframes, crop, stretch, flips, blend
/// modes, audio fades, pan and channels, and deactivated clips (ADR-0011); 0.4.0 added
/// effects, adjustment clips and LUT assets (ADR-0011).
pub const FORMAT_VERSION: &str = "0.4.0";

/// Older format versions this build reads. Each is a subset of the current format, so it
/// loads unchanged and is saved as [`FORMAT_VERSION`].
pub const READABLE_VERSIONS: &[&str] = &["0.1.0", "0.2.0", "0.3.0"];

/// JSON Schemas of the stored documents, keyed by file stem, for `cargo xtask schemas`.
pub fn document_schemas() -> Vec<(&'static str, schemars::Schema)> {
    vec![
        ("project", schemars::schema_for!(Manifest)),
        ("media", schemars::schema_for!(Media)),
        ("timeline", schemars::schema_for!(Timeline)),
    ]
}

pub(crate) fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}
