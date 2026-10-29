//! FFmpeg backend through `rsmpeg` (ADR-0009): probing and decoding video, with
//! VideoToolbox hardware decoding on macOS.
//!
//! This is one of the modules allowed to use `unsafe` (ADR-0003); every block says why it
//! is sound.

#![allow(unsafe_code)]

mod audio;
mod report;
mod writer;

pub use audio::{FfmpegAudioDecoder, samples_at};
pub use report::probe_output;
pub use writer::{FfmpegWriter, encoder_capabilities};

/// The mono or stereo channel layout.
fn channel_layout(channels: u16) -> Result<rsmpeg::avutil::AVChannelLayout, String> {
    let mask = match channels {
        1 => ffi::AV_CH_LAYOUT_MONO,
        2 => ffi::AV_CH_LAYOUT_STEREO,
        n => return Err(format!("{n} channels are not supported")),
    };
    rsmpeg::avutil::AVChannelLayout::from_mask(mask).ok_or_else(|| "bad channel layout".to_string())
}

/// Keeps FFmpeg to errors on stderr; its informational output is not useful to editors or
/// agents. Called by every entry point of this module.
fn quiet() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: `av_log_set_level` only stores an integer in FFmpeg's global state.
    ONCE.call_once(|| unsafe { ffi::av_log_set_level(ffi::AV_LOG_ERROR as i32) });
}

use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;
use std::sync::Arc;

use fosscut_time::{Flicks, Rate};
use rsmpeg::avcodec::{AVCodec, AVCodecContext, AVPacket};
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::{AVFrame, AVHWDeviceContext, get_pix_fmt_name};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;

use crate::color::{ColorInfo, Matrix, Primaries, Range, Transfer};
use crate::frame::{
    Acceleration, DecoderOptions, HardwareDecoding, PixelFormat, Plane, VideoDecoder, VideoFrame,
    VideoStreamInfo,
};
use crate::{MediaError, timebase};

/// `frame_at` decodes forward instead of seeking when the target is less than this far
/// ahead; a seek costs a decode from the previous keyframe anyway.
const FORWARD_DECODE_LIMIT_SECONDS: i64 = 2;

/// Reads facts about the best video stream of a file without decoding it.
pub fn probe_video(path: &Path) -> Result<VideoStreamInfo, MediaError> {
    let (input, index, codec) = open_input(path)?;
    Ok(stream_info(&input, index, &codec))
}

/// A decoded frame still owned by FFmpeg (possibly in GPU memory), with its timing.
struct Decoded {
    frame: AVFrame,
    pts: Flicks,
    duration: Flicks,
}

impl Decoded {
    fn contains(&self, time: Flicks, slack: Flicks) -> bool {
        covers(self.pts, self.duration, time, slack)
    }
}

/// Whether a frame starting at `pts` and lasting `duration` is displayed at `time`.
/// `slack` absorbs timestamps the container rounded (WebM stores milliseconds, so frame 17
/// at 30 fps is stored at 567 ms instead of 566.67 ms).
fn covers(pts: Flicks, duration: Flicks, time: Flicks, slack: Flicks) -> bool {
    pts - slack <= time && time < pts + duration - slack
}

/// Half a time-base tick when the container's ticks cannot represent the frame duration
/// exactly (so timestamps were rounded), capped at a quarter frame; zero otherwise.
fn rounding_slack(time_base: (i32, i32), frame_duration: Flicks) -> Flicks {
    let (num, den) = time_base;
    if num <= 0 || den <= 0 {
        return Flicks::ZERO;
    }
    // Frame duration in ticks is frame_duration * den / (num * FLICKS_PER_SECOND).
    let numerator = i128::from(frame_duration.get()) * i128::from(den);
    let denominator = i128::from(num) * i128::from(fosscut_time::FLICKS_PER_SECOND);
    if denominator == 0 || numerator % denominator == 0 {
        return Flicks::ZERO;
    }
    let half_tick = timebase::to_flicks(1, num, den).get() / 2;
    Flicks::new(half_tick.min(frame_duration.get() / 4))
}

