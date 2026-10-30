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

use crate::EngineError;

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
    /// A part of the timeline, a cover image and subtitles written beside the file.
    pub options: ExportOptions,
}

/// What an export covers and writes besides the media file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportOptions {
    /// Only this part of the timeline, start to end; the whole timeline when `None`.
    pub range: Option<(Flicks, Flicks)>,
    /// Write a PNG cover image of the frame at this time of the export (from its start)
    /// beside the file, as `<name>.cover.png`.
    pub cover: Option<Flicks>,
    /// Write the first caption track as subtitles beside the file.
    pub subtitles: Option<SubtitleFormat>,
}

/// A subtitle file format.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleFormat {
    /// SubRip (`.srt`).
    Srt,
    /// WebVTT (`.vtt`), for the web.
    Vtt,
}

impl SubtitleFormat {
    /// The format named `srt` or `vtt`.
    pub fn named(name: &str) -> Result<SubtitleFormat, EngineError> {
        match name {
            "srt" => Ok(SubtitleFormat::Srt),
            "vtt" => Ok(SubtitleFormat::Vtt),
            other => Err(EngineError::InvalidSettings {
                message: format!("subtitles are srt or vtt, not {other:?}"),
            }),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            SubtitleFormat::Srt => "srt",
            SubtitleFormat::Vtt => "vtt",
        }
    }
}

/// Changes to a preset's settings: size, frame rate and bit rates (CC-163 to CC-166).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsChoice {
    /// The shorter side in pixels (720 for 720p, 2160 for 4K); the canvas's when `None`.
    /// The other side follows the canvas's shape, rounded to an even number.
    pub resolution: Option<u32>,
    /// The frame rate; the timeline's when `None`.
    pub rate: Option<fosscut_time::Rate>,
    /// Video bit rate in bits per second; the encoder's choice for the size when `None`.
    pub video_bit_rate: Option<u64>,
    /// Audio bit rate in bits per second; the preset's when `None`.
    pub audio_bit_rate: Option<u64>,
}

/// The resolutions export offers by name, as shorter sides in pixels.
pub const RESOLUTIONS: [(&str, u32); 6] = [
    ("480p", 480),
    ("720p", 720),
    ("1080p", 1080),
    ("2k", 1440),
    ("4k", 2160),
    ("8k", 4320),
];

/// A resolution by name (`720p`, `4k`) or as a number of pixels for the shorter side.
pub fn resolution(name: &str) -> Result<u32, EngineError> {
    let lower = name.trim().to_lowercase();
    if let Some((_, pixels)) = RESOLUTIONS.iter().find(|(n, _)| *n == lower) {
        return Ok(*pixels);
    }
    lower
        .trim_end_matches('p')
        .parse::<u32>()
        .ok()
        .filter(|p| (16..=8640).contains(p))
        .ok_or_else(|| EngineError::InvalidSettings {
            message: format!(
                "{name} is not a resolution: use 480p, 720p, 1080p, 2k, 4k, 8k or a number of pixels"
            ),
        })
}

/// A frame rate by its usual name: whole numbers, or 23.976, 29.97 and 59.94 for the NTSC
/// rates, or an exact fraction such as 30000/1001.
pub fn frame_rate(name: &str) -> Result<fosscut_time::Rate, EngineError> {
    let exact = match name.trim() {
        "23.976" | "23.98" => "24000/1001",
        "29.97" => "30000/1001",
        "59.94" => "60000/1001",
        other => other,
    };
    exact
        .parse::<fosscut_time::Rate>()
        .map_err(|e| EngineError::InvalidSettings {
            message: format!("{name} is not a frame rate: {e}"),
        })
}

/// `preset`'s settings for a timeline of `format`, changed by `choice`. GIFs default to a
/// 480-pixel shorter side at 15 frames per second, which keeps them small.
pub fn settings_for(
    preset: crate::Preset,
    format: &fosscut_model::TimelineFormat,
    choice: &SettingsChoice,
) -> OutputSettings {
    let mut settings = preset.settings(format);
    let gif = settings.container.extension() == "gif";
    if let Some(video) = &mut settings.video {
        // A GIF never grows past the canvas; a chosen resolution may scale up.
        let current = video.width.min(video.height);
        let shorter = choice.resolution.or(gif.then_some(480.min(current)));
        if let Some(target) = shorter {
            let (w, h) = (
                f64::from(format.width.max(1)),
                f64::from(format.height.max(1)),
            );
            let factor = f64::from(target) / w.min(h);
            let even = |v: f64| (((v * factor) / 2.0).round() as u32 * 2).max(2);
            video.width = even(w);
            video.height = even(h);
        }
        let gif_rate = fosscut_time::Rate::new(15, 1).unwrap_or(video.rate);
        if let Some(rate) = choice.rate.or(gif.then_some(gif_rate)) {
            video.rate = rate;
        }
        if choice.video_bit_rate.is_some() {
            video.bit_rate = choice.video_bit_rate;
        }
    }
    if let (Some(audio), Some(bits)) = (&mut settings.audio, choice.audio_bit_rate) {
        audio.bit_rate = bits;
    }
    settings
}

