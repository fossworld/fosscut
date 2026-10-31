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
mod mask;
mod media;
mod project;
mod shape;
mod speed;
mod store;
mod text;
mod timeline;
mod transition;
mod validate;

pub use animation::{
    Applies, BLEND_MODES, CHANNEL_MODES, Interpolation, KeyValue, Keyframe, Property, PropertyKind,
    clip_properties, clip_property, evaluate,
};
pub use bundle::{DocumentKey, MANIFEST_PATH, MEDIA_PATH, OPS_LOG_PATH, Project, TIMELINES_DIR};
pub use caption::{
    Animated, Animator, CaptionBox, CaptionContent, CaptionLine, CaptionStyle, CaptionWord,
    FontWeight, PageFade, PagePop, Placement, Stroke, TextBox, TranslationStyle, WordEmphasis,
    WordHighlight, WordKaraoke, WordPop, WordReveal, safe_margins,
};
pub use effect::{
    AssetRef, Effect, EffectCategory, EffectSpec, HSL_BANDS, HSL_PARAMS, ParamDefault, ParamKind,
    ParamSpec, ParamValue, effect_path, effect_type, effect_types,
};
pub use error::ModelError;
pub use id::{Id, IdKind};
pub use mask::{BrushStroke, MASK_PROPERTIES, Mask, MaskMode, MaskShape, PathPoint, mask_path};
pub use media::{
    Asset, AssetKind, AudioInfo, ContentIdentity, FontInfo, LutInfo, Media, MediaInfo, PathHints,
    VideoInfo, folder_and_parents, valid_folder,
};
pub use project::{Manifest, TimelineEntry, TimelineRole};
pub use shape::{Gradient, Paint, ShapeContent, ShapeOutline};
pub use speed::{
    Interpolate, MAX_RATE, MIN_RATE, Pitch, SPEED_PRESETS, Speed, SpeedPoint, SpeedPreset,
    speed_preset,
};
pub use store::{DirStore, MemoryStore, ProjectStore, StoreError};
pub use text::{
    BUNDLED_FONT, DEFAULT_LINE_HEIGHT, Glow, RevealMotion, RevealUnit, Shadow, TextCase,
    TextContent, TextPreset, TextReveal, TextSpan, TextStyle,
};
pub use timeline::{
    Align, BlendMode, CanvasFill, ChannelMode, Clip, ClipAudio, ClipContent, Color, Crop, Marker,
    Timeline, TimelineFormat, Track, TrackKind, Transform,
};
pub use transition::{
    ANIMATION_PRESETS, Animation, AnimationDelta, ClipAnimations, Transition, TransitionSpec,
    animate, transition_type, transition_types,
};
pub use validate::{Issue, shape_issues, text_issues, text_style_issues, validate};

/// Current project format version (semantic version, ADR-0004). 0.2.0 added caption
/// tracks and caption clips (ADR-0021); 0.3.0 added keyframes, crop, stretch, flips, blend
/// modes, audio fades, pan and channels, and deactivated clips (ADR-0011); 0.4.0 added
/// effects, adjustment clips and LUT assets (ADR-0011); 0.5.0 added masks and the color
/// parameter kind (chroma key); 0.6.0 added transitions and clip animations; 0.7.0 added
/// clip speed (ADR-0023); 0.8.0 added markers, clip groups and detached audio (ADR-0024);
/// 0.9.0 added media folders; 0.10.0 added compound clips (ADR-0025); 0.11.0 added text
/// spans, reveals, spacing, outlines, boxes, shadows and glows, text presets and font assets
/// (ADR-0027), audio effects (ADR-0026) and caption translations; 0.12.0 added shape clips
/// (ADR-0028); 0.13.0 added multicam angles on compound clips (ADR-0029); 0.14.0 added
/// canvas fills behind video and image clips.
pub const FORMAT_VERSION: &str = "0.14.0";

/// Older format versions this build reads. Each is a subset of the current format, so it
/// loads unchanged and is saved as [`FORMAT_VERSION`].
pub const READABLE_VERSIONS: &[&str] = &[
    "0.1.0", "0.2.0", "0.3.0", "0.4.0", "0.5.0", "0.6.0", "0.7.0", "0.8.0", "0.9.0", "0.10.0",
    "0.11.0", "0.12.0", "0.13.0",
];

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