/// Decodes the best video stream of a file with FFmpeg.
pub struct FfmpegVideoDecoder {
    path: String,
    input: AVFormatContextInput,
    decoder: AVCodecContext,
    info: VideoStreamInfo,
    hardware: HardwareDecoding,
    acceleration: Acceleration,
    start_ticks: i64,
    frame_duration: Flicks,
    /// A packet the decoder refused because its output queue was full.
    pending: Option<AVPacket>,
    input_done: bool,
    decoder_done: bool,
    /// True until anything is decoded or a seek happens: the decoder is at the start.
    at_start: bool,
    /// The most recently decoded frame, not yet copied out.
    held: Option<Decoded>,
    /// The most recently returned frame, for repeated requests of the same time.
    last: Option<VideoFrame>,
    /// Tolerance for rounded container timestamps; see [`rounding_slack`].
    slack: Flicks,
    scalers: HashMap<i32, SwsContext>,
}

impl FfmpegVideoDecoder {
    /// Opens the best video stream of `path`.
    pub fn open(path: &Path, options: DecoderOptions) -> Result<FfmpegVideoDecoder, MediaError> {
        let (input, index, codec) = open_input(path)?;
        let info = stream_info(&input, index, &codec);
        let stream = &input.streams()[index];
        let mut decoder = AVCodecContext::new(&codec);
        decoder
            .apply_codecpar(&stream.codecpar())
            .map_err(|e| decode_failed(&e))?;
        decoder.set_pkt_timebase(stream.time_base);
        // SAFETY: `thread_count` is a plain integer field of a context we own and that is
        // not open yet; 0 lets FFmpeg pick the number of threads.
        unsafe {
            use rsmpeg::UnsafeDerefMut;
            decoder.deref_mut().thread_count = 0;
        }

        let want_hardware = options.hardware != HardwareDecoding::Off;
        if want_hardware {
            match videotoolbox_device() {
                Ok(device) => {
                    decoder.set_hw_device_ctx(device);
                    decoder.set_get_format(Some(prefer_videotoolbox));
                }
                Err(message) if options.hardware == HardwareDecoding::Require => {
                    return Err(MediaError::HardwareUnavailable { message });
                }
                Err(_) => {}
            }
        }
        decoder
            .open(None)
            .map_err(|e| MediaError::DecoderUnavailable {
                codec: format!("{} ({e})", info.codec),
            })?;

        let start_ticks = if stream.start_time == ffi::AV_NOPTS_VALUE {
            0
        } else {
            stream.start_time
        };
        let frame_duration = info
            .rate
            .map(Rate::frame_duration)
            .unwrap_or(Rate::FPS_30.frame_duration());
        let slack = rounding_slack(info.time_base, frame_duration);
        Ok(FfmpegVideoDecoder {
            path: path.display().to_string(),
            input,
            decoder,
            info,
            hardware: options.hardware,
            acceleration: Acceleration::Software,
            start_ticks,
            frame_duration,
            pending: None,
            input_done: false,
            decoder_done: false,
            at_start: true,
            held: None,
            last: None,
            slack,
            scalers: HashMap::new(),
        })
    }

    /// The file this decoder reads.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Feeds the decoder one packet of the video stream, or the end-of-stream flush.
    fn feed(&mut self) -> Result<(), MediaError> {
        if self.input_done {
            return Ok(());
        }
        loop {
            let packet = match self.pending.take() {
                Some(packet) => packet,
                None => match self.input.read_packet().map_err(|e| decode_failed(&e))? {
                    Some(packet) if packet.stream_index as usize == self.info.index => packet,
                    Some(_) => continue,
                    None => {
                        match self.decoder.send_packet(None) {
                            Ok(()) | Err(RsmpegError::DecoderFlushedError) => {}
                            Err(e) => return Err(decode_failed(&e)),
                        }
                        self.input_done = true;
                        return Ok(());
                    }
                },
            };
            return match self.decoder.send_packet(Some(&packet)) {
                Ok(()) => Ok(()),
                Err(RsmpegError::DecoderFullError) => {
                    self.pending = Some(packet);
                    Ok(())
                }
                Err(e) => Err(decode_failed(&e)),
            };
        }
    }

