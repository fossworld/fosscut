//! Writing media files through FFmpeg (ADR-0009): which encoders this machine can use,
//! and a writer that encodes video and audio and muxes them.
//!
//! GPL encoders (`libx264`, `libx265`) are never used, even when the linked FFmpeg has
//! them: H.264 and HEVC come only from hardware encoders. MP3 uses LAME (LGPL) when the
//! build has it and is reported unavailable otherwise, never replaced by another codec.

use std::ffi::CString;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avformat::AVFormatContextOutput;
use rsmpeg::avutil::{AVDictionary, AVFrame};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;

use crate::palette::Quantizer;
use crate::{
    AudioBuffer, AudioCodec, AudioSettings, ColorInfo, Container, EncoderCapabilities, EncoderInfo,
    HardwareEncoding, Matrix, MediaError, OutputSettings, PixelFormat, Primaries, Range, RgbaFrame,
    Transfer, VideoCodec, VideoFrame, VideoSettings,
};

/// Candidate encoders per video codec, in order of preference, with whether each is a
/// hardware encoder.
fn video_candidates(codec: VideoCodec) -> &'static [(&'static str, bool)] {
    match codec {
        VideoCodec::H264 => &[
            ("h264_videotoolbox", true),
            ("h264_nvenc", true),
            ("h264_qsv", true),
            ("h264_amf", true),
            ("h264_mf", false),
        ],
        VideoCodec::Hevc => &[
            ("hevc_videotoolbox", true),
            ("hevc_nvenc", true),
            ("hevc_qsv", true),
            ("hevc_amf", true),
            ("hevc_mf", false),
        ],
        VideoCodec::ProRes422
        | VideoCodec::ProResProxy
        | VideoCodec::ProResLt
        | VideoCodec::ProResHq
        | VideoCodec::ProRes4444 => &[("prores_ks", false)],
        VideoCodec::Av1 => &[
            ("libsvtav1", false),
            ("libaom-av1", false),
            ("librav1e", false),
        ],
        VideoCodec::Vp9 => &[("libvpx-vp9", false)],
        VideoCodec::Gif => &[("gif", false)],
    }
}

/// The candidates for `codec` that `hardware` allows, in order of preference: every one
/// when hardware encoders may be used, otherwise only the software encoders.
fn allowed_candidates(
    codec: VideoCodec,
    hardware: HardwareEncoding,
) -> impl Iterator<Item = &'static (&'static str, bool)> {
    video_candidates(codec)
        .iter()
        .filter(move |(_, is_hardware)| hardware == HardwareEncoding::Auto || !is_hardware)
}

/// Whether hardware encoders may be used when no choice is passed; see
/// [`set_hardware_encoding`].
static HARDWARE_ENCODERS: AtomicBool = AtomicBool::new(true);

/// Sets, for the whole process, whether [`FfmpegWriter::create`], [`video_encoder`] and
/// [`encoder_capabilities`] may use hardware encoders (ADR-0032, item 6). Hosts set it
/// from the user's `performance.hardware_encoding` preference; it changes which encoder
/// writes a codec, never the codec. [`HardwareEncoding::Auto`] until set.
pub fn set_hardware_encoding(choice: HardwareEncoding) {
    HARDWARE_ENCODERS.store(choice == HardwareEncoding::Auto, Ordering::Relaxed);
}

/// The process's hardware encoding choice; see [`set_hardware_encoding`].
pub fn hardware_encoding() -> HardwareEncoding {
    if HARDWARE_ENCODERS.load(Ordering::Relaxed) {
        HardwareEncoding::Auto
    } else {
        HardwareEncoding::Off
    }
}

fn audio_candidates(codec: AudioCodec) -> &'static [&'static str] {
    match codec {
        AudioCodec::Aac => &["aac"],
        AudioCodec::Opus => &["libopus"],
        AudioCodec::Pcm16 => &["pcm_s16le"],
        AudioCodec::Pcm24 => &["pcm_s24le"],
        AudioCodec::Flac => &["flac"],
        AudioCodec::Mp3 => &["libmp3lame"],
    }
}

fn c(text: &str) -> CString {
    CString::new(text).unwrap_or_default()
}

fn rational(num: u32, den: u32) -> ffi::AVRational {
    ffi::AVRational {
        num: num as i32,
        den: den as i32,
    }
}

/// The pixel format an encoder gets: 10-bit 4:4:4 with alpha for ProRes 4444, 10-bit
/// 4:2:2 for the other ProRes profiles, palette frames for GIF, otherwise NV12 when the
/// encoder takes it (hardware encoders prefer it) or planar 4:2:0.
fn pixel_format_for(codec: VideoCodec, encoder: &AVCodec) -> i32 {
    match codec {
        VideoCodec::ProRes4444 => return ffi::AV_PIX_FMT_YUVA444P10LE,
        VideoCodec::ProRes422
        | VideoCodec::ProResProxy
        | VideoCodec::ProResLt
        | VideoCodec::ProResHq => return ffi::AV_PIX_FMT_YUV422P10LE,
        VideoCodec::Gif => return ffi::AV_PIX_FMT_PAL8,
        VideoCodec::H264 | VideoCodec::Hevc | VideoCodec::Av1 | VideoCodec::Vp9 => {}
    }
    let supported = encoder.pix_fmts().unwrap_or(&[]);
    if supported.contains(&ffi::AV_PIX_FMT_NV12) {
        ffi::AV_PIX_FMT_NV12
    } else {
        ffi::AV_PIX_FMT_YUV420P
    }
}

/// The `prores_ks` profile for a ProRes codec.
fn prores_profile(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::ProResProxy => "proxy",
        VideoCodec::ProResLt => "lt",
        VideoCodec::ProResHq => "hq",
        VideoCodec::ProRes4444 => "4444",
        _ => "standard",
    }
}

/// Encoder-specific options: speed and quality settings, the ProRes profile, and for
/// hardware encoders a refusal to fall back to software. The software encoders' constant
/// quality settings apply only when no bit rate was asked for, since each of them would
/// override the bit rate.
fn encoder_options(
    name: &str,
    codec: VideoCodec,
    bit_rate_requested: bool,
) -> Option<AVDictionary> {
    let mut pairs: Vec<(&str, &str)> = match name {
        "h264_videotoolbox" => vec![("allow_sw", "0"), ("profile", "high")],
        "hevc_videotoolbox" => vec![("allow_sw", "0"), ("profile", "main")],
        "prores_ks" => vec![("profile", prores_profile(codec)), ("vendor", "apl0")],
        "libsvtav1" => vec![("preset", "10")],
        "libaom-av1" => vec![("cpu-used", "8"), ("row-mt", "1")],
        "librav1e" => vec![("speed", "10")],
        "libvpx-vp9" => vec![("deadline", "realtime"), ("cpu-used", "8")],
        _ => Vec::new(),
    };
    if !bit_rate_requested {
        pairs.extend_from_slice(match name {
            "libsvtav1" | "libaom-av1" => &[("crf", "35")][..],
            "librav1e" => &[("qp", "100")],
            "libvpx-vp9" => &[("crf", "32"), ("b", "0")],
            _ => &[],
        });
    }
    let mut dict: Option<AVDictionary> = None;
    for (key, value) in pairs {
        dict = Some(match dict {
            Some(dict) => dict.set(&c(key), &c(value), 0),
            None => AVDictionary::new(&c(key), &c(value), 0),
        });
    }
    dict
}

