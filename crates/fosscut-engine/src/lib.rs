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
mod error;
mod export;
pub mod frame;
pub mod import;
pub mod jobs;
pub mod live;
mod operations;
mod preset;
pub mod preview;
pub mod project;

pub use cache::{cache_dir, models_dir};
pub use error::{EngineError, engine_error_catalogue};
pub use export::{
    Encoders, ExportPlan, ExportReport, ExportRequest, Progress, compare, export, plan,
};
pub use operations::{OperationInfo, capabilities, operations};
pub use preset::{Preset, PresetInfo, presets};