    /// The next frame in presentation order without copying it out of FFmpeg.
    fn next_decoded(&mut self) -> Result<Option<Decoded>, MediaError> {
        loop {
            if self.decoder_done {
                return Ok(None);
            }
            match self.decoder.receive_frame() {
                Ok(frame) => {
                    self.at_start = false;
                    return Ok(Some(self.timed(frame)));
                }
                Err(RsmpegError::DecoderDrainError) if !self.input_done => self.feed()?,
                Err(RsmpegError::DecoderDrainError) | Err(RsmpegError::DecoderFlushedError) => {
                    self.decoder_done = true;
                }
                Err(e) => return Err(decode_failed(&e)),
            }
        }
    }

    fn timed(&self, frame: AVFrame) -> Decoded {
        let (num, den) = self.info.time_base;
        let ticks = if frame.best_effort_timestamp != ffi::AV_NOPTS_VALUE {
            Some(frame.best_effort_timestamp)
        } else if frame.pts != ffi::AV_NOPTS_VALUE {
            Some(frame.pts)
        } else {
            None
        };
        let pts = match (ticks, &self.held) {
            (Some(ticks), _) => timebase::to_flicks(ticks - self.start_ticks, num, den),
            (None, Some(previous)) => previous.pts + previous.duration,
            (None, None) => Flicks::ZERO,
        };
        let duration = if frame.duration > 0 {
            timebase::to_flicks(frame.duration, num, den)
        } else {
            self.frame_duration
        };
        Decoded {
            frame,
            pts,
            duration,
        }
    }

    fn seek(&mut self, time: Flicks) -> Result<(), MediaError> {
        let (num, den) = self.info.time_base;
        let target = self.start_ticks + timebase::from_flicks(time, num, den);
        self.input
            .seek(
                self.info.index as i32,
                target,
                ffi::AVSEEK_FLAG_BACKWARD as i32,
            )
            .map_err(|e| MediaError::SeekFailed {
                message: e.to_string(),
            })?;
        self.decoder.flush_buffers();
        self.pending = None;
        self.input_done = false;
        self.decoder_done = false;
        self.at_start = false;
        self.held = None;
        Ok(())
    }

    /// Copies a decoded frame into system memory as NV12 or YUV 4:2:0.
    fn copy_out(&mut self, decoded: &Decoded) -> Result<VideoFrame, MediaError> {
        let source = &decoded.frame;
        let transferred;
        let image = if source.format == ffi::AV_PIX_FMT_VIDEOTOOLBOX {
            self.acceleration = Acceleration::VideoToolbox;
            let mut system = AVFrame::new();
            system
                .hwframe_transfer_data(source)
                .map_err(|e| decode_failed(&e))?;
            transferred = system;
            &transferred
        } else {
            if self.hardware == HardwareDecoding::Require {
                return Err(MediaError::HardwareUnavailable {
                    message: format!("FFmpeg decodes {} in software here", self.info.codec),
                });
            }
            self.acceleration = Acceleration::Software;
            source
        };

        let mut color = color_info(
            source.colorspace,
            source.color_range,
            source.color_primaries,
            source.color_trc,
        );
        let (format, converted);
        let image = match image.format {
            ffi::AV_PIX_FMT_NV12 => {
                format = PixelFormat::Nv12;
                image
            }
            ffi::AV_PIX_FMT_YUV420P => {
                format = PixelFormat::Yuv420p;
                image
            }
            ffi::AV_PIX_FMT_YUVJ420P => {
                if color.range == Range::Unspecified {
                    color.range = Range::Full;
                }
                format = PixelFormat::Yuv420p;
                image
            }
            other => {
                format = PixelFormat::Nv12;
                converted = self.convert_to_nv12(image, other)?;
                &converted
            }
        };

        let (width, height) = (image.width as u32, image.height as u32);
        let planes = (0..format.plane_count())
            .map(|index| {
                let (row_bytes, rows) = format.plane_size(index, width, height);
                copy_plane(image, index, row_bytes, rows)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(VideoFrame {
            pts: decoded.pts,
            duration: decoded.duration,
            width,
            height,
            format,
            planes,
            color: color.resolved(height),
        })
    }

    fn convert_to_nv12(&mut self, image: &AVFrame, format: i32) -> Result<AVFrame, MediaError> {
        let (width, height) = (image.width, image.height);
        let scaler = match self.scalers.entry(format) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let context = SwsContext::get_context(
                    width,
                    height,
                    format,
                    width,
                    height,
                    ffi::AV_PIX_FMT_NV12,
                    ffi::SWS_BILINEAR,
                    None,
                    None,
                    None,
                )
                .ok_or_else(|| MediaError::DecodeFailed {
                    message: format!("cannot convert {} to NV12", pixel_format_name(format)),
                })?;
                entry.insert(context)
            }
        };
        let mut output = AVFrame::new();
        output.set_format(ffi::AV_PIX_FMT_NV12);
        output.set_width(width);
        output.set_height(height);
        output.alloc_buffer().map_err(|e| decode_failed(&e))?;
        scaler
            .scale_frame(image, 0, height, &mut output)
            .map_err(|e| decode_failed(&e))?;
        Ok(output)
    }