/// A default bit rate for codecs that use one, scaled by pixels per second.
fn default_bit_rate(codec: VideoCodec, settings: &VideoSettings) -> i64 {
    let pixels_per_second =
        f64::from(settings.width) * f64::from(settings.height) * f64::from(settings.rate.num())
            / f64::from(settings.rate.den().max(1));
    let bits_per_pixel = match codec {
        VideoCodec::H264 => 0.2,
        VideoCodec::Hevc => 0.12,
        _ => 0.0,
    };
    (pixels_per_second * bits_per_pixel) as i64
}

/// FFmpeg's values for color tags: range, matrix, primaries and transfer.
type AvColor = (
    ffi::AVColorRange,
    ffi::AVColorSpace,
    ffi::AVColorPrimaries,
    ffi::AVColorTransferCharacteristic,
);

/// FFmpeg's tags for `color`.
fn av_color(color: ColorInfo) -> AvColor {
    (
        match color.range {
            Range::Limited => ffi::AVCOL_RANGE_MPEG,
            Range::Full => ffi::AVCOL_RANGE_JPEG,
            Range::Unspecified => ffi::AVCOL_RANGE_UNSPECIFIED,
        },
        match color.matrix {
            Matrix::Bt601 => ffi::AVCOL_SPC_SMPTE170M,
            Matrix::Bt709 => ffi::AVCOL_SPC_BT709,
            Matrix::Bt2020Ncl => ffi::AVCOL_SPC_BT2020_NCL,
            Matrix::Identity => ffi::AVCOL_SPC_RGB,
            Matrix::Unspecified => ffi::AVCOL_SPC_UNSPECIFIED,
        },
        match color.primaries {
            Primaries::Bt709 => ffi::AVCOL_PRI_BT709,
            Primaries::Bt601Ntsc => ffi::AVCOL_PRI_SMPTE170M,
            Primaries::Bt601Pal => ffi::AVCOL_PRI_BT470BG,
            Primaries::Bt2020 => ffi::AVCOL_PRI_BT2020,
            Primaries::DisplayP3 => ffi::AVCOL_PRI_SMPTE432,
            Primaries::Unspecified => ffi::AVCOL_PRI_UNSPECIFIED,
        },
        match color.transfer {
            Transfer::Bt709 => ffi::AVCOL_TRC_BT709,
            Transfer::Srgb => ffi::AVCOL_TRC_IEC61966_2_1,
            Transfer::Linear => ffi::AVCOL_TRC_LINEAR,
            Transfer::Pq => ffi::AVCOL_TRC_SMPTE2084,
            Transfer::Hlg => ffi::AVCOL_TRC_ARIB_STD_B67,
            Transfer::Unspecified => ffi::AVCOL_TRC_UNSPECIFIED,
        },
    )
}

/// BT.709 limited range, what [`FfmpegWriter::push_rgba`] produces.
const BT709: ColorInfo = ColorInfo {
    matrix: Matrix::Bt709,
    range: Range::Limited,
    primaries: Primaries::Bt709,
    transfer: Transfer::Bt709,
};

fn open_video_encoder(
    name: &str,
    codec: VideoCodec,
    settings: &VideoSettings,
    global_header: bool,
    color: ColorInfo,
) -> Result<(AVCodecContext, i32), String> {
    let encoder = AVCodec::find_encoder_by_name(&c(name))
        .ok_or_else(|| format!("{name} is not in this FFmpeg build"))?;
    let format = pixel_format_for(codec, &encoder);
    let mut context = AVCodecContext::new(&encoder);
    context.set_width(settings.width as i32);
    context.set_height(settings.height as i32);
    context.set_pix_fmt(format);
    context.set_time_base(rational(settings.rate.den(), settings.rate.num()));
    context.set_framerate(rational(settings.rate.num(), settings.rate.den()));
    if codec.uses_bit_rate() {
        let bit_rate = settings.bit_rate.map_or_else(
            || default_bit_rate(codec, settings),
            |b| i64::try_from(b).unwrap_or(i64::MAX),
        );
        if bit_rate > 0 {
            context.set_bit_rate(bit_rate);
        }
    }
    context.set_gop_size((settings.rate.nominal_fps() * 2).max(1) as i32);
    if global_header {
        context.set_flags(context.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    }
    if format != ffi::AV_PIX_FMT_PAL8 {
        let (range, matrix, primaries, transfer) = av_color(color);
        // SAFETY: plain enum fields of a context we own that is not open yet.
        unsafe {
            use rsmpeg::UnsafeDerefMut;
            let raw = context.deref_mut();
            raw.color_range = range;
            raw.colorspace = matrix;
            raw.color_primaries = primaries;
            raw.color_trc = transfer;
        }
    }
    context
        .open(encoder_options(name, codec, settings.bit_rate.is_some()))
        .map_err(|e| format!("{name} cannot open: {e}"))?;
    Ok((context, format))
}

/// How the writer stores samples in an audio encoder's frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SampleEncoding {
    /// 32-bit floats, -1 to 1.
    Float,
    /// Signed 16-bit integers.
    Int16,
    /// Signed 32-bit integers whose top `bits` bits hold the sample, as FFmpeg feeds
    /// 24-bit PCM.
    Int32 {
        /// Significant bits, 24 or 32.
        bits: u32,
    },
}

/// The sample format an audio encoder gets: the codec's own depth for PCM and FLAC,
/// otherwise floats when the encoder takes them (planar first), then integers.
fn sample_format_for(codec: AudioCodec, encoder: &AVCodec) -> Option<ffi::AVSampleFormat> {
    let preference: &[ffi::AVSampleFormat] = match codec {
        AudioCodec::Pcm16 | AudioCodec::Flac => &[ffi::AV_SAMPLE_FMT_S16, ffi::AV_SAMPLE_FMT_S16P],
        AudioCodec::Pcm24 => &[ffi::AV_SAMPLE_FMT_S32, ffi::AV_SAMPLE_FMT_S32P],
        AudioCodec::Aac | AudioCodec::Opus | AudioCodec::Mp3 => &[
            ffi::AV_SAMPLE_FMT_FLTP,
            ffi::AV_SAMPLE_FMT_FLT,
            ffi::AV_SAMPLE_FMT_S32P,
            ffi::AV_SAMPLE_FMT_S32,
            ffi::AV_SAMPLE_FMT_S16P,
            ffi::AV_SAMPLE_FMT_S16,
        ],
    };
    let supported = encoder.sample_fmts().unwrap_or(&[]);
    if supported.is_empty() {
        return preference.first().copied();
    }
    preference.iter().copied().find(|f| supported.contains(f))
}

