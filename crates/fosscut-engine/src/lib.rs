//! The FossCut engine (ADR-0003): the session host that front ends share. It opens and
//! locks projects ([`project`]), imports media ([`import`]), and drives work over the
//! media, render and audio crates: export (render every frame of a timeline, mix its
//! audio, encode it with a named [`Preset`] and verify the written file against the
//! request, [`export`]), frame grabs and contact sheets for checking an edit
//! ([`frame`]), and the desktop preview ([`preview`]). Long-lived hosts run exports in
//! the background as [`jobs`], and serve projects live to other front ends ([`live`]).
//!
//! Caches outside projects make editing faster without changing results (ADR-0032):
//! proxies ([`proxy`]) and remuxed copies ([`remux`]) that only the preview decodes, rendered
//! preview frames ([`frame_cache`]), and the policy that keeps them bounded ([`cache`]).
//! Hardware decoding and encoding are chosen for the process in [`hardware`].

#![forbid(unsafe_code)]

pub mod analysis;
pub mod cache;
pub mod captions;
pub mod color;
mod error;
mod export;
pub mod frame;
pub mod frame_cache;
pub mod hardware;
pub mod import;
pub mod jobs;
pub mod library;
pub mod live;
pub mod loudness;
pub mod motion;
mod operations;
pub mod preferences;
mod preset;
pub mod preview;
pub mod project;
pub mod proxy;
pub mod record;
pub mod remux;
pub mod scenes;
pub mod updates;

pub use cache::{cache_dir, models_dir};
pub use error::{EngineError, engine_error_catalogue};
pub use export::{
    Encoders, ExportOptions, ExportPlan, ExportReport, ExportRequest, Progress, RESOLUTIONS,
    RightsNote, SettingsChoice, SubtitleFormat, bit_rate, compare, export, frame_rate, plan,
    resolution, settings_for,
};
pub use operations::{OperationInfo, capabilities, operations};
pub use preset::{Preset, PresetInfo, presets};