    fn end_of_stream(&self, time: Flicks) -> MediaError {
        let end = self
            .held
            .as_ref()
            .map(|d| d.pts + d.duration)
            .or(self.last.as_ref().map(VideoFrame::end))
            .unwrap_or(Flicks::ZERO);
        MediaError::EndOfStream {
            requested_flicks: time.get(),
            end_flicks: end.get(),
        }
    }

    fn deliver(&mut self, decoded: Decoded) -> Result<VideoFrame, MediaError> {
        let frame = self.copy_out(&decoded)?;
        self.held = Some(decoded);
        self.last = Some(frame.clone());
        Ok(frame)
    }
}

impl VideoDecoder for FfmpegVideoDecoder {
    fn info(&self) -> &VideoStreamInfo {
        &self.info
    }

    fn acceleration(&self) -> Acceleration {
        self.acceleration
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, MediaError> {
        match self.next_decoded()? {
            Some(decoded) => self.deliver(decoded).map(Some),
            None => Ok(None),
        }
    }

    fn frame_at(&mut self, time: Flicks) -> Result<VideoFrame, MediaError> {
        if let Some(last) = &self.last
            && covers(last.pts, last.duration, time, self.slack)
        {
            return Ok(last.clone());
        }
        let limit = Flicks::from_seconds(FORWARD_DECODE_LIMIT_SECONDS).unwrap_or(Flicks::ZERO);
        let must_seek = match &self.held {
            Some(held) if held.contains(time, self.slack) => {
                let held = self.held.take();
                if let Some(held) = held {
                    return self.deliver(held);
                }
                false
            }
            Some(held) => time < held.pts - self.slack || time >= held.pts + held.duration + limit,
            None => !(self.at_start && time < limit),
        };
        if must_seek {
            self.seek(time)?;
        }
        let mut previous: Option<Decoded> = None;
        loop {
            match self.next_decoded()? {
                Some(decoded) if decoded.contains(time, self.slack) => {
                    return self.deliver(decoded);
                }
                Some(decoded) if decoded.pts - self.slack > time => {
                    // The target falls before this frame: before the first frame of the
                    // stream, or in a gap between frames. Show the earlier frame if any.
                    return self.deliver(previous.unwrap_or(decoded));
                }
                Some(decoded) => previous = Some(decoded),
                None => {
                    if let Some(previous) = previous {
                        self.held = Some(previous);
                    }
                    return Err(self.end_of_stream(time));
                }
            }
        }
    }
}

fn open_input(
    path: &Path,
) -> Result<
    (
        AVFormatContextInput,
        usize,
        rsmpeg::avcodec::AVCodecRef<'static>,
    ),
    MediaError,
> {
    open_stream(path, ffi::AVMEDIA_TYPE_VIDEO)
}

/// Opens `path` and finds its best stream of `kind` (video or audio).
fn open_stream(
    path: &Path,
    kind: ffi::AVMediaType,
) -> Result<
    (
        AVFormatContextInput,
        usize,
        rsmpeg::avcodec::AVCodecRef<'static>,
    ),
    MediaError,
> {
    quiet();
    let shown = path.display().to_string();
    let open_failed = |message: String| MediaError::OpenFailed {
        path: shown.clone(),
        message,
    };
    let c_path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| open_failed("the path contains a NUL byte".to_string()))?;
    let input = AVFormatContextInput::open(&c_path).map_err(|e| open_failed(e.to_string()))?;
    let found = input
        .find_best_stream(kind)
        .map_err(|e| open_failed(e.to_string()))?;
    let (index, codec) = found.ok_or_else(|| {
        if kind == ffi::AVMEDIA_TYPE_AUDIO {
            MediaError::NoAudioStream {
                path: shown.clone(),
            }
        } else {
            MediaError::NoVideoStream {
                path: shown.clone(),
            }
        }
    })?;
    Ok((input, index, codec))
}