/// The encoding and planarity of one of the formats [`sample_format_for`] picks.
fn sample_layout(format: ffi::AVSampleFormat, codec: AudioCodec) -> Option<(SampleEncoding, bool)> {
    let bits = if codec == AudioCodec::Pcm24 { 24 } else { 32 };
    Some(match format {
        ffi::AV_SAMPLE_FMT_FLT => (SampleEncoding::Float, false),
        ffi::AV_SAMPLE_FMT_FLTP => (SampleEncoding::Float, true),
        ffi::AV_SAMPLE_FMT_S16 => (SampleEncoding::Int16, false),
        ffi::AV_SAMPLE_FMT_S16P => (SampleEncoding::Int16, true),
        ffi::AV_SAMPLE_FMT_S32 => (SampleEncoding::Int32 { bits }, false),
        ffi::AV_SAMPLE_FMT_S32P => (SampleEncoding::Int32 { bits }, true),
        _ => return None,
    })
}

fn open_audio_encoder(
    name: &str,
    codec: AudioCodec,
    settings: &AudioSettings,
    global_header: bool,
) -> Result<AVCodecContext, String> {
    let encoder = AVCodec::find_encoder_by_name(&c(name))
        .ok_or_else(|| format!("{name} is not in this FFmpeg build"))?;
    let format = sample_format_for(codec, &encoder)
        .ok_or_else(|| format!("{name} takes none of the sample formats FossCut writes"))?;
    let mut context = AVCodecContext::new(&encoder);
    context.set_sample_fmt(format);
    context.set_sample_rate(settings.sample_rate as i32);
    context.set_time_base(rational(1, settings.sample_rate));
    // Lossless codecs have no bit rate to aim for; whatever was asked is ignored.
    if !codec.is_lossless() {
        context.set_bit_rate(i64::try_from(settings.bit_rate).unwrap_or(i64::MAX));
    }
    if codec == AudioCodec::Pcm24 {
        // SAFETY: a plain integer field of a context we own that is not open yet.
        unsafe {
            use rsmpeg::UnsafeDerefMut;
            context.deref_mut().bits_per_raw_sample = 24;
        }
    }
    context.set_ch_layout(super::channel_layout(settings.channels)?.into_inner());
    if global_header {
        context.set_flags(context.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    }
    context
        .open(None)
        .map_err(|e| format!("{name} cannot open: {e}"))?;
    Ok(context)
}

/// The encoder this machine uses for a video codec, if any, under the process's
/// [`hardware_encoding`] choice. Candidates are opened at a small size, because an encoder
/// can be compiled in and still be unusable (for example a hardware encoder in a virtual
/// machine). Probed once per codec, choice and process, only when asked, so an H.264
/// export never loads the AV1 encoders (whose libraries print banners).
pub fn video_encoder(codec: VideoCodec) -> Option<&'static EncoderInfo> {
    video_encoder_for(codec, hardware_encoding())
}

/// The encoder this machine uses for a video codec when `hardware` decides whether hardware
/// encoders may be used; probed like [`video_encoder`]. With [`HardwareEncoding::Off`] a
/// codec that only hardware encodes here (H.264 and HEVC on macOS) has none.
pub fn video_encoder_for(
    codec: VideoCodec,
    hardware: HardwareEncoding,
) -> Option<&'static EncoderInfo> {
    static PROBED: [[OnceLock<Option<EncoderInfo>>; VideoCodec::ALL.len()]; 2] =
        [const { [const { OnceLock::new() }; VideoCodec::ALL.len()] }; 2];
    let index = VideoCodec::ALL.iter().position(|c| *c == codec)?;
    let row = usize::from(hardware == HardwareEncoding::Off);
    super::quiet();
    PROBED[row][index]
        .get_or_init(|| {
            let probe = VideoSettings {
                codec,
                width: 256,
                height: 144,
                rate: fosscut_time::Rate::FPS_30,
                bit_rate: None,
            };
            allowed_candidates(codec, hardware)
                .find(|(name, _)| open_video_encoder(name, codec, &probe, false, BT709).is_ok())
                .map(|(name, hardware)| EncoderInfo {
                    encoder: (*name).to_string(),
                    hardware: *hardware,
                })
        })
        .as_ref()
}

/// The encoder this machine uses for an audio codec, if any; probed like
/// [`video_encoder`].
pub fn audio_encoder(codec: AudioCodec) -> Option<&'static EncoderInfo> {
    static PROBED: [OnceLock<Option<EncoderInfo>>; AudioCodec::ALL.len()] =
        [const { OnceLock::new() }; AudioCodec::ALL.len()];
    let index = AudioCodec::ALL.iter().position(|c| *c == codec)?;
    super::quiet();
    PROBED[index]
        .get_or_init(|| {
            audio_candidates(codec)
                .iter()
                .find(|name| {
                    open_audio_encoder(name, codec, &AudioSettings::stereo(codec), false).is_ok()
                })
                .map(|name| EncoderInfo {
                    encoder: (*name).to_string(),
                    hardware: false,
                })
        })
        .as_ref()
}

/// Every codec this machine can encode under the process's [`hardware_encoding`] choice
/// (probes all of them).
pub fn encoder_capabilities() -> &'static EncoderCapabilities {
    encoder_capabilities_for(hardware_encoding())
}

/// Every codec this machine can encode when `hardware` decides whether hardware encoders
/// may be used.
pub fn encoder_capabilities_for(hardware: HardwareEncoding) -> &'static EncoderCapabilities {
    static CAPABILITIES: [OnceLock<EncoderCapabilities>; 2] = [const { OnceLock::new() }; 2];
    CAPABILITIES[usize::from(hardware == HardwareEncoding::Off)].get_or_init(|| {
        EncoderCapabilities {
            video: VideoCodec::ALL
                .iter()
                .filter_map(|&c| video_encoder_for(c, hardware).map(|e| (c, e.clone())))
                .collect(),
            audio: AudioCodec::ALL
                .iter()
                .filter_map(|&c| audio_encoder(c).map(|e| (c, e.clone())))
                .collect(),
        }
    })
}

/// The codec names this machine can encode, each once (the ProRes profiles share one).
fn available_names(capabilities: &EncoderCapabilities, audio: bool) -> Vec<String> {
    let names: Vec<&str> = if audio {
        capabilities
            .audio
            .iter()
            .map(|(c, _)| c.probe_name())
            .collect()
    } else {
        capabilities
            .video
            .iter()
            .map(|(c, _)| c.probe_name())
            .collect()
    };
    let mut unique: Vec<String> = Vec::new();
    for name in names {
        if !unique.iter().any(|n| n == name) {
            unique.push(name.to_string());
        }
    }
    unique
}

