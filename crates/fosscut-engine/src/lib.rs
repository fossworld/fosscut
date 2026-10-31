//! The FossCut engine (ADR-0003): the session host that front ends share. It opens and
//! locks projects ([`project`]), imports media ([`import`]), and drives work over the
//! media, render and audio crates: export (render every frame of a timeline, mix its
//! audio, encode it with a named [`Preset`] and verify the written file against the
//! request, [`export`]), frame grabs and contact sheets for checking an edit
//! ([`frame`]), and the desktop preview ([`preview`]). Long-lived hosts run exports in
//! the background as [`jobs`], and serve projects live to other front ends ([`live`]).

#![forbid(unsafe_code)]

pub mod analysis;
mod cache;
pub mod captions;
pub mod color;
mod error;
mod export;
pub mod frame;
pub mod import;
pub mod jobs;
pub mod library;
pub mod live;
pub mod loudness;
pub mod motion;
mod operations;
mod preset;
pub mod preview;
pub mod project;
pub mod record;
pub mod scenes;

pub use cache::{cache_dir, models_dir};
pub use error::{EngineError, engine_error_catalogue};
pub use export::{
    Encoders, ExportOptions, ExportPlan, ExportReport, ExportRequest, Progress, RESOLUTIONS,
    RightsNote, SettingsChoice, SubtitleFormat, bit_rate, compare, export, frame_rate, plan,
    resolution, settings_for,
};
pub use operations::{OperationInfo, capabilities, operations};
pub use preset::{Preset, PresetInfo, presets};
