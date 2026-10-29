//! Runs analyzers over a media file: audio decoded at 48 kHz in one-second chunks, video
//! frame by frame, each fed to the analyzers that need it.

use std::path::Path;

use fosscut_media::ffmpeg::{FfmpegAudioDecoder, FfmpegVideoDecoder};
use fosscut_media::{DecoderOptions, MediaError, VideoDecoder};
use fosscut_time::{Flicks, Rate};

use crate::loudness::LoudnessMeter;
use crate::scenes::{Luma, SceneDetector};
use crate::silence::SilenceFinder;
use crate::{AnalysisError, Analyzer, Needs, Options, Progress, Results};

/// What the caller knows about the file, from the project's media table.
#[derive(Clone, Copy, Debug)]
pub struct Media<'a> {
    /// The file.
    pub path: &'a Path,
    /// Its duration.
    pub duration: Flicks,
    /// Audio channels, if it has audio.
    pub audio_channels: Option<u16>,
    /// Frame rate, if it has video (for progress estimates).
    pub video_rate: Option<Rate>,
}

/// Runs `analyzers` over `media`, calling `progress` as it goes; returning `false` from it
/// cancels.
pub fn analyze(
    media: Media<'_>,
    analyzers: &[Analyzer],
    options: &Options,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<Results, AnalysisError> {
    let wants = |needs: Needs| analyzers.iter().any(|a| a.needs() == needs);
    let second = Flicks::from_seconds(1).unwrap_or(Flicks::new(705_600_000));
    let seconds = ((media.duration.get() + second.get() - 1) / second.get()).max(0) as u64;
    let audio_units = if wants(Needs::Audio) { seconds } else { 0 };
    let video_units = if wants(Needs::Video) {
        media.video_rate.map_or(seconds * 30, |rate| {
            rate.frame_at(media.duration).max(0) as u64
        })
    } else {
        0
    };
    let total = audio_units + video_units;
    let mut results = Results::default();

    if wants(Needs::Audio) {
        let channels = media
            .audio_channels
            .ok_or(AnalysisError::NoAudio)?
            .clamp(1, 2);
        let mut decoder = FfmpegAudioDecoder::open(media.path, loudness_rate(), channels)
            .map_err(|e| no_stream(e, AnalysisError::NoAudio))?;
        let mut meter = analyzers
            .contains(&Analyzer::Loudness)
            .then(|| LoudnessMeter::new(channels));
        let mut finder = analyzers
            .contains(&Analyzer::Silences)
            .then(|| SilenceFinder::new(channels, options.silences.clone()));
        for chunk in 0..seconds {
            let start = Flicks::new(chunk as i64 * second.get());
            let length = (media.duration - start).min(second);
            let buffer = decoder.read(start, length)?;
            if let Some(meter) = meter.as_mut() {
                meter.push(&buffer.samples);
            }
            if let Some(finder) = finder.as_mut() {
                finder.push(&buffer.samples);
            }
            if !progress(Progress {
                done: chunk + 1,
                total,
            }) {
                return Err(AnalysisError::Cancelled);
            }
        }
        results.loudness = meter.map(LoudnessMeter::finish);
        results.silences = finder.map(SilenceFinder::finish);
    }

    if wants(Needs::Video) {
        if media.video_rate.is_none() {
            return Err(AnalysisError::NoVideo);
        }
        let mut decoder = FfmpegVideoDecoder::open(media.path, DecoderOptions::default())
            .map_err(|e| no_stream(e, AnalysisError::NoVideo))?;
        let mut detector = SceneDetector::new(options.scenes.clone());
        let mut done = 0;
        while let Some(frame) = decoder.next_frame()? {
            if let Some(plane) = frame.planes.first() {
                detector.push(
                    frame.pts,
                    Luma {
                        width: frame.width,
                        height: frame.height,
                        stride: plane.stride,
                        data: &plane.data,
                    },
                );
            }
            done += 1;
            if done % 10 == 0
                && !progress(Progress {
                    done: audio_units + done.min(video_units),
                    total,
                })
            {
                return Err(AnalysisError::Cancelled);
            }
        }
        results.scenes = Some(detector.finish());
    }
    progress(Progress { done: total, total });
    Ok(results)
}

/// The rate every audio analyzer reads.
fn loudness_rate() -> u32 {
    crate::loudness::RATE
}

/// A missing stream becomes `missing`; other decoding errors pass through.
fn no_stream(error: MediaError, missing: AnalysisError) -> AnalysisError {
    match error {
        MediaError::NoAudioStream { .. } | MediaError::NoVideoStream { .. } => missing,
        other => AnalysisError::Media(other),
    }
}