struct VideoOut {
    encoder: AVCodecContext,
    stream: usize,
    format: i32,
    width: u32,
    height: u32,
    next_pts: i64,
    /// Converts pushed frames, keyed by their format: to the encoder's format, or to RGBA
    /// for GIF, whose frames are indexed by `quantizer`.
    scaler: Option<(i32, SwsContext)>,
    /// Turns RGBA into palette frames for GIF.
    quantizer: Option<Quantizer>,
    /// Scales frames of another size, keyed by their format, size and range; see
    /// [`FfmpegWriter::push_frame_scaled`].
    resizer: Option<(ResizeKey, SwsContext)>,
    /// The color tags written with every frame.
    color: ColorInfo,
}

/// What a resizing scaler converts from: pixel format, width, height and full range.
type ResizeKey = (i32, u32, u32, bool);

/// How a writer encodes, besides its streams' settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriterOptions {
    /// Whether hardware encoders may be used (ADR-0032, item 6).
    pub hardware: HardwareEncoding,
    /// The color tags of the video stream, for frames pushed with
    /// [`FfmpegWriter::push_frame`] and [`FfmpegWriter::push_frame_scaled`], whose Y'CbCr
    /// values are stored as they are. `None` tags BT.709 limited range, what
    /// [`FfmpegWriter::push_rgba`] produces; RGBA frames are refused with other tags.
    pub color: Option<ColorInfo>,
}

struct AudioOut {
    encoder: AVCodecContext,
    stream: usize,
    channels: usize,
    sample_rate: u32,
    frame_size: usize,
    small_last_frame: bool,
    encoding: SampleEncoding,
    planar: bool,
    pending: Vec<f32>,
    next_pts: i64,
}

/// Encodes and muxes one output file. Push every video frame in order, push audio in any
/// chunk sizes, then call [`FfmpegWriter::finish`].
pub struct FfmpegWriter {
    path: String,
    output: AVFormatContextOutput,
    video: Option<VideoOut>,
    audio: Option<AudioOut>,
}

impl FfmpegWriter {
    /// Creates `path` with the streams in `settings`, using the encoders this machine has
    /// under the process's [`hardware_encoding`] choice.
    pub fn create(path: &Path, settings: &OutputSettings) -> Result<FfmpegWriter, MediaError> {
        FfmpegWriter::create_with(
            path,
            settings,
            &WriterOptions {
                hardware: hardware_encoding(),
                color: None,
            },
        )
    }

    /// Creates `path` with the streams in `settings` as `options` say. A video codec that no
    /// allowed encoder writes fails with [`MediaError::EncoderUnavailable`], listing the
    /// codecs that can be written with the same choice; another codec is never used.
    pub fn create_with(
        path: &Path,
        settings: &OutputSettings,
        options: &WriterOptions,
    ) -> Result<FfmpegWriter, MediaError> {
        let shown = path.display().to_string();
        let video_codec = settings.video.as_ref().map(|v| v.codec);
        let audio_codec = settings.audio.as_ref().map(|a| a.codec);
        if !settings.container.supports(video_codec, audio_codec) {
            return Err(MediaError::Unsupported {
                message: format!(
                    "{} cannot hold {} with {}",
                    settings.container.extension(),
                    video_codec.map_or("no video", VideoCodec::probe_name),
                    audio_codec.map_or("no audio", AudioCodec::probe_name),
                ),
            });
        }
        if settings.video.is_none() && settings.audio.is_none() {
            return Err(MediaError::Unsupported {
                message: "an output needs a video or an audio stream".into(),
            });
        }
        if let Some(video) = &settings.video
            && (video.width % 2 != 0 || video.height % 2 != 0 || video.width == 0)
        {
            return Err(MediaError::Unsupported {
                message: format!("{}x{} is not an even size", video.width, video.height),
            });
        }
        let video_encoder = match video_codec {
            Some(codec) => Some(video_encoder_for(codec, options.hardware).ok_or_else(|| {
                MediaError::EncoderUnavailable {
                    codec: codec.probe_name().to_string(),
                    available: available_names(encoder_capabilities_for(options.hardware), false),
                }
            })?),
            None => None,
        };
        let audio_encoder = match audio_codec {
            Some(codec) => {
                Some(
                    audio_encoder(codec).ok_or_else(|| MediaError::EncoderUnavailable {
                        codec: codec.probe_name().to_string(),
                        available: available_names(encoder_capabilities(), true),
                    })?,
                )
            }
            None => None,
        };

        let write_failed = |message: String| MediaError::WriteFailed {
            path: shown.clone(),
            message,
        };
        let c_path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| write_failed("the path contains a NUL byte".into()))?;
        let muxer = match settings.container {
            Container::Mp4 => c"mp4",
            Container::Mov => c"mov",
            Container::Webm => c"webm",
            Container::Gif => c"gif",
            Container::Wav => c"wav",
            Container::Flac => c"flac",
            Container::Mp3 => c"mp3",
        };
        let mut output = AVFormatContextOutput::builder()
            .format_name(muxer)
            .filename(&c_path)
            .build()
            .map_err(|e| write_failed(e.to_string()))?;
        let global_header = output.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32 != 0;

