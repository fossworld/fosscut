//! Copying a file's streams into QuickTime without re-encoding them (ADR-0032, item 3):
//! MPEG transport and program streams, AVI and FLV seek slowly or inexactly, while the same
//! packets in QuickTime seek exactly by their index.

use std::ffi::CString;
use std::path::Path;

use rsmpeg::avformat::{AVFormatContextInput, AVFormatContextOutput};
use rsmpeg::ffi;

use super::{probe_output, quiet};
use crate::{MediaError, OutputReport};

/// Copies the video and audio streams of `input` into a new QuickTime file at `output`
/// without re-encoding, and probes what was written. Streams QuickTime cannot hold
/// (subtitles, data, audio in a codec it has no tag for) are left out; a file whose video
/// it cannot hold fails with [`MediaError::Unsupported`]. Every packet is kept, with its
/// timestamps' spacing and the streams' relative start times, so the copy decodes to the
/// same frames in the same order.
///
/// Frame times measured from the start of the stream match the original's, with one
/// exception: AVI records only decoding times, so with reordered frames (B-frames) FFmpeg
/// starts the original's stream one reordering delay before its first frame, while the
/// copy, which records presentation times, starts at its first frame. Callers that must
/// show the original's frame at a time compare the first frames' times once and shift by
/// the difference, as the engine's preview does.
pub fn remux(input: &Path, output: &Path) -> Result<OutputReport, MediaError> {
    remux_with_progress(input, output, &mut |_, _| true)
}

/// [`remux`], calling `progress` with the milliseconds copied and the file's duration in
/// milliseconds (0 when unknown) as it goes; returning `false` stops with
/// [`MediaError::Cancelled`], leaving a partial `output` for the caller to remove.
pub fn remux_with_progress(
    input: &Path,
    output: &Path,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<OutputReport, MediaError> {
    quiet();
    let shown = input.display().to_string();
    let open_failed = |message: String| MediaError::OpenFailed {
        path: shown.clone(),
        message,
    };
    let c_input = CString::new(input.as_os_str().as_encoded_bytes())
        .map_err(|_| open_failed("the path contains a NUL byte".to_string()))?;
    let mut source =
        AVFormatContextInput::open(&c_input).map_err(|e| open_failed(e.to_string()))?;
    // AVI and some program streams leave presentation times of reordered frames unset;
    // QuickTime needs them, so FFmpeg derives them while reading.
    // SAFETY: a plain integer field of a context we own, read by `av_read_frame`.
    unsafe {
        use rsmpeg::UnsafeDerefMut;
        source.deref_mut().flags |= ffi::AVFMT_FLAG_GENPTS as i32;
    }

    let written = output.display().to_string();
    let write_failed = |message: String| MediaError::WriteFailed {
        path: written.clone(),
        message,
    };
    let c_output = CString::new(output.as_os_str().as_encoded_bytes())
        .map_err(|_| write_failed("the path contains a NUL byte".to_string()))?;
    let mut target = AVFormatContextOutput::builder()
        .format_name(c"mov")
        .filename(&c_output)
        .build()
        .map_err(|e| write_failed(e.to_string()))?;

    // Which output stream each input stream goes to.
    let mut mapping: Vec<Option<usize>> = Vec::with_capacity(source.streams().len());
    let mut has_video = false;
    for stream in source.streams() {
        let parameters = stream.codecpar();
        let kind = parameters.codec_type;
        let picture = stream.disposition & ffi::AV_DISPOSITION_ATTACHED_PIC as i32 != 0;
        let wanted =
            (kind == ffi::AVMEDIA_TYPE_VIDEO && !picture) || kind == ffi::AVMEDIA_TYPE_AUDIO;
        // SAFETY: `avformat_query_codec` only reads the output format's static tables.
        let holds = unsafe {
            ffi::avformat_query_codec(
                target.oformat().as_ptr(),
                parameters.codec_id,
                ffi::FF_COMPLIANCE_NORMAL as i32,
            )
        } == 1;
        if !(wanted && holds) {
            mapping.push(None);
            continue;
        }
        has_video |= kind == ffi::AVMEDIA_TYPE_VIDEO;
        let (time_base, rate, aspect) = (
            stream.time_base,
            stream.avg_frame_rate,
            stream.sample_aspect_ratio,
        );
        let mut copy = target.new_stream();
        copy.codecpar_mut().copy(&parameters);
        // SAFETY: a plain integer field of parameters this output owns; QuickTime picks
        // its own tag for the codec when it is zero.
        unsafe {
            use rsmpeg::UnsafeDerefMut;
            copy.codecpar_mut().deref_mut().codec_tag = 0;
        }
        copy.set_time_base(time_base);
        copy.set_avg_frame_rate(rate);
        copy.set_sample_aspect_ratio(aspect);
        mapping.push(Some(copy.index as usize));
    }
    if !has_video {
        return Err(MediaError::Unsupported {
            message: format!("{shown} has no video stream QuickTime can hold"),
        });
    }
    target
        .write_header(&mut None)
        .map_err(|e| write_failed(e.to_string()))?;

    // The copy starts where the original's first stream starts, as players show it.
    let start = if source.start_time == ffi::AV_NOPTS_VALUE {
        0
    } else {
        source.start_time
    };
    let microseconds = ffi::AVRational {
        num: 1,
        den: ffi::AV_TIME_BASE as i32,
    };
    let total_ms = u64::try_from(source.duration.max(0) / 1000).unwrap_or(0);
    let input_bases: Vec<ffi::AVRational> = source.streams().iter().map(|s| s.time_base).collect();
    let output_bases: Vec<ffi::AVRational> = target.streams().iter().map(|s| s.time_base).collect();
    let mut last_report = 0u64;
    while let Some(mut packet) = source.read_packet().map_err(|e| MediaError::DecodeFailed {
        message: e.to_string(),
    })? {
        let from = packet.stream_index as usize;
        let Some(Some(to)) = mapping.get(from).copied() else {
            continue;
        };
        let (input_base, output_base) = (input_bases[from], output_bases[to]);
        // SAFETY: `av_rescale_q` is pure arithmetic.
        let offset = unsafe { ffi::av_rescale_q(start, microseconds, input_base) };
        if packet.pts != ffi::AV_NOPTS_VALUE {
            packet.set_pts(packet.pts - offset);
        }
        if packet.dts != ffi::AV_NOPTS_VALUE {
            packet.set_dts(packet.dts - offset);
        }
        let shown_at = (packet.pts != ffi::AV_NOPTS_VALUE).then(|| {
            // SAFETY: pure arithmetic.
            let ms = unsafe {
                ffi::av_rescale_q(
                    packet.pts,
                    input_base,
                    ffi::AVRational { num: 1, den: 1000 },
                )
            };
            u64::try_from(ms).unwrap_or(0)
        });
        packet.rescale_ts(input_base, output_base);
        packet.set_stream_index(to as i32);
        packet.set_pos(-1);
        target
            .interleaved_write_frame(&mut packet)
            .map_err(|e| write_failed(e.to_string()))?;
        if let Some(done) = shown_at
            && done >= last_report + 250
        {
            last_report = done;
            let done = if total_ms > 0 {
                done.min(total_ms)
            } else {
                done
            };
            if !progress(done, total_ms) {
                return Err(MediaError::Cancelled);
            }
        }
    }
    target
        .write_trailer()
        .map_err(|e| write_failed(e.to_string()))?;
    drop(target);
    if !progress(total_ms, total_ms) {
        return Err(MediaError::Cancelled);
    }
    probe_output(output)
}
