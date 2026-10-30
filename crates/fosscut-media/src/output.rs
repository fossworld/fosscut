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
    /// Animated GIF (`.gif`): GIF video only, looping forever.
    Gif,
    /// WAVE (`.wav`): PCM audio only.
    Wav,
    /// FLAC (`.flac`): FLAC audio only.
    Flac,
    /// MPEG audio layer III (`.mp3`): MP3 audio only.
    Mp3,
}

impl Container {
    /// The usual file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mov => "mov",
            Container::Webm => "webm",
            Container::Gif => "gif",
            Container::Wav => "wav",
            Container::Flac => "flac",
            Container::Mp3 => "mp3",
        }
    }

    /// Whether the container can hold `video` and `audio` streams of these codecs.
    pub fn supports(self, video: Option<VideoCodec>, audio: Option<AudioCodec>) -> bool {
        use AudioCodec as A;
        use VideoCodec as V;
        let video_ok = match (self, video) {
            (_, None) => true,
            (Container::Mp4, Some(codec)) => matches!(codec, V::H264 | V::Hevc | V::Av1 | V::Vp9),
            (Container::Mov, Some(codec)) => codec != V::Gif,
            (Container::Webm, Some(codec)) => matches!(codec, V::Vp9 | V::Av1),
            (Container::Gif, Some(codec)) => codec == V::Gif,
            (Container::Wav | Container::Flac | Container::Mp3, Some(_)) => false,
        };
        let audio_ok = match (self, audio) {
            (_, None) => true,
            (Container::Mp4, Some(codec)) => matches!(codec, A::Aac | A::Opus),
            (Container::Mov, Some(codec)) => matches!(codec, A::Aac | A::Pcm16 | A::Pcm24),
            (Container::Webm, Some(codec)) => codec == A::Opus,
            (Container::Gif, Some(_)) => false,
            (Container::Wav, Some(codec)) => matches!(codec, A::Pcm16 | A::Pcm24),
            (Container::Flac, Some(codec)) => codec == A::Flac,
            (Container::Mp3, Some(codec)) => codec == A::Mp3,
        };
        video_ok && audio_ok
    }

    /// Whether a probed file is in this container: `format` is the demuxer name FFmpeg
    /// reports ([`OutputReport::container`], for example `mov,mp4,m4a,3gp,3g2,mj2`) and
    /// `brand` the major brand of MP4 and QuickTime files ([`OutputReport::brand`]), which
    /// tells the two apart since one demuxer reads both.
    pub fn matches_probe(self, format: &str, brand: Option<&str>) -> bool {
        let named = |name: &str| format.split(',').any(|n| n == name);
        match self {
            Container::Mp4 => named("mp4") && brand != Some("qt  "),
            Container::Mov => named("mov") && brand == Some("qt  "),
            Container::Webm => named("webm"),
            Container::Gif => named("gif"),
            Container::Wav => named("wav"),
            Container::Flac => named("flac"),
            Container::Mp3 => named("mp3"),
        }
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
    /// Apple ProRes 422 Proxy, 10-bit 4:2:2: the lightest profile, for offline editing.
    ProResProxy,
    /// Apple ProRes 422 LT, 10-bit 4:2:2: lighter than standard.
    ProResLt,
    /// Apple ProRes 422 HQ, 10-bit 4:2:2: heavier than standard, for mastering.
    ProResHq,
    /// Apple ProRes 4444, 10-bit 4:4:4 with an alpha channel taken from the frames' alpha.
    ProRes4444,
    /// AV1, encoded in software with permissively licensed encoders.
    Av1,
    /// VP9.
    Vp9,
    /// Animated GIF: at most 256 colours per frame, ordered dithering, no sound.
    Gif,
}

impl VideoCodec {
    /// Every codec, in the order capabilities are reported.
    pub const ALL: [VideoCodec; 10] = [
        VideoCodec::H264,
        VideoCodec::Hevc,
        VideoCodec::ProRes422,
        VideoCodec::ProResProxy,
        VideoCodec::ProResLt,
        VideoCodec::ProResHq,
        VideoCodec::ProRes4444,
        VideoCodec::Av1,
        VideoCodec::Vp9,
        VideoCodec::Gif,
    ];

