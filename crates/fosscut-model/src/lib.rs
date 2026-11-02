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
mod camera;
mod caption;
mod effect;
mod error;
mod id;
pub mod light;
mod mask;
mod media;
mod model3d;
mod particles;
mod project;
mod shape;
mod speed;
mod store;
mod text;
mod timeline;
mod transition;
mod validate;
mod vector;
pub mod voice_filters;

pub use animation::{
    Applies, BLEND_MODES, CHANNEL_MODES, Interpolation, KeyValue, Keyframe, Property, PropertyKind,
    clip_properties, clip_property, evaluate,
};
pub use bundle::{DocumentKey, MANIFEST_PATH, MEDIA_PATH, OPS_LOG_PATH, Project, TIMELINES_DIR};
pub use camera::{
    CAMERA_ANIMATABLE, CameraContent, DEFAULT_APERTURE, DEFAULT_FIELD_OF_VIEW, DEFAULT_MAX_BLUR,
    FIELD_OF_VIEW_RANGE, MAX_BLUR_LIMIT, camera_issues,
};
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
    Asset, AssetKind, AudioInfo, ContentIdentity, FontInfo, LutInfo, Media, MediaInfo, ModelInfo,
    PathHints, VectorInfo, VideoInfo, folder_and_parents, valid_folder,
};
pub use model3d::{
    AssetObject, BlocksObject, BoxObject, CylinderObject, Faces, Floor, Light, MAX_GRID, MAX_SIZE,
    MAX_TEXT_CHARS, MODEL_PROPERTIES, Material, ModelContent, ModelObject, ModelProperty,
    PathObject, PlaneObject, SphereObject, TextObject, TorusObject, model_issues,
};
pub use particles::{
    Aim, Burst, ColorBy, Emitter, EmitterShape, MAX_BURST, MAX_BURSTS, MAX_LIFE, MAX_PARTICLE_RATE,
    MAX_PARTICLES, MAX_PREWARM_SECONDS, MAX_STOPS, MIN_LIFE, PARTICLE_PROPERTIES, ParticleBlend,
    ParticleContent, ParticleProperty, Sprite, SpriteKind, particle_issues,
};
pub use project::{Manifest, TimelineEntry, TimelineRole};
pub use shape::{
    Contour, FillRule, Gradient, LineCap, LineJoin, Paint, ShapeContent, ShapeOutline, ShapeStroke,
};
pub use speed::{
    Interpolate, MAX_RATE, MIN_RATE, Pitch, SPEED_PRESETS, Speed, SpeedPoint, SpeedPreset,
    speed_preset,
};
pub use store::{DirStore, MemoryStore, ProjectStore, StoreError};
pub use text::{
    BUNDLED_FONT, DEFAULT_LINE_HEIGHT, DEFAULT_REVEAL_OVERLAP, Glow, RevealEasing, RevealLook,
    RevealMotion, RevealOrder, RevealUnit, Shadow, TextCase, TextContent, TextPreset, TextReveal,
    TextSpan, TextStyle,
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
pub use vector::{
    FittedPath, Follow, FollowPlacement, fit_path, follow_placement, parse_path_data, path_length,
    point_along, trim_path,
};

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
/// canvas fills behind video and image clips; 0.15.0 added optical-flow slow motion
/// (`Interpolate::Flow`) and the effects that read neighbouring frames, `motion.blur`,
/// `detail.denoise` and `color.deflicker` (ADR-0035); 0.16.0 added motion graphics: strokes,
/// open paths, trims and path following on shape clips and vector assets (ADR-0036), 3D
/// placement and camera clips (ADR-0037), particle clips (ADR-0038), web composition assets
/// (ADR-0039), 3D object clips and model assets (ADR-0040), light effects, animation delays
/// and more text reveal motions.
pub const FORMAT_VERSION: &str = "0.16.0";

/// Older format versions this build reads. Each is a subset of the current format, so it
/// loads unchanged and is saved as [`FORMAT_VERSION`].
pub const READABLE_VERSIONS: &[&str] = &[
    "0.1.0", "0.2.0", "0.3.0", "0.4.0", "0.5.0", "0.6.0", "0.7.0", "0.8.0", "0.9.0", "0.10.0",
    "0.11.0", "0.12.0", "0.13.0", "0.14.0", "0.15.0",
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