        let mut video = None;
        let color = options.color.unwrap_or(BT709);
        if let (Some(settings), Some(info)) = (&settings.video, video_encoder) {
            let (encoder, format) = open_video_encoder(
                &info.encoder,
                settings.codec,
                settings,
                global_header,
                color,
            )
            .map_err(|message| MediaError::EncodeFailed { message })?;
            let mut stream = output.new_stream();
            stream.codecpar_mut().from_context(&encoder);
            stream.set_time_base(encoder.time_base);
            stream.set_avg_frame_rate(encoder.framerate);
            let index = stream.index as usize;
            video = Some(VideoOut {
                encoder,
                stream: index,
                format,
                width: settings.width,
                height: settings.height,
                next_pts: 0,
                scaler: None,
                quantizer: (settings.codec == VideoCodec::Gif).then(Quantizer::new),
                resizer: None,
                color,
            });
        }
        let mut audio = None;
        if let (Some(settings), Some(info)) = (&settings.audio, audio_encoder) {
            let encoder =
                open_audio_encoder(&info.encoder, settings.codec, settings, global_header)
                    .map_err(|message| MediaError::EncodeFailed { message })?;
            let (encoding, planar) =
                sample_layout(encoder.sample_fmt, settings.codec).ok_or_else(|| {
                    MediaError::EncodeFailed {
                        message: format!(
                            "{} opened with an unexpected sample format",
                            info.encoder
                        ),
                    }
                })?;
            let mut stream = output.new_stream();
            stream.codecpar_mut().from_context(&encoder);
            stream.set_time_base(encoder.time_base);
            let index = stream.index as usize;
            let frame_size = usize::try_from(encoder.frame_size)
                .ok()
                .filter(|n| *n > 0)
                .unwrap_or(1024);
            let small_last_frame = encoder.codec().capabilities
                & (ffi::AV_CODEC_CAP_SMALL_LAST_FRAME | ffi::AV_CODEC_CAP_VARIABLE_FRAME_SIZE)
                    as i32
                != 0;
            audio = Some(AudioOut {
                encoder,
                stream: index,
                channels: usize::from(settings.channels),
                sample_rate: settings.sample_rate,
                frame_size,
                small_last_frame,
                encoding,
                planar,
                pending: Vec::new(),
                next_pts: 0,
            });
        }
        // GIFs loop forever: the muxer's default, set so the file never depends on it.
        let mut header_options =
            (settings.container == Container::Gif).then(|| AVDictionary::new(c"loop", c"0", 0));
        output
            .write_header(&mut header_options)
            .map_err(|e| write_failed(e.to_string()))?;
        Ok(FfmpegWriter {
            path: shown,
            output,
            video,
            audio,
        })
    }

    /// Number of video frames written so far.
    pub fn frames_written(&self) -> u64 {
        self.video.as_ref().map_or(0, |v| v.next_pts as u64)
    }

    /// Encodes the next video frame (NV12 or YUV 4:2:0, the output size). GIF outputs
    /// convert it to RGBA first and index it like [`FfmpegWriter::push_rgba`].
    pub fn push_frame(&mut self, frame: &VideoFrame) -> Result<(), MediaError> {
        let source_format = match frame.format {
            PixelFormat::Nv12 => ffi::AV_PIX_FMT_NV12,
            PixelFormat::Yuv420p => ffi::AV_PIX_FMT_YUV420P,
        };
        let mut source = self.source_frame(source_format, frame.width, frame.height)?;
        for (index, plane) in frame.planes.iter().enumerate() {
            let (row_bytes, rows) = frame.format.plane_size(index, frame.width, frame.height);
            fill_plane(
                &mut source,
                index,
                &plane.data,
                plane.stride,
                row_bytes,
                rows,
            );
        }
        if let Some(video) = &mut self.video
            && video.quantizer.is_some()
        {
            let rgba = scale(&mut video.scaler, &source, ffi::AV_PIX_FMT_RGBA)?;
            let (row_bytes, rows) = (frame.width as usize * 4, frame.height as usize);
            let plane = super::copy_plane(&rgba, 0, row_bytes, rows)?;
            return self.push_indexed(&plane.data, frame.width, frame.height);
        }
        self.encode_video(&source)
    }

    /// Encodes the next video frame (NV12 or YUV 4:2:0) of any size, scaled to the output
    /// size by averaging areas. Its Y'CbCr values keep their matrix, so the output should be
    /// tagged with the frame's colors ([`WriterOptions::color`]); a full-range frame becomes
    /// limited range when the output is tagged limited. GIF outputs refuse it.
    pub fn push_frame_scaled(&mut self, frame: &VideoFrame) -> Result<(), MediaError> {
        let Some(video) = &mut self.video else {
            return Err(MediaError::Unsupported {
                message: "this output has no video stream".into(),
            });
        };
        if video.quantizer.is_some() {
            return Err(MediaError::Unsupported {
                message: "GIF outputs take frames of their own size".into(),
            });
        }
        let source_format = match frame.format {
            PixelFormat::Nv12 => ffi::AV_PIX_FMT_NV12,
            PixelFormat::Yuv420p => ffi::AV_PIX_FMT_YUV420P,
        };
        let mut source = AVFrame::new();
        source.set_format(source_format);
        source.set_width(frame.width as i32);
        source.set_height(frame.height as i32);
        source.alloc_buffer().map_err(|e| encode_failed(&e))?;
        for (index, plane) in frame.planes.iter().enumerate() {
            let (row_bytes, rows) = frame.format.plane_size(index, frame.width, frame.height);
            fill_plane(
                &mut source,
                index,
                &plane.data,
                plane.stride,
                row_bytes,
                rows,
            );
        }
        let full_in = frame.color.range == Range::Full;
        let full_out = video.color.range == Range::Full;
        let key = (source_format, frame.width, frame.height, full_in);
        let scaler = match &mut video.resizer {
            Some((kept, scaler)) if *kept == key => scaler,
            slot => {
                let scaler = SwsContext::get_context(
                    frame.width as i32,
                    frame.height as i32,
                    source_format,
                    video.width as i32,
                    video.height as i32,
                    video.format,
                    ffi::SWS_AREA,
                    None,
                    None,
                    None,
                )
                .ok_or_else(|| MediaError::EncodeFailed {
                    message: "cannot scale frames to the output size".into(),
                })?;
                set_ranges(&scaler, full_in, full_out);
                &mut slot.insert((key, scaler)).1
            }
        };
        let mut scaled = AVFrame::new();
        scaled.set_format(video.format);
        scaled.set_width(video.width as i32);
        scaled.set_height(video.height as i32);
        scaled.alloc_buffer().map_err(|e| encode_failed(&e))?;
        scaler
            .scale_frame(&source, 0, frame.height as i32, &mut scaled)
            .map_err(|e| encode_failed(&e))?;
        self.encode_video(&scaled)
    }

    /// Encodes the next video frame from non-linear BT.709 RGBA, converting it to the
    /// encoder's limited-range BT.709 Y'CbCr. ProRes 4444 keeps the alpha bytes as its
    /// alpha channel; GIF indexes the colours with a palette per frame and ignores alpha.
    pub fn push_rgba(&mut self, frame: &RgbaFrame) -> Result<(), MediaError> {
        let row = frame.width as usize * 4;
        let needed = row * frame.height as usize;
        if frame.rgba.len() < needed {
            return Err(MediaError::Unsupported {
                message: format!(
                    "an RGBA frame of {}x{} needs {needed} bytes, not {}",
                    frame.width,
                    frame.height,
                    frame.rgba.len()
                ),
            });
        }
        if self.video.as_ref().is_some_and(|v| v.quantizer.is_some()) {
            return self.push_indexed(&frame.rgba, frame.width, frame.height);
        }
        if self.video.as_ref().is_some_and(|v| v.color != BT709) {
            return Err(MediaError::Unsupported {
                message: "RGBA frames are written as BT.709; this output is tagged otherwise"
                    .into(),
            });
        }
        let mut source = self.source_frame(ffi::AV_PIX_FMT_RGBA, frame.width, frame.height)?;
        fill_plane(&mut source, 0, &frame.rgba, row, row, frame.height as usize);
        self.encode_video(&source)
    }

    /// Indexes `width * height` RGBA pixels with the GIF quantizer and encodes the palette
    /// frame.
    fn push_indexed(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<(), MediaError> {
        let mut source = self.source_frame(ffi::AV_PIX_FMT_PAL8, width, height)?;
        let Some(quantizer) = self.video.as_mut().and_then(|v| v.quantizer.as_mut()) else {
            return Ok(());
        };
        let (w, h) = (width as usize, height as usize);
        let indexed = quantizer.index(rgba, w, h);
        fill_plane(&mut source, 0, &indexed.indices, w, w, h);
        fill_palette(&mut source, &indexed.palette)?;
        self.encode_video(&source)
    }

    /// Appends interleaved samples at the output's sample rate and channel count.
    pub fn push_audio(&mut self, buffer: &AudioBuffer) -> Result<(), MediaError> {
        let Some(audio) = &mut self.audio else {
            return Err(MediaError::Unsupported {
                message: "this output has no audio stream".into(),
            });
        };
        if buffer.sample_rate != audio.sample_rate || usize::from(buffer.channels) != audio.channels
        {
            return Err(MediaError::Unsupported {
                message: format!(
                    "audio is {} Hz with {} channels; the output expects {} Hz with {}",
                    buffer.sample_rate, buffer.channels, audio.sample_rate, audio.channels
                ),
            });
        }
        audio.pending.extend_from_slice(&buffer.samples);
        let chunk = audio.frame_size * audio.channels;
        let mut ready = Vec::new();
        while audio.pending.len() >= chunk {
            ready.push(audio.pending.drain(..chunk).collect::<Vec<f32>>());
        }
        for samples in ready {
            self.encode_audio_chunk(&samples)?;
        }
        Ok(())
    }

    /// Flushes the encoders and writes the end of the file.
    pub fn finish(mut self) -> Result<(), MediaError> {
        if let Some(audio) = &mut self.audio
            && !audio.pending.is_empty()
        {
            let mut samples = std::mem::take(&mut audio.pending);
            if !audio.small_last_frame {
                samples.resize(audio.frame_size * audio.channels, 0.0);
            }
            self.encode_audio_chunk(&samples)?;
        }
        if self.video.is_some() {
            self.send(true, None)?;
        }
        if self.audio.is_some() {
            self.send(false, None)?;
        }
        let path = self.path.clone();
        self.output
            .write_trailer()
            .map_err(|e| MediaError::WriteFailed {
                path,
                message: e.to_string(),
            })
    }

    fn source_frame(&self, format: i32, width: u32, height: u32) -> Result<AVFrame, MediaError> {
        let Some(video) = &self.video else {
            return Err(MediaError::Unsupported {
                message: "this output has no video stream".into(),
            });
        };
        if (width, height) != (video.width, video.height) {
            return Err(MediaError::Unsupported {
                message: format!(
                    "frame is {width}x{height}; the output is {}x{}",
                    video.width, video.height
                ),
            });
        }
        let mut frame = AVFrame::new();
        frame.set_format(format);
        frame.set_width(width as i32);
        frame.set_height(height as i32);
        frame.alloc_buffer().map_err(|e| encode_failed(&e))?;
        Ok(frame)
    }

    fn encode_video(&mut self, source: &AVFrame) -> Result<(), MediaError> {
        let Some(video) = &mut self.video else {
            return Ok(());
        };
        let mut frame = if source.format == video.format {
            source.clone()
        } else {
            scale(&mut video.scaler, source, video.format)?
        };
        frame.set_pts(video.next_pts);
        if video.format != ffi::AV_PIX_FMT_PAL8 {
            let (range, matrix, primaries, transfer) = av_color(video.color);
            // SAFETY: plain enum fields of a frame we own.
            unsafe {
                use rsmpeg::UnsafeDerefMut;
                let raw = frame.deref_mut();
                raw.color_range = range;
                raw.colorspace = matrix;
                raw.color_primaries = primaries;
                raw.color_trc = transfer;
            }
        }
        video.next_pts += 1;
        self.send(true, Some(&frame))
    }

    fn encode_audio_chunk(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        let Some(audio) = &mut self.audio else {
            return Ok(());
        };
        let frames = samples.len() / audio.channels;
        let mut frame = AVFrame::new();
        frame.set_format(audio.encoder.sample_fmt);
        frame.set_sample_rate(audio.sample_rate as i32);
        frame.set_nb_samples(frames as i32);
        frame.set_ch_layout(
            super::channel_layout(audio.channels as u16)
                .map_err(|message| MediaError::EncodeFailed { message })?
                .into_inner(),
        );
        frame.alloc_buffer().map_err(|e| encode_failed(&e))?;
        let planes = encode_samples(
            &samples[..frames * audio.channels],
            audio.channels,
            audio.encoding,
            audio.planar,
        );
        let capacity = usize::try_from(frame.linesize[0]).unwrap_or(0);
        for (index, bytes) in planes.iter().enumerate() {
            let destination = frame.data[index];
            if destination.is_null() || capacity < bytes.len() {
                return Err(MediaError::EncodeFailed {
                    message: format!("audio plane {index} is missing or too small"),
                });
            }
            // SAFETY: the frame was just allocated for `frames` samples of `channels`
            // channels in the encoder's sample format, so each of its planes (one when
            // interleaved, one per channel when planar, at most two here, all in `data`)
            // holds `linesize[0]` bytes, which was checked to cover `bytes`.
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len());
            }
        }
        frame.set_pts(audio.next_pts);
        audio.next_pts += frames as i64;
        self.send(false, Some(&frame))
    }

    /// Sends a frame (or the end-of-stream flush) to the video or audio encoder and writes
    /// every packet it returns.
    fn send(&mut self, is_video: bool, frame: Option<&AVFrame>) -> Result<(), MediaError> {
        let (encoder, stream) = match (is_video, &mut self.video, &mut self.audio) {
            (true, Some(video), _) => (&mut video.encoder, video.stream),
            (false, _, Some(audio)) => (&mut audio.encoder, audio.stream),
            _ => return Ok(()),
        };
        encoder.send_frame(frame).map_err(|e| encode_failed(&e))?;
        let stream_time_base = self.output.streams()[stream].time_base;
        loop {
            match encoder.receive_packet() {
                Ok(mut packet) => {
                    packet.set_stream_index(stream as i32);
                    packet.rescale_ts(encoder.time_base, stream_time_base);
                    self.output
                        .interleaved_write_frame(&mut packet)
                        .map_err(|e| MediaError::WriteFailed {
                            path: self.path.clone(),
                            message: e.to_string(),
                        })?;
                }
                Err(RsmpegError::EncoderDrainError) | Err(RsmpegError::EncoderFlushedError) => {
                    return Ok(());
                }
                Err(e) => return Err(encode_failed(&e)),
            }
        }
    }
}

