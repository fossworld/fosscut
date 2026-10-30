//! Media probing and decoding for FossCut (ADR-0009).
//!
//! The engine depends only on the backend-neutral types and traits here: [`VideoDecoder`]
//! yields [`VideoFrame`]s in system memory with exact [`fosscut_time::Flicks`] timestamps
//! and the stream's color tags, [`OutputSettings`] describe files to write,
//! [`OutputReport`] describes what a written file actually contains, and [`capture`]
//! records from microphones, cameras and screens. The native backend, [`ffmpeg`], uses
//! FFmpeg 8 shared libraries (LGPL builds for distribution) with VideoToolbox hardware
//! decoding on macOS and FFmpeg's input devices for recording.
//! A future web backend will implement the same traits over WebCodecs.

pub mod capture;
mod color;
mod error;
mod frame;
mod output;
#[cfg(feature = "ffmpeg")]
mod palette;
pub mod timebase;

#[cfg(feature = "ffmpeg")]
pub mod ffmpeg;

pub use capture::{CaptureInput, Device, DeviceKind, Recording};
pub use color::{ColorInfo, Matrix, Primaries, Range, Transfer};
pub use error::MediaError;
pub use frame::{
    Acceleration, DecoderOptions, HardwareDecoding, PixelFormat, Plane, VideoDecoder, VideoFrame,
    VideoStreamInfo,
};
pub use output::{
    AudioBuffer, AudioCodec, AudioReport, AudioSettings, Container, EncoderCapabilities,
    EncoderInfo, OutputReport, OutputSettings, RgbaFrame, VideoCodec, VideoReport, VideoSettings,
};