fn stream_info(input: &AVFormatContextInput, index: usize, codec: &AVCodec) -> VideoStreamInfo {
    let stream = &input.streams()[index];
    let parameters = stream.codecpar();
    let time_base = (stream.time_base.num, stream.time_base.den);
    let duration = if stream.duration != ffi::AV_NOPTS_VALUE && stream.duration > 0 {
        Some(timebase::to_flicks(
            stream.duration,
            time_base.0,
            time_base.1,
        ))
    } else if input.duration > 0 {
        Some(timebase::to_flicks(
            input.duration,
            1,
            ffi::AV_TIME_BASE as i32,
        ))
    } else {
        None
    };
    VideoStreamInfo {
        index,
        codec: codec.name().to_string_lossy().into_owned(),
        width: parameters.width.max(0) as u32,
        height: parameters.height.max(0) as u32,
        rate: timebase::rate(stream.avg_frame_rate.num, stream.avg_frame_rate.den),
        time_base,
        duration,
        frame_count: u64::try_from(stream.nb_frames).ok().filter(|n| *n > 0),
        rotation: rotation(&parameters),
        color: color_info(
            parameters.color_space,
            parameters.color_range,
            parameters.color_primaries,
            parameters.color_trc,
        ),
        source_pixel_format: pixel_format_name(parameters.format),
    }
}

/// Degrees clockwise from the stream's display matrix, rounded to a multiple of 90.
fn rotation(parameters: &ffi::AVCodecParameters) -> u16 {
    // SAFETY: `coded_side_data` points to `nb_coded_side_data` entries owned by the stream,
    // which outlives this call; FFmpeg returns null when there is no display matrix.
    let side = unsafe {
        ffi::av_packet_side_data_get(
            parameters.coded_side_data,
            parameters.nb_coded_side_data,
            ffi::AV_PKT_DATA_DISPLAYMATRIX,
        )
    };
    if side.is_null() {
        return 0;
    }
    // SAFETY: a display matrix side data entry holds nine i32 values (FFmpeg's
    // `av_display_rotation_get` contract), and `side` is non-null and owned by the stream.
    let counterclockwise = unsafe {
        let data = (*side).data;
        if data.is_null() || (*side).size < 36 {
            return 0;
        }
        ffi::av_display_rotation_get(data.cast::<i32>())
    };
    if !counterclockwise.is_finite() {
        return 0;
    }
    let quarter_turns = (-counterclockwise / 90.0).round() as i64;
    (quarter_turns.rem_euclid(4) * 90) as u16
}

/// Takes FFmpeg's enum aliases rather than a fixed integer type: bindgen makes C enums
/// unsigned on macOS and Linux but signed with MSVC on Windows.
fn color_info(
    matrix: ffi::AVColorSpace,
    range: ffi::AVColorRange,
    primaries: ffi::AVColorPrimaries,
    transfer: ffi::AVColorTransferCharacteristic,
) -> ColorInfo {
    ColorInfo {
        matrix: match matrix {
            ffi::AVCOL_SPC_BT709 => Matrix::Bt709,
            ffi::AVCOL_SPC_BT470BG | ffi::AVCOL_SPC_SMPTE170M => Matrix::Bt601,
            ffi::AVCOL_SPC_BT2020_NCL => Matrix::Bt2020Ncl,
            ffi::AVCOL_SPC_RGB => Matrix::Identity,
            _ => Matrix::Unspecified,
        },
        range: match range {
            ffi::AVCOL_RANGE_MPEG => Range::Limited,
            ffi::AVCOL_RANGE_JPEG => Range::Full,
            _ => Range::Unspecified,
        },
        primaries: match primaries {
            ffi::AVCOL_PRI_BT709 => Primaries::Bt709,
            ffi::AVCOL_PRI_SMPTE170M => Primaries::Bt601Ntsc,
            ffi::AVCOL_PRI_BT470BG => Primaries::Bt601Pal,
            ffi::AVCOL_PRI_BT2020 => Primaries::Bt2020,
            ffi::AVCOL_PRI_SMPTE432 => Primaries::DisplayP3,
            _ => Primaries::Unspecified,
        },
        transfer: match transfer {
            ffi::AVCOL_TRC_BT709
            | ffi::AVCOL_TRC_SMPTE170M
            | ffi::AVCOL_TRC_BT2020_10
            | ffi::AVCOL_TRC_BT2020_12 => Transfer::Bt709,
            ffi::AVCOL_TRC_IEC61966_2_1 => Transfer::Srgb,
            ffi::AVCOL_TRC_LINEAR => Transfer::Linear,
            ffi::AVCOL_TRC_SMPTE2084 => Transfer::Pq,
            ffi::AVCOL_TRC_ARIB_STD_B67 => Transfer::Hlg,
            _ => Transfer::Unspecified,
        },
    }
}

