//! Runs analyzers over a media file: audio decoded at 48 kHz in one-second chunks for
//! loudness and silences, at 16 kHz mono for speech, and video frame by frame, each fed to
//! the analyzers that need it (scene cuts, pose and mattes).

use std::path::Path;

use fosscut_media::ffmpeg::{FfmpegAudioDecoder, FfmpegVideoDecoder};
use fosscut_media::{DecoderOptions, MediaError, VideoDecoder};
use fosscut_time::{Flicks, Rate};

use crate::loudness::LoudnessMeter;
use crate::matte::MatteAnalyzer;
use crate::models::{self, Manifest, ModelError};
use crate::pose::PoseAnalyzer;
use crate::scenes::{Luma, SceneDetector};
use crate::silence::SilenceFinder;
use crate::vision::DataFile;
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

/// Checks that the vision models `analyzers` run with `options` are installed in `models`,
/// with ONNX Runtime: [`ModelError::NotInstalled`] names every one that is missing.
pub fn vision_models(
    analyzers: &[Analyzer],
    options: &Options,
    models: &Path,
) -> Result<(), ModelError> {
    let mut missing: Vec<&str> = Vec::new();
    for analyzer in analyzers {
        let Some(name) = analyzer.vision_model(options) else {
            continue;
        };
        let installed = models::manifest(name).is_ok_and(|m| models::is_installed(models, m));
        if !installed && !missing.contains(&name) {
            missing.push(name);
        }
    }
    let wants_runtime = analyzers.iter().any(|a| a.vision_model(options).is_some());
    if wants_runtime && crate::onnx::runtime_path(models).is_none() {
        missing.push(crate::onnx::RUNTIME_MODEL);
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ModelError::NotInstalled {
            what: missing.join(" and "),
        })
    }
}

/// Runs `analyzers` over `media`, with speech and vision models from `models`, calling
/// `progress` as it goes; returning `false` from it cancels. Pose and matte results come
/// with their data files in [`Results::data`].
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
        vision_models(analyzers, options, models)?;
        let mut decoder = FfmpegVideoDecoder::open(media.path, DecoderOptions::default())
            .map_err(|e| no_stream(e, AnalysisError::NoVideo))?;
        let mut detector = analyzers
            .contains(&Analyzer::Scenes)
            .then(|| SceneDetector::new(options.scenes.clone()));
        let mut pose = match analyzers.contains(&Analyzer::Pose) {
            true => Some(PoseAnalyzer::new(&options.pose, models)?),
            false => None,
        };
        let mut matte = match analyzers.contains(&Analyzer::Matte) {
            true => Some(MatteAnalyzer::new(&options.matte, models)?),
            false => None,
        };
        // Models take long enough per frame that progress is worth reporting every frame.
        let every = if pose.is_some() || matte.is_some() {
            1
        } else {
            10
        };
        let mut done = 0;
        while let Some(frame) = decoder.next_frame()? {
            if let (Some(detector), Some(plane)) = (detector.as_mut(), frame.planes.first()) {
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
            if let Some(pose) = pose.as_mut() {
                pose.push(&frame)?;
            }
            if let Some(matte) = matte.as_mut() {
                matte.push(&frame)?;
            }
            done += 1;
            if done % every == 0
                && !progress(Progress {
                    done: audio_units + done.min(video_units),
                    total,
                })
            {
                return Err(AnalysisError::Cancelled);
            }
        }
        results.scenes = detector.map(SceneDetector::finish);
        if let Some(pose) = pose {
            let track = pose.finish();
            let data: std::sync::Arc<[u8]> = track.encode().into();
            results.pose = Some(Box::new(track.summary(DataFile::of(&data))));
            results.data.insert(Analyzer::Pose, data);
        }
        if let Some(matte) = matte {
            let track = matte.finish();
            let data: std::sync::Arc<[u8]> = track.encode().into();
            results.matte = Some(Box::new(
                track.summary(options.matte.model, DataFile::of(&data)),
            ));
            results.data.insert(Analyzer::Matte, data);
        }
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
