//! Media probing and decoding for FossCut (ADR-0009).
//!
//! The engine depends only on the backend-neutral types and traits here: [`VideoDecoder`]
//! yields [`VideoFrame`]s in system memory with exact [`fosscut_time::Flicks`] timestamps
//! and the stream's color tags. The native backend, [`ffmpeg`], uses FFmpeg 8 shared
//! libraries (LGPL builds for distribution) with VideoToolbox hardware decoding on macOS.
//! A future web backend will implement the same traits over WebCodecs.

mod color;
mod error;
mod frame;
pub mod timebase;

#[cfg(feature = "ffmpeg")]
pub mod ffmpeg;

pub use color::{ColorInfo, Matrix, Primaries, Range, Transfer};
pub use error::MediaError;
pub use frame::{
    Acceleration, DecoderOptions, HardwareDecoding, PixelFormat, Plane, VideoDecoder, VideoFrame,
    VideoStreamInfo,
};
