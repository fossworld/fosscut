use fosscut_core::{ApiError, ErrorCode};
use fosscut_media::OutputReport;
use fosscut_time::Flicks;
use serde_json::{Value, json};
use thiserror::Error;

/// Why an engine job failed: an export that did not produce a verified file, or a frame
/// grab that could not render.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum EngineError {
    /// The timeline does not exist.
    #[error("timeline {timeline} does not exist")]
    TimelineNotFound {
        /// The requested timeline.
        timeline: String,
    },
    /// The timeline has nothing to export or show.
    #[error("the timeline is empty")]
    EmptyTimeline,
    /// The requested time is outside the timeline.
    #[error(
        "{} s is outside the timeline, which lasts {} s",
        time.to_seconds_string(),
        duration.to_seconds_string()
    )]
    TimeOutOfRange {
        /// The requested time.
        time: Flicks,
        /// The timeline's duration.
        duration: Flicks,
    },
    /// An argument is out of its allowed range.
    #[error("{message}")]
    InvalidArguments {
        /// What is wrong.
        message: String,
    },
    /// The settings cannot work: odd size, impossible container and codec, no streams.
    #[error("invalid export settings: {message}")]
    InvalidSettings {
        /// What is wrong.
        message: String,
    },
    /// This machine has no encoder for the requested codec. Never substituted (ADR-0009).
    #[error("this machine cannot encode {codec}")]
    CodecUnavailable {
        /// The requested codec.
        codec: String,
        /// Codecs that work here instead.
        available: Vec<String>,
    },
    /// The project changed since the revision the request expected (ADR-0006, item 1).
    #[error("the project is at revision {current}, not {expected}")]
    RevisionConflict {
        /// The revision the request expected.
        expected: u64,
        /// The project's revision.
        current: u64,
    },
    /// The output exists and overwriting it was not allowed (ADR-0006, item 6).
    #[error("{path} already exists")]
    OutputExists {
        /// The output path.
        path: String,
    },
    /// Another running job is writing the same output.
    #[error("{job} is already writing {path}")]
    OutputBusy {
        /// The output path.
        path: String,
        /// The running job.
        job: String,
    },
    /// No job has this id in this session.
    #[error("there is no job {job}")]
    JobNotFound {
        /// The requested job id.
        job: String,
    },
    /// No asset has this id.
    #[error("asset {asset} does not exist")]
    AssetNotFound {
        /// The requested asset.
        asset: String,
    },
    /// An audio analyzer was asked for an asset without sound.
    #[error("asset {asset} has no audio to analyze")]
    NoAudio {
        /// The asset.
        asset: String,
    },
    /// A video analyzer was asked for an asset without moving pictures.
    #[error("asset {asset} has no video to analyze")]
    NoVideo {
        /// The asset.
        asset: String,
    },
    /// No analyzer has this name.
    #[error("there is no analyzer {name}")]
    UnknownAnalyzer {
        /// The requested name.
        name: String,
        /// The analyzers that exist.
        known: Vec<String>,
    },
    /// The analysis was cancelled; nothing was cached.
    #[error("the analysis was cancelled")]
    AnalysisCancelled,
    /// An export, frame or plan needs an analysis that is not cached: a clip's `key.subject`
    /// reads a matte of its media, a body effect its pose (ADR-0033), and reframing and
    /// shorts read pose, mattes, scene cuts or transcripts. Nothing was rendered or planned.
    #[error("{effect} needs the {analyzer} analysis of {asset}, which is not cached")]
    AnalysisMissing {
        /// The asset to analyze.
        asset: String,
        /// The analyzer to run, such as `matte`, `pose`, `scenes` or `transcript`.
        analyzer: String,
        /// The analyzer's options that matter, as `media_analyze` takes them, for example
        /// `{"matte": {"model": "subject"}}`.
        options: Value,
        /// What needs it: an effect such as `key.subject`, or a planner (`reframe`,
        /// `camera_track`, `vertical`, `shorts`).
        effect: String,
    },
    /// No model has this name.
    #[error("there is no model {name}")]
    ModelUnknown {
        /// The requested name.
        name: String,
    },
    /// A model the analysis needs is not installed.
    #[error("{what} is not installed")]
    ModelNotInstalled {
        /// The model, or the kind of model needed.
        what: String,
    },
    /// A downloaded or copied model does not match its manifest.
    #[error("{name} does not match its checksum: expected {expected}, got {actual}")]
    ModelChecksum {
        /// The model.
        name: String,
        /// What the manifest says.
        expected: String,
        /// What the file has.
        actual: String,
    },
    /// A model could not be downloaded or copied.
    #[error("cannot install {name}: {message}")]
    ModelDownload {
        /// The model.
        name: String,
        /// What went wrong.
        message: String,
    },
    /// Installing a model downloads it; the caller has to confirm first.
    #[error(
        "installing {name} downloads {} ({license} license) and needs confirmation",
        megabytes(.size_bytes)
    )]
    ModelConfirmation {
        /// The model.
        name: String,
        /// Its download size.
        size_bytes: u64,
        /// Its license.
        license: String,
    },
    /// The model has no build for this system.
    #[error("{name} has no build for {platform}")]
    ModelUnsupported {
        /// The model.
        name: String,
        /// The system, for example `linux aarch64`.
        platform: String,
    },
    /// ONNX Runtime, which the vision, voice and translation models run on, is not
    /// installed or cannot be used (ADR-0033).
    #[error("{message}")]
    ModelRuntimeMissing {
        /// What is wrong.
        message: String,
        /// The libraries looked for, or the one that cannot be used.
        tried: Vec<String>,
    },
    /// A model could not be loaded into an ONNX Runtime session.
    #[error("cannot load model {model}: {message}")]
    ModelLoadFailed {
        /// The model file.
        model: String,
        /// What ONNX Runtime reported.
        message: String,
    },
    /// Speech recognition failed.
    #[error("speech recognition failed: {message}")]
    SpeechFailed {
        /// What whisper.cpp reported.
        message: String,
    },
    /// This build has no speech recognition.
    #[error("this build of FossCut has no speech recognition")]
    SpeechUnavailable,
    /// Captions were asked for from transcripts that are not cached.
    #[error("{message}")]
    NoTranscript {
        /// Assets without a cached transcript.
        assets: Vec<String>,
        /// What is missing.
        message: String,
    },
    /// A subtitle file could not be read.
    #[error("line {line}: {message}")]
    InvalidSrt {
        /// The line, counting from 1.
        line: usize,
        /// What is wrong.
        message: String,
    },
    /// The timeline has no captions to export.
    #[error("timeline {timeline} has no captions")]
    NoCaptions {
        /// The timeline.
        timeline: String,
    },
    /// A clip's media file cannot be found.
    #[error("media for asset {asset} is missing")]
    MissingMedia {
        /// The asset.
        asset: String,
        /// Paths that were tried.
        tried: Vec<String>,
    },
    /// The job was cancelled; no output was written.
    #[error("the export was cancelled")]
    Cancelled,
    /// The request id was used before for a different export.
    #[error("request id {request_id} was already used for another export")]
    RequestReused {
        /// The request id.
        request_id: String,
    },
    /// The written file does not match the request (ADR-0009, item 6). The file is kept.
    #[error("the exported file does not match the request: {}", mismatches.join("; "))]
    VerificationFailed {
        /// Each difference between request and file.
        mismatches: Vec<String>,
        /// What the file contains.
        report: Box<OutputReport>,
    },
    /// A proxy or a remuxed copy was asked for an asset without moving pictures.
    #[error("asset {asset} is not a video, so it has no proxy or remuxed copy")]
    NotVideo {
        /// The asset.
        asset: String,
    },
    /// A remuxed copy was asked for media whose container already seeks well (ADR-0032,
    /// item 3).
    #[error("asset {asset} is in {container}, which seeks well without a remuxed copy")]
    RemuxNotNeeded {
        /// The asset.
        asset: String,
        /// Its container, as probed.
        container: String,
    },
    /// A proxy or remuxed copy did not come out as asked; it was removed.
    #[error("the {kind} does not match its source: {}", mismatches.join("; "))]
    CopyMismatch {
        /// `proxy` or `remux`.
        kind: String,
        /// Each difference.
        mismatches: Vec<String>,
    },
    /// A background job other than an export or an analysis was cancelled; nothing it was
    /// making was kept.
    #[error("the {kind} was cancelled")]
    JobCancelled {
        /// What the job was making, such as `proxy` or `remux`.
        kind: String,
    },
    /// No beat was found in the music: it is silent, or has no pulse to follow.
    #[error("no beats were found in the sound of {asset}")]
    NoBeats {
        /// The asset whose sound was analyzed.
        asset: String,
    },
    /// No subject was found to frame in any frame of the clip.
    #[error("no {subject} was found in {clip}")]
    NoSubject {
        /// The clip.
        clip: String,
        /// What was looked for: `person`, `subject` or `region`.
        subject: String,
    },
    /// No stretch of whole sentences in the clip fits the length asked for.
    #[error("no short of {min_seconds} to {max_seconds} s fits the speech of {clip}")]
    NoShortCandidates {
        /// The clip.
        clip: String,
        /// Sentences found in the part of the media it plays.
        sentences: u32,
        /// The shortest length asked for, in seconds.
        min_seconds: f64,
        /// The longest length asked for, in seconds.
        max_seconds: f64,
    },
    /// Rendering, decoding, mixing or encoding failed.
    #[error("{message}")]
    Failed {
        /// The underlying error code, for example `fosscut.media.encode_failed`.
        code: String,
        /// What went wrong.
        message: String,
    },
}

