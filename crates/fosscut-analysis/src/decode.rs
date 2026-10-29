//! Runs analyzers over a media file: audio decoded at 48 kHz in one-second chunks for
//! loudness and silences, at 16 kHz mono for speech, and video frame by frame, each fed to
//! the analyzers that need it.

use std::path::Path;

use fosscut_media::ffmpeg::{FfmpegAudioDecoder, FfmpegVideoDecoder};
use fosscut_media::{DecoderOptions, MediaError, VideoDecoder};
use fosscut_time::{Flicks, Rate};

use crate::loudness::LoudnessMeter;
use crate::models::{self, Manifest, ModelError};
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

/// The speech model `options` ask for, or the most capable installed one for the language.
pub fn speech_model(options: &Options, models: &Path) -> Result<&'static Manifest, ModelError> {
    match &options.transcript.model {
        Some(name) => {
            let model = models::manifest(name)?;
            if models::is_installed(models, model) {
                Ok(model)
            } else {
                Err(ModelError::NotInstalled { what: name.clone() })
            }
        }
        None => {
            let language = options.transcript.language.as_deref();
            models::best_speech_model(models, language).ok_or_else(|| ModelError::NotInstalled {
                what: match language {
                    Some(language) if language != "en" => {
                        format!("a multilingual speech model (for {language})")
                    }
                    _ => "a speech model".into(),
                },
            })
        }
    }
}

/// Runs `analyzers` over `media`, with speech models from `models`, calling `progress` as it
/// goes; returning `false` from it cancels.
pub fn analyze(
    media: Media<'_>,
    analyzers: &[Analyzer],
    options: &Options,
    models: &Path,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<Results, AnalysisError> {
    let wants = |needs: Needs| analyzers.iter().any(|a| a.needs() == needs);
    let levels = analyzers.contains(&Analyzer::Loudness) || analyzers.contains(&Analyzer::Silences);
    let speech = analyzers.contains(&Analyzer::Transcript);
    let second = Flicks::from_seconds(1).unwrap_or(Flicks::new(705_600_000));
    let seconds = ((media.duration.get() + second.get() - 1) / second.get()).max(0) as u64;
    let audio_units = if levels { seconds } else { 0 };
    let speech_units = if speech { seconds } else { 0 };
    let video_units = if wants(Needs::Video) {
        media.video_rate.map_or(seconds * 30, |rate| {
            rate.frame_at(media.duration).max(0) as u64
        })
    } else {
        0
    };
    let total = audio_units + speech_units + video_units;
    let mut results = Results::default();
    if wants(Needs::Audio) && media.audio_channels.is_none() {
        return Err(AnalysisError::NoAudio);
    }

    if levels {
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

    if speech {
        let model = speech_model(options, models)?;
        let samples = read_all(media, crate::speech::RATE)?;
        results.transcript = Some(recognise(
            model,
            models,
            &samples,
            options,
            &mut |percent| {
                progress(Progress {
                    done: audio_units + percent * speech_units / 100,
                    total,
                })
            },
        )?);
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

/// The whole audio stream as mono samples at `rate`.
fn read_all(media: Media<'_>, rate: u32) -> Result<Vec<f32>, AnalysisError> {
    let mut decoder = FfmpegAudioDecoder::open(media.path, rate, 1)
        .map_err(|e| no_stream(e, AnalysisError::NoAudio))?;
    let chunk = Flicks::from_seconds(30).unwrap_or(Flicks::new(30 * 705_600_000));
    let mut samples = Vec::new();
    let mut start = Flicks::ZERO;
    while start < media.duration {
        let length = (media.duration - start).min(chunk);
        samples.extend(decoder.read(start, length)?.samples);
        start = start + length;
    }
    Ok(samples)
}

#[cfg(feature = "speech")]
fn recognise(
    model: &Manifest,
    models: &Path,
    samples: &[f32],
    options: &Options,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<crate::Transcript, AnalysisError> {
    crate::speech::transcribe(
        &models::path(models, model),
        model,
        samples,
        &options.transcript,
        progress,
    )
    .map_err(|e| match e {
        crate::speech::SpeechError::Cancelled => AnalysisError::Cancelled,
        crate::speech::SpeechError::Whisper(message) => AnalysisError::Speech(message),
    })
}

#[cfg(not(feature = "speech"))]
fn recognise(
    _: &Manifest,
    _: &Path,
    _: &[f32],
    _: &Options,
    _: &mut dyn FnMut(u64) -> bool,
) -> Result<crate::Transcript, AnalysisError> {
    Err(AnalysisError::SpeechUnavailable)
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
