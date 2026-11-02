//! Frame evaluation for FossCut (ADR-0008).
//!
//! Rendering a frame is a pure function of the project, the time, the source frames and
//! the output settings: [`plan::plan_frame`] decides what to draw, and [`Renderer`]
//! draws it with wgpu (Metal, Vulkan, DX12, and WebGPU later). The color pipeline
//! ([`color`]) converts sources to linear light, composites there, and encodes the output.
//! [`ExportGraph`] and [`PreviewGraph`] drive the same evaluation for export and preview,
//! which is what makes preview/export parity testable. Effects that follow a subject read
//! cached analyses through the frame source ([`subject`]); the renderer never runs a model.

pub mod caption;
pub mod color;
pub mod effects;
mod error;
pub mod geometry;
mod gpu;
mod graph;
pub mod light;
pub mod lut;
pub mod mask;
pub mod objects;
pub mod particles;
pub mod plan;
mod raster;
pub mod shape;
pub mod space;
pub mod subject;
pub mod text;
pub mod transitions;
pub mod vector;

pub use error::RenderError;
pub use gpu::{Image, Renderer};
pub use graph::{DecoderFrames, ExportGraph, FrameProvider, PreviewGraph, WINDOW_BYTES};
