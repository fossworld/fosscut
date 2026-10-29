//! Export jobs (ADR-0006 item 8, ADR-0009 items 5 and 6): render every frame, mix the
//! audio, encode, then verify the file against the request.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use fosscut_audio::{AudioError, AudioSource, mix, samples_in};
use fosscut_media::ffmpeg::{
    FfmpegAudioDecoder, FfmpegVideoDecoder, FfmpegWriter, audio_encoder, encoder_capabilities,
    probe_output, video_encoder,
};
use fosscut_media::{
    AudioBuffer, Container, DecoderOptions, OutputReport, OutputSettings, RgbaFrame,
};
use fosscut_model::{Id, Project};
use fosscut_render::color::TransferFunction;
use fosscut_render::plan::plan_frame;
use fosscut_render::{DecoderFrames, Renderer};
use fosscut_time::Flicks;
use serde::{Deserialize, Serialize};

use crate::ExportError;

/// What to export, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportRequest {
    /// The project bundle directory; relative media paths resolve against it.
    pub bundle: PathBuf,
    /// The timeline to export.
    pub timeline: Id,
    /// Container, codecs, size and rate.
    pub settings: OutputSettings,
    /// The file to write.
    pub output: PathBuf,
    /// Replace `output` if it exists (ADR-0006, item 6).
    pub allow_overwrite: bool,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision, so an
    /// agent exports exactly the edit it reviewed (ADR-0006, item 1).
    pub expected_revision: Option<u64>,
    /// Idempotency key: a repeated request returns the first result (ADR-0006, item 8).
    pub request_id: Option<String>,
    /// Where records of requests with a `request_id` are kept, normally [`cache_dir`].
    /// They hold absolute paths, so they never go in the bundle (ADR-0004, item 9).
    ///
    /// [`cache_dir`]: crate::cache_dir
    pub cache: PathBuf,
}

/// How far an export has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Frames (or one-second audio chunks for audio-only exports) done.
    pub done: u64,
    /// Total frames or chunks.
    pub total: u64,
}

/// The encoders an export used.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Encoders {
    /// Video encoder name, for example `h264_videotoolbox`.
    pub video: Option<String>,
    /// Audio encoder name.
    pub audio: Option<String>,
}

/// What an export would write, as checked by [`plan`] without writing anything.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportPlan {
    /// The file that would be written.
    pub output: String,
    /// The settings it would be written with.
    pub settings: OutputSettings,
    /// Video frames that would be written (0 for audio-only exports).
    pub frames: u64,
    /// The duration that would be exported.
    pub duration: Flicks,
    /// Encoders that would be used.
    pub encoders: Encoders,
}

/// A verified export: the request, what was written, and how long it took.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportReport {
    /// The written file.
    pub output: String,
    /// The settings it was written with.
    pub settings: OutputSettings,
    /// Video frames written (0 for audio-only exports).
    pub frames: u64,
    /// The exported duration.
    pub duration: Flicks,
    /// Encoders used.
    pub encoders: Encoders,
    /// What the file contains, as probed after writing; it matched the request.
    pub verified: OutputReport,
    /// Wall-clock time in milliseconds.
    pub elapsed_ms: u64,
    /// True when this result was returned for a repeated request id.
    #[serde(default)]
    pub replayed: bool,
}