/// Copies `rows` rows of `row_bytes` from `data` (rows `stride` apart) into plane `index`.
fn fill_plane(
    frame: &mut AVFrame,
    index: usize,
    data: &[u8],
    stride: usize,
    row_bytes: usize,
    rows: usize,
) {
    let destination = frame.data[index];
    let line = usize::try_from(frame.linesize[index]).unwrap_or(0);
    if destination.is_null() || line < row_bytes {
        return;
    }
    for y in 0..rows {
        let source = &data[y * stride..y * stride + row_bytes];
        // SAFETY: the frame was allocated for its format and size, so plane `index` holds
        // at least `rows` rows of `linesize >= row_bytes` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(source.as_ptr(), destination.add(y * line), row_bytes);
        }
    }
}

/// Converts `source` to `target` with BT.709 coefficients, reusing the scaler in `slot`
/// while frames keep the same format.
fn scale(
    slot: &mut Option<(i32, SwsContext)>,
    source: &AVFrame,
    target: i32,
) -> Result<AVFrame, MediaError> {
    let scaler = match slot {
        Some((format, scaler)) if *format == source.format => scaler,
        slot => {
            let scaler = SwsContext::get_context(
                source.width,
                source.height,
                source.format,
                source.width,
                source.height,
                target,
                ffi::SWS_BICUBIC,
                None,
                None,
                None,
            )
            .ok_or_else(|| MediaError::EncodeFailed {
                message: "cannot convert frames to the encoder's format".into(),
            })?;
            set_bt709(
                &scaler,
                source.format == ffi::AV_PIX_FMT_RGBA,
                target == ffi::AV_PIX_FMT_RGBA,
            );
            &mut slot.insert((source.format, scaler)).1
        }
    };
    let mut converted = AVFrame::new();
    converted.set_format(target);
    converted.set_width(source.width);
    converted.set_height(source.height);
    converted.alloc_buffer().map_err(|e| encode_failed(&e))?;
    scaler
        .scale_frame(source, 0, source.height, &mut converted)
        .map_err(|e| encode_failed(&e))?;
    Ok(converted)
}

