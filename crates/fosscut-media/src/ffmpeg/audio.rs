//! Audio decoding through FFmpeg, resampled to a working format with swresample.

use std::path::Path;

use fosscut_time::{FLICKS_PER_SECOND, Flicks};
use rsmpeg::avcodec::AVCodecContext;
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swresample::SwrContext;

use super::{decode_failed, open_stream};
use crate::{AudioBuffer, MediaError, timebase};

/// Decodes the best audio stream of a file into interleaved `f32` at a fixed sample rate
/// and channel count, whatever the source format.
pub struct FfmpegAudioDecoder {
    path: String,
    input: AVFormatContextInput,
    decoder: AVCodecContext,
    index: usize,
    time_base: (i32, i32),
    start_ticks: i64,
    sample_rate: u32,
    channels: u16,
    codec: String,
}

/// Number of sample frames covering `time` at `sample_rate`, rounded to the nearest frame.
/// Exact for the common rates, which all divide a second of flicks.
pub fn samples_at(time: Flicks, sample_rate: u32) -> i64 {
    let numerator = i128::from(time.get()) * i128::from(sample_rate);
    let denominator = i128::from(FLICKS_PER_SECOND);
    let rounded = (2 * numerator + denominator).div_euclid(2 * denominator);
    i64::try_from(rounded).unwrap_or(if rounded < 0 { i64::MIN } else { i64::MAX })
}

impl FfmpegAudioDecoder {
    /// Opens the best audio stream of `path`, producing `sample_rate` Hz with `channels`
    /// channels (1 or 2).
    pub fn open(
        path: &Path,
        sample_rate: u32,
        channels: u16,
    ) -> Result<FfmpegAudioDecoder, MediaError> {
        if !(1..=2).contains(&channels) || sample_rate == 0 {
            return Err(MediaError::Unsupported {
                message: format!("{channels} channels at {sample_rate} Hz"),
            });
        }
        let (input, index, codec) = open_stream(path, ffi::AVMEDIA_TYPE_AUDIO)?;
        let stream = &input.streams()[index];
        let mut decoder = AVCodecContext::new(&codec);
        decoder
            .apply_codecpar(&stream.codecpar())
            .map_err(|e| decode_failed(&e))?;
        decoder.set_pkt_timebase(stream.time_base);
        decoder
            .open(None)
            .map_err(|e| MediaError::DecoderUnavailable {
                codec: format!("{} ({e})", codec.name().to_string_lossy()),
            })?;
        let time_base = (stream.time_base.num, stream.time_base.den);
        let start_ticks = if stream.start_time == ffi::AV_NOPTS_VALUE {
            0
        } else {
            stream.start_time
        };
        Ok(FfmpegAudioDecoder {
            path: path.display().to_string(),
            codec: codec.name().to_string_lossy().into_owned(),
            input,
            decoder,
            index,
            time_base,
            start_ticks,
            sample_rate,
            channels,
        })
    }

    /// The source codec name, for example `aac`.
    pub fn codec(&self) -> &str {
        &self.codec
    }

    /// The file this decoder reads.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The audio from `start` for `duration`, exactly `duration` long at the output rate.
    /// Time before the stream starts or after it ends is silence.
    pub fn read(&mut self, start: Flicks, duration: Flicks) -> Result<AudioBuffer, MediaError> {
        let rate = self.sample_rate;
        let channels = usize::from(self.channels);
        let wanted = samples_at(duration, rate).max(0) as usize;
        let mut out = AudioBuffer::silence(rate, self.channels, wanted);
        if wanted == 0 {
            return Ok(out);
        }
        let (num, den) = self.time_base;
        let target = self.start_ticks + timebase::from_flicks(start.max(Flicks::ZERO), num, den);
        self.input
            .seek(self.index as i32, target, ffi::AVSEEK_FLAG_BACKWARD as i32)
            .map_err(|e| MediaError::SeekFailed {
                message: e.to_string(),
            })?;
        self.decoder.flush_buffers();

        // Output sample index of `start`; decoded audio is placed relative to it.
        let start_sample = samples_at(start, rate);
        let mut resampler: Option<SwrContext> = None;
        let mut written_until: Option<i64> = None;
        let mut input_done = false;
        loop {
            let frame = match self.decoder.receive_frame() {
                Ok(frame) => Some(frame),
                Err(RsmpegError::DecoderDrainError) if !input_done => {
                    match self.input.read_packet().map_err(|e| decode_failed(&e))? {
                        Some(packet) if packet.stream_index as usize == self.index => {
                            match self.decoder.send_packet(Some(&packet)) {
                                Ok(()) | Err(RsmpegError::DecoderFullError) => {}
                                Err(e) => return Err(decode_failed(&e)),
                            }
                        }
                        Some(_) => {}
                        None => {
                            let _ = self.decoder.send_packet(None);
                            input_done = true;
                        }
                    }
                    continue;
                }
                Err(RsmpegError::DecoderDrainError) | Err(RsmpegError::DecoderFlushedError) => None,
                Err(e) => return Err(decode_failed(&e)),
            };
            let Some(mut frame) = frame else {
                // End of stream: flush what the resampler still holds.
                if let (Some(swr), Some(position)) = (&resampler, written_until) {
                    let converted = self.convert(swr, None)?;
                    place(&mut out, start_sample, position, &converted, channels);
                }
                break;
            };
            normalize_layout(&mut frame);
            if resampler.is_none() {
                let ticks = if frame.best_effort_timestamp != ffi::AV_NOPTS_VALUE {
                    frame.best_effort_timestamp
                } else {
                    frame.pts
                };
                let time = timebase::to_flicks(ticks - self.start_ticks, num, den);
                written_until = Some(samples_at(time, rate));
                resampler = Some(self.resampler_for(&frame)?);
            }
            let (Some(swr), Some(position)) = (&resampler, written_until) else {
                continue;
            };
            let converted = self.convert(swr, Some(&frame))?;
            let produced = converted.len() / channels;
            place(&mut out, start_sample, position, &converted, channels);
            written_until = Some(position + produced as i64);
            if position + produced as i64 >= start_sample + wanted as i64 {
                break;
            }
        }
        Ok(out)
    }

