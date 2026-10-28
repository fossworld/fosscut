use std::sync::Arc;

use fosscut_time::{Flicks, Rate};
use serde::{Deserialize, Serialize};

use crate::ColorInfo;

/// Pixel layout of a decoded frame in system memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    /// 8-bit 4:2:0: a luma plane and one interleaved Cb/Cr plane at half size.
    Nv12,
    /// 8-bit 4:2:0: separate luma, Cb and Cr planes, chroma at half size.
    Yuv420p,
}

impl PixelFormat {
    /// Number of planes.
    pub fn plane_count(self) -> usize {
        match self {
            PixelFormat::Nv12 => 2,
            PixelFormat::Yuv420p => 3,
        }
    }

    /// Bytes per row and number of rows of plane `index` for a `width` x `height` frame.
    pub fn plane_size(self, index: usize, width: u32, height: u32) -> (usize, usize) {
        let (w, h) = (width as usize, height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        match (self, index) {
            (_, 0) => (w, h),
            (PixelFormat::Nv12, _) => (cw * 2, ch),
            (PixelFormat::Yuv420p, _) => (cw, ch),
        }
    }
}

/// One plane of a decoded frame. Rows are `stride` bytes apart; only the first
/// [`PixelFormat::plane_size`] bytes of each row are image data. The bytes are shared, so
/// cloning a frame is cheap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
    /// Distance between rows in bytes.
    pub stride: usize,
    /// Number of rows.
    pub rows: usize,
    /// `stride * rows` bytes.
    pub data: Arc<[u8]>,
}

impl Plane {
    /// The bytes of row `y`, including padding up to the stride.
    pub fn row(&self, y: usize) -> &[u8] {
        &self.data[y * self.stride..(y + 1) * self.stride]
    }
}

/// A decoded video frame in system memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFrame {
    /// Presentation time relative to the start of the stream.
    pub pts: Flicks,
    /// How long the frame is displayed.
    pub duration: Flicks,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Plane layout.
    pub format: PixelFormat,
    /// Image planes, [`PixelFormat::plane_count`] of them.
    pub planes: Vec<Plane>,
    /// Color tags with defaults filled in ([`ColorInfo::resolved`]).
    pub color: ColorInfo,
}

impl VideoFrame {
    /// End of the frame's display interval.
    pub fn end(&self) -> Flicks {
        self.pts + self.duration
    }

    /// True when the frame is displayed at `time`.
    pub fn contains(&self, time: Flicks) -> bool {
        self.pts <= time && time < self.end()
    }

    /// Mean luma code value of the frame, used by tests and analysis.
    pub fn mean_luma(&self) -> f64 {
        let plane = &self.planes[0];
        let width = self.width as usize;
        let total: u64 = (0..plane.rows)
            .map(|y| {
                plane.row(y)[..width]
                    .iter()
                    .map(|&v| u64::from(v))
                    .sum::<u64>()
            })
            .sum();
        total as f64 / (width * plane.rows) as f64
    }
}

/// How a decoder produces frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Acceleration {
    /// Decoded on the CPU.
    Software,
    /// Decoded by Apple VideoToolbox.
    VideoToolbox,
}

/// Whether to use hardware decoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareDecoding {
    /// Use hardware when the platform supports the codec, otherwise software.
    #[default]
    Auto,
    /// Always decode in software.
    Off,
    /// Fail with [`crate::MediaError::HardwareUnavailable`] instead of falling back.
    Require,
}

/// Options for opening a decoder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecoderOptions {
    /// Hardware decoding preference.
    pub hardware: HardwareDecoding,
}

/// Facts about a video stream, as stored in the file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoStreamInfo {
    /// Stream index in the container.
    pub index: usize,
    /// Codec name, for example `h264`.
    pub codec: String,
    /// Coded width in pixels.
    pub width: u32,
    /// Coded height in pixels.
    pub height: u32,
    /// Frame rate when the stream declares an average rate.
    pub rate: Option<Rate>,
    /// Stream time base as numerator and denominator.
    pub time_base: (i32, i32),
    /// Stream duration, when known.
    pub duration: Option<Flicks>,
    /// Number of frames, when the container records it.
    pub frame_count: Option<u64>,
    /// Degrees clockwise the frame must be rotated for display: 0, 90, 180 or 270.
    pub rotation: u16,
    /// Color tags as stored in the file (unresolved).
    pub color: ColorInfo,
    /// Pixel format name in the file, for example `yuv420p` or `yuv420p10le`.
    pub source_pixel_format: String,
}

/// A source of decoded video frames (ADR-0009, item 1).
pub trait VideoDecoder {
    /// Facts about the stream being decoded.
    fn info(&self) -> &VideoStreamInfo;

    /// How frames are being decoded. Known after the first frame; [`Acceleration::Software`]
    /// before that.
    fn acceleration(&self) -> Acceleration;

    /// The next frame in presentation order, or `None` at the end of the stream.
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, crate::MediaError>;

    /// The frame displayed at `time` (the last frame whose presentation time is not after
    /// `time`). Seeks only when the target is behind the current position or far ahead
    /// of it, so sequential calls decode forward without seeking.
    fn frame_at(&mut self, time: Flicks) -> Result<VideoFrame, crate::MediaError>;
}