/// Writes the 256 entries of a `pal8` frame's palette.
fn fill_palette(frame: &mut AVFrame, palette: &[u32; 256]) -> Result<(), MediaError> {
    let destination = frame.data[1];
    if destination.is_null() {
        return Err(MediaError::EncodeFailed {
            message: "the palette frame has no palette".into(),
        });
    }
    // SAFETY: `av_frame_get_buffer` gives a `pal8` frame its palette as a second plane of
    // AVPALETTE_SIZE (1024) bytes in `data[1]`, which is non-null here; `palette` is 256
    // native-endian u32 entries, exactly that size, and does not overlap the frame.
    unsafe {
        std::ptr::copy_nonoverlapping(
            palette.as_ptr().cast::<u8>(),
            destination,
            std::mem::size_of_val(palette),
        );
    }
    Ok(())
}

/// Converts interleaved floats to the encoder's planes in native byte order: one plane
/// when interleaved, one per channel when planar. Integers are rounded and clipped, with
/// no dither, so a 16-bit file decodes back to within half a step of the input.
fn encode_samples(
    samples: &[f32],
    channels: usize,
    encoding: SampleEncoding,
    planar: bool,
) -> Vec<Vec<u8>> {
    let channels = channels.max(1);
    let plane_count = if planar { channels } else { 1 };
    let bytes_per_sample = match encoding {
        SampleEncoding::Int16 => 2,
        SampleEncoding::Float | SampleEncoding::Int32 { .. } => 4,
    };
    let mut planes =
        vec![Vec::with_capacity(samples.len() / plane_count * bytes_per_sample); plane_count];
    for (i, &sample) in samples.iter().enumerate() {
        let plane = &mut planes[if planar { i % channels } else { 0 }];
        match encoding {
            SampleEncoding::Float => plane.extend_from_slice(&sample.to_ne_bytes()),
            SampleEncoding::Int16 => {
                plane.extend_from_slice(&(quantize(sample, 16) as i16).to_ne_bytes());
            }
            SampleEncoding::Int32 { bits } => {
                plane.extend_from_slice(&(quantize(sample, bits) << (32 - bits)).to_ne_bytes());
            }
        }
    }
    planes
}

/// A sample from -1 to 1 as a signed integer of `bits` bits (at most 32), rounded to the
/// nearest step and clipped; NaN becomes 0.
fn quantize(sample: f32, bits: u32) -> i32 {
    let scale = f64::from(1u32 << (bits.clamp(2, 32) - 1));
    (f64::from(sample) * scale)
        .round()
        .clamp(-scale, scale - 1.0) as i32
}

/// Makes swscale use BT.709 coefficients instead of its BT.601 default, with full range
/// on the RGB side (`rgb_in` for the input, `rgb_out` for the output) and limited range
/// on the Y'CbCr side.
fn set_bt709(scaler: &SwsContext, rgb_in: bool, rgb_out: bool) {
    // SAFETY: `sws_getCoefficients` returns a pointer to a static table, and the context is
    // valid and owned by the caller for the duration of the call.
    unsafe {
        let table = ffi::sws_getCoefficients(ffi::SWS_CS_ITU709 as i32);
        ffi::sws_setColorspaceDetails(
            scaler.as_ptr() as *mut _,
            table,
            i32::from(rgb_in),
            table,
            i32::from(rgb_out),
            0,
            1 << 16,
            1 << 16,
        );
    }
}

/// Makes a Y'CbCr to Y'CbCr scaler convert from full range (`full_in`) or limited range to
/// full range (`full_out`) or limited range, keeping swscale's default coefficients, which
/// only matter when converting to or from RGB.
fn set_ranges(scaler: &SwsContext, full_in: bool, full_out: bool) {
    // SAFETY: `sws_getCoefficients` returns a pointer to a static table, and the context is
    // valid and owned by the caller for the duration of the call.
    unsafe {
        let table = ffi::sws_getCoefficients(ffi::SWS_CS_DEFAULT as i32);
        ffi::sws_setColorspaceDetails(
            scaler.as_ptr() as *mut _,
            table,
            i32::from(full_in),
            table,
            i32::from(full_out),
            0,
            1 << 16,
            1 << 16,
        );
    }
}

