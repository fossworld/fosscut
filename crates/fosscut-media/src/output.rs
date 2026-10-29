//! Backend-neutral types for writing media files and checking them (ADR-0009, items 5
//! and 6).

use fosscut_time::{Flicks, Rate};
use serde::{Deserialize, Serialize};

use crate::ColorInfo;

/// File container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    /// MPEG-4 Part 14 (`.mp4`).
    Mp4,
    /// QuickTime (`.mov`).
    Mov,
    /// WebM (`.webm`).
    Webm,
}

impl Container {
    /// The usual file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mov => "mov",
            Container::Webm => "webm",
        }
    }

    /// Whether the container can hold `video` and `audio` streams of these codecs.
    pub fn supports(self, video: Option<VideoCodec>, audio: Option<AudioCodec>) -> bool {
        let video_ok = match (self, video) {
            (_, None) => true,
            (Container::Webm, Some(codec)) => matches!(codec, VideoCodec::Vp9 | VideoCodec::Av1),
            (Container::Mp4, Some(codec)) => codec != VideoCodec::ProRes422,
            (Container::Mov, Some(_)) => true,
        };
        let audio_ok = match (self, audio) {
            (_, None) => true,
            (Container::Webm, Some(codec)) => codec == AudioCodec::Opus,
            (Container::Mov, Some(codec)) => codec == AudioCodec::Aac,
            (Container::Mp4, Some(_)) => true,
        };
        video_ok && audio_ok
    }
}

/// A video codec as requested by a user or agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    /// H.264 / AVC: hardware encoders only, since software encoders are GPL (ADR-0009).
    H264,
    /// H.265 / HEVC: hardware encoders only.
    Hevc,
    /// Apple ProRes 422 (standard), 10-bit 4:2:2.
    ProRes422,
    /// AV1, encoded in software with permissively licensed encoders.
    Av1,
    /// VP9.
    Vp9,
}

impl VideoCodec {
    /// Every codec, in the order capabilities are reported.
    pub const ALL: [VideoCodec; 5] = [
        VideoCodec::H264,
        VideoCodec::Hevc,
        VideoCodec::ProRes422,
        VideoCodec::Av1,
        VideoCodec::Vp9,
    ];

    /// The codec's name as FFmpeg probes report it (`h264`, `hevc`, `prores`, `av1`, `vp9`).
    pub fn probe_name(self) -> &'static str {
        match self {
            VideoCodec::H264 => "h264",
            VideoCodec::Hevc => "hevc",
            VideoCodec::ProRes422 => "prores",
            VideoCodec::Av1 => "av1",
            VideoCodec::Vp9 => "vp9",
        }
    }
}

/// An audio codec as requested by a user or agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    /// AAC-LC.
    Aac,
    /// Opus.
    Opus,
}

impl AudioCodec {
    /// Every codec, in the order capabilities are reported.
    pub const ALL: [AudioCodec; 2] = [AudioCodec::Aac, AudioCodec::Opus];

    /// The codec's name as FFmpeg probes report it.
    pub fn probe_name(self) -> &'static str {
        match self {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "opus",
        }
    }
}

/// Video stream settings for a new file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoSettings {
    /// Codec.
    pub codec: VideoCodec,
    /// Width in pixels (even).
    pub width: u32,
    /// Height in pixels (even).
    pub height: u32,
    /// Exact frame rate; frame `n` is stamped at `n / rate`.
    pub rate: Rate,
    /// Target bit rate in bits per second; `None` picks a codec default for the size.
    pub bit_rate: Option<u64>,
}

/// Audio stream settings for a new file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSettings {
    /// Codec.
    pub codec: AudioCodec,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels (1 or 2).
    pub channels: u16,
    /// Target bit rate in bits per second.
    pub bit_rate: u64,
}

impl AudioSettings {
    /// 48 kHz stereo at 192 kb/s, the editor's working format.
    pub fn stereo(codec: AudioCodec) -> AudioSettings {
        AudioSettings {
            codec,
            sample_rate: 48_000,
            channels: 2,
            bit_rate: 192_000,
        }
    }
}

/// What a new file contains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSettings {
    /// Container.
    pub container: Container,
    /// Video stream, if any.
    pub video: Option<VideoSettings>,
    /// Audio stream, if any.
    pub audio: Option<AudioSettings>,
}

/// Interleaved `f32` audio samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioBuffer {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of interleaved channels.
    pub channels: u16,
    /// `frames * channels` samples, -1 to 1.
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    /// Silence of `frames` sample frames.
    pub fn silence(sample_rate: u32, channels: u16, frames: usize) -> AudioBuffer {
        AudioBuffer {
            sample_rate,
            channels,
            samples: vec![0.0; frames * usize::from(channels)],
        }
    }

    /// Number of sample frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }
}

/// A frame of 8-bit RGBA pixels, as the renderer reads them back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, rows top to bottom, non-linear BT.709 R'G'B'.
    pub rgba: Vec<u8>,
}

/// One encoder this machine can use for a codec.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderInfo {
    /// The encoder's FFmpeg name, for example `h264_videotoolbox` or `libsvtav1`.
    pub encoder: String,
    /// True for hardware encoders.
    pub hardware: bool,
}

/// Which codecs this machine can encode, found by opening each candidate encoder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderCapabilities {
    /// Usable video encoders by codec.
    pub video: Vec<(VideoCodec, EncoderInfo)>,
    /// Usable audio encoders by codec.
    pub audio: Vec<(AudioCodec, EncoderInfo)>,
}

impl EncoderCapabilities {
    /// The encoder for a video codec, if this machine has one.
    pub fn video(&self, codec: VideoCodec) -> Option<&EncoderInfo> {
        self.video.iter().find(|(c, _)| *c == codec).map(|(_, e)| e)
    }

    /// The encoder for an audio codec, if this machine has one.
    pub fn audio(&self, codec: AudioCodec) -> Option<&EncoderInfo> {
        self.audio.iter().find(|(c, _)| *c == codec).map(|(_, e)| e)
    }
}

/// The video stream of a written file, as probed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoReport {
    /// Codec name (`h264`, `hevc`, `prores`, `av1`, `vp9`).
    pub codec: String,
    /// Codec profile, for example `High` or `422`.
    pub profile: Option<String>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Average frame rate.
    pub rate: Option<Rate>,
    /// Number of frames.
    pub frame_count: u64,
    /// Duration of the video stream.
    pub duration: Option<Flicks>,
    /// Pixel format name.
    pub pixel_format: String,
    /// Color tags.
    pub color: ColorInfo,
}

/// The audio stream of a written file, as probed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioReport {
    /// Codec name (`aac`, `opus`).
    pub codec: String,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels.
    pub channels: u16,
    /// Duration of the audio stream.
    pub duration: Option<Flicks>,
}

/// What a written file actually contains (ADR-0009, item 6), for comparison with the
/// request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputReport {
    /// Container format as FFmpeg names it, for example `mov,mp4,m4a,3gp,3g2,mj2`.
    pub container: String,
    /// The major brand for MP4 and QuickTime files, for example `isom` or `qt  `.
    pub brand: Option<String>,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Duration of the whole file.
    pub duration: Option<Flicks>,
    /// The video stream.
    pub video: Option<VideoReport>,
    /// The audio stream.
    pub audio: Option<AudioReport>,
}