impl EngineError {
    /// The stable error code used in FossCut API responses (ADR-0006).
    pub fn code(&self) -> &str {
        match self {
            EngineError::TimelineNotFound { .. } => "fosscut.timeline.not_found",
            EngineError::EmptyTimeline => "fosscut.export.empty_timeline",
            EngineError::TimeOutOfRange { .. } => "fosscut.time.out_of_range",
            EngineError::InvalidArguments { .. } => "fosscut.request.invalid_arguments",
            EngineError::InvalidSettings { .. } => "fosscut.export.invalid_settings",
            EngineError::CodecUnavailable { .. } => "fosscut.export.codec_unavailable",
            EngineError::RevisionConflict { .. } => "fosscut.revision.conflict",
            EngineError::OutputExists { .. } => "fosscut.guard.destructive",
            EngineError::OutputBusy { .. } => "fosscut.export.output_busy",
            EngineError::JobNotFound { .. } => "fosscut.job.not_found",
            EngineError::AssetNotFound { .. } => "fosscut.asset.not_found",
            EngineError::NoAudio { .. } => "fosscut.analysis.no_audio",
            EngineError::NoVideo { .. } => "fosscut.analysis.no_video",
            EngineError::UnknownAnalyzer { .. } => "fosscut.analysis.unknown_analyzer",
            EngineError::AnalysisCancelled => "fosscut.analysis.cancelled",
            EngineError::AnalysisMissing { .. } => "fosscut.analysis.missing",
            EngineError::ModelUnknown { .. } => "fosscut.model.unknown",
            EngineError::ModelNotInstalled { .. } => "fosscut.model.not_installed",
            EngineError::ModelChecksum { .. } => "fosscut.model.checksum_mismatch",
            EngineError::ModelDownload { .. } => "fosscut.model.download_failed",
            EngineError::ModelConfirmation { .. } => "fosscut.model.confirmation_required",
            EngineError::ModelUnsupported { .. } => "fosscut.model.unsupported_platform",
            EngineError::ModelRuntimeMissing { .. } => "fosscut.model.runtime_missing",
            EngineError::ModelLoadFailed { .. } => "fosscut.model.load_failed",
            EngineError::SpeechFailed { .. } => "fosscut.analysis.speech_failed",
            EngineError::SpeechUnavailable => "fosscut.analysis.speech_unavailable",
            EngineError::NoTranscript { .. } => "fosscut.captions.no_transcript",
            EngineError::InvalidSrt { .. } => "fosscut.captions.invalid_srt",
            EngineError::NoCaptions { .. } => "fosscut.captions.none",
            EngineError::MissingMedia { .. } => "fosscut.export.missing_media",
            EngineError::Cancelled => "fosscut.export.cancelled",
            EngineError::RequestReused { .. } => "fosscut.request.reused",
            EngineError::VerificationFailed { .. } => "fosscut.export.verification_failed",
            EngineError::NotVideo { .. } => "fosscut.cache.not_video",
            EngineError::RemuxNotNeeded { .. } => "fosscut.cache.remux_not_needed",
            EngineError::CopyMismatch { .. } => "fosscut.cache.verification_failed",
            EngineError::JobCancelled { .. } => "fosscut.job.cancelled",
            EngineError::NoBeats { .. } => "fosscut.beats.none",
            EngineError::NoSubject { .. } => "fosscut.reframe.no_subject",
            EngineError::NoShortCandidates { .. } => "fosscut.shorts.no_candidates",
            EngineError::Failed { code, .. } => code,
        }
    }

