//! Writing media files through FFmpeg (ADR-0009): which encoders this machine can use,
//! and a writer that encodes video and audio and muxes them.
//!
//! GPL encoders (`libx264`, `libx265`) are never used, even when the linked FFmpeg has
//! them: H.264 and HEVC come only from hardware encoders.

use std::ffi::CString;
use std::path::Path;
use std::sync::OnceLock;

use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avformat::AVFormatContextOutput;
use rsmpeg::avutil::{AVDictionary, AVFrame};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;

use crate::{
    AudioBuffer, AudioCodec, AudioSettings, Container, EncoderCapabilities, EncoderInfo,
    MediaError, OutputSettings, PixelFormat, RgbaFrame, VideoCodec, VideoFrame, VideoSettings,
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
        VideoCodec::ProRes422 => &[("prores_ks", false)],
        VideoCodec::Av1 => &[
            ("libsvtav1", false),
            ("libaom-av1", false),
            ("librav1e", false),
        ],
        VideoCodec::Vp9 => &[("libvpx-vp9", false)],
    }
}

fn audio_candidates(codec: AudioCodec) -> &'static [&'static str] {
    match codec {
        AudioCodec::Aac => &["aac"],
        AudioCodec::Opus => &["libopus"],
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

/// The pixel format an encoder gets: 10-bit 4:2:2 for ProRes, otherwise NV12 when the
/// encoder takes it (hardware encoders prefer it) or planar 4:2:0.
fn pixel_format_for(codec: VideoCodec, encoder: &AVCodec) -> i32 {
    if codec == VideoCodec::ProRes422 {
        return ffi::AV_PIX_FMT_YUV422P10LE;
    }
    let supported = encoder.pix_fmts().unwrap_or(&[]);
    if supported.contains(&ffi::AV_PIX_FMT_NV12) {
        ffi::AV_PIX_FMT_NV12
    } else {
        ffi::AV_PIX_FMT_YUV420P
    }
}

/// Encoder-specific options: speed and quality settings, and for hardware encoders a
/// refusal to fall back to software.
fn encoder_options(name: &str) -> Option<AVDictionary> {
    let pairs: &[(&str, &str)] = match name {
        "h264_videotoolbox" => &[("allow_sw", "0"), ("profile", "high")],
        "hevc_videotoolbox" => &[("allow_sw", "0"), ("profile", "main")],
        "prores_ks" => &[("profile", "2"), ("vendor", "apl0")],
        "libsvtav1" => &[("preset", "10"), ("crf", "35")],
        "libaom-av1" => &[("cpu-used", "8"), ("crf", "35"), ("row-mt", "1")],
        "librav1e" => &[("speed", "10"), ("qp", "100")],
        "libvpx-vp9" => &[
            ("deadline", "realtime"),
            ("cpu-used", "8"),
            ("crf", "32"),
            ("b", "0"),
        ],
        _ => &[],
    };
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

fn open_video_encoder(
    name: &str,
    codec: VideoCodec,
    settings: &VideoSettings,
    global_header: bool,
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
    let bit_rate = settings
        .bit_rate
        .map_or_else(|| default_bit_rate(codec, settings), |b| b as i64);
    if bit_rate > 0 {
        context.set_bit_rate(bit_rate);
    }
    context.set_gop_size((settings.rate.nominal_fps() * 2).max(1) as i32);
    if global_header {
        context.set_flags(context.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    }
    // SAFETY: plain enum fields of a context we own that is not open yet.
    unsafe {
        use rsmpeg::UnsafeDerefMut;
        let raw = context.deref_mut();
        raw.color_range = ffi::AVCOL_RANGE_MPEG;
        raw.colorspace = ffi::AVCOL_SPC_BT709;
        raw.color_primaries = ffi::AVCOL_PRI_BT709;
        raw.color_trc = ffi::AVCOL_TRC_BT709;
    }
    context
        .open(encoder_options(name))
        .map_err(|e| format!("{name} cannot open: {e}"))?;
    Ok((context, format))
}

fn open_audio_encoder(
    name: &str,
    settings: &AudioSettings,
    global_header: bool,
) -> Result<AVCodecContext, String> {
    let encoder = AVCodec::find_encoder_by_name(&c(name))
        .ok_or_else(|| format!("{name} is not in this FFmpeg build"))?;
    let formats = encoder.sample_fmts().unwrap_or(&[]);
    let format = if formats.contains(&ffi::AV_SAMPLE_FMT_FLTP) || formats.is_empty() {
        ffi::AV_SAMPLE_FMT_FLTP
    } else if formats.contains(&ffi::AV_SAMPLE_FMT_FLT) {
        ffi::AV_SAMPLE_FMT_FLT
    } else {
        return Err(format!("{name} takes no floating-point samples"));
    };
    let mut context = AVCodecContext::new(&encoder);
    context.set_sample_fmt(format);
    context.set_sample_rate(settings.sample_rate as i32);
    context.set_time_base(rational(1, settings.sample_rate));
    context.set_bit_rate(settings.bit_rate as i64);
    context.set_ch_layout(super::channel_layout(settings.channels)?.into_inner());
    if global_header {
        context.set_flags(context.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    }
    context
        .open(None)
        .map_err(|e| format!("{name} cannot open: {e}"))?;
    Ok(context)
}

/// The encoder this machine uses for a video codec, if any. Candidates are opened at a
/// small size, because an encoder can be compiled in and still be unusable (for example a
/// hardware encoder in a virtual machine). Probed once per codec and process, only when
/// asked, so an H.264 export never loads the AV1 encoders (whose libraries print banners).
pub fn video_encoder(codec: VideoCodec) -> Option<&'static EncoderInfo> {
    static PROBED: [OnceLock<Option<EncoderInfo>>; 5] = [const { OnceLock::new() }; 5];
    let index = VideoCodec::ALL.iter().position(|c| *c == codec)?;
    super::quiet();
    PROBED[index]
        .get_or_init(|| {
            let probe = VideoSettings {
                codec,
                width: 256,
                height: 144,
                rate: fosscut_time::Rate::FPS_30,
                bit_rate: None,
            };
            video_candidates(codec)
                .iter()
                .find(|(name, _)| open_video_encoder(name, codec, &probe, false).is_ok())
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
    static PROBED: [OnceLock<Option<EncoderInfo>>; 2] = [const { OnceLock::new() }; 2];
    let index = AudioCodec::ALL.iter().position(|c| *c == codec)?;
    super::quiet();
    PROBED[index]
        .get_or_init(|| {
            audio_candidates(codec)
                .iter()
                .find(|name| open_audio_encoder(name, &AudioSettings::stereo(codec), false).is_ok())
                .map(|name| EncoderInfo {
                    encoder: (*name).to_string(),
                    hardware: false,
                })
        })
        .as_ref()
}

/// Every codec this machine can encode (probes all of them).
pub fn encoder_capabilities() -> &'static EncoderCapabilities {
    static CAPABILITIES: OnceLock<EncoderCapabilities> = OnceLock::new();
    CAPABILITIES.get_or_init(|| EncoderCapabilities {
        video: VideoCodec::ALL
            .iter()
            .filter_map(|&c| video_encoder(c).map(|e| (c, e.clone())))
            .collect(),
        audio: AudioCodec::ALL
            .iter()
            .filter_map(|&c| audio_encoder(c).map(|e| (c, e.clone())))
            .collect(),
    })
}

fn available_names(capabilities: &EncoderCapabilities, audio: bool) -> Vec<String> {
    if audio {
        capabilities
            .audio
            .iter()
            .map(|(c, _)| c.probe_name().to_string())
            .collect()
    } else {
        capabilities
            .video
            .iter()
            .map(|(c, _)| c.probe_name().to_string())
            .collect()
    }
}

struct VideoOut {
    encoder: AVCodecContext,
    stream: usize,
    format: i32,
    width: u32,
    height: u32,
    next_pts: i64,
    scaler: Option<(i32, SwsContext)>,
}

struct AudioOut {
    encoder: AVCodecContext,
    stream: usize,
    channels: usize,
    sample_rate: u32,
    frame_size: usize,
    small_last_frame: bool,
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
    /// Creates `path` with the streams in `settings`, using the encoders this machine has.
    pub fn create(path: &Path, settings: &OutputSettings) -> Result<FfmpegWriter, MediaError> {
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
            Some(codec) => {
                Some(
                    video_encoder(codec).ok_or_else(|| MediaError::EncoderUnavailable {
                        codec: codec.probe_name().to_string(),
                        available: available_names(encoder_capabilities(), false),
                    })?,
                )
            }
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
        };
        let mut output = AVFormatContextOutput::builder()
            .format_name(muxer)
            .filename(&c_path)
            .build()
            .map_err(|e| write_failed(e.to_string()))?;
        let global_header = output.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32 != 0;

        let mut video = None;
        if let (Some(settings), Some(info)) = (&settings.video, video_encoder) {
            let (encoder, format) =
                open_video_encoder(&info.encoder, settings.codec, settings, global_header)
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
            });
        }
        let mut audio = None;
        if let (Some(settings), Some(info)) = (&settings.audio, audio_encoder) {
            let encoder = open_audio_encoder(&info.encoder, settings, global_header)
                .map_err(|message| MediaError::EncodeFailed { message })?;
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
                pending: Vec::new(),
                next_pts: 0,
            });
        }
        output
            .write_header(&mut None)
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

    /// Encodes the next video frame (NV12 or YUV 4:2:0, the output size).
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
        self.encode_video(&source, false)
    }

    /// Encodes the next video frame from non-linear BT.709 RGBA, converting it to the
    /// encoder's limited-range BT.709 Y'CbCr.
    pub fn push_rgba(&mut self, frame: &RgbaFrame) -> Result<(), MediaError> {
        let mut source = self.source_frame(ffi::AV_PIX_FMT_RGBA, frame.width, frame.height)?;
        let row = frame.width as usize * 4;
        fill_plane(&mut source, 0, &frame.rgba, row, row, frame.height as usize);
        self.encode_video(&source, true)
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

    fn encode_video(&mut self, source: &AVFrame, from_rgb: bool) -> Result<(), MediaError> {
        let Some(video) = &mut self.video else {
            return Ok(());
        };
        let mut frame = if source.format == video.format {
            source.clone()
        } else {
            let scaler = match &mut video.scaler {
                Some((format, scaler)) if *format == source.format => scaler,
                slot => {
                    let scaler = SwsContext::get_context(
                        source.width,
                        source.height,
                        source.format,
                        source.width,
                        source.height,
                        video.format,
                        ffi::SWS_BICUBIC,
                        None,
                        None,
                        None,
                    )
                    .ok_or_else(|| MediaError::EncodeFailed {
                        message: "cannot convert frames to the encoder's format".into(),
                    })?;
                    set_bt709(&scaler, from_rgb);
                    &mut slot.insert((source.format, scaler)).1
                }
            };
            let mut converted = AVFrame::new();
            converted.set_format(video.format);
            converted.set_width(source.width);
            converted.set_height(source.height);
            converted.alloc_buffer().map_err(|e| encode_failed(&e))?;
            scaler
                .scale_frame(source, 0, source.height, &mut converted)
                .map_err(|e| encode_failed(&e))?;
            converted
        };
        frame.set_pts(video.next_pts);
        // SAFETY: plain enum fields of a frame we own.
        unsafe {
            use rsmpeg::UnsafeDerefMut;
            let raw = frame.deref_mut();
            raw.color_range = ffi::AVCOL_RANGE_MPEG;
            raw.colorspace = ffi::AVCOL_SPC_BT709;
            raw.color_primaries = ffi::AVCOL_PRI_BT709;
            raw.color_trc = ffi::AVCOL_TRC_BT709;
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
        let planar = audio.encoder.sample_fmt == ffi::AV_SAMPLE_FMT_FLTP;
        // SAFETY: the frame was just allocated for `frames` samples of `channels` channels
        // in the encoder's format: one plane of `frames * channels` floats when interleaved,
        // or `channels` planes of `frames` floats when planar.
        unsafe {
            if planar {
                for channel in 0..audio.channels {
                    let plane =
                        std::slice::from_raw_parts_mut(frame.data[channel].cast::<f32>(), frames);
                    for (i, sample) in plane.iter_mut().enumerate() {
                        *sample = samples[i * audio.channels + channel];
                    }
                }
            } else {
                let plane = std::slice::from_raw_parts_mut(
                    frame.data[0].cast::<f32>(),
                    frames * audio.channels,
                );
                plane.copy_from_slice(&samples[..frames * audio.channels]);
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

/// Makes swscale use BT.709 coefficients, full-range input for RGB and limited-range
/// Y'CbCr output, instead of its BT.601 default.
fn set_bt709(scaler: &SwsContext, from_rgb: bool) {
    // SAFETY: `sws_getCoefficients` returns a pointer to a static table, and the context is
    // valid and owned by the caller for the duration of the call.
    unsafe {
        let table = ffi::sws_getCoefficients(ffi::SWS_CS_ITU709 as i32);
        ffi::sws_setColorspaceDetails(
            scaler.as_ptr() as *mut _,
            table,
            i32::from(from_rgb),
            table,
            0,
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
