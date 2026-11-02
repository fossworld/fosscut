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
//!
//! Planners measure media, or read cached analyses, and return the commands that apply what
//! they found: [`scenes`] splits at shot cuts, [`motion`] stabilizes and tracks, [`beats`]
//! marks beats and moves cuts onto them, [`reframe`] makes clips follow their subject on a
//! canvas of another shape, [`shorts`] finds and builds short clips of long footage, [`duck`]
//! lowers music under speech, [`sync`] places a separate recording under the camera's clips
//! and [`punch`] zooms in on a face for a moment.
//!
//! Programs the user installed run only after the user agreed to the command shown
//! ([`external`]): their own Remotion ([`remotion`]) and their own agent ([`agent`]), which
//! edits a live project through FossCut's MCP server like any other client.

#![forbid(unsafe_code)]

pub mod agent;
pub mod analysis;
pub mod beats;
pub mod cache;
pub mod captions;
pub mod color;
pub mod duck;
mod error;
mod export;
pub mod external;
pub mod frame;
pub mod frame_cache;
pub mod hardware;
pub mod import;
pub mod jobs;
pub mod library;
pub mod live;
pub mod loudness;
mod measured;
pub mod model;
pub mod motion;
mod operations;
pub mod preferences;
mod preset;
pub mod preview;
pub mod project;
pub mod proxy;
pub mod punch;
pub mod record;
pub mod reframe;
pub mod remotion;
pub mod remux;
pub mod scenes;
pub mod shorts;
pub mod sync;
pub mod updates;
pub mod vector;
pub mod voice;
pub mod web;

pub use cache::{cache_dir, models_dir};
pub use error::{EngineError, engine_error_catalogue};
pub use export::{
    Encoders, ExportOptions, ExportPlan, ExportReport, ExportRequest, Progress, RESOLUTIONS,
    RightsNote, SettingsChoice, SubtitleFormat, bit_rate, compare, export, frame_rate, plan,
    resolution, settings_for,
};
pub use operations::{OperationInfo, capabilities, operations};
pub use preset::{Preset, PresetInfo, presets};