    /// The codec's name as FFmpeg probes report it (`h264`, `hevc`, `prores`, `av1`, `vp9`,
    /// `gif`). Every ProRes profile is `prores`; [`VideoCodec::profile_name`] tells them
    /// apart.
    pub fn probe_name(self) -> &'static str {
        match self {
            VideoCodec::H264 => "h264",
            VideoCodec::Hevc => "hevc",
            VideoCodec::ProRes422
            | VideoCodec::ProResProxy
            | VideoCodec::ProResLt
            | VideoCodec::ProResHq
            | VideoCodec::ProRes4444 => "prores",
            VideoCodec::Av1 => "av1",
            VideoCodec::Vp9 => "vp9",
            VideoCodec::Gif => "gif",
        }
    }

    /// The profile FFmpeg probes report for files of this codec
    /// ([`VideoReport::profile`]), for codecs whose variants are profiles: `Proxy`, `LT`,
    /// `Standard`, `HQ` and `4444` for ProRes. `None` for the others, whose profile is the
    /// encoder's choice.
    pub fn profile_name(self) -> Option<&'static str> {
        match self {
            VideoCodec::ProRes422 => Some("Standard"),
            VideoCodec::ProResProxy => Some("Proxy"),
            VideoCodec::ProResLt => Some("LT"),
            VideoCodec::ProResHq => Some("HQ"),
            VideoCodec::ProRes4444 => Some("4444"),
            _ => None,
        }
    }

    /// Whether encoders of this codec aim for [`VideoSettings::bit_rate`]. ProRes sets its
    /// data rate by profile and GIF has no rate control, so both ignore it.
    pub fn uses_bit_rate(self) -> bool {
        matches!(
            self,
            VideoCodec::H264 | VideoCodec::Hevc | VideoCodec::Av1 | VideoCodec::Vp9
        )
    }

    /// Whether files of this codec keep the frames' alpha channel (ProRes 4444).
    pub fn has_alpha(self) -> bool {
        self == VideoCodec::ProRes4444
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
    /// Uncompressed 16-bit little-endian PCM.
    Pcm16,
    /// Uncompressed 24-bit little-endian PCM.
    Pcm24,
    /// FLAC, lossless, 16-bit.
    Flac,
    /// MP3 through LAME (LGPL), when the FFmpeg build has it.
    Mp3,
}

impl AudioCodec {
    /// Every codec, in the order capabilities are reported.
    pub const ALL: [AudioCodec; 6] = [
        AudioCodec::Aac,
        AudioCodec::Opus,
        AudioCodec::Pcm16,
        AudioCodec::Pcm24,
        AudioCodec::Flac,
        AudioCodec::Mp3,
    ];

    /// The codec's name as FFmpeg probes report it (`aac`, `opus`, `pcm_s16le`,
    /// `pcm_s24le`, `flac`, `mp3`).
    pub fn probe_name(self) -> &'static str {
        match self {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "opus",
            AudioCodec::Pcm16 => "pcm_s16le",
            AudioCodec::Pcm24 => "pcm_s24le",
            AudioCodec::Flac => "flac",
            AudioCodec::Mp3 => "mp3",
        }
    }

    /// Whether the codec keeps every sample (PCM and FLAC); lossless codecs ignore
    /// [`AudioSettings::bit_rate`].
    pub fn is_lossless(self) -> bool {
        matches!(
            self,
            AudioCodec::Pcm16 | AudioCodec::Pcm24 | AudioCodec::Flac
        )
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
    /// Target bit rate in bits per second for codecs that take one
    /// ([`VideoCodec::uses_bit_rate`]); `None` picks a codec default for the size. ProRes
    /// and GIF ignore it.
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
    /// Target bit rate in bits per second for lossy codecs; PCM and FLAC ignore it.
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
    /// `width * height * 4` bytes, rows top to bottom, non-linear BT.709 R'G'B'. ProRes
    /// 4444 encodes the alpha bytes as its alpha channel; every other codec ignores them.
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
    /// Codec name (`h264`, `hevc`, `prores`, `av1`, `vp9`, `gif`).
    pub codec: String,
    /// Codec profile, for example `High`, or `HQ` and `4444` for ProRes
    /// ([`VideoCodec::profile_name`]).
    pub profile: Option<String>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Average frame rate. For GIF, whose frame delays are whole centiseconds, it is the
    /// frame count over the duration.
    pub rate: Option<Rate>,
    /// Number of frames.
    pub frame_count: u64,
    /// Duration of the video stream.
    pub duration: Option<Flicks>,
    /// Pixel format name as the decoder produces it, for example `yuva444p12le` for ProRes
    /// 4444 with alpha.
    pub pixel_format: String,
    /// Color tags.
    pub color: ColorInfo,
}