/// A bit rate such as `8M`, `192k` or `2500000`, in bits per second.
pub fn bit_rate(text: &str) -> Result<u64, EngineError> {
    let lower = text.trim().to_lowercase();
    let (number, factor) = match lower.chars().last() {
        Some('k') => (&lower[..lower.len() - 1], 1_000.0),
        Some('m') => (&lower[..lower.len() - 1], 1_000_000.0),
        _ => (lower.as_str(), 1.0),
    };
    number
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| (v * factor).round() as u64)
        .filter(|&b| (8_000..=2_000_000_000).contains(&b))
        .ok_or_else(|| EngineError::InvalidSettings {
            message: format!("{text} is not a bit rate: use for example 8M or 192k"),
        })
}

/// Where an asset an export uses comes from, and what that means for publishing it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RightsNote {
    /// The asset.
    pub asset: String,
    /// Its name.
    pub name: String,
    /// `library` (made by FossCut, free to use) or `yours` (brought into the project).
    pub origin: String,
    /// What to check before publishing.
    pub note: String,
}

/// Rights notes for the media `assets` of `project` (CC-172 is a check against a rights
/// database; FossCut has none and sends nothing anywhere, so it says what it knows).
fn rights(project: &Project, assets: &[Id]) -> Vec<RightsNote> {
    let mut notes: Vec<RightsNote> = Vec::new();
    for asset in assets.iter().filter_map(|id| project.media.get(id)) {
        let library = asset
            .path
            .relative
            .as_deref()
            .is_some_and(|p| p.starts_with(crate::library::FOLDER));
        notes.push(RightsNote {
            asset: asset.id.to_string(),
            name: asset.name.clone(),
            origin: if library { "library" } else { "yours" }.to_string(),
            note: if library {
                "Made by FossCut: free to use, also commercially.".to_string()
            } else {
                "Brought in by you: make sure you may publish it (music and footage you did not make often need a licence).".to_string()
            },
        });
    }
    for asset in project
        .media
        .assets
        .iter()
        .filter(|a| a.kind == fosscut_model::AssetKind::Font)
    {
        notes.push(RightsNote {
            asset: asset.id.to_string(),
            name: asset.name.clone(),
            origin: "yours".to_string(),
            note: "A font you imported: check that its licence allows using it in videos."
                .to_string(),
        });
    }
    notes
}

/// How far an export has come.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Where the media the export uses comes from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rights: Vec<RightsNote>,
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
    /// The cover image written, if one was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// The subtitle file written, if one was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitles: Option<String>,
    /// Where the media the export uses comes from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rights: Vec<RightsNote>,
}