fn pixel_format_name(format: i32) -> String {
    get_pix_fmt_name(format)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("pixel format {format}"))
}

/// Copies `rows` rows of `row_bytes` bytes from plane `index` into tightly packed memory.
fn copy_plane(
    frame: &AVFrame,
    index: usize,
    row_bytes: usize,
    rows: usize,
) -> Result<Plane, MediaError> {
    let source = frame.data[index];
    let stride = usize::try_from(frame.linesize[index]).unwrap_or(0);
    if source.is_null() || stride < row_bytes {
        return Err(MediaError::DecodeFailed {
            message: format!("plane {index} is missing or has an unexpected layout"),
        });
    }
    let mut data = Vec::with_capacity(row_bytes * rows);
    for y in 0..rows {
        // SAFETY: FFmpeg allocates each plane of a decoded or converted frame with at least
        // `linesize` bytes per row for every row of the plane, and `row_bytes <= linesize`
        // was checked above. The frame stays alive for the duration of the copy.
        let row = unsafe { std::slice::from_raw_parts(source.add(y * stride), row_bytes) };
        data.extend_from_slice(row);
    }
    Ok(Plane {
        stride: row_bytes,
        rows,
        data: Arc::from(data),
    })
}

fn videotoolbox_device() -> Result<AVHWDeviceContext, String> {
    if !cfg!(target_os = "macos") {
        return Err("hardware decoding is only implemented for VideoToolbox on macOS".into());
    }
    AVHWDeviceContext::create(ffi::AV_HWDEVICE_TYPE_VIDEOTOOLBOX, None, None, 0)
        .map_err(|e| format!("cannot create a VideoToolbox device: {e}"))
}

/// FFmpeg `get_format` callback: VideoToolbox when offered, otherwise the first software
/// format, so unsupported streams fall back to software decoding.
unsafe extern "C" fn prefer_videotoolbox(
    _context: *mut ffi::AVCodecContext,
    formats: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    let mut software = ffi::AV_PIX_FMT_NONE;
    let mut cursor = formats;
    loop {
        // SAFETY: FFmpeg passes a list terminated by AV_PIX_FMT_NONE, so every element up
        // to and including the terminator may be read.
        let format = unsafe { *cursor };
        if format == ffi::AV_PIX_FMT_NONE {
            return software;
        }
        if format == ffi::AV_PIX_FMT_VIDEOTOOLBOX {
            return format;
        }
        // SAFETY: `av_pix_fmt_desc_get` returns null or a pointer to a static descriptor.
        let descriptor = unsafe { ffi::av_pix_fmt_desc_get(format) };
        // SAFETY: checked non-null; descriptors are static and immutable.
        let hardware = !descriptor.is_null()
            && unsafe { (*descriptor).flags } & u64::from(ffi::AV_PIX_FMT_FLAG_HWACCEL) != 0;
        if !hardware && software == ffi::AV_PIX_FMT_NONE {
            software = format;
        }
        // SAFETY: the current element is not the terminator, so the next one exists.
        cursor = unsafe { cursor.add(1) };
    }
}