fn encode_failed(error: &RsmpegError) -> MediaError {
    MediaError::EncodeFailed {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_hardware_encoding_off_keeps_only_software_candidates() {
        let names = |codec, hardware| -> Vec<&str> {
            allowed_candidates(codec, hardware)
                .map(|(name, _)| *name)
                .collect()
        };
        assert_eq!(
            names(VideoCodec::H264, HardwareEncoding::Auto),
            [
                "h264_videotoolbox",
                "h264_nvenc",
                "h264_qsv",
                "h264_amf",
                "h264_mf"
            ],
            "every candidate, hardware first"
        );
        assert_eq!(
            names(VideoCodec::H264, HardwareEncoding::Off),
            ["h264_mf"],
            "only Media Foundation, which may run in software"
        );
        assert_eq!(names(VideoCodec::Hevc, HardwareEncoding::Off), ["hevc_mf"]);
        for codec in VideoCodec::ALL {
            let off: Vec<_> = allowed_candidates(codec, HardwareEncoding::Off).collect();
            assert!(
                off.iter().all(|(_, hardware)| !hardware),
                "{codec:?} keeps no hardware encoder"
            );
            let software = video_candidates(codec).iter().filter(|(_, h)| !h).count();
            assert_eq!(
                off.len(),
                software,
                "{codec:?} keeps every software encoder"
            );
        }
        assert_eq!(
            names(VideoCodec::ProResProxy, HardwareEncoding::Off),
            ["prores_ks"]
        );
        assert_eq!(
            names(VideoCodec::Av1, HardwareEncoding::Off),
            names(VideoCodec::Av1, HardwareEncoding::Auto),
            "software codecs are not affected"
        );
    }

    #[test]
    fn color_tags_map_to_ffmpeg_and_back() {
        let tags = ColorInfo {
            matrix: Matrix::Bt601,
            range: Range::Full,
            primaries: Primaries::Bt601Pal,
            transfer: Transfer::Hlg,
        };
        let (range, matrix, primaries, transfer) = av_color(tags);
        assert_eq!(
            super::super::color_info(matrix, range, primaries, transfer),
            tags
        );
        let (range, matrix, primaries, transfer) = av_color(BT709);
        assert_eq!(
            (range, matrix, primaries, transfer),
            (
                ffi::AVCOL_RANGE_MPEG,
                ffi::AVCOL_SPC_BT709,
                ffi::AVCOL_PRI_BT709,
                ffi::AVCOL_TRC_BT709
            ),
            "what every writer wrote before tags were a choice"
        );
    }

    #[test]
    fn samples_are_rounded_and_clipped_to_integers() {
        assert_eq!(quantize(0.0, 16), 0);
        assert_eq!(quantize(0.5, 16), 16_384);
        assert_eq!(quantize(-1.0, 16), -32_768);
        assert_eq!(
            quantize(1.0, 16),
            32_767,
            "full scale clips to the largest step"
        );
        assert_eq!(quantize(3.0, 24), 8_388_607);
        assert_eq!(quantize(-3.0, 24), -8_388_608);
        assert_eq!(
            quantize(1.0 / 65_536.0, 16),
            1,
            "half a step rounds away from zero"
        );
        assert_eq!(quantize(f32::NAN, 16), 0);
        assert_eq!(quantize(1.0, 32), i32::MAX);
        assert_eq!(quantize(-1.0, 32), i32::MIN);
    }

    #[test]
    fn samples_fill_interleaved_and_planar_layouts() {
        let samples = [0.5f32, -0.5, 0.25, -0.25];
        let planar = encode_samples(&samples, 2, SampleEncoding::Float, true);
        assert_eq!(planar.len(), 2);
        assert_eq!(
            planar[0],
            [0.5f32.to_ne_bytes(), 0.25f32.to_ne_bytes()].concat()
        );
        assert_eq!(
            planar[1],
            [(-0.5f32).to_ne_bytes(), (-0.25f32).to_ne_bytes()].concat()
        );
        let interleaved = encode_samples(&samples, 2, SampleEncoding::Int16, false);
        let expected: Vec<u8> = [16_384i16, -16_384, 8_192, -8_192]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        assert_eq!(interleaved, [expected]);
        // 24-bit samples sit in the top three bytes of 32-bit integers.
        let wide = encode_samples(&[0.5], 1, SampleEncoding::Int32 { bits: 24 }, false);
        assert_eq!(wide, [(4_194_304i32 << 8).to_ne_bytes().to_vec()]);
        let split = encode_samples(&samples, 2, SampleEncoding::Int16, true);
        assert_eq!(
            split[1],
            [(-16_384i16).to_ne_bytes(), (-8_192i16).to_ne_bytes()].concat()
        );
    }

    #[test]
    fn encoders_get_the_sample_format_of_their_depth() {
        let format = |name: &str, codec| {
            AVCodec::find_encoder_by_name(&c(name)).and_then(|e| sample_format_for(codec, &e))
        };
        assert_eq!(
            format("pcm_s16le", AudioCodec::Pcm16),
            Some(ffi::AV_SAMPLE_FMT_S16)
        );
        assert_eq!(
            format("pcm_s24le", AudioCodec::Pcm24),
            Some(ffi::AV_SAMPLE_FMT_S32)
        );
        assert_eq!(
            format("flac", AudioCodec::Flac),
            Some(ffi::AV_SAMPLE_FMT_S16)
        );
        assert_eq!(
            format("aac", AudioCodec::Aac),
            Some(ffi::AV_SAMPLE_FMT_FLTP)
        );
        if let Some(lame) = format("libmp3lame", AudioCodec::Mp3) {
            assert_eq!(lame, ffi::AV_SAMPLE_FMT_FLTP);
        }
        assert_eq!(
            sample_layout(ffi::AV_SAMPLE_FMT_S32, AudioCodec::Pcm24),
            Some((SampleEncoding::Int32 { bits: 24 }, false))
        );
        assert_eq!(
            sample_layout(ffi::AV_SAMPLE_FMT_S32P, AudioCodec::Mp3),
            Some((SampleEncoding::Int32 { bits: 32 }, true))
        );
        assert_eq!(sample_layout(ffi::AV_SAMPLE_FMT_DBL, AudioCodec::Aac), None);
    }

    #[test]
    fn constant_quality_yields_to_a_requested_bit_rate() {
        let keys = |name: &str, requested: bool| -> Vec<String> {
            let dict = encoder_options(name, VideoCodec::Vp9, requested);
            let mut keys = Vec::new();
            if let Some(dict) = &dict {
                let mut previous = None;
                while let Some(entry) = dict.get(c"", previous, ffi::AV_DICT_IGNORE_SUFFIX) {
                    keys.push(entry.key().to_string_lossy().into_owned());
                    previous = Some(entry);
                }
            }
            keys
        };
        assert!(keys("libvpx-vp9", false).contains(&"crf".to_string()));
        assert!(!keys("libvpx-vp9", true).contains(&"crf".to_string()));
        assert!(!keys("libvpx-vp9", true).contains(&"b".to_string()));
        assert!(!keys("libsvtav1", true).contains(&"crf".to_string()));
        assert!(!keys("librav1e", true).contains(&"qp".to_string()));
        assert_eq!(prores_profile(VideoCodec::ProRes422), "standard");
        assert_eq!(prores_profile(VideoCodec::ProRes4444), "4444");
    }
}