    fn resampler_for(&self, frame: &AVFrame) -> Result<SwrContext, MediaError> {
        let unsupported = |message: String| MediaError::Unsupported { message };
        let output_layout = super::channel_layout(self.channels).map_err(unsupported)?;
        let input_layout = frame.ch_layout().clone();
        let mut swr = SwrContext::new(
            &output_layout,
            ffi::AV_SAMPLE_FMT_FLT,
            self.sample_rate as i32,
            &input_layout,
            frame.format,
            frame.sample_rate,
        )
        .map_err(|e| unsupported(format!("cannot resample this audio: {e}")))?;
        swr.init()
            .map_err(|e| unsupported(format!("cannot resample this audio: {e}")))?;
        Ok(swr)
    }

    /// Resamples one frame (or flushes with `None`) to interleaved `f32`.
    fn convert(&self, swr: &SwrContext, frame: Option<&AVFrame>) -> Result<Vec<f32>, MediaError> {
        let mut output = AVFrame::new();
        let layout = super::channel_layout(self.channels)
            .map_err(|message| MediaError::Unsupported { message })?;
        output.set_ch_layout(layout.into_inner());
        output.set_sample_rate(self.sample_rate as i32);
        output.set_format(ffi::AV_SAMPLE_FMT_FLT);
        let pending = frame.map_or(0, |f| f.nb_samples);
        output.set_nb_samples(swr.get_out_samples(pending).max(1));
        output.alloc_buffer().map_err(|e| decode_failed(&e))?;
        swr.convert_frame(frame, &mut output)
            .map_err(|e| decode_failed(&e))?;
        let count = output.nb_samples.max(0) as usize * usize::from(self.channels);
        if count == 0 || output.data[0].is_null() {
            return Ok(Vec::new());
        }
        // SAFETY: interleaved `f32` output has one plane holding `nb_samples * channels`
        // samples, allocated above and filled by swr_convert_frame; we copy before `output`
        // is dropped.
        let samples = unsafe { std::slice::from_raw_parts(output.data[0].cast::<f32>(), count) };
        Ok(samples.to_vec())
    }
}

/// Gives frames with an unspecified channel order (common for PCM in WAV) the default
/// layout for their channel count, so the resampler sees the same layout it was built for.
fn normalize_layout(frame: &mut AVFrame) {
    if frame.ch_layout.order != ffi::AV_CHANNEL_ORDER_UNSPEC {
        return;
    }
    // SAFETY: the frame is owned and not shared with the decoder anymore;
    // `av_channel_layout_default` overwrites its layout with a native layout of the same
    // channel count, which needs no allocation.
    unsafe {
        use rsmpeg::UnsafeDerefMut;
        let layout = &mut frame.deref_mut().ch_layout;
        let count = layout.nb_channels;
        ffi::av_channel_layout_default(layout, count);
    }
}

/// Copies `samples` that start at output sample `position` into `out`, which starts at
/// `start_sample`, dropping what falls outside.
fn place(
    out: &mut AudioBuffer,
    start_sample: i64,
    position: i64,
    samples: &[f32],
    channels: usize,
) {
    let frames = samples.len() / channels;
    let wanted = out.frames() as i64;
    for i in 0..frames {
        let target = position + i as i64 - start_sample;
        if (0..wanted).contains(&target) {
            let dst = target as usize * channels;
            let src = i * channels;
            out.samples[dst..dst + channels].copy_from_slice(&samples[src..src + channels]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_positions_are_exact_at_common_rates() {
        let second = Flicks::from_seconds(1).unwrap();
        assert_eq!(samples_at(second, 48_000), 48_000);
        assert_eq!(samples_at(second, 44_100), 44_100);
        assert_eq!(
            samples_at(Flicks::new(14_700), 48_000),
            1,
            "one 48 kHz sample"
        );
        assert_eq!(samples_at(Flicks::new(-14_700), 48_000), -1);
    }

    #[test]
    fn placing_drops_samples_outside_the_window() {
        let mut out = AudioBuffer::silence(48_000, 2, 4);
        place(
            &mut out,
            10,
            8,
            &[1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0],
            2,
        );
        assert_eq!(out.samples, vec![3.0, 3.0, 4.0, 4.0, 0.0, 0.0, 0.0, 0.0]);
        place(&mut out, 10, 13, &[5.0, 5.0, 6.0, 6.0], 2);
        assert_eq!(&out.samples[6..], &[5.0, 5.0]);
    }
}
