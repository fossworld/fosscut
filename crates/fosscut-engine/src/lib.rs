//! The FossCut engine (ADR-0003): the session host that front ends share. It opens and
//! locks projects ([`project`]), imports media ([`import`]), and drives work over the
//! media, render and audio crates: export (render every frame of a timeline, mix its
//! audio, encode it with a named [`Preset`] and verify the written file against the
//! request, [`export`]), and frame grabs and contact sheets for checking an edit
//! ([`frame`]). Long-lived hosts run exports in the background as [`jobs`].

#![forbid(unsafe_code)]

mod cache;
mod error;
mod export;
pub mod frame;
pub mod import;
pub mod jobs;
mod operations;
mod preset;
pub mod project;

pub use cache::cache_dir;
pub use error::EngineError;
pub use export::{
    Encoders, ExportPlan, ExportReport, ExportRequest, Progress, compare, export, plan,
};
pub use operations::{OperationInfo, operations};
pub use preset::{Preset, PresetInfo, presets};