/// Exports `request.timeline` of `project`, calling `progress` after every frame; return
/// `false` from it to cancel. The file is written next to the output under a temporary
/// name and moved into place only when complete, then probed and compared with the request.
pub fn export(
    project: &Project,
    request: &ExportRequest,
    renderer: &mut Renderer,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<ExportReport, EngineError> {
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
            return Err(EngineError::RequestReused {
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
    if request.settings.video.is_some() {
        for asset in crate::frame::timeline_luts(project, &request.timeline) {
            frames.insert_lut(
                asset.clone(),
                crate::frame::load_lut(project, &request.bundle, &asset)?,
            );
        }
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
    // `rename` replaces an existing file in one step on every platform. It fails when the
    // output is held open elsewhere (Windows locks open files) or became a folder; then the
    // finished temporary file is removed too.
    if let Err(error) = std::fs::rename(&temporary, &request.output) {
        let _ = std::fs::remove_file(&temporary);
        return Err(failed_io(&request.output, &error));
    }

    let verified = probe_output(&request.output)?;
    let expected_duration = match &request.settings.video {
        Some(video) => video.rate.frames_to_flicks(frame_count as i64),
        None => checked.timeline_duration,
    };
    let mismatches = compare(&request.settings, frame_count, expected_duration, &verified);
    if !mismatches.is_empty() {
        return Err(EngineError::VerificationFailed {
            mismatches,
            report: Box::new(verified),
        });
    }
    let cover = match request.options.cover {
        Some(at) => Some(write_cover(project, request, checked.start + at, renderer)?),
        None => None,
    };
    let subtitles = match request.options.subtitles {
        Some(format) => write_subtitles(project, request, &checked, format)?,
        None => None,
    };
    let report = ExportReport {
        output: request.output.display().to_string(),
        settings: request.settings.clone(),
        frames: frame_count,
        duration: expected_duration,
        encoders: checked.plan.encoders.clone(),
        verified,
        elapsed_ms: started.elapsed().as_millis() as u64,
        replayed: false,
        cover,
        subtitles,
        rights: checked.plan.rights.clone(),
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
pub fn plan(project: &Project, request: &ExportRequest) -> Result<ExportPlan, EngineError> {
    check(project, request).map(|checked| checked.plan)
}

/// A checked export: its plan, the part of the timeline it covers and each asset's media
/// file.
struct Checked {
    plan: ExportPlan,
    /// Where the export starts on the timeline.
    start: Flicks,
    /// How much of the timeline it covers.
    timeline_duration: Flicks,
    media: Vec<(Id, PathBuf)>,
}

/// A file beside `output`: its name with `suffix` instead of its extension.
fn beside(output: &Path, suffix: &str) -> PathBuf {
    let stem = output
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".into());
    output_dir(output).join(format!("{stem}.{suffix}"))
}

/// Renders the frame at `time` as the cover image beside the output.
fn write_cover(
    project: &Project,
    request: &ExportRequest,
    time: Flicks,
    renderer: &mut Renderer,
) -> Result<String, EngineError> {
    let width = request.settings.video.as_ref().map(|v| v.width);
    let still = crate::frame::frame(
        project,
        &request.bundle,
        &request.timeline,
        time,
        width,
        renderer,
    )?;
    let path = beside(&request.output, "cover.png");
    let bytes = crate::frame::png(&still.image)?;
    std::fs::write(&path, bytes).map_err(|e| failed_io(&path, &e))?;
    Ok(path.display().to_string())
}

/// Writes the first caption track of the exported part as subtitles beside the output, or
/// nothing when the timeline has no captions.
fn write_subtitles(
    project: &Project,
    request: &ExportRequest,
    checked: &Checked,
    format: SubtitleFormat,
) -> Result<Option<String>, EngineError> {
    let range = (checked.start, checked.start + checked.timeline_duration);
    let text = match crate::captions::subtitles(project, Some(&request.timeline), range, format) {
        Ok(text) => text,
        Err(EngineError::NoCaptions { .. }) => return Ok(None),
        Err(other) => return Err(other),
    };
    let path = beside(&request.output, format.extension());
    std::fs::write(&path, text).map_err(|e| failed_io(&path, &e))?;
    Ok(Some(path.display().to_string()))
}

fn check(project: &Project, request: &ExportRequest) -> Result<Checked, EngineError> {
    let current = project.manifest.revision;
    if let Some(expected) = request.expected_revision
        && expected != current
    {
        return Err(EngineError::RevisionConflict { expected, current });
    }
    let doc =
        project
            .timelines
            .get(&request.timeline)
            .ok_or_else(|| EngineError::TimelineNotFound {
                timeline: request.timeline.to_string(),
            })?;
    let whole = doc.duration();
    if !whole.is_positive() {
        return Err(EngineError::EmptyTimeline);
    }
    let (start, duration) = match request.options.range {
        Some((from, to)) => {
            if from.is_negative() || to <= from || from >= whole {
                return Err(EngineError::InvalidSettings {
                    message: "the range must start inside the timeline and end after it starts"
                        .to_string(),
                });
            }
            (from, to.min(whole) - from)
        }
        None => (Flicks::ZERO, whole),
    };
    if let Some(cover) = request.options.cover
        && (cover.is_negative() || cover >= duration)
    {
        return Err(EngineError::InvalidSettings {
            message: "the cover's time must be inside the export".to_string(),
        });
    }
    validate(&request.settings)?;
    if request.output.is_dir() {
        return Err(EngineError::InvalidSettings {
            message: format!("{} is a folder, not a file", request.output.display()),
        });
    }
    if request.output.exists() && !request.allow_overwrite {
        return Err(EngineError::OutputExists {
            path: request.output.display().to_string(),
        });
    }
    let parent = output_dir(&request.output);
    if !parent.is_dir() {
        return Err(EngineError::InvalidSettings {
            message: format!("{} is not a directory", parent.display()),
        });
    }
    let used = assets_in(project, &request.timeline);
    let notes = rights(project, &used);
    let media = used
        .into_iter()
        .map(|asset| {
            let path = resolve(project, &request.bundle, &asset)?;
            Ok((asset, path))
        })
        .collect::<Result<Vec<_>, EngineError>>()?;
    // LUT files must be there too, so a missing one fails before anything renders.
    for asset in crate::frame::timeline_luts(project, &request.timeline) {
        resolve(project, &request.bundle, &asset)?;
    }
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
            rights: notes,
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
        start,
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
) -> Result<u64, EngineError> {
    let settings = &request.settings;
    let duration = checked.timeline_duration;
    let start = checked.start;
    let mut writer = FfmpegWriter::create(path, settings)?;
    let audio_format = settings.audio.as_ref().map(|a| (a.sample_rate, a.channels));
    let mut push_audio =
        |writer: &mut FfmpegWriter, from: Flicks, to: Flicks| -> Result<(), EngineError> {
            if let Some((rate, channels)) = audio_format {
                let buffer = mix(
                    project,
                    &request.timeline,
                    start + from,
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
                return Err(EngineError::Cancelled);
            }
        }
        writer.finish()?;
        return Ok(0);
    };

    let total = checked.plan.frames as i64;
    for n in 0..total {
        let time = video.rate.frames_to_flicks(n);
        let plan = plan_frame(project, &request.timeline, start + time)?;
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
            return Err(EngineError::Cancelled);
        }
    }
    writer.finish()?;
    Ok(total as u64)
}

fn validate(settings: &OutputSettings) -> Result<(), EngineError> {
    let video = settings.video.as_ref();
    if let Some(v) = video
        && (v.width == 0 || v.height == 0 || v.width % 2 != 0 || v.height % 2 != 0)
    {
        return Err(EngineError::InvalidSettings {
            message: format!("{}x{} is not an even, non-zero size", v.width, v.height),
        });
    }
    if !settings.container.supports(
        video.map(|v| v.codec),
        settings.audio.as_ref().map(|a| a.codec),
    ) {
        return Err(EngineError::InvalidSettings {
            message: format!(
                "{} cannot hold these codecs",
                settings.container.extension()
            ),
        });
    }
    if let Some(a) = settings.audio.as_ref()
        && audio_encoder(a.codec).is_none()
    {
        return Err(EngineError::CodecUnavailable {
            codec: a.codec.probe_name().to_string(),
            available: encoder_capabilities()
                .audio
                .iter()
                .map(|(c, _)| c.probe_name().to_string())
                .collect(),
        });
    }
    if let Some(v) = video
        && video_encoder(v.codec).is_none()
    {
        return Err(EngineError::CodecUnavailable {
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
    let family_ok = settings
        .container
        .matches_probe(&report.container, report.brand.as_deref());
    // GIF frame delays are whole centiseconds, so a GIF's rate and length are close, not
    // exact.
    let gif = settings.container == Container::Gif;
    let slack = if gif {
        Flicks::from_millis(20)
    } else {
        Flicks::ZERO
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
                gif || got.rate == Some(want.rate),
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
                    (file_duration - duration).get().abs() <= slack.get(),
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

/// Assets that clips on `timeline` read frames or audio from, inside compound clips too.
fn assets_in(project: &Project, timeline: &Id) -> Vec<Id> {
    let mut assets = Vec::new();
    let mut seen = vec![timeline.clone()];
    let mut pending = vec![timeline.clone()];
    while let Some(id) = pending.pop() {
        let clips = project
            .timelines
            .get(&id)
            .into_iter()
            .flat_map(|doc| doc.tracks.iter())
            .flat_map(|track| track.clips.iter());
        for clip in clips {
            if let Some(asset) = clip.content.asset() {
                assets.push(asset.clone());
            }
            if let Some(nested) = clip.content.nested()
                && !seen.contains(nested)
            {
                seen.push(nested.clone());
                pending.push(nested.clone());
            }
        }
    }
    assets.sort();
    assets.dedup();
    assets
}

/// The media file of `asset`: the path relative to the bundle first, then the absolute path.
pub(crate) fn resolve(
    project: &Project,
    bundle: &Path,
    asset: &Id,
) -> Result<PathBuf, EngineError> {
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
    Err(EngineError::MissingMedia {
        asset: asset.to_string(),
        tried,
    })
}

/// Audio decoders per asset, opened on first use.
#[derive(Default)]
pub(crate) struct DecoderAudio {
    pub(crate) paths: HashMap<Id, PathBuf>,
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

fn failed_io(path: &Path, error: &std::io::Error) -> EngineError {
    EngineError::Failed {
        code: "fosscut.media.write_failed".into(),
        message: format!("cannot write {}: {error}", path.display()),
    }
}
