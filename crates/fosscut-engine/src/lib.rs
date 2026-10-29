//! The FossCut engine (ADR-0003): the session host that drives long jobs over the media,
//! render and audio crates. It starts with export: render every frame of a timeline, mix
//! its audio, encode it with a named [`Preset`], and verify the written file against the
//! request ([`export`]).

#![forbid(unsafe_code)]

mod cache;
mod error;
mod export;
mod preset;

pub use cache::cache_dir;
pub use error::ExportError;
pub use export::{
    Encoders, ExportPlan, ExportReport, ExportRequest, Progress, compare, export, plan,
};
pub use preset::{Preset, PresetInfo, presets};
