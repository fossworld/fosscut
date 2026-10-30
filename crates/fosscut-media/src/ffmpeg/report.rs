//! What a written file actually contains (ADR-0009, item 6): probed from the file itself,
//! so callers can compare it with what they asked for.

use std::ffi::CStr;
use std::path::Path;

use rsmpeg::ffi;

use super::{color_info, open_stream, pixel_format_name};
use crate::{AudioReport, MediaError, OutputReport, VideoReport, timebase};

fn codec_name(id: ffi::AVCodecID) -> String {
    // SAFETY: `avcodec_get_name` returns a pointer to a static string for any id.
    unsafe { CStr::from_ptr(ffi::avcodec_get_name(id)) }
        .to_string_lossy()
        .into_owned()
}

fn profile_name(id: ffi::AVCodecID, profile: i32) -> Option<String> {
    // SAFETY: `avcodec_profile_name` returns null or a pointer to a static string.
    let name = unsafe { ffi::avcodec_profile_name(id, profile) };
    (!name.is_null()).then(|| {
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    })
}

/// Probes `path` and reports its container, streams, frame count and color tags. Frame
/// counts come from the container when it records them and are counted otherwise. Files
/// with no video (WAV, FLAC, MP3, M4A) report only their audio, and GIFs no audio.
pub fn probe_output(path: &Path) -> Result<OutputReport, MediaError> {
    let size_bytes = std::fs::metadata(path)
        .map_err(|e| MediaError::OpenFailed {
            path: path.display().to_string(),
            message: e.to_string(),
        })?
        .len();
    let video = match open_stream(path, ffi::AVMEDIA_TYPE_VIDEO) {
        Ok((mut input, index, _)) => Some(video_report(&mut input, index)?),
        Err(MediaError::NoVideoStream { .. }) => None,
        Err(error) => return Err(error),
    };
    let (input, audio) = match open_stream(path, ffi::AVMEDIA_TYPE_AUDIO) {
        Ok((input, index, _)) => {
            let audio = {
                let stream = &input.streams()[index];
                let parameters = stream.codecpar();
                AudioReport {
                    codec: codec_name(parameters.codec_id),
                    sample_rate: parameters.sample_rate.max(0) as u32,
                    channels: parameters.ch_layout.nb_channels.max(0) as u16,
                    duration: stream_duration(stream.duration, stream.time_base),
                }
            };
            (Some(input), Some(audio))
        }
        Err(MediaError::NoAudioStream { .. }) => (None, None),
        Err(error) => return Err(error),
    };
    let input = match input {
        Some(input) => input,
        None => open_stream(path, ffi::AVMEDIA_TYPE_VIDEO)?.0,
    };
    let brand = input.metadata().and_then(|m| {
        m.get(c"major_brand", None, 0)
            .map(|v| v.value().to_string_lossy().into_owned())
    });
    Ok(OutputReport {
        container: input.iformat().name().to_string_lossy().into_owned(),
        brand,
        size_bytes,
        duration: (input.duration > 0)
            .then(|| timebase::to_flicks(input.duration, 1, ffi::AV_TIME_BASE as i32)),
        video,
        audio,
    })
}

fn stream_duration(duration: i64, time_base: ffi::AVRational) -> Option<fosscut_time::Flicks> {
    (duration != ffi::AV_NOPTS_VALUE && duration > 0)
        .then(|| timebase::to_flicks(duration, time_base.num, time_base.den))
}

/// The average rate of `frames` frames lasting `duration` ticks of `time_base`, exact.
fn average_rate(
    frames: u64,
    duration: i64,
    time_base: ffi::AVRational,
) -> Option<fosscut_time::Rate> {
    let num = u64::try_from(time_base.num).ok()?;
    let den = u64::try_from(time_base.den).ok()?;
    let ticks = u64::try_from(duration).ok()?;
    // frames / (ticks * num / den) frames per second.
    let (mut top, mut bottom) = (frames.checked_mul(den)?, ticks.checked_mul(num)?);
    let divisor = gcd(top, bottom);
    if divisor == 0 {
        return None;
    }
    top /= divisor;
    bottom /= divisor;
    fosscut_time::Rate::new(u32::try_from(top).ok()?, u32::try_from(bottom).ok()?).ok()
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn video_report(
    input: &mut rsmpeg::avformat::AVFormatContextInput,
    index: usize,
) -> Result<VideoReport, MediaError> {
    let (report, recorded, ticks) = {
        let stream = &input.streams()[index];
        let parameters = stream.codecpar();
        let report = VideoReport {
            codec: codec_name(parameters.codec_id),
            profile: profile_name(parameters.codec_id, parameters.profile),
            width: parameters.width.max(0) as u32,
            height: parameters.height.max(0) as u32,
            rate: timebase::rate(stream.avg_frame_rate.num, stream.avg_frame_rate.den),
            frame_count: 0,
            duration: stream_duration(stream.duration, stream.time_base),
            pixel_format: pixel_format_name(parameters.format),
            color: color_info(
                parameters.color_space,
                parameters.color_range,
                parameters.color_primaries,
                parameters.color_trc,
            ),
        };
        let ticks = (stream.duration != ffi::AV_NOPTS_VALUE && stream.duration > 0)
            .then_some((stream.duration, stream.time_base));
        (report, u64::try_from(stream.nb_frames).unwrap_or(0), ticks)
    };
    let frame_count = if recorded > 0 {
        recorded
    } else {
        let mut count = 0;
        while let Some(packet) = input.read_packet().map_err(|e| MediaError::DecodeFailed {
            message: e.to_string(),
        })? {
            if packet.stream_index as usize == index {
                count += 1;
            }
        }
        count
    };
    // GIF frame delays are whole centiseconds, so FFmpeg's estimate from the first few
    // frames is off (25 for a 24 fps file); the true average is frames over duration.
    let rate = match ticks {
        Some((duration, time_base)) if report.codec == "gif" => {
            average_rate(frame_count, duration, time_base).or(report.rate)
        }
        _ => report.rate,
    };
    Ok(VideoReport {
        frame_count,
        rate,
        ..report
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn average_rates_are_exact_fractions() {
        let centiseconds = ffi::AVRational { num: 1, den: 100 };
        assert_eq!(
            average_rate(12, 50, centiseconds),
            Some(fosscut_time::Rate::FPS_24)
        );
        assert_eq!(
            average_rate(31, 103, centiseconds),
            fosscut_time::Rate::new(3100, 103).ok()
        );
        assert_eq!(average_rate(0, 50, centiseconds), None);
        assert_eq!(average_rate(12, 0, centiseconds), None);
        assert_eq!(average_rate(12, -5, centiseconds), None);
    }
}