fn decode_failed(error: &RsmpegError) -> MediaError {
    MediaError::DecodeFailed {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_tags_map_to_named_values() {
        let tagged = color_info(
            ffi::AVCOL_SPC_BT709,
            ffi::AVCOL_RANGE_MPEG,
            ffi::AVCOL_PRI_BT709,
            ffi::AVCOL_TRC_BT709,
        );
        assert_eq!(tagged.matrix, Matrix::Bt709);
        assert_eq!(tagged.range, Range::Limited);
        assert_eq!(tagged.primaries, Primaries::Bt709);
        assert_eq!(tagged.transfer, Transfer::Bt709);
        let hdr = color_info(
            ffi::AVCOL_SPC_BT2020_NCL,
            ffi::AVCOL_RANGE_JPEG,
            ffi::AVCOL_PRI_BT2020,
            ffi::AVCOL_TRC_SMPTE2084,
        );
        assert_eq!(
            (hdr.matrix, hdr.range, hdr.primaries, hdr.transfer),
            (
                Matrix::Bt2020Ncl,
                Range::Full,
                Primaries::Bt2020,
                Transfer::Pq
            )
        );
        let untagged = color_info(
            ffi::AVCOL_SPC_UNSPECIFIED,
            ffi::AVCOL_RANGE_UNSPECIFIED,
            ffi::AVCOL_PRI_UNSPECIFIED,
            ffi::AVCOL_TRC_UNSPECIFIED,
        );
        assert_eq!(untagged, ColorInfo::default());
        let sd = color_info(
            ffi::AVCOL_SPC_SMPTE170M,
            ffi::AVCOL_RANGE_MPEG,
            ffi::AVCOL_PRI_SMPTE170M,
            ffi::AVCOL_TRC_SMPTE170M,
        );
        assert_eq!(
            (sd.matrix, sd.primaries, sd.transfer),
            (Matrix::Bt601, Primaries::Bt601Ntsc, Transfer::Bt709)
        );
        let pal = color_info(
            ffi::AVCOL_SPC_BT470BG,
            ffi::AVCOL_RANGE_MPEG,
            ffi::AVCOL_PRI_BT470BG,
            ffi::AVCOL_TRC_IEC61966_2_1,
        );
        assert_eq!(
            (pal.matrix, pal.primaries, pal.transfer),
            (Matrix::Bt601, Primaries::Bt601Pal, Transfer::Srgb)
        );
        let other = color_info(
            ffi::AVCOL_SPC_RGB,
            ffi::AVCOL_RANGE_JPEG,
            ffi::AVCOL_PRI_SMPTE432,
            ffi::AVCOL_TRC_LINEAR,
        );
        assert_eq!(
            (other.matrix, other.primaries, other.transfer),
            (Matrix::Identity, Primaries::DisplayP3, Transfer::Linear)
        );
        let hlg = color_info(0, 0, 0, ffi::AVCOL_TRC_ARIB_STD_B67);
        assert_eq!(hlg.transfer, Transfer::Hlg);
    }

    #[test]
    fn rounding_slack_only_applies_to_rounded_time_bases() {
        let frame = Rate::FPS_30.frame_duration();
        // WebM's millisecond ticks cannot hold 1/30 s: half a millisecond of slack.
        assert_eq!(rounding_slack((1, 1000), frame), Flicks::new(352_800));
        // Exact time bases need none.
        assert_eq!(rounding_slack((1, 30), frame), Flicks::ZERO);
        assert_eq!(rounding_slack((1, 15_360), frame), Flicks::ZERO);
        assert_eq!(rounding_slack((1, 90_000), frame), Flicks::ZERO);
        // Coarse ticks are capped at a quarter frame.
        assert_eq!(rounding_slack((1, 7), frame), Flicks::new(frame.get() / 4));
        assert_eq!(rounding_slack((0, 0), frame), Flicks::ZERO);
        assert!(covers(
            Flicks::new(400_075_200),
            frame,
            Flicks::new(399_840_000),
            Flicks::new(352_800)
        ));
    }

    #[test]
    fn pixel_format_names_come_from_ffmpeg() {
        assert_eq!(pixel_format_name(ffi::AV_PIX_FMT_NV12), "nv12");
        assert_eq!(pixel_format_name(-5), "pixel format -5");
    }
}