    /// What an agent or person can do about it.
    pub fn hint(&self) -> Option<String> {
        match self {
            EngineError::CodecUnavailable { available, .. } => Some(format!(
                "Choose one of the codecs this machine can encode: {}.",
                available.join(", ")
            )),
            EngineError::OutputExists { .. } => {
                Some(
                    "Pass allow_destructive (--allow-destructive on the command line) to overwrite it, or choose another path."
                        .into(),
                )
            }
            EngineError::OutputBusy { .. } => Some(
                "Wait for that job with job_status, cancel it, or choose another path.".into(),
            ),
            EngineError::JobNotFound { .. } => Some(
                "Jobs live as long as the session that started them; start the export again."
                    .into(),
            ),
            EngineError::AssetNotFound { .. } => {
                Some("media_list (fosscut media list) shows the project's assets.".into())
            }
            EngineError::NoAudio { .. } => Some(
                "Choose an analyzer that reads video (scenes), or an asset with sound.".into(),
            ),
            EngineError::NoVideo { .. } => Some(
                "Choose an analyzer that reads audio (loudness, silences), or a video asset."
                    .into(),
            ),
            EngineError::UnknownAnalyzer { known, .. } => {
                Some(format!("Analyzers: {}.", known.join(", ")))
            }
            EngineError::AnalysisMissing {
                asset, analyzer, ..
            } => Some(format!(
                "Analyze it first: media_analyze with asset {asset}, analyzers [\"{analyzer}\"] and the options in details (fosscut media analyze {asset} {analyzer} on the command line), then render again. It runs a local model, which must be installed."
            )),
            EngineError::ModelUnknown { .. } => {
                Some(format!("Models: {}.", model_names()))
            }
            EngineError::ModelNotInstalled { what } => Some(format!(
                "Ask the user before installing, since it downloads: models install (fosscut models install <name>). Models: {}.",
                model_sizes(what)
            )),
            EngineError::ModelChecksum { .. } => Some(
                "The file is damaged or not the published model; install it again from its URL."
                    .into(),
            ),
            EngineError::ModelDownload { .. } => Some(
                "Check the network and that curl is installed, or download the file yourself and install it with --from <file>."
                    .into(),
            ),
            EngineError::ModelConfirmation { .. } => Some(
                "Tell the user the size and license; install with --yes (CLI) or confirm: true (MCP) only if they agree."
                    .into(),
            ),
            EngineError::ModelUnsupported { .. } => Some(
                "This model cannot run on this system; models lists the others for the same task."
                    .into(),
            ),
            EngineError::ModelRuntimeMissing { .. } => Some(format!(
                "Ask the user before installing, since it downloads: models install onnxruntime (fosscut models install onnxruntime), {}; or set {} to an ONNX Runtime {} or later library.",
                runtime_size(),
                fosscut_analysis::onnx::LIBRARY_VARIABLE,
                fosscut_analysis::onnx::MINIMUM_VERSION
            )),
            EngineError::ModelLoadFailed { .. } => Some(
                "The model file may be damaged or incomplete: install the model again (models install <name> after removing it), or report the message."
                    .into(),
            ),
            EngineError::SpeechFailed { .. } => Some(
                "Try another speech model (options.transcript.model), or report the message."
                    .into(),
            ),
            EngineError::SpeechUnavailable => {
                Some("Use a FossCut build with the speech feature.".into())
            }
            EngineError::NoTranscript { .. } => Some(
                "Transcribe the clips' media first: media_analyze with analyzers [\"transcript\"] (fosscut media analyze <asset> transcript), with the same transcript options as here."
                    .into(),
            ),
            EngineError::InvalidSrt { .. } => Some(
                "Each cue needs a timing line HH:MM:SS,mmm --> HH:MM:SS,mmm, text, and a blank line before the next; cues must not overlap."
                    .into(),
            ),
            EngineError::NoCaptions { .. } => Some(
                "Add captions first: the captions tool with action generate or import, or captions.add.".into(),
            ),
            EngineError::MissingMedia { .. } => {
                Some("Relink the media or restore the file at one of the tried paths.".into())
            }
            EngineError::EmptyTimeline => Some("Add clips before exporting.".into()),
            EngineError::TimeOutOfRange { .. } => Some(
                "Choose a time from 0 up to the timeline's duration, which timeline.read reports."
                    .into(),
            ),
            EngineError::RevisionConflict { .. } => Some(
                "Read the timeline again, then export with the current revision.".into(),
            ),
            EngineError::NotVideo { .. } => {
                Some("Proxies and remuxed copies are made of video assets only.".into())
            }
            EngineError::RemuxNotNeeded { .. } => Some(
                "Only MPEG transport and program streams, AVI and FLV files are remuxed; this file previews well as it is."
                    .into(),
            ),
            EngineError::CopyMismatch { .. } => Some(
                "The preview keeps using the original media; report the mismatches.".into(),
            ),
            EngineError::NoBeats { .. } => Some(
                "Choose music with a steady pulse, or give its tempo in options.beats.bpm; markers can also be added by hand with marker.add.".into(),
            ),
            EngineError::NoSubject { subject, .. } => Some(match subject.as_str() {
                "region" => "The region was lost from the first frame: give a region around something with texture, at a time it is visible.".into(),
                "person" => "Nobody was found in its pose analysis: frame it with clip.set, or use source subject (a matte) or region.".into(),
                _ => "Its matte is empty: frame it with clip.set, or use source person or region.".into(),
            }),
            EngineError::NoShortCandidates { .. } => Some(
                "Lower min_seconds or raise max_seconds, or choose a longer clip; candidates start and end at sentence ends in its transcript.".into(),
            ),
            _ => None,
        }
    }

    /// Machine-readable details.
    pub fn details(&self) -> Value {
        match self {
            EngineError::CodecUnavailable { codec, available } => {
                json!({ "codec": codec, "available": available })
            }
            EngineError::MissingMedia { asset, tried } => json!({ "asset": asset, "tried": tried }),
            EngineError::VerificationFailed { mismatches, report } => {
                json!({ "mismatches": mismatches, "report": report })
            }
            EngineError::OutputExists { path } => json!({ "path": path }),
            EngineError::OutputBusy { path, job } => json!({ "path": path, "job": job }),
            EngineError::JobNotFound { job } => json!({ "job": job }),
            EngineError::AssetNotFound { asset }
            | EngineError::NoAudio { asset }
            | EngineError::NoVideo { asset } => json!({ "asset": asset }),
            EngineError::UnknownAnalyzer { name, known } => {
                json!({ "name": name, "known": known })
            }
            EngineError::AnalysisMissing {
                asset,
                analyzer,
                options,
                effect,
            } => json!({
                "asset": asset,
                "analyzer": analyzer,
                "options": options,
                "effect": effect,
            }),
            EngineError::ModelUnknown { name } => json!({ "name": name }),
            EngineError::ModelNotInstalled { what } => json!({ "what": what }),
            EngineError::ModelChecksum {
                name,
                expected,
                actual,
            } => json!({ "name": name, "expected": expected, "actual": actual }),
            EngineError::ModelDownload { name, message } => {
                json!({ "name": name, "message": message })
            }
            EngineError::ModelConfirmation {
                name,
                size_bytes,
                license,
            } => json!({ "name": name, "size_bytes": size_bytes, "license": license }),
            EngineError::ModelUnsupported { name, platform } => {
                json!({ "name": name, "platform": platform })
            }
            EngineError::ModelRuntimeMissing { tried, .. } => json!({ "tried": tried }),
            EngineError::ModelLoadFailed { model, message } => {
                json!({ "model": model, "message": message })
            }
            EngineError::SpeechFailed { message } => json!({ "message": message }),
            EngineError::NoTranscript { assets, .. } => json!({ "assets": assets }),
            EngineError::InvalidSrt { line, .. } => json!({ "line": line }),
            EngineError::NoCaptions { timeline } => json!({ "timeline": timeline }),
            EngineError::TimeOutOfRange { time, duration } => {
                json!({ "time": time, "duration": duration })
            }
            EngineError::RevisionConflict { expected, current } => {
                json!({ "expected_revision": expected, "current_revision": current })
            }
            EngineError::NotVideo { asset } => json!({ "asset": asset }),
            EngineError::RemuxNotNeeded { asset, container } => {
                json!({ "asset": asset, "container": container })
            }
            EngineError::CopyMismatch { kind, mismatches } => {
                json!({ "kind": kind, "mismatches": mismatches })
            }
            EngineError::JobCancelled { kind } => json!({ "kind": kind }),
            EngineError::NoBeats { asset } => json!({ "asset": asset }),
            EngineError::NoSubject { clip, subject } => {
                json!({ "clip": clip, "subject": subject })
            }
            EngineError::NoShortCandidates {
                clip,
                sentences,
                min_seconds,
                max_seconds,
            } => json!({
                "clip": clip,
                "sentences": sentences,
                "min_seconds": min_seconds,
                "max_seconds": max_seconds,
            }),
            _ => Value::Null,
        }
    }
}