/// Exports `request.timeline` of `project`, calling `progress` after every frame; return
/// `false` from it to cancel. The file is written next to the output under a temporary
/// name and moved into place only when complete, then probed and compared with the request.
pub fn export(
    project: &Project,
    request: &ExportRequest,
    renderer: &mut Renderer,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<ExportReport, ExportError> {
    let started = Instant::now();
    let record = request
        .request_id
        .as_deref()
        .map(|id| job_record_path(&request.cache, &project.manifest.id, id));
    if let (Some(path), Some(id)) = (&record, &request.request_id)
        && let Some(previous) = read_record(path)
    {
        if previous.output != request.output.display().to_string()
            || previous.settings != request.settings
        {
            return Err(ExportError::RequestReused {
                request_id: id.clone(),
            });
        }
        if request.output.exists() {
            return Ok(ExportReport {
                replayed: true,
                ..previous
            });
        }
    }

    let checked = check(project, request)?;
    let parent = output_dir(&request.output);
    let mut frames = DecoderFrames::new();
    let mut audio = DecoderAudio::default();
    for (asset, path) in &checked.media {
        let has_video = project
            .media
            .get(asset)
            .is_some_and(|a| a.info.video.is_some());
        if has_video && request.settings.video.is_some() {
            let decoder = FfmpegVideoDecoder::open(path, DecoderOptions::default())?;
            frames.insert(asset.clone(), Box::new(decoder));
        }
        audio.paths.insert(asset.clone(), path.clone());
    }

    let temporary = parent.join(format!(
        ".{}.partial",
        request
            .output
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "export".into())
    ));
    let result = write(
        project,
        request,
        &checked,
        &temporary,
        renderer,
        &mut frames,
        &mut audio,
        progress,
    );
    let frame_count = match result {
        Ok(count) => count,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if request.output.exists() {
        std::fs::remove_file(&request.output).map_err(|e| failed_io(&request.output, &e))?;
    }
    std::fs::rename(&temporary, &request.output).map_err(|e| failed_io(&request.output, &e))?;

    let verified = probe_output(&request.output)?;
    let expected_duration = match &request.settings.video {
        Some(video) => video.rate.frames_to_flicks(frame_count as i64),
        None => checked.timeline_duration,
    };
    let mismatches = compare(&request.settings, frame_count, expected_duration, &verified);
    if !mismatches.is_empty() {
        return Err(ExportError::VerificationFailed {
            mismatches,
            report: Box::new(verified),
        });
    }
    let report = ExportReport {
        output: request.output.display().to_string(),
        settings: request.settings.clone(),
        frames: frame_count,
        duration: expected_duration,
        encoders: checked.plan.encoders,
        verified,
        elapsed_ms: started.elapsed().as_millis() as u64,
        replayed: false,
    };
    if let Some(path) = record {
        write_record(&path, &report);
    }
    Ok(report)
}

/// Checks an export exactly as [`export`] would before rendering: the expected revision,
/// the timeline, the settings and encoders, the overwrite guard, the output folder and
/// every clip's media. Returns what it would write; nothing is rendered or written, so
/// this is the export's dry run (ADR-0006, item 2).
pub fn plan(project: &Project, request: &ExportRequest) -> Result<ExportPlan, ExportError> {
    check(project, request).map(|checked| checked.plan)
}

/// A checked export: its plan, the timeline's own duration and each asset's media file.
struct Checked {
    plan: ExportPlan,
    timeline_duration: Flicks,
    media: Vec<(Id, PathBuf)>,
}

fn check(project: &Project, request: &ExportRequest) -> Result<Checked, ExportError> {
    let current = project.manifest.revision;
    if let Some(expected) = request.expected_revision
        && expected != current
    {
        return Err(ExportError::RevisionConflict { expected, current });
    }
    let doc =
        project
            .timelines
            .get(&request.timeline)
            .ok_or_else(|| ExportError::TimelineNotFound {
                timeline: request.timeline.to_string(),
            })?;
    let duration = doc.duration();
    if !duration.is_positive() {
        return Err(ExportError::EmptyTimeline);
    }
    validate(&request.settings)?;
    if request.output.exists() && !request.allow_overwrite {
        return Err(ExportError::OutputExists {
            path: request.output.display().to_string(),
        });
    }
    let parent = output_dir(&request.output);
    if !parent.is_dir() {
        return Err(ExportError::InvalidSettings {
            message: format!("{} is not a directory", parent.display()),
        });
    }
    let media = assets_in(project, &request.timeline)
        .into_iter()
        .map(|asset| {
            let path = resolve(project, &request.bundle, &asset)?;
            Ok((asset, path))
        })
        .collect::<Result<Vec<_>, ExportError>>()?;
    let settings = &request.settings;
    let (frames, exported) = match &settings.video {
        // Every frame whose start is before the end of the timeline.
        Some(video) => {
            let total = video.rate.frame_at(duration - Flicks::new(1)) + 1;
            (total as u64, video.rate.frames_to_flicks(total))
        }
        None => (0, duration),
    };
    Ok(Checked {
        plan: ExportPlan {
            output: request.output.display().to_string(),
            settings: settings.clone(),
            frames,
            duration: exported,
            encoders: Encoders {
                video: settings
                    .video
                    .as_ref()
                    .and_then(|v| video_encoder(v.codec))
                    .map(|e| e.encoder.clone()),
                audio: settings
                    .audio
                    .as_ref()
                    .and_then(|a| audio_encoder(a.codec))
                    .map(|e| e.encoder.clone()),
            },
        },
        timeline_duration: duration,
        media,
    })
}

/// The folder an output is written in; a bare file name means the current folder.
fn output_dir(output: &Path) -> &Path {
    output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Renders, mixes and encodes into `path`; returns the number of video frames.
#[allow(clippy::too_many_arguments)]
fn write(
    project: &Project,
    request: &ExportRequest,
    checked: &Checked,
    path: &Path,
    renderer: &mut Renderer,
    frames: &mut DecoderFrames,
    audio: &mut DecoderAudio,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<u64, ExportError> {
    let settings = &request.settings;
    let duration = checked.timeline_duration;
    let mut writer = FfmpegWriter::create(path, settings)?;
    let audio_format = settings.audio.as_ref().map(|a| (a.sample_rate, a.channels));
    let mut push_audio =
        |writer: &mut FfmpegWriter, from: Flicks, to: Flicks| -> Result<(), ExportError> {
            if let Some((rate, channels)) = audio_format {
                let buffer = mix(
                    project,
                    &request.timeline,
                    from,
                    to - from,
                    rate,
                    channels,
                    audio,
                )?;
                writer.push_audio(&buffer)?;
            }
            Ok(())
        };

    let Some(video) = &settings.video else {
        // Audio only: one-second chunks.
        let second = Flicks::from_seconds(1).unwrap_or(duration);
        let total = (duration.get() + second.get() - 1) / second.get();
        for chunk in 0..total {
            let from = Flicks::new(chunk * second.get());
            let to = (from + second).min(duration);
            push_audio(&mut writer, from, to)?;
            if !progress(Progress {
                done: chunk as u64 + 1,
                total: total as u64,
            }) {
                return Err(ExportError::Cancelled);
            }
        }
        writer.finish()?;
        return Ok(0);
    };

    let total = checked.plan.frames as i64;
    for n in 0..total {
        let time = video.rate.frames_to_flicks(n);
        let plan = plan_frame(project, &request.timeline, time)?;
        let image = renderer.render_image(
            &plan,
            frames,
            (video.width, video.height),
            TransferFunction::Bt709,
        )?;
        writer.push_rgba(&RgbaFrame {
            width: image.width,
            height: image.height,
            rgba: image.rgba,
        })?;
        push_audio(&mut writer, time, video.rate.frames_to_flicks(n + 1))?;
        if !progress(Progress {
            done: n as u64 + 1,
            total: total as u64,
        }) {
            return Err(ExportError::Cancelled);
        }
    }
    writer.finish()?;
    Ok(total as u64)
}

fn validate(settings: &OutputSettings) -> Result<(), ExportError> {
    let video = settings.video.as_ref();
    if let Some(v) = video
        && (v.width == 0 || v.height == 0 || v.width % 2 != 0 || v.height % 2 != 0)
    {
        return Err(ExportError::InvalidSettings {
            message: format!("{}x{} is not an even, non-zero size", v.width, v.height),
        });
    }
    if !settings.container.supports(
        video.map(|v| v.codec),
        settings.audio.as_ref().map(|a| a.codec),
    ) {
        return Err(ExportError::InvalidSettings {
            message: format!(
                "{} cannot hold these codecs",
                settings.container.extension()
            ),
        });
    }
    if let Some(v) = video
        && video_encoder(v.codec).is_none()
    {
        return Err(ExportError::CodecUnavailable {
            codec: v.codec.probe_name().to_string(),
            available: encoder_capabilities()
                .video
                .iter()
                .map(|(c, _)| c.probe_name().to_string())
                .collect(),
        });
    }
    Ok(())
}

/// Every difference between the request and the written file.
pub fn compare(
    settings: &OutputSettings,
    frames: u64,
    duration: Flicks,
    report: &OutputReport,
) -> Vec<String> {
    let mut mismatches = Vec::new();
    let mut expect = |ok: bool, what: String| {
        if !ok {
            mismatches.push(what);
        }
    };
    let family_ok = match settings.container {
        Container::Mp4 => {
            report.container.contains("mp4") && report.brand.as_deref() != Some("qt  ")
        }
        Container::Mov => {
            report.container.contains("mov") && report.brand.as_deref() == Some("qt  ")
        }
        Container::Webm => report.container.contains("webm"),
    };
    expect(
        family_ok,
        format!(
            "container: requested {}, file is {} (brand {:?})",
            settings.container.extension(),
            report.container,
            report.brand
        ),
    );
    match (&settings.video, &report.video) {
        (Some(want), Some(got)) => {
            expect(
                got.codec == want.codec.probe_name(),
                format!(
                    "video codec: requested {}, file has {}",
                    want.codec.probe_name(),
                    got.codec
                ),
            );
            expect(
                (got.width, got.height) == (want.width, want.height),
                format!(
                    "size: requested {}x{}, file is {}x{}",
                    want.width, want.height, got.width, got.height
                ),
            );
            expect(
                got.rate == Some(want.rate),
                format!(
                    "frame rate: requested {}, file has {:?}",
                    want.rate,
                    got.rate.map(|r| r.to_string())
                ),
            );
            expect(
                got.frame_count == frames,
                format!("frames: rendered {frames}, file has {}", got.frame_count),
            );
            if let Some(file_duration) = got.duration {
                expect(
                    file_duration == duration,
                    format!(
                        "video duration: expected {} flicks, file has {}",
                        duration.get(),
                        file_duration.get()
                    ),
                );
            }
        }
        (Some(_), None) => expect(false, "video: requested, file has none".into()),
        (None, Some(_)) => expect(
            false,
            "video: not requested, file has a video stream".into(),
        ),
        (None, None) => {}
    }
    match (&settings.audio, &report.audio) {
        (Some(want), Some(got)) => {
            expect(
                got.codec == want.codec.probe_name(),
                format!(
                    "audio codec: requested {}, file has {}",
                    want.codec.probe_name(),
                    got.codec
                ),
            );
            expect(
                got.sample_rate == want.sample_rate && got.channels == want.channels,
                format!(
                    "audio format: requested {} Hz x {}, file has {} Hz x {}",
                    want.sample_rate, want.channels, got.sample_rate, got.channels
                ),
            );
        }
        (Some(_), None) => expect(false, "audio: requested, file has none".into()),
        _ => {}
    }
    mismatches
}

/// Assets that clips on `timeline` read frames or audio from.
fn assets_in(project: &Project, timeline: &Id) -> Vec<Id> {
    let mut assets: Vec<Id> = project
        .timelines
        .get(timeline)
        .into_iter()
        .flat_map(|doc| doc.tracks.iter())
        .flat_map(|track| track.clips.iter())
        .filter_map(|clip| clip.content.asset().cloned())
        .collect();
    assets.sort();
    assets.dedup();
    assets
}

/// The media file of `asset`: the path relative to the bundle first, then the absolute path.
fn resolve(project: &Project, bundle: &Path, asset: &Id) -> Result<PathBuf, ExportError> {
    let hints = project.media.get(asset).map(|a| &a.path);
    let mut tried = Vec::new();
    if let Some(hints) = hints {
        if let Some(relative) = &hints.relative {
            let candidate = relative
                .split('/')
                .fold(bundle.to_path_buf(), |path, part| path.join(part));
            if candidate.is_file() {
                return Ok(candidate);
            }
            tried.push(candidate.display().to_string());
        }
        if let Some(absolute) = &hints.absolute {
            let candidate = PathBuf::from(absolute);
            if candidate.is_file() {
                return Ok(candidate);
            }
            tried.push(absolute.clone());
        }
    }
    Err(ExportError::MissingMedia {
        asset: asset.to_string(),
        tried,
    })
}

/// Audio decoders per asset, opened on first use.
#[derive(Default)]
struct DecoderAudio {
    paths: HashMap<Id, PathBuf>,
    decoders: HashMap<(Id, u32, u16), FfmpegAudioDecoder>,
}

impl AudioSource for DecoderAudio {
    fn read(
        &mut self,
        asset: &Id,
        start: Flicks,
        duration: Flicks,
        sample_rate: u32,
        channels: u16,
    ) -> Result<AudioBuffer, AudioError> {
        let failed = |message: String| AudioError::SourceFailed {
            asset: asset.to_string(),
            message,
        };
        let key = (asset.clone(), sample_rate, channels);
        if !self.decoders.contains_key(&key) {
            let path = self
                .paths
                .get(asset)
                .ok_or_else(|| failed("no file for this asset".into()))?;
            match FfmpegAudioDecoder::open(path, sample_rate, channels) {
                Ok(decoder) => {
                    self.decoders.insert(key.clone(), decoder);
                }
                // A video without an audio stream plays as silence.
                Err(fosscut_media::MediaError::NoAudioStream { .. }) => {
                    let frames = samples_in(duration, sample_rate).max(0) as usize;
                    return Ok(AudioBuffer::silence(sample_rate, channels, frames));
                }
                Err(e) => return Err(failed(e.to_string())),
            }
        }
        let decoder = self
            .decoders
            .get_mut(&key)
            .ok_or_else(|| failed("decoder missing".into()))?;
        decoder
            .read(start, duration)
            .map_err(|e| failed(e.to_string()))
    }
}

/// One record per project and request id. The id is hashed, so any string makes a valid
/// file name and different ids never share a record.
fn job_record_path(cache: &Path, project: &Id, request_id: &str) -> PathBuf {
    let key = blake3::hash(request_id.as_bytes()).to_hex();
    cache
        .join("jobs")
        .join(project.to_string())
        .join(format!("export-{}.json", &key.as_str()[..32]))
}

fn read_record(path: &Path) -> Option<ExportReport> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_record(path: &Path, report: &ExportReport) {
    // The record only enables replay; failing to write it does not fail the export.
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(report) {
        let _ = std::fs::write(path, bytes);
    }
}

fn failed_io(path: &Path, error: &std::io::Error) -> ExportError {
    ExportError::Failed {
        code: "fosscut.media.write_failed".into(),
        message: format!("cannot write {}: {error}", path.display()),
    }
}