/// The audio stream of a written file, as probed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioReport {
    /// Codec name (`aac`, `opus`, `pcm_s16le`, `pcm_s24le`, `flac`, `mp3`).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_hold_only_the_codecs_they_support() {
        use AudioCodec as A;
        use VideoCodec as V;
        let prores = [
            V::ProRes422,
            V::ProResProxy,
            V::ProResLt,
            V::ProResHq,
            V::ProRes4444,
        ];
        for codec in prores {
            assert!(Container::Mov.supports(Some(codec), Some(A::Pcm24)));
            assert!(!Container::Mp4.supports(Some(codec), None));
            assert_eq!(codec.probe_name(), "prores");
            assert!(codec.profile_name().is_some());
            assert!(!codec.uses_bit_rate());
        }
        assert!(Container::Gif.supports(Some(V::Gif), None));
        assert!(!Container::Gif.supports(Some(V::Gif), Some(A::Aac)));
        assert!(!Container::Gif.supports(Some(V::H264), None));
        assert!(!Container::Mov.supports(Some(V::Gif), None));
        assert!(!Container::Mp4.supports(Some(V::Gif), None));
        for (container, codecs) in [
            (Container::Wav, [A::Pcm16, A::Pcm24].as_slice()),
            (Container::Flac, &[A::Flac]),
            (Container::Mp3, &[A::Mp3]),
        ] {
            for codec in AudioCodec::ALL {
                assert_eq!(
                    container.supports(None, Some(codec)),
                    codecs.contains(&codec),
                    "{container:?} {codec:?}"
                );
            }
            assert!(!container.supports(Some(V::Gif), Some(codecs[0])));
            assert!(!container.supports(Some(V::H264), None));
        }
        assert!(Container::Mp4.supports(Some(V::H264), Some(A::Aac)));
        assert!(!Container::Mp4.supports(None, Some(A::Pcm16)));
        assert!(!Container::Webm.supports(None, Some(A::Flac)));
        assert_eq!(
            [
                Container::Gif,
                Container::Wav,
                Container::Flac,
                Container::Mp3
            ]
            .map(Container::extension),
            ["gif", "wav", "flac", "mp3"]
        );
        assert!(V::ProRes4444.has_alpha() && !V::ProResHq.has_alpha());
        assert!(V::H264.uses_bit_rate() && !V::Gif.uses_bit_rate());
        let lossless: Vec<_> = A::ALL.into_iter().filter(|c| c.is_lossless()).collect();
        assert_eq!(lossless, [A::Pcm16, A::Pcm24, A::Flac]);
    }

    #[test]
    fn probed_formats_match_their_containers() {
        let mp4 = "mov,mp4,m4a,3gp,3g2,mj2";
        assert!(Container::Mp4.matches_probe(mp4, Some("isom")));
        assert!(!Container::Mp4.matches_probe(mp4, Some("qt  ")));
        assert!(Container::Mov.matches_probe(mp4, Some("qt  ")));
        assert!(!Container::Mov.matches_probe(mp4, Some("isom")));
        assert!(Container::Webm.matches_probe("matroska,webm", None));
        assert!(Container::Gif.matches_probe("gif", None));
        assert!(Container::Wav.matches_probe("wav", None));
        assert!(Container::Flac.matches_probe("flac", None));
        assert!(Container::Mp3.matches_probe("mp3", None));
        assert!(!Container::Mp3.matches_probe("wav", None));
        assert!(!Container::Gif.matches_probe("gif_pipe2", None));
    }
}