/// Every error code the engine adds to the core's catalogue
/// ([`fosscut_core::error_catalogue`]), with recovery hints.
pub fn engine_error_catalogue() -> Vec<ErrorCode> {
    let entry = |code, meaning, hint| ErrorCode {
        code,
        meaning,
        hint,
    };
    vec![
        entry(
            "fosscut.export.empty_timeline",
            "The timeline has no clips, so there is nothing to export or show.",
            "Add clips first.",
        ),
        entry(
            "fosscut.time.out_of_range",
            "The time is before 0 or at or after the end of the timeline.",
            "Choose a time from 0 up to the timeline's duration, which timeline.read reports.",
        ),
        entry(
            "fosscut.export.invalid_settings",
            "The export settings cannot work: an odd size, a codec the container cannot hold, or a missing folder.",
            "Use a preset from the export presets, or fix the setting the message names.",
        ),
        entry(
            "fosscut.export.codec_unavailable",
            "This machine has no working encoder for the requested codec. FossCut never substitutes another.",
            "Choose one of the codecs listed in details.available, or another preset.",
        ),
        entry(
            "fosscut.export.output_busy",
            "Another running job is writing the same output file.",
            "Wait for that job (details.job), cancel it, or choose another path.",
        ),
        entry(
            "fosscut.job.not_found",
            "No job with this id exists in this session.",
            "Jobs end with the session that started them; start the export again.",
        ),
        entry(
            "fosscut.analysis.no_audio",
            "An audio analyzer (loudness, silences) was asked for an asset without sound.",
            "Choose another analyzer or asset.",
        ),
        entry(
            "fosscut.analysis.no_video",
            "A video analyzer (scenes) was asked for an asset without moving pictures.",
            "Choose another analyzer or asset.",
        ),
        entry(
            "fosscut.analysis.unknown_analyzer",
            "No analyzer has this name.",
            "Use one of the names in details.known; capabilities lists them.",
        ),
        entry(
            "fosscut.analysis.cancelled",
            "The analysis was cancelled; nothing was cached.",
            "Start it again when wanted.",
        ),
        entry(
            "fosscut.analysis.missing",
            "An analysis of a clip's media is not cached: a matte or pose for key.subject or a body effect (nothing was rendered), or the pose, matte, scenes or transcript that reframing and shorts read (nothing was planned).",
            "Run media_analyze with details.asset, analyzers [details.analyzer] and options details.options, then try again.",
        ),
        entry(
            "fosscut.beats.none",
            "No beat was found in the music: it is silent or has no steady pulse.",
            "Choose other music, give its tempo in options.beats.bpm, or add markers with marker.add.",
        ),
        entry(
            "fosscut.reframe.no_subject",
            "No subject was found in any frame of the clip: no person in its pose analysis, an empty matte, or a region lost at once.",
            "Frame the clip yourself with clip.set, or follow another source (person, subject or region).",
        ),
        entry(
            "fosscut.shorts.no_candidates",
            "No stretch of whole sentences in the clip's transcript lasts from min_seconds to max_seconds.",
            "Widen the length range, or use a longer clip; details.sentences counts the sentences found.",
        ),
        entry(
            "fosscut.voice.unknown_voice",
            "There is no voice with this name.",
            "List the voices (fosscut speak --voices, or the record tool's action voices) and choose one, such as af_heart.",
        ),
        entry(
            "fosscut.voice.language_unavailable",
            "Text to speech speaks English only; the text or the language asked for is not English.",
            "Give English text, or caption the speech instead of dubbing it.",
        ),
        entry(
            "fosscut.voice.synthesis_failed",
            "The voice model could not speak the text.",
            "Try shorter text or another voice, and check the kokoro-82m model is installed intact (fosscut models list).",
        ),
        entry(
            "fosscut.voice.write_failed",
            "The speech file could not be written into the bundle's speech folder.",
            "Check the bundle is writable and the disk has room.",
        ),
        entry(
            "fosscut.voice.unavailable",
            "This build of FossCut has no text to speech.",
            "Use a build with the onnx feature.",
        ),
        entry(
            "fosscut.translate.unsupported_pair",
            "No translation model covers these two languages.",
            "Translate between English and Hungarian (en>hu, hu>en), or translate the captions yourself with captions.edit.",
        ),
        entry(
            "fosscut.translate.failed",
            "The translation model could not translate the text.",
            "Check the opus-mt model is installed intact (fosscut models list), or translate the captions yourself with captions.edit.",
        ),
        entry(
            "fosscut.translate.unavailable",
            "This build of FossCut has no translation models.",
            "Use a build with the onnx feature, or translate the captions yourself with captions.edit.",
        ),
        entry(
            "fosscut.model.inference_failed",
            "An installed vision model failed while it ran.",
            "Try again, install the model again, or report the message.",
        ),
        entry(
            "fosscut.model.unknown",
            "No model has this name.",
            "capabilities lists the models.",
        ),
        entry(
            "fosscut.model.not_installed",
            "The analysis needs a model that is not installed.",
            "Ask the user, then install one with models install; the hint lists sizes.",
        ),
        entry(
            "fosscut.model.checksum_mismatch",
            "The downloaded or copied model does not match its published checksum.",
            "Install it again from its URL.",
        ),
        entry(
            "fosscut.model.download_failed",
            "The model could not be downloaded or copied.",
            "Check the network and curl, or install from a local file with --from.",
        ),
        entry(
            "fosscut.model.confirmation_required",
            "Installing a model downloads it, so the call must confirm.",
            "Tell the user the size and license; confirm only if they agree.",
        ),
        entry(
            "fosscut.model.unsupported_platform",
            "The model has no build for this operating system and processor.",
            "Choose another model for the same task; models lists them.",
        ),
        entry(
            "fosscut.model.runtime_missing",
            "ONNX Runtime, which the vision, voice and translation models run on, is not installed or cannot be used.",
            "Ask the user, then install it with models install onnxruntime, or set FOSSCUT_ORT_DYLIB to an ONNX Runtime 1.22 or later library.",
        ),
        entry(
            "fosscut.model.load_failed",
            "An installed model could not be loaded into ONNX Runtime.",
            "Install the model again, or report the message in details.",
        ),
        entry(
            "fosscut.analysis.speech_failed",
            "Speech recognition failed.",
            "Try another speech model, or report the message.",
        ),
        entry(
            "fosscut.analysis.speech_unavailable",
            "This build of FossCut has no speech recognition.",
            "Use a build with the speech feature.",
        ),
        entry(
            "fosscut.captions.no_transcript",
            "Captions were asked for from media without a cached transcript.",
            "Run media_analyze with transcript on the listed assets first.",
        ),
        entry(
            "fosscut.captions.invalid_srt",
            "The subtitle file is not valid SRT.",
            "Fix the line named in details.",
        ),
        entry(
            "fosscut.captions.none",
            "There are no captions to export.",
            "Add captions first with generate, import or captions.add.",
        ),
        entry(
            "fosscut.export.missing_media",
            "A clip's media file is not at its recorded paths.",
            "Restore the file at one of details.tried, or import it again and replace the clip.",
        ),
        entry(
            "fosscut.export.cancelled",
            "The export was cancelled; nothing was written.",
            "Start it again when wanted.",
        ),
        entry(
            "fosscut.export.verification_failed",
            "The written file does not match the request; details.mismatches lists each difference.",
            "Report it with details.report; the file is kept for inspection.",
        ),
        entry(
            "fosscut.session.already_hosted",
            "Another program already has this project open as its live host; details name it.",
            "Edit through it: the CLI and MCP reach it on their own. Close it there to host the project here.",
        ),
        entry(
            "fosscut.session.unauthorized",
            "A live host or client could not prove it knows the session key.",
            "Something other than FossCut answers at the project's endpoint. Close the app holding the project and open it again.",
        ),
        entry(
            "fosscut.session.protocol",
            "The live host speaks another protocol version, or a message broke the protocol.",
            "Use the same FossCut version for the app and the CLI or MCP server.",
        ),
        entry(
            "fosscut.session.unavailable",
            "The live host could not be reached, or was lost during a request.",
            "When details.outcome is unknown, read history before retrying; retry with the same request_id when there was one.",
        ),
        entry(
            "fosscut.session.diverged",
            "Files of the project changed outside FossCut while it was open, so the host stopped writing.",
            "Close the project in the app and open it again; check what changed the files (a sync tool, version control).",
        ),
        entry(
            "fosscut.media.write_failed",
            "The output file could not be written or moved into place.",
            "Check the folder exists and is writable, and that no other program holds the file open.",
        ),
        entry(
            "fosscut.media.device_unavailable",
            "The microphone, camera or screen cannot be opened: there is none of that kind, it is in use, or the system has not allowed FossCut to use it.",
            "List the devices (fosscut record --list), choose another, or allow FossCut in the system's privacy settings for the microphone, camera or screen recording.",
        ),
        entry(
            "fosscut.media.input_format_unavailable",
            "This FFmpeg build has no capture input of the requested kind, such as avfoundation or dshow.",
            "Use one of the input formats in details.available, or an FFmpeg build with device support.",
        ),
        entry(
            "fosscut.record.confirmation_required",
            "Recording uses the user's microphone, camera or screen, so an agent's call must confirm that the user asked for it.",
            "Record only when the user asked you to; then call again with confirm true.",
        ),
        entry(
            "fosscut.cache.not_video",
            "A proxy or a remuxed copy was asked for an asset that is not a video.",
            "Choose a video asset; media_list shows each asset's kind.",
        ),
        entry(
            "fosscut.cache.remux_not_needed",
            "The asset's container already seeks well, so it gets no remuxed copy.",
            "Only MPEG transport and program streams, AVI and FLV files are remuxed.",
        ),
        entry(
            "fosscut.cache.verification_failed",
            "A proxy or remuxed copy did not match its source and was removed; the preview keeps the original.",
            "Report it with details.mismatches.",
        ),
        entry(
            "fosscut.job.cancelled",
            "The job was cancelled; nothing it was making was kept.",
            "Start it again when wanted.",
        ),
        entry(
            "fosscut.media.nothing_recorded",
            "The recording stopped before the device delivered anything, so no file was written.",
            "Record for longer, or check that the device works and is allowed.",
        ),
    ]
}

/// A size in megabytes with one decimal, for messages.
fn megabytes(bytes: &u64) -> String {
    format!("{:.1} MB", *bytes as f64 / 1e6)
}

/// Every model name, for hints.
fn model_names() -> String {
    fosscut_analysis::models::MODELS
        .iter()
        .map(|m| m.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The models that would satisfy `what` (a model's name, a kind of speech model, or
/// anything else) with their download sizes, for hints.
fn model_sizes(what: &str) -> String {
    use fosscut_analysis::models::{MODELS, manifest};
    let named = manifest(what).ok();
    MODELS
        .iter()
        .filter(|m| match named {
            Some(named) => m.name == named.name,
            None if what.contains("speech model") => m.task == "speech",
            None => true,
        })
        .map(|m| format!("{} ({})", m.name, megabytes(&m.size())))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The ONNX Runtime download's size on this system, for hints.
fn runtime_size() -> String {
    fosscut_analysis::models::manifest(fosscut_analysis::onnx::RUNTIME_MODEL)
        .map(|m| megabytes(&m.size()))
        .unwrap_or_default()
}

impl From<fosscut_analysis::models::ModelError> for EngineError {
    fn from(error: fosscut_analysis::models::ModelError) -> EngineError {
        use fosscut_analysis::models::ModelError;
        match error {
            ModelError::Unknown { name } => EngineError::ModelUnknown { name },
            ModelError::NotInstalled { what } => EngineError::ModelNotInstalled { what },
            ModelError::ChecksumMismatch {
                name,
                expected,
                actual,
            } => EngineError::ModelChecksum {
                name,
                expected,
                actual,
            },
            ModelError::DownloadFailed { name, message } => {
                EngineError::ModelDownload { name, message }
            }
            ModelError::Unsupported { name, platform } => {
                EngineError::ModelUnsupported { name, platform }
            }
        }
    }
}

impl From<fosscut_analysis::onnx::OnnxError> for EngineError {
    fn from(error: fosscut_analysis::onnx::OnnxError) -> EngineError {
        use fosscut_analysis::onnx::OnnxError;
        let message = error.to_string();
        match error {
            OnnxError::RuntimeMissing { tried } => {
                EngineError::ModelRuntimeMissing { message, tried }
            }
            OnnxError::RuntimeUnusable { library, .. } => EngineError::ModelRuntimeMissing {
                message,
                tried: vec![library],
            },
            OnnxError::LoadFailed { model, message } => {
                EngineError::ModelLoadFailed { model, message }
            }
        }
    }
}

impl From<&EngineError> for ApiError {
    /// The error as front ends return it: code, message, hint and details.
    fn from(error: &EngineError) -> ApiError {
        let mut api = ApiError::new(error.code(), error.to_string()).with_details(error.details());
        if let Some(hint) = error.hint() {
            api = api.with_hint(hint);
        }
        api
    }
}

impl From<EngineError> for ApiError {
    fn from(error: EngineError) -> ApiError {
        ApiError::from(&error)
    }
}

impl From<fosscut_media::MediaError> for EngineError {
    fn from(error: fosscut_media::MediaError) -> EngineError {
        match error {
            fosscut_media::MediaError::EncoderUnavailable { codec, available } => {
                EngineError::CodecUnavailable { codec, available }
            }
            fosscut_media::MediaError::Unsupported { message } => {
                EngineError::InvalidSettings { message }
            }
            other => EngineError::Failed {
                code: other.code().to_string(),
                message: other.to_string(),
            },
        }
    }
}

impl From<fosscut_render::RenderError> for EngineError {
    fn from(error: fosscut_render::RenderError) -> EngineError {
        EngineError::Failed {
            code: error.code().to_string(),
            message: error.to_string(),
        }
    }
}

impl From<fosscut_audio::AudioError> for EngineError {
    fn from(error: fosscut_audio::AudioError) -> EngineError {
        EngineError::Failed {
            code: error.code().to_string(),
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> OutputReport {
        OutputReport {
            container: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            brand: Some("isom".into()),
            size_bytes: 1,
            duration: None,
            video: None,
            audio: None,
        }
    }

    #[test]
    fn every_error_has_a_stable_code_message_hint_and_details() {
        let cases = [
            (
                EngineError::TimelineNotFound {
                    timeline: "timeline_0009".into(),
                },
                "fosscut.timeline.not_found",
                "timeline timeline_0009 does not exist",
                false,
            ),
            (
                EngineError::EmptyTimeline,
                "fosscut.export.empty_timeline",
                "the timeline is empty",
                true,
            ),
            (
                EngineError::InvalidSettings {
                    message: "odd width".into(),
                },
                "fosscut.export.invalid_settings",
                "invalid export settings: odd width",
                false,
            ),
            (
                EngineError::CodecUnavailable {
                    codec: "h264".into(),
                    available: vec!["av1".into(), "prores".into()],
                },
                "fosscut.export.codec_unavailable",
                "this machine cannot encode h264",
                true,
            ),
            (
                EngineError::RevisionConflict {
                    expected: 3,
                    current: 5,
                },
                "fosscut.revision.conflict",
                "the project is at revision 5, not 3",
                true,
            ),
            (
                EngineError::OutputExists {
                    path: "Out.mp4".into(),
                },
                "fosscut.guard.destructive",
                "Out.mp4 already exists",
                true,
            ),
            (
                EngineError::AnalysisMissing {
                    asset: "asset_0001".into(),
                    analyzer: "matte".into(),
                    options: json!({ "matte": { "model": "subject" } }),
                    effect: "key.subject".into(),
                },
                "fosscut.analysis.missing",
                "key.subject needs the matte analysis of asset_0001, which is not cached",
                true,
            ),
            (
                EngineError::MissingMedia {
                    asset: "asset_0001".into(),
                    tried: vec!["clip.mp4".into()],
                },
                "fosscut.export.missing_media",
                "media for asset asset_0001 is missing",
                true,
            ),
            (
                EngineError::TimeOutOfRange {
                    time: Flicks::from_seconds(9).unwrap(),
                    duration: Flicks::from_seconds(8).unwrap(),
                },
                "fosscut.time.out_of_range",
                "9 s is outside the timeline, which lasts 8 s",
                true,
            ),
            (
                EngineError::InvalidArguments {
                    message: "count must be 1 to 100".into(),
                },
                "fosscut.request.invalid_arguments",
                "count must be 1 to 100",
                false,
            ),
            (
                EngineError::OutputBusy {
                    path: "Out.mp4".into(),
                    job: "job_0001".into(),
                },
                "fosscut.export.output_busy",
                "job_0001 is already writing Out.mp4",
                true,
            ),
            (
                EngineError::JobNotFound {
                    job: "job_0009".into(),
                },
                "fosscut.job.not_found",
                "there is no job job_0009",
                true,
            ),
            (
                EngineError::Cancelled,
                "fosscut.export.cancelled",
                "the export was cancelled",
                false,
            ),
            (
                EngineError::AssetNotFound {
                    asset: "asset_0009".into(),
                },
                "fosscut.asset.not_found",
                "asset asset_0009 does not exist",
                true,
            ),
            (
                EngineError::NoAudio {
                    asset: "asset_0001".into(),
                },
                "fosscut.analysis.no_audio",
                "asset asset_0001 has no audio to analyze",
                true,
            ),
            (
                EngineError::NoVideo {
                    asset: "asset_0001".into(),
                },
                "fosscut.analysis.no_video",
                "asset asset_0001 has no video to analyze",
                true,
            ),
            (
                EngineError::UnknownAnalyzer {
                    name: "faces".into(),
                    known: vec!["loudness".into(), "scenes".into()],
                },
                "fosscut.analysis.unknown_analyzer",
                "there is no analyzer faces",
                true,
            ),
            (
                EngineError::AnalysisCancelled,
                "fosscut.analysis.cancelled",
                "the analysis was cancelled",
                false,
            ),
            (
                EngineError::ModelUnknown {
                    name: "whisper-huge".into(),
                },
                "fosscut.model.unknown",
                "there is no model whisper-huge",
                true,
            ),
            (
                EngineError::ModelNotInstalled {
                    what: "a speech model".into(),
                },
                "fosscut.model.not_installed",
                "a speech model is not installed",
                true,
            ),
            (
                EngineError::ModelChecksum {
                    name: "whisper-tiny.en".into(),
                    expected: "5 bytes".into(),
                    actual: "2 bytes".into(),
                },
                "fosscut.model.checksum_mismatch",
                "whisper-tiny.en does not match its checksum: expected 5 bytes, got 2 bytes",
                true,
            ),
            (
                EngineError::ModelDownload {
                    name: "whisper-tiny.en".into(),
                    message: "curl: (6) Could not resolve host".into(),
                },
                "fosscut.model.download_failed",
                "cannot install whisper-tiny.en: curl: (6) Could not resolve host",
                true,
            ),
            (
                EngineError::ModelConfirmation {
                    name: "whisper-small".into(),
                    size_bytes: 487_601_967,
                    license: "MIT".into(),
                },
                "fosscut.model.confirmation_required",
                "installing whisper-small downloads 487.6 MB (MIT license) and needs confirmation",
                true,
            ),
            (
                EngineError::ModelUnsupported {
                    name: "onnxruntime".into(),
                    platform: "linux aarch64".into(),
                },
                "fosscut.model.unsupported_platform",
                "onnxruntime has no build for linux aarch64",
                true,
            ),
            (
                EngineError::ModelRuntimeMissing {
                    message: "ONNX Runtime is not installed (looked for /m/onnxruntime/lib)".into(),
                    tried: vec!["/m/onnxruntime/lib".into()],
                },
                "fosscut.model.runtime_missing",
                "ONNX Runtime is not installed (looked for /m/onnxruntime/lib)",
                true,
            ),
            (
                EngineError::ModelLoadFailed {
                    model: "/m/birefnet-lite/model.onnx".into(),
                    message: "protobuf parsing failed".into(),
                },
                "fosscut.model.load_failed",
                "cannot load model /m/birefnet-lite/model.onnx: protobuf parsing failed",
                true,
            ),
            (
                EngineError::SpeechFailed {
                    message: "failed to encode".into(),
                },
                "fosscut.analysis.speech_failed",
                "speech recognition failed: failed to encode",
                true,
            ),
            (
                EngineError::SpeechUnavailable,
                "fosscut.analysis.speech_unavailable",
                "this build of FossCut has no speech recognition",
                true,
            ),
            (
                EngineError::NoTranscript {
                    assets: vec!["asset_0001".into()],
                    message: "no cached transcript for asset_0001".into(),
                },
                "fosscut.captions.no_transcript",
                "no cached transcript for asset_0001",
                true,
            ),
            (
                EngineError::InvalidSrt {
                    line: 7,
                    message: "the cue has no text".into(),
                },
                "fosscut.captions.invalid_srt",
                "line 7: the cue has no text",
                true,
            ),
            (
                EngineError::NoCaptions {
                    timeline: "timeline_0000".into(),
                },
                "fosscut.captions.none",
                "timeline timeline_0000 has no captions",
                true,
            ),
            (
                EngineError::RequestReused {
                    request_id: "r1".into(),
                },
                "fosscut.request.reused",
                "request id r1 was already used for another export",
                false,
            ),
            (
                EngineError::VerificationFailed {
                    mismatches: vec!["frames 239, expected 240".into(), "no audio".into()],
                    report: Box::new(report()),
                },
                "fosscut.export.verification_failed",
                "the exported file does not match the request: frames 239, expected 240; no audio",
                false,
            ),
            (
                EngineError::Failed {
                    code: "fosscut.media.encode_failed".into(),
                    message: "encoder stopped".into(),
                },
                "fosscut.media.encode_failed",
                "encoder stopped",
                false,
            ),
        ];
        for (error, code, message, has_hint) in cases {
            assert_eq!(error.code(), code);
            assert_eq!(error.to_string(), message);
            assert_eq!(error.hint().is_some(), has_hint, "{code}");
        }

        let unavailable = EngineError::CodecUnavailable {
            codec: "h264".into(),
            available: vec!["av1".into(), "prores".into()],
        };
        assert!(unavailable.hint().unwrap().ends_with("av1, prores."));
        assert_eq!(
            unavailable.details(),
            json!({ "codec": "h264", "available": ["av1", "prores"] })
        );
        let missing = EngineError::MissingMedia {
            asset: "asset_0001".into(),
            tried: vec!["clip.mp4".into()],
        };
        assert_eq!(
            missing.details(),
            json!({ "asset": "asset_0001", "tried": ["clip.mp4"] })
        );
        let exists = EngineError::OutputExists {
            path: "Out.mp4".into(),
        };
        assert_eq!(exists.details(), json!({ "path": "Out.mp4" }));
        let missing = EngineError::AnalysisMissing {
            asset: "asset_0002".into(),
            analyzer: "pose".into(),
            options: json!({}),
            effect: "body.glow".into(),
        };
        assert_eq!(
            missing.details(),
            json!({ "asset": "asset_0002", "analyzer": "pose", "options": {}, "effect": "body.glow" })
        );
        assert!(
            missing
                .hint()
                .unwrap()
                .contains("fosscut media analyze asset_0002 pose")
        );
        let conflict = EngineError::RevisionConflict {
            expected: 3,
            current: 5,
        };
        assert_eq!(
            conflict.details(),
            json!({ "expected_revision": 3, "current_revision": 5 })
        );
        let mismatch = EngineError::VerificationFailed {
            mismatches: vec!["no audio".into()],
            report: Box::new(report()),
        };
        assert_eq!(mismatch.details()["mismatches"], json!(["no audio"]));
        assert_eq!(mismatch.details()["report"]["brand"], json!("isom"));
        assert_eq!(EngineError::Cancelled.details(), Value::Null);
        let api = ApiError::from(EngineError::JobNotFound {
            job: "job_0009".into(),
        });
        assert_eq!(api.code, "fosscut.job.not_found");
        assert_eq!(api.details, json!({ "job": "job_0009" }));
        assert!(api.hint.is_some());
        let late = EngineError::TimeOutOfRange {
            time: Flicks::new(1),
            duration: Flicks::new(0),
        };
        assert_eq!(late.details(), json!({ "time": 1, "duration": 0 }));
        let busy = EngineError::OutputBusy {
            path: "Out.mp4".into(),
            job: "job_0001".into(),
        };
        assert_eq!(
            busy.details(),
            json!({ "path": "Out.mp4", "job": "job_0001" })
        );
        let gone = EngineError::JobNotFound {
            job: "job_0009".into(),
        };
        assert_eq!(gone.details(), json!({ "job": "job_0009" }));
    }

    #[test]
    fn every_engine_code_is_catalogued() {
        let mut known: Vec<&str> = fosscut_core::error_catalogue()
            .iter()
            .map(|e| e.code)
            .collect();
        known.extend(engine_error_catalogue().iter().map(|e| e.code));
        let variants = [
            EngineError::TimelineNotFound {
                timeline: String::new(),
            },
            EngineError::EmptyTimeline,
            EngineError::TimeOutOfRange {
                time: Flicks::ZERO,
                duration: Flicks::ZERO,
            },
            EngineError::InvalidArguments {
                message: String::new(),
            },
            EngineError::InvalidSettings {
                message: String::new(),
            },
            EngineError::CodecUnavailable {
                codec: String::new(),
                available: Vec::new(),
            },
            EngineError::RevisionConflict {
                expected: 0,
                current: 1,
            },
            EngineError::OutputExists {
                path: String::new(),
            },
            EngineError::OutputBusy {
                path: String::new(),
                job: String::new(),
            },
            EngineError::JobNotFound { job: String::new() },
            EngineError::MissingMedia {
                asset: String::new(),
                tried: Vec::new(),
            },
            EngineError::Cancelled,
            EngineError::AssetNotFound {
                asset: String::new(),
            },
            EngineError::NoAudio {
                asset: String::new(),
            },
            EngineError::NoVideo {
                asset: String::new(),
            },
            EngineError::UnknownAnalyzer {
                name: String::new(),
                known: Vec::new(),
            },
            EngineError::AnalysisCancelled,
            EngineError::AnalysisMissing {
                asset: String::new(),
                analyzer: String::new(),
                options: Value::Null,
                effect: String::new(),
            },
            EngineError::ModelUnknown {
                name: String::new(),
            },
            EngineError::ModelNotInstalled {
                what: String::new(),
            },
            EngineError::ModelChecksum {
                name: String::new(),
                expected: String::new(),
                actual: String::new(),
            },
            EngineError::ModelDownload {
                name: String::new(),
                message: String::new(),
            },
            EngineError::ModelConfirmation {
                name: String::new(),
                size_bytes: 0,
                license: String::new(),
            },
            EngineError::ModelUnsupported {
                name: String::new(),
                platform: String::new(),
            },
            EngineError::ModelRuntimeMissing {
                message: String::new(),
                tried: Vec::new(),
            },
            EngineError::ModelLoadFailed {
                model: String::new(),
                message: String::new(),
            },
            EngineError::SpeechFailed {
                message: String::new(),
            },
            EngineError::SpeechUnavailable,
            EngineError::NoTranscript {
                assets: Vec::new(),
                message: String::new(),
            },
            EngineError::InvalidSrt {
                line: 0,
                message: String::new(),
            },
            EngineError::NoCaptions {
                timeline: String::new(),
            },
            EngineError::RequestReused {
                request_id: String::new(),
            },
            EngineError::VerificationFailed {
                mismatches: Vec::new(),
                report: Box::new(report()),
            },
            EngineError::NotVideo {
                asset: String::new(),
            },
            EngineError::RemuxNotNeeded {
                asset: String::new(),
                container: String::new(),
            },
            EngineError::CopyMismatch {
                kind: String::new(),
                mismatches: Vec::new(),
            },
            EngineError::JobCancelled {
                kind: String::new(),
            },
            EngineError::NoBeats {
                asset: String::new(),
            },
            EngineError::NoSubject {
                clip: String::new(),
                subject: String::new(),
            },
            EngineError::NoShortCandidates {
                clip: String::new(),
                sentences: 0,
                min_seconds: 15.0,
                max_seconds: 60.0,
            },
        ];
        for error in &variants {
            assert!(
                known.contains(&error.code()),
                "{} is not catalogued",
                error.code()
            );
        }
        let engine = engine_error_catalogue();
        assert!(
            engine.iter().all(|e| !fosscut_core::error_catalogue()
                .iter()
                .any(|core| core.code == e.code)),
            "the engine catalogue only adds codes"
        );
    }

    #[test]
    fn cache_errors_name_what_they_were_making() {
        let mismatch = EngineError::CopyMismatch {
            kind: "proxy".into(),
            mismatches: vec!["frames 3, expected 4".into()],
        };
        assert_eq!(mismatch.code(), "fosscut.cache.verification_failed");
        assert_eq!(
            mismatch.to_string(),
            "the proxy does not match its source: frames 3, expected 4"
        );
        assert_eq!(
            mismatch.details(),
            json!({ "kind": "proxy", "mismatches": ["frames 3, expected 4"] })
        );
        let cancelled = EngineError::JobCancelled {
            kind: "remux".into(),
        };
        assert_eq!(cancelled.to_string(), "the remux was cancelled");
        assert_eq!(cancelled.code(), "fosscut.job.cancelled");
        let not_needed = EngineError::RemuxNotNeeded {
            asset: "asset_0001".into(),
            container: "mov,mp4,m4a,3gp,3g2,mj2".into(),
        };
        assert_eq!(not_needed.code(), "fosscut.cache.remux_not_needed");
        assert!(not_needed.hint().is_some());
        assert_eq!(
            EngineError::NotVideo {
                asset: "asset_0002".into()
            }
            .details(),
            json!({ "asset": "asset_0002" })
        );
    }

    #[test]
    fn lower_layer_errors_keep_their_codes() {
        let unavailable = EngineError::from(fosscut_media::MediaError::EncoderUnavailable {
            codec: "hevc".into(),
            available: vec!["av1".into()],
        });
        assert_eq!(
            unavailable,
            EngineError::CodecUnavailable {
                codec: "hevc".into(),
                available: vec!["av1".into()],
            }
        );
        let unsupported = EngineError::from(fosscut_media::MediaError::Unsupported {
            message: "vp9 in mp4".into(),
        });
        assert_eq!(unsupported.code(), "fosscut.export.invalid_settings");
        let open = fosscut_media::MediaError::OpenFailed {
            path: "clip.mp4".into(),
            message: "no such file".into(),
        };
        let expected = open.code().to_string();
        assert_eq!(EngineError::from(open).code(), expected);

        let render = fosscut_render::RenderError::NoAdapter {
            message: "no GPU".into(),
        };
        let expected = render.code().to_string();
        let converted = EngineError::from(render);
        assert_eq!(converted.code(), expected);
        assert!(converted.to_string().contains("no GPU"));

        let audio = fosscut_audio::AudioError::SourceFailed {
            asset: "asset_0002".into(),
            message: "corrupt".into(),
        };
        let converted = EngineError::from(audio);
        assert_eq!(converted.code(), "fosscut.audio.source_failed");
        assert!(converted.to_string().contains("asset_0002"));
    }

    #[test]
    fn model_runtime_errors_say_how_to_get_the_runtime() {
        use fosscut_analysis::models::ModelError;
        use fosscut_analysis::onnx::OnnxError;
        let missing = OnnxError::RuntimeMissing {
            tried: vec!["/a/lib".into(), "/b/lib".into()],
        };
        let converted = EngineError::from(missing.clone());
        assert_eq!(converted.code(), missing.code());
        assert_eq!(converted.to_string(), missing.to_string());
        assert_eq!(
            converted.details(),
            json!({ "tried": ["/a/lib", "/b/lib"] })
        );
        let hint = converted.hint().unwrap();
        assert!(
            hint.contains("models install onnxruntime")
                && hint.contains("FOSSCUT_ORT_DYLIB")
                && hint.contains("1.22"),
            "{hint}"
        );
        let unusable = EngineError::from(OnnxError::RuntimeUnusable {
            library: "/c/lib".into(),
            message: "expected version >= 1.22".into(),
        });
        assert_eq!(unusable.code(), "fosscut.model.runtime_missing");
        assert_eq!(unusable.details(), json!({ "tried": ["/c/lib"] }));
        assert!(unusable.to_string().contains("cannot be used"));
        let load = EngineError::from(OnnxError::LoadFailed {
            model: "m.onnx".into(),
            message: "bad graph".into(),
        });
        assert_eq!(load.code(), "fosscut.model.load_failed");
        assert_eq!(
            load.details(),
            json!({ "model": "m.onnx", "message": "bad graph" })
        );
        let unsupported = EngineError::from(ModelError::Unsupported {
            name: "onnxruntime".into(),
            platform: "linux aarch64".into(),
        });
        assert_eq!(unsupported.code(), "fosscut.model.unsupported_platform");
        assert_eq!(
            unsupported.details(),
            json!({ "name": "onnxruntime", "platform": "linux aarch64" })
        );
    }

    #[test]
    fn not_installed_hints_list_the_models_that_would_do() {
        let hint = |what: &str| {
            EngineError::ModelNotInstalled { what: what.into() }
                .hint()
                .unwrap()
        };
        let speech = hint("a speech model");
        assert!(speech.contains("whisper-tiny.en (77.7 MB)"), "{speech}");
        assert!(!speech.contains("onnxruntime") && !speech.contains("kokoro"));
        assert!(hint("a multilingual speech model (for de)").contains("whisper-small"));
        let named = hint("birefnet-lite");
        assert!(named.contains("birefnet-lite (224.0 MB)") && !named.contains("whisper"));
        let other = hint("a matte model");
        assert!(other.contains("birefnet-lite") && other.contains("whisper-tiny.en"));
    }
}
