//! Local models as the window uses them (ADR-0033): which models are installed, and
//! installing one in the background once the person agreed to its size and license;
//! analyses for the effects that follow a subject (background removal and body effects);
//! voiceovers from text; and translating and dubbing caption tracks.
//!
//! Models run only in the engine, never in the core. Nothing here writes the project but
//! through the calls the engine proposes, applied as the user's one undoable step against
//! the revision the window shows; a model that is missing is the engine's typed error
//! (`fosscut.model.not_installed`, `fosscut.model.runtime_missing`), which the window turns
//! into its install flow.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fosscut_core::{ApiError, Response, codes};
use fosscut_engine::analysis::{
    AnalysisRequest, Analyzer, MatteModel, ModelInfo, ModelInstall, ModelSource, Options,
};
use fosscut_engine::captions::{TranslateMode, TranslateRequest};
use fosscut_engine::jobs::{JobState, JobStatus, Jobs};
use fosscut_engine::voice::{DubRequest, DubText, SpeakRequest, SpeechFile, VoiceInfo};
use fosscut_model::{AssetKind, ClipContent, Project};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{apply_measured, ensure_revision, given, invalid, open_project, parse_id};
use crate::workspace::Workspace;

type Answer<T> = Result<T, ApiError>;

/// The ONNX Runtime model, which every other model here runs on.
pub const RUNTIME_MODEL: &str = "onnxruntime";

/// The variable that points at an ONNX Runtime library outside the models folder.
const RUNTIME_VARIABLE: &str = "FOSSCUT_ORT_DYLIB";

/// The ONNX Runtime library's file name on this system.
const RUNTIME_LIBRARY: &str = if cfg!(target_os = "macos") {
    "libonnxruntime.dylib"
} else if cfg!(windows) {
    "onnxruntime.dll"
} else {
    "libonnxruntime.so"
};

/// How long starting an analysis waits for it to fail at once (a model that is missing)
/// or to answer from the cache, before answering the running job.
const EARLY: Duration = Duration::from_secs(2);

/// How long cancelling an analysis waits for it to stop.
const CANCEL_WAIT: Duration = Duration::from_secs(2);

/// The models and where they live, as `models_list` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelListing {
    /// The models folder.
    pub dir: String,
    /// Every model with whether it is installed there.
    pub models: Vec<ModelInfo>,
    /// The ONNX Runtime library the models will load, wherever it was found: the
    /// `FOSSCUT_ORT_DYLIB` variable, beside the app, or the installed `onnxruntime` model;
    /// `None` when there is none, and the runtime must be installed first.
    pub runtime: Option<String>,
}

/// Every model in `dir`, with the runtime the engine would load.
pub fn listing(dir: &Path) -> ModelListing {
    ModelListing {
        dir: dir.display().to_string(),
        models: fosscut_engine::analysis::models(dir),
        runtime: runtime_found(dir).map(|path| path.display().to_string()),
    }
}

/// The ONNX Runtime library the engine loads for models in `dir`, looked for where the
/// engine looks (ADR-0033 item 1): the variable, the executable's folder, then the
/// installed `onnxruntime` model.
fn runtime_found(dir: &Path) -> Option<PathBuf> {
    let variable = std::env::var_os(RUNTIME_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(RUNTIME_LIBRARY)));
    let installed = Some(dir.join(RUNTIME_MODEL).join(RUNTIME_LIBRARY));
    [variable, beside, installed]
        .into_iter()
        .flatten()
        .find(|path| path.is_file())
}

// ---- Installing models ----

/// A model install as the window follows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InstallState {
    /// The model.
    pub name: String,
    /// `running`, `succeeded` or `failed`.
    pub state: JobState,
    /// Bytes downloaded so far, from the files being written; the total once done.
    pub done_bytes: u64,
    /// The download's size on this system.
    pub total_bytes: u64,
    /// Time since it started, in milliseconds.
    pub elapsed_ms: u64,
    /// What installing did, once it succeeded: the model, where it is, and `installed` or
    /// `unchanged` (it already was).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install: Option<ModelInstall>,
    /// Why it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

/// One install, running or finished.
struct Install {
    name: String,
    total: u64,
    /// The file or folder the download is written to until it is verified.
    partial: PathBuf,
    started: Instant,
    outcome: Mutex<Option<Result<ModelInstall, ApiError>>>,
}

impl Install {
    fn state(&self) -> InstallState {
        let outcome = guard(&self.outcome).clone();
        let (state, done, install, error) = match outcome {
            None => (
                JobState::Running,
                bytes_in(&self.partial).min(self.total),
                None,
                None,
            ),
            Some(Ok(install)) => (JobState::Succeeded, self.total, Some(install), None),
            Some(Err(error)) => (JobState::Failed, 0, None, Some(error)),
        };
        InstallState {
            name: self.name.clone(),
            state,
            done_bytes: done,
            total_bytes: self.total,
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            install,
            error,
        }
    }

    fn running(&self) -> bool {
        guard(&self.outcome).is_none()
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The bytes in a file, or in every file under a folder; 0 when there is nothing.
fn bytes_in(path: &Path) -> u64 {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .map(|entries| entries.flatten().map(|entry| bytes_in(&entry.path())).sum())
        .unwrap_or(0)
}

/// The model installs of this run of the app, by model name. Each runs on a thread of its
/// own; one model installs once at a time.
#[derive(Default)]
pub struct ModelInstalls {
    installs: Mutex<BTreeMap<String, Arc<Install>>>,
}

impl ModelInstalls {
    /// Starts installing model `name` into `dir` from `source` and answers at once. A
    /// download needs `confirmed`, which the window sets only after the person agreed to
    /// the size and license it showed; without it nothing is fetched and the answer is
    /// `fosscut.model.confirmation_required` with the size and license in its details. An
    /// unknown model, or one with no build for this system, is refused before anything
    /// starts; an installed one answers `succeeded` with status `unchanged`; the same model
    /// while it installs joins that install.
    pub fn start(
        &self,
        dir: &Path,
        name: &str,
        source: ModelSource,
        confirmed: bool,
    ) -> Answer<InstallState> {
        use fosscut_engine::analysis::install_model;
        let planned = install_model(dir, name, &source, false, true)?;
        let total = planned.model.size_bytes;
        if planned.status == "unchanged" {
            return Ok(InstallState {
                name: name.to_string(),
                state: JobState::Succeeded,
                done_bytes: total,
                total_bytes: total,
                elapsed_ms: 0,
                install: Some(planned),
                error: None,
            });
        }
        if matches!(source, ModelSource::Download(_)) && !confirmed {
            // The engine refuses with the size and license, which the window shows.
            install_model(dir, name, &source, false, false)?;
        }
        let mut installs = guard(&self.installs);
        if let Some(running) = installs.get(name).filter(|install| install.running()) {
            return Ok(running.state());
        }
        let install = Arc::new(Install {
            name: name.to_string(),
            total,
            partial: PathBuf::from(format!("{}.partial", planned.path)),
            started: Instant::now(),
            outcome: Mutex::new(None),
        });
        installs.insert(name.to_string(), Arc::clone(&install));
        drop(installs);
        let worker = Arc::clone(&install);
        let (dir, name) = (dir.to_path_buf(), name.to_string());
        std::thread::Builder::new()
            .name("fosscut-model-install".into())
            .spawn(move || {
                let result =
                    install_model(&dir, &name, &source, true, false).map_err(ApiError::from);
                *guard(&worker.outcome) = Some(result);
            })
            .map_err(|e| ApiError::new(codes::INTERNAL, format!("cannot start installing: {e}")))?;
        Ok(install.state())
    }

    /// The install of model `name` started last: its progress, and once it finished, what
    /// it did or why it failed.
    pub fn status(&self, name: &str) -> Answer<InstallState> {
        guard(&self.installs)
            .get(name)
            .map(|install| install.state())
            .ok_or_else(|| invalid(format!("no install of {name} was started")))
    }
}

// ---- Subject analyses ----

/// What `subject_analyze` analyzes.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SubjectRequest {
    /// The clip whose media is analyzed: a video clip for `matte`, `pose` and `scenes`, a
    /// video or audio clip with sound for the analyzers of sound.
    pub clip: String,
    /// `matte` (for background removal) or `pose` (for body effects), or another analyzer
    /// as `fosscut.analysis.missing` names it: `scenes`, `transcript`, `beats`, `levels`,
    /// `loudness` or `silences`.
    pub kind: String,
    /// For a matte: `person` (the default, fast) or `subject` (any subject, slow).
    pub model: Option<String>,
    /// For a matte, run the model on every `every`-th frame and follow the motion between:
    /// 1 for `person` and 5 for `subject` when omitted, since that takes seconds a frame.
    pub every: Option<u32>,
    /// The analyzer's options, as `details.options` of `fosscut.analysis.missing` holds them
    /// under its name (`{"model": "subject"}` for a matte); its defaults when omitted.
    /// `model` and `every` win over the same options given here.
    pub options: Option<Value>,
}

/// The analyzers of moving pictures; the others read sound.
fn reads_pictures(analyzer: Analyzer) -> bool {
    match analyzer {
        Analyzer::Scenes | Analyzer::Pose | Analyzer::Matte => true,
        Analyzer::Loudness
        | Analyzer::Silences
        | Analyzer::Transcript
        | Analyzer::Beats
        | Analyzer::Levels => false,
    }
}

/// `given` as the options of `analyzer`, keyed by its name as the engine keeps them.
fn analyzer_options(analyzer: Analyzer, given: Option<&Value>) -> Answer<Options> {
    let name = analyzer.name();
    let Some(given) = given.filter(|options| !options.is_null()) else {
        return Ok(Options::default());
    };
    let Some(fields) = given.as_object() else {
        return Err(invalid(format!(
            "options of {name}: give an object of its options"
        )));
    };
    if analyzer == Analyzer::Loudness {
        return if fields.is_empty() {
            Ok(Options::default())
        } else {
            Err(invalid("loudness takes no options"))
        };
    }
    serde_json::from_value(json!({ name: given }))
        .map_err(|e| invalid(format!("options of {name}: {e}")))
}

/// The asset of `clip` an analysis reads: a video asset for one of moving pictures, any
/// asset with sound (a video or audio clip's) for one of sound.
fn analyzed_asset(
    project: &Project,
    clip: &fosscut_model::Clip,
    analyzer: Analyzer,
) -> Answer<fosscut_model::Id> {
    let id = &clip.id;
    if reads_pictures(analyzer) {
        return match &clip.content {
            ClipContent::Video { asset, .. }
                if project
                    .media
                    .get(asset)
                    .is_some_and(|a| a.kind == AssetKind::Video) =>
            {
                Ok(asset.clone())
            }
            _ => {
                let what = if analyzer == Analyzer::Scenes {
                    "shot cuts"
                } else {
                    "its subject"
                };
                Err(invalid(format!(
                    "{id} is not a video clip; only video is analyzed for {what}"
                ))
                .with_hint("Select a video clip."))
            }
        };
    }
    match &clip.content {
        ClipContent::Video { asset, .. } | ClipContent::Audio { asset, .. } => {
            if project.media.get(asset).is_some_and(|a| a.has_audio()) {
                Ok(asset.clone())
            } else {
                Err(ApiError::from(fosscut_engine::EngineError::NoAudio {
                    asset: asset.to_string(),
                }))
            }
        }
        other => Err(invalid(format!(
            "{id} is a {} clip; {} reads the sound of video and audio clips",
            other.kind_name(),
            analyzer.name()
        ))
        .with_hint("Select a video or audio clip.")),
    }
}

/// The analysis `request` asks for, of the project's clip it names.
fn subject_analysis(
    project: &Project,
    bundle: &Path,
    request: &SubjectRequest,
    cache: &Path,
    models: &Path,
) -> Answer<AnalysisRequest> {
    let clip = parse_id(&request.clip, "clip")?;
    let found = project
        .timelines
        .values()
        .flat_map(|timeline| timeline.tracks.iter())
        .flat_map(|track| track.clips.iter())
        .find(|candidate| candidate.id == clip)
        .ok_or_else(|| ApiError::new(codes::CLIP_NOT_FOUND, format!("there is no clip {clip}")))?;
    let analyzer = Analyzer::parse(request.kind.trim()).ok_or_else(|| {
        invalid(format!(
            "kind {:?}: analyze a matte or a pose, or scenes, transcript, beats, levels, loudness or silences",
            request.kind
        ))
    })?;
    let asset = analyzed_asset(project, found, analyzer)?;
    let mut options = analyzer_options(analyzer, request.options.as_ref())?;
    let model = given(request.model.as_deref());
    match analyzer {
        Analyzer::Matte => {
            let model = match model {
                None => options.matte.model,
                Some("person") => MatteModel::Person,
                Some("subject") => MatteModel::Subject,
                Some(other) => {
                    return Err(invalid(format!(
                        "model {other:?}: a matte is of a person or any subject"
                    )));
                }
            };
            let every_given = request
                .options
                .as_ref()
                .is_some_and(|options| options.get("every").is_some());
            options.matte.model = model;
            options.matte.every = match request.every {
                Some(every) => every,
                None if every_given => options.matte.every,
                None => match model {
                    MatteModel::Person => 1,
                    MatteModel::Subject => 5,
                },
            };
        }
        other if model.is_some() => {
            return Err(invalid(format!(
                "a {} analysis takes no model",
                other.name()
            )));
        }
        _ => {}
    }
    Ok(AnalysisRequest {
        bundle: bundle.to_path_buf(),
        asset,
        analyzers: vec![analyzer],
        options,
        refresh: false,
        cache: cache.to_path_buf(),
        models: models.to_path_buf(),
    })
}

/// A job's error as the window shows a refusal.
fn job_error(status: &JobStatus) -> ApiError {
    match &status.error {
        Some(error) => {
            let mut api = ApiError::new(&error.code, error.message.clone())
                .with_details(error.details.clone());
            if let Some(hint) = &error.hint {
                api = api.with_hint(hint.clone());
            }
            api
        }
        None => ApiError::new(codes::INTERNAL, format!("{} failed", status.job)),
    }
}

/// Analyzes the media of a video clip of the project open in `workspace` for the effects
/// that follow a subject, as a job of `jobs`, caching the result in `cache`. Answers the job
/// once it finished, when it answers from the cache or fails at once (a model missing from
/// `models` fails with `fosscut.model.not_installed`, naming it), or while it runs.
pub fn analyze_subject(
    workspace: &Workspace,
    jobs: &Jobs,
    cache: &Path,
    models: &Path,
    request: &SubjectRequest,
) -> Answer<JobStatus> {
    let (bundle, project, _) = open_project(workspace)?;
    let analysis = subject_analysis(&project, &bundle, request, cache, models)?;
    let started = jobs.start_analysis(project, analysis);
    let status = jobs.status(&started.job, EARLY)?;
    if status.state == JobState::Failed {
        return Err(job_error(&status));
    }
    Ok(status)
}

/// An analysis job's status, or the job stopped.
pub fn subject_status(jobs: &Jobs, job: &str, cancel: bool) -> Answer<JobStatus> {
    if cancel {
        Ok(jobs.cancel(job, CANCEL_WAIT)?)
    } else {
        Ok(jobs.status(job, Duration::ZERO)?)
    }
}

// ---- Text to speech ----

/// The voices of the voice model, with whether each is installed in `models`.
pub fn voices(models: &Path) -> Vec<VoiceInfo> {
    fosscut_engine::voice::voices(models)
}

/// What `speech_add` says and where.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpeechAdd {
    /// What to say, in English.
    pub text: String,
    /// The voice (`voices_list`); the engine's default when omitted.
    pub voice: Option<String>,
    /// From 0.5 to 2; 1 when omitted.
    pub speed: Option<f32>,
    /// Where the speech starts on the primary timeline, in flicks: on the first audio track
    /// free there. After the first audio track's last clip when omitted.
    pub at: Option<i64>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
}

/// A report of the engine without the calls, which the response shows applied.
fn without_calls(report: impl Serialize) -> Value {
    let mut value = json!(report);
    if let Some(object) = value.as_object_mut() {
        object.remove("calls");
    }
    value
}

/// Where previews of speech are written: a folder of their own in the system's temporary
/// folder, holding the last one only.
pub fn preview_folder() -> PathBuf {
    std::env::temp_dir().join("fosscut-speech-preview")
}

/// Copies the preview of the same speech into the bundle's speech folder, where the engine
/// then finds it instead of synthesizing it again; nothing when there is none.
fn reuse_preview(bundle: &Path, previews: &Path, text: &str, voice: &str, speed: Option<f32>) {
    let Ok(speed) = fosscut_engine::voice::speed(speed) else {
        return;
    };
    let Ok(name) = fosscut_engine::voice::speech_name(text.trim(), voice, speed) else {
        return;
    };
    let from = previews.join(fosscut_engine::voice::FOLDER).join(&name);
    let to = bundle.join(fosscut_engine::voice::FOLDER).join(&name);
    let folder_made = to
        .parent()
        .is_some_and(|folder| std::fs::create_dir_all(folder).is_ok());
    if from.is_file() && !to.exists() && folder_made {
        // Copied beside it and renamed, so the engine never finds half a file.
        let partial = to.with_extension("wav.part");
        if std::fs::copy(&from, &partial).is_ok() && std::fs::rename(&partial, &to).is_err() {
            let _ = std::fs::remove_file(&partial);
        }
    }
}

/// Speaks `request.text` and places it on the primary timeline of the project open in
/// `workspace` as the user's one undoable step: the speech file is written into the
/// bundle's `speech/` folder (or found there, or copied from a preview in `previews`),
/// imported and placed. Answers `{speech: {revision, asset, file}, response}`.
pub fn add_speech(
    workspace: &Workspace,
    models: &Path,
    previews: &Path,
    request: &SpeechAdd,
) -> Answer<Value> {
    let (bundle, project, timeline) = open_project(workspace)?;
    ensure_revision(&project, request.expected_revision)?;
    if let Some(voice) = given(request.voice.as_deref()) {
        reuse_preview(&bundle, previews, &request.text, voice, request.speed);
    }
    let spoken = fosscut_engine::voice::speak(
        &project,
        &SpeakRequest {
            bundle,
            models: models.to_path_buf(),
            text: request.text.clone(),
            voice: given(request.voice.as_deref()).map(str::to_string),
            speed: request.speed,
            language: None,
            timeline: Some(timeline),
            track: None,
            start: request.at.map(|flicks| json!({ "flicks": flicks })),
        },
    )?;
    let calls = spoken.calls.clone();
    let response = apply_measured(workspace, calls, Some(spoken.revision), "Text to speech")?;
    Ok(json!({ "speech": without_calls(&spoken), "response": response }))
}

/// A preview of speech, synthesized but not added.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpeechPreview {
    /// The speech file in the preview folder.
    pub file: SpeechFile,
    /// Its samples, mono, at `rate`.
    #[serde(skip)]
    pub samples: Vec<f32>,
    /// Samples per second.
    #[serde(skip)]
    pub rate: u32,
}

/// Synthesizes `text` with `voice` at `speed` into `previews` (or finds it there), leaving
/// no other preview there, and reads its samples to play.
pub fn preview_speech(
    models: &Path,
    previews: &Path,
    text: &str,
    voice: &str,
    speed: Option<f32>,
) -> Answer<SpeechPreview> {
    let speed = fosscut_engine::voice::speed(speed)?;
    let file = fosscut_engine::voice::speech_file(&mut None, previews, models, text, voice, speed)?;
    let path = previews.join(&file.relative);
    // Only the newest preview is kept.
    if let Ok(entries) = std::fs::read_dir(previews.join(fosscut_engine::voice::FOLDER)) {
        for entry in entries.flatten() {
            if entry.path() != path {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let (samples, rate) = read_speech(&path).ok_or_else(|| {
        ApiError::new(
            "fosscut.voice.write_failed",
            format!("cannot read {}", path.display()),
        )
    })?;
    Ok(SpeechPreview {
        file,
        samples,
        rate,
    })
}

/// The samples of a 16-bit mono WAV file as the engine writes speech, and its rate.
pub fn read_speech(path: &Path) -> Option<(Vec<f32>, u32)> {
    let bytes = std::fs::read(path).ok()?;
    let header = bytes.get(..44)?;
    let u16_at = |i: usize| u16::from_le_bytes([header[i], header[i + 1]]);
    let u32_at =
        |i: usize| u32::from_le_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
    let fits = &header[..4] == b"RIFF"
        && &header[8..16] == b"WAVEfmt "
        && u16_at(20) == 1
        && u16_at(22) == 1
        && u16_at(34) == 16
        && &header[36..40] == b"data";
    if !fits {
        return None;
    }
    let data = bytes.get(44..)?;
    let samples = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32_768.0)
        .collect();
    Some((samples, u32_at(24)))
}

// ---- Captions ----

/// What `captions_translate` translates.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CaptionsTranslate {
    /// The caption track; the primary timeline's first when omitted.
    pub track: Option<String>,
    /// The language translated into: `hu` (from English) or `en` (from Hungarian).
    pub to: String,
    /// `bilingual` (the default: into each page's translation, shown under its words) or
    /// `replace` (instead of its words).
    pub mode: Option<String>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
}

/// The direction of a translation into `to`.
fn pair_into(to: &str) -> Answer<&'static str> {
    match to {
        "hu" => Ok("en>hu"),
        "en" => Ok("hu>en"),
        other => Err(
            invalid(format!("captions translate into hu or en, not {other:?}")).with_hint(
                "Translate English captions into Hungarian, or Hungarian ones into English.",
            ),
        ),
    }
}

/// Translates a caption track of the project open in `workspace` with the installed
/// translation model in `models`, as the user's one undoable step. Answers
/// `{translation: {revision, timeline, track, pair, mode, model, attribution, pages,
/// skipped}, response}`.
pub fn translate_captions(
    workspace: &Workspace,
    models: &Path,
    request: &CaptionsTranslate,
) -> Answer<Value> {
    let pair = pair_into(request.to.trim())?;
    let mode = TranslateMode::named(given(request.mode.as_deref()).unwrap_or("bilingual"))?;
    let track = given(request.track.as_deref())
        .map(|track| parse_id(track, "track"))
        .transpose()?;
    let (_, project, timeline) = open_project(workspace)?;
    ensure_revision(&project, request.expected_revision)?;
    let found = fosscut_engine::captions::translate(
        &project,
        &TranslateRequest {
            models: models.to_path_buf(),
            timeline: Some(timeline),
            track,
            pair: pair.into(),
            mode,
        },
    )?;
    let calls = found.calls.clone();
    let response = apply_measured(workspace, calls, Some(found.revision), "Translate captions")?;
    Ok(json!({ "translation": without_calls(&found), "response": response }))
}

/// What `captions_dub` speaks.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CaptionsDub {
    /// The caption track; the primary timeline's first when omitted.
    pub track: Option<String>,
    /// The voice (`voices_list`); the engine's default when omitted.
    pub voice: Option<String>,
    /// From 0.5 to 2; 1 when omitted.
    pub speed: Option<f32>,
    /// Speak each page's translation instead of its words.
    pub translation: bool,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
}

/// Dubs a caption track of the project open in `workspace` with text to speech, one clip
/// per page at the page's start, as the user's one undoable step. Answers `{dub: {revision,
/// timeline, track, voice, pages, skipped}, response}`, the response null when no page had
/// anything to say.
pub fn dub_captions(workspace: &Workspace, models: &Path, request: &CaptionsDub) -> Answer<Value> {
    let track = given(request.track.as_deref())
        .map(|track| parse_id(track, "track"))
        .transpose()?;
    let (bundle, project, timeline) = open_project(workspace)?;
    ensure_revision(&project, request.expected_revision)?;
    let found = fosscut_engine::voice::dub(
        &project,
        &DubRequest {
            bundle,
            models: models.to_path_buf(),
            timeline: Some(timeline),
            track,
            voice: given(request.voice.as_deref()).map(str::to_string),
            speed: request.speed,
            language: None,
            text: if request.translation {
                DubText::Translation
            } else {
                DubText::Words
            },
            audio_track: None,
        },
    )?;
    let calls = found.calls.clone();
    let response: Option<Response> =
        apply_measured(workspace, calls, Some(found.revision), "Dub captions")?;
    Ok(json!({ "dub": without_calls(&found), "response": response }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::workspace::NewProject;
    use fosscut_core::Request;

    const SECOND: i64 = fosscut_time::FLICKS_PER_SECOND;

    /// A folder of its own for a test, empty.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fcd-models-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A workspace with a new 1280 by 720 project open, a short runtime directory (Unix
    /// socket paths are limited to about 100 bytes), and the folder it is in.
    fn opened(name: &str) -> (Workspace, PathBuf, PathBuf) {
        let base = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let runtime = base.join(format!("fcd-m{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        let work = folder(name);
        let workspace = Workspace::new(runtime.clone(), None);
        workspace
            .create(&NewProject {
                path: work.join("Models.fosscut"),
                name: None,
                width: 1280,
                height: 720,
                rate: "30".into(),
            })
            .unwrap();
        (workspace, runtime, work)
    }

    fn done(workspace: Workspace, runtime: PathBuf, work: PathBuf) {
        workspace.close();
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
    }

    fn revision(workspace: &Workspace) -> u64 {
        workspace.query("project.info", Value::Null).unwrap()["revision"]
            .as_u64()
            .unwrap()
    }

    fn last_edit(workspace: &Workspace) -> Value {
        workspace
            .query("history.list", json!({ "limit": 1 }))
            .unwrap()["entries"][0]
            .clone()
    }

    /// Imports a video that is nowhere on disk, with its identity recorded, places it at
    /// the start and returns the clip.
    fn video_clip(workspace: &Workspace) -> String {
        let added = workspace
            .execute(Request::single(
                "media.add",
                json!({
                    "name": "talk.mp4",
                    "kind": "video",
                    "path": { "absolute": "/nowhere/talk.mp4" },
                    "identity": { "size": 1234, "sample_blake3": "cd".repeat(32) },
                    "info": {
                        "duration": 4 * SECOND,
                        "container": "mov,mp4,m4a,3gp,3g2,mj2",
                        "video": { "width": 320, "height": 180, "rate": "30", "codec": "h264" },
                    }
                }),
            ))
            .unwrap();
        let asset = added.results[0]["asset"].as_str().unwrap().to_string();
        let placed = workspace
            .execute(Request::single("clip.add", json!({ "asset": asset })))
            .unwrap();
        placed.results[0]["clip"].as_str().unwrap().to_string()
    }

    /// Adds a caption track of one page per entry of `pages`, each word a third of a
    /// second, and returns the track.
    fn captions(workspace: &Workspace, pages: &[&str]) -> String {
        let mut words = Vec::new();
        let mut at = 0;
        for page in pages {
            for (index, word) in page.split_whitespace().enumerate() {
                words.push(json!({
                    "text": word,
                    "start": { "flicks": at },
                    "end": { "flicks": at + SECOND / 3 },
                    "new_page": index == 0,
                }));
                at += SECOND / 3;
            }
            at += SECOND / 2;
        }
        let added = workspace
            .execute(Request::single(
                "captions.add",
                json!({
                    "words": words,
                    "paging": {
                        "max_words": 100, "max_lines": 10, "max_chars_per_line": 200,
                        "max_gap_ms": 100000, "linger_ms": 0,
                    },
                }),
            ))
            .unwrap();
        added.results[0]["track"].as_str().unwrap().to_string()
    }

    fn required() -> bool {
        std::env::var_os("FOSSCUT_REQUIRE_MODELS").is_some_and(|v| !v.is_empty() && v != "0")
    }

    /// `FOSSCUT_MODELS_DIR` when ONNX Runtime and every one of `names` are installed there;
    /// `None` (the test skips) otherwise, unless `FOSSCUT_REQUIRE_MODELS` asks for them.
    fn installed(names: &[&str]) -> Option<PathBuf> {
        let dir = std::env::var_os("FOSSCUT_MODELS_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let ready = dir.as_deref().is_some_and(|dir| {
            let models = fosscut_engine::analysis::models(dir);
            runtime_found(dir).is_some()
                && names
                    .iter()
                    .all(|name| models.iter().any(|m| m.name == *name && m.installed))
        });
        assert!(
            ready || !required(),
            "FOSSCUT_REQUIRE_MODELS is set but ONNX Runtime or {names:?} is not installed in FOSSCUT_MODELS_DIR"
        );
        if !ready {
            eprintln!("skipped: ONNX Runtime or {names:?} is not installed in FOSSCUT_MODELS_DIR");
        }
        ready.then(|| dir.unwrap())
    }

    /// Waits for an install to finish, for at most a minute.
    fn finished(installs: &ModelInstalls, name: &str) -> InstallState {
        let started = Instant::now();
        loop {
            let state = installs.status(name).unwrap();
            if state.state != JobState::Running || started.elapsed() > Duration::from_secs(60) {
                return state;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn the_listing_says_what_is_installed_and_where_the_runtime_is() {
        let dir = folder("listing");
        let listed = listing(&dir);
        assert_eq!(listed.dir, dir.display().to_string());
        for task in ["runtime", "matte", "pose", "voice", "translate", "speech"] {
            assert!(
                listed.models.iter().any(|m| m.task == task),
                "a {task} model is listed"
            );
        }
        assert!(listed.models.iter().all(|m| !m.installed), "{listed:?}");
        if std::env::var_os(RUNTIME_VARIABLE).is_none() {
            assert_eq!(listed.runtime, None, "no runtime in an empty folder");
        }
        // A runtime library in the models folder is the one the engine loads.
        let library = dir.join(RUNTIME_MODEL).join(RUNTIME_LIBRARY);
        std::fs::create_dir_all(library.parent().unwrap()).unwrap();
        std::fs::write(&library, b"not really a library").unwrap();
        if std::env::var_os(RUNTIME_VARIABLE).is_none() {
            assert_eq!(listing(&dir).runtime, Some(library.display().to_string()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installs_are_refused_without_consent_or_a_known_model() {
        let dir = folder("consent");
        let installs = ModelInstalls::default();
        let unknown = installs
            .start(&dir, "no-such-model", ModelSource::Download(None), true)
            .unwrap_err();
        assert_eq!(unknown.code, "fosscut.model.unknown");
        let unasked = installs
            .start(
                &dir,
                "mediapipe-pose-full",
                ModelSource::Download(None),
                false,
            )
            .unwrap_err();
        assert_eq!(unasked.code, "fosscut.model.confirmation_required");
        assert_eq!(unasked.details["name"], "mediapipe-pose-full");
        assert_eq!(unasked.details["license"], "Apache-2.0");
        assert!(unasked.details["size_bytes"].as_u64().unwrap() > 1_000_000);
        let never = installs.status("mediapipe-pose-full").unwrap_err();
        assert_eq!(never.code, codes::INVALID_ARGUMENTS, "nothing started");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "nothing was fetched"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_model_installs_in_the_background_and_then_answers_unchanged() {
        let Some(models) = installed(&["mediapipe-pose-full"]) else {
            return;
        };
        let dir = folder("install");
        let installs = ModelInstalls::default();
        // Local files need no consent; a download would.
        let from = ModelSource::File(models.join("mediapipe-pose-full"));
        let started = installs
            .start(&dir, "mediapipe-pose-full", from.clone(), false)
            .unwrap();
        assert!(started.total_bytes > 20_000_000, "{started:?}");
        let state = finished(&installs, "mediapipe-pose-full");
        assert_eq!(state.state, JobState::Succeeded, "{state:?}");
        assert_eq!(state.done_bytes, state.total_bytes);
        assert_eq!(state.install.as_ref().unwrap().status, "installed");
        let model = listing(&dir)
            .models
            .into_iter()
            .find(|m| m.name == "mediapipe-pose-full")
            .unwrap();
        assert!(model.installed);
        let again = installs
            .start(
                &dir,
                "mediapipe-pose-full",
                ModelSource::Download(None),
                false,
            )
            .unwrap();
        assert_eq!(again.state, JobState::Succeeded, "installed already");
        assert_eq!(again.install.unwrap().status, "unchanged");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subject_analyses_refuse_what_they_cannot_analyze() {
        let (workspace, runtime, work) = opened("subject");
        let (models, cache) = (folder("subject-models"), folder("subject-cache"));
        let jobs = Jobs::new();
        let ask = |clip: &str, kind: &str, model: Option<&str>| SubjectRequest {
            clip: clip.into(),
            kind: kind.into(),
            model: model.map(str::to_string),
            every: None,
            options: None,
        };
        let clip = video_clip(&workspace);
        let title = workspace
            .execute(Request::single(
                "text.add",
                json!({ "text": "Hello", "duration": "1s" }),
            ))
            .unwrap()
            .results[0]["clip"]
            .as_str()
            .unwrap()
            .to_string();

        let analyze = |request: &SubjectRequest| {
            analyze_subject(&workspace, &jobs, &cache, &models, request).unwrap_err()
        };
        let text = analyze(&ask(&title, "matte", None));
        assert_eq!(text.code, codes::INVALID_ARGUMENTS);
        assert!(text.message.contains("not a video clip"), "{text:?}");
        let gone = analyze(&ask("clip_0999", "pose", None));
        assert_eq!(gone.code, codes::CLIP_NOT_FOUND);
        let kind = analyze(&ask(&clip, "depth", None));
        assert!(kind.message.contains("matte or a pose"), "{kind:?}");
        let model = analyze(&ask(&clip, "matte", Some("cat")));
        assert!(model.message.contains("person or any subject"), "{model:?}");
        let posed = analyze(&ask(&clip, "pose", Some("subject")));
        assert!(posed.message.contains("no model"), "{posed:?}");
        assert!(
            jobs.list().is_empty(),
            "nothing started before these checks"
        );

        // Without the models, the analysis fails at once, naming what to install.
        let person = analyze(&ask(&clip, "matte", None));
        assert_eq!(person.code, "fosscut.model.not_installed", "{person:?}");
        assert!(
            person.details["what"]
                .as_str()
                .unwrap()
                .contains("mediapipe-pose-full"),
            "{person:?}"
        );
        let subject = analyze(&ask(&clip, "matte", Some("subject")));
        assert!(
            subject.details["what"]
                .as_str()
                .unwrap()
                .contains("birefnet-lite"),
            "{subject:?}"
        );
        let pose = analyze(&ask(&clip, "pose", None));
        assert_eq!(pose.code, "fosscut.model.not_installed");
        let failed = jobs.list();
        assert_eq!(failed.len(), 3, "each ran as a job");
        let first = subject_status(&jobs, &failed[0].job, false).unwrap();
        assert_eq!(first.state, JobState::Failed);
        assert_eq!(first.kind, "analysis");
        let unknown = subject_status(&jobs, "job_0999", false).unwrap_err();
        assert_eq!(unknown.code, "fosscut.job.not_found");
        assert_eq!(revision(&workspace), 3, "analyses write nothing");
        done(workspace, runtime, work);
        let _ = std::fs::remove_dir_all(&models);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn speech_and_captions_refuse_without_their_models_and_change_nothing() {
        let (workspace, runtime, work) = opened("refuse");
        let models = folder("refuse-models");
        let previews = folder("refuse-previews");
        let speech = |text: &str, speed: Option<f32>, expected: Option<u64>| SpeechAdd {
            text: text.into(),
            voice: Some("af_heart".into()),
            speed,
            at: Some(SECOND),
            expected_revision: expected,
        };
        let missing = |error: &ApiError| {
            matches!(
                error.code.as_str(),
                "fosscut.model.not_installed" | "fosscut.model.runtime_missing"
            )
        };
        let none = add_speech(
            &workspace,
            &models,
            &previews,
            &speech("Hello there", None, None),
        )
        .unwrap_err();
        assert!(missing(&none), "{none:?}");
        let empty =
            add_speech(&workspace, &models, &previews, &speech("  ", None, None)).unwrap_err();
        assert_eq!(empty.code, codes::INVALID_ARGUMENTS, "{empty:?}");
        let fast = add_speech(
            &workspace,
            &models,
            &previews,
            &speech("Hi", Some(3.0), None),
        )
        .unwrap_err();
        assert_eq!(fast.code, codes::INVALID_ARGUMENTS, "{fast:?}");
        let stale =
            add_speech(&workspace, &models, &previews, &speech("Hi", None, Some(9))).unwrap_err();
        assert_eq!(stale.code, codes::REVISION_CONFLICT);
        let voice = SpeechAdd {
            voice: Some("zz_nobody".into()),
            ..speech("Hi", None, None)
        };
        let unknown = add_speech(&workspace, &models, &previews, &voice).unwrap_err();
        assert_eq!(unknown.code, "fosscut.voice.unknown_voice", "{unknown:?}");
        let preview = preview_speech(&models, &previews, "Hello", "af_heart", None).unwrap_err();
        assert!(missing(&preview), "{preview:?}");

        let translate = |to: &str, mode: Option<&str>| CaptionsTranslate {
            track: None,
            to: to.into(),
            mode: mode.map(str::to_string),
            expected_revision: None,
        };
        let french = translate_captions(&workspace, &models, &translate("fr", None)).unwrap_err();
        assert!(french.message.contains("hu or en"), "{french:?}");
        let mode =
            translate_captions(&workspace, &models, &translate("hu", Some("both"))).unwrap_err();
        assert_eq!(mode.code, codes::INVALID_ARGUMENTS, "{mode:?}");
        let track = captions(&workspace, &["Good morning"]);
        let before = revision(&workspace);
        let untranslated =
            translate_captions(&workspace, &models, &translate("hu", None)).unwrap_err();
        assert!(missing(&untranslated), "{untranslated:?}");
        let dub = CaptionsDub {
            track: Some(track),
            voice: Some("af_heart".into()),
            speed: Some(1.0),
            translation: false,
            expected_revision: Some(before),
        };
        let undubbed = dub_captions(&workspace, &models, &dub).unwrap_err();
        assert!(missing(&undubbed), "{undubbed:?}");
        let stale = CaptionsDub {
            expected_revision: Some(before + 5),
            ..dub
        };
        assert_eq!(
            dub_captions(&workspace, &models, &stale).unwrap_err().code,
            codes::REVISION_CONFLICT
        );
        assert_eq!(revision(&workspace), before, "nothing was written");
        done(workspace, runtime, work);
        let _ = std::fs::remove_dir_all(&models);
        let _ = std::fs::remove_dir_all(&previews);
    }

    /// The audio clips of the primary timeline: (asset, start in flicks).
    fn audio_clips(workspace: &Workspace) -> Vec<(String, i64)> {
        let timeline = workspace
            .query("timeline.read", json!({ "detail": "full" }))
            .unwrap();
        timeline["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["kind"] == "audio")
            .flat_map(|t| t["clips"].as_array().unwrap().clone())
            .map(|c| {
                (
                    c["asset"].as_str().unwrap().to_string(),
                    c["start"]["flicks"].as_i64().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn speech_is_previewed_then_inserted_at_the_playhead_as_one_step() {
        let Some(models) = installed(&["kokoro-82m"]) else {
            return;
        };
        let (workspace, runtime, work) = opened("speak");
        let previews = folder("speak-previews");
        let preview = preview_speech(
            &models,
            &previews,
            "Welcome to FossCut.",
            "af_heart",
            Some(1.0),
        )
        .unwrap();
        assert!(!preview.file.reused);
        assert_eq!(preview.rate, 24_000);
        assert!(preview.samples.len() > 12_000, "at least half a second");
        assert!(
            preview.samples.iter().any(|s| s.abs() > 0.05),
            "it says something"
        );
        let other =
            preview_speech(&models, &previews, "Something else.", "af_heart", None).unwrap();
        let kept: Vec<_> = std::fs::read_dir(previews.join(fosscut_engine::voice::FOLDER))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(kept.len(), 1, "only the newest preview is kept");
        assert!(other.file.relative.ends_with(".wav"));
        let again = preview_speech(
            &models,
            &previews,
            "Welcome to FossCut.",
            "af_heart",
            Some(1.0),
        )
        .unwrap();

        let before = revision(&workspace);
        let added = add_speech(
            &workspace,
            &models,
            &previews,
            &SpeechAdd {
                text: "Welcome to FossCut.".into(),
                voice: Some("af_heart".into()),
                speed: Some(1.0),
                at: Some(2 * SECOND),
                expected_revision: Some(before),
            },
        )
        .unwrap();
        assert_eq!(
            added["speech"]["file"]["reused"], true,
            "the preview was used"
        );
        assert_eq!(
            added["speech"]["file"]["relative"],
            json!(again.file.relative)
        );
        assert!(added["speech"].get("calls").is_none());
        let response = &added["response"];
        assert_eq!(response["committed"], true);
        assert_eq!(revision(&workspace), before + 1, "one step");
        let asset = added["speech"]["asset"].as_str().unwrap().to_string();
        assert_eq!(audio_clips(&workspace), vec![(asset, 2 * SECOND)]);
        let edit = last_edit(&workspace);
        assert_eq!(edit["actor"], "human");
        assert_eq!(edit["commands"], json!(["media.add", "clip.add"]));
        let (bundle, _, _) = workspace.snapshot().unwrap();
        assert!(
            bundle.join(&again.file.relative).is_file(),
            "written into the bundle"
        );
        workspace
            .execute(Request::single("history.undo", Value::Null))
            .unwrap();
        assert!(audio_clips(&workspace).is_empty(), "undone in one step");
        done(workspace, runtime, work);
        let _ = std::fs::remove_dir_all(&previews);
    }

    /// The caption pages of the primary timeline: (words, translation).
    fn pages(workspace: &Workspace) -> Vec<(String, Option<String>)> {
        let (_, project, timeline) = workspace.snapshot().unwrap();
        project.timelines[&timeline]
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match &c.content {
                ClipContent::Caption(caption) => {
                    Some((caption.text(), caption.translation.clone()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_caption_track_is_translated_into_hungarian_as_one_step() {
        let Some(models) = installed(&["opus-mt-en-hu"]) else {
            return;
        };
        let (workspace, runtime, work) = opened("translate");
        let track = captions(&workspace, &["Good morning!", "Thank you very much."]);
        let before = revision(&workspace);
        let translated = translate_captions(
            &workspace,
            &models,
            &CaptionsTranslate {
                track: Some(track.clone()),
                to: "hu".into(),
                mode: None,
                expected_revision: Some(before),
            },
        )
        .unwrap();
        let report = &translated["translation"];
        assert_eq!(report["pair"], "en>hu");
        assert_eq!(report["mode"], "bilingual");
        assert_eq!(report["track"], json!(track));
        assert_eq!(report["model"], "opus-mt-en-hu");
        assert!(report["attribution"].as_str().unwrap().contains("OPUS-MT"));
        assert!(report.get("calls").is_none());
        assert_eq!(translated["response"]["committed"], true);
        assert_eq!(revision(&workspace), before + 1, "one step");
        let shown = pages(&workspace);
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[0].0, "Good morning!", "the words stay");
        let first = shown[0].1.clone().unwrap().to_lowercase();
        assert!(
            first.contains("reggel"),
            "Good morning in Hungarian: {first}"
        );
        let second = shown[1].1.clone().unwrap().to_lowercase();
        assert!(
            second.contains("köszönöm"),
            "Thank you in Hungarian: {second}"
        );
        assert_eq!(last_edit(&workspace)["actor"], "human");
        workspace
            .execute(Request::single("history.undo", Value::Null))
            .unwrap();
        assert!(pages(&workspace).iter().all(|(_, t)| t.is_none()), "undone");
        done(workspace, runtime, work);
    }

    #[test]
    fn a_caption_track_is_dubbed_one_clip_per_page() {
        let Some(models) = installed(&["kokoro-82m"]) else {
            return;
        };
        let (workspace, runtime, work) = opened("dub");
        captions(&workspace, &["Hello there", "See you soon"]);
        let before = revision(&workspace);
        let dubbed = dub_captions(
            &workspace,
            &models,
            &CaptionsDub {
                track: None,
                voice: Some("am_adam".into()),
                speed: Some(1.0),
                translation: false,
                expected_revision: Some(before),
            },
        )
        .unwrap();
        let pages = dubbed["dub"]["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(dubbed["dub"]["voice"], "am_adam");
        assert_eq!(revision(&workspace), before + 1, "one step");
        let starts: Vec<i64> = audio_clips(&workspace).iter().map(|(_, s)| *s).collect();
        let expected: Vec<i64> = pages.iter().map(|p| p["start"].as_i64().unwrap()).collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, expected, "each page's speech starts with the page");
        // Without translations, dubbing them has nothing to say and writes nothing.
        let nothing = dub_captions(
            &workspace,
            &models,
            &CaptionsDub {
                translation: true,
                ..CaptionsDub::default()
            },
        )
        .unwrap();
        assert!(nothing["response"].is_null());
        assert_eq!(nothing["dub"]["skipped"].as_array().unwrap().len(), 2);
        assert_eq!(revision(&workspace), before + 1);
        done(workspace, runtime, work);
    }

    #[test]
    fn speech_files_are_read_as_written_and_nothing_else_is() {
        let dir = folder("wav");
        let path = dir.join("speech.wav");
        let samples = [0.0_f32, 0.5, -0.5, 0.25];
        std::fs::write(&path, fosscut_audio::wav::wav_bytes(&samples, 1, 24_000)).unwrap();
        let (read, rate) = read_speech(&path).unwrap();
        assert_eq!(rate, 24_000);
        assert_eq!(read.len(), 4);
        for (a, b) in read.iter().zip(samples) {
            assert!((a - b).abs() < 1e-3, "{a} {b}");
        }
        std::fs::write(&path, fosscut_audio::wav::wav_bytes(&samples, 2, 24_000)).unwrap();
        assert!(read_speech(&path).is_none(), "stereo is not speech");
        std::fs::write(&path, b"RIFF").unwrap();
        assert!(read_speech(&path).is_none());
        assert!(read_speech(&dir.join("missing.wav")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_progress_counts_the_bytes_written() {
        let dir = folder("bytes");
        assert_eq!(bytes_in(&dir.join("nothing")), 0);
        std::fs::write(dir.join("a.partial"), [0_u8; 10]).unwrap();
        std::fs::create_dir_all(dir.join("m.partial/voices")).unwrap();
        std::fs::write(dir.join("m.partial/model.onnx"), [0_u8; 7]).unwrap();
        std::fs::write(dir.join("m.partial/voices/a.bin"), [0_u8; 5]).unwrap();
        assert_eq!(bytes_in(&dir.join("a.partial")), 10);
        assert_eq!(bytes_in(&dir.join("m.partial")), 12);
        let install = Install {
            name: "m".into(),
            total: 11,
            partial: dir.join("m.partial"),
            started: Instant::now(),
            outcome: Mutex::new(None),
        };
        let running = install.state();
        assert_eq!(running.state, JobState::Running);
        assert_eq!(running.done_bytes, 11, "never more than the total");
        *guard(&install.outcome) = Some(Err(invalid("broken")));
        let failed = install.state();
        assert_eq!(failed.state, JobState::Failed);
        assert_eq!(failed.error.unwrap().message, "broken");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// True when the FFmpeg programs are here; in CI (`FOSSCUT_REQUIRE_FFMPEG`) they must be.
    fn ffmpeg_programs() -> bool {
        let runs = |program: &str| {
            std::process::Command::new(program)
                .arg("-version")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        let found = runs("ffmpeg") && runs("ffprobe");
        let required =
            std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
        assert!(
            found || !required,
            "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg or ffprobe is missing"
        );
        found
    }

    #[test]
    fn a_pose_is_analyzed_as_a_job_then_answered_from_the_cache() {
        if !ffmpeg_programs() {
            eprintln!("skipped: ffmpeg or ffprobe is missing");
            return;
        }
        let Some(models) = installed(&["mediapipe-pose-full"]) else {
            return;
        };
        let (workspace, runtime, work) = opened("pose");
        let cache = folder("pose-cache");
        let status = std::process::Command::new("ffmpeg")
            .current_dir(&work)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=320x240:rate=10:duration=1")
            .args(["-pix_fmt", "yuv420p", "pose.mp4"])
            .status()
            .unwrap();
        assert!(status.success());
        let (bundle, _, _) = workspace.snapshot().unwrap();
        let args =
            fosscut_engine::import::media_add_args(&work.join("pose.mp4"), None, &bundle).unwrap();
        let asset = workspace
            .execute(Request::single("media.add", args))
            .unwrap()
            .results[0]["asset"]
            .as_str()
            .unwrap()
            .to_string();
        let clip = workspace
            .execute(Request::single("clip.add", json!({ "asset": asset })))
            .unwrap()
            .results[0]["clip"]
            .as_str()
            .unwrap()
            .to_string();
        let jobs = Jobs::new();
        let request = SubjectRequest {
            clip,
            kind: "pose".into(),
            model: None,
            every: None,
            options: None,
        };
        let started = analyze_subject(&workspace, &jobs, &cache, &models, &request).unwrap();
        assert_eq!(started.kind, "analysis");
        assert_eq!(started.asset.as_deref(), Some(asset.as_str()));
        let begun = Instant::now();
        let mut status = started;
        while status.state == JobState::Running && begun.elapsed() < Duration::from_secs(120) {
            std::thread::sleep(Duration::from_millis(100));
            status = subject_status(&jobs, &status.job, false).unwrap();
        }
        assert_eq!(status.state, JobState::Succeeded, "{status:?}");
        let report = status.analysis.unwrap();
        assert!(report.results.pose.is_some());
        assert_eq!(report.computed, vec![Analyzer::Pose]);
        // The same analysis again answers from the cache, at once.
        let again = analyze_subject(&workspace, &jobs, &cache, &models, &request).unwrap();
        assert_eq!(again.state, JobState::Succeeded, "{again:?}");
        assert_eq!(again.analysis.unwrap().cached, vec![Analyzer::Pose]);
        done(workspace, runtime, work);
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// Imports a sound that is nowhere on disk, with its identity recorded, places it and
    /// returns the clip.
    fn audio_clip(workspace: &Workspace) -> String {
        let added = workspace
            .execute(Request::single(
                "media.add",
                json!({
                    "name": "song.wav",
                    "kind": "audio",
                    "path": { "absolute": "/nowhere/song.wav" },
                    "identity": { "size": 999, "sample_blake3": "ef".repeat(32) },
                    "info": {
                        "duration": 4 * SECOND,
                        "audio": { "sample_rate": 48000, "channels": 2, "codec": "pcm_s16le" },
                    }
                }),
            ))
            .unwrap();
        let asset = added.results[0]["asset"].as_str().unwrap().to_string();
        workspace
            .execute(Request::single("clip.add", json!({ "asset": asset })))
            .unwrap()
            .results[0]["clip"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn every_analyzer_a_plan_names_can_be_asked_for_with_its_options() {
        let (workspace, runtime, work) = opened("kinds");
        let (models, cache) = (folder("kinds-models"), folder("kinds-cache"));
        let jobs = Jobs::new();
        let ask = |clip: &str, kind: &str, options: Option<Value>| SubjectRequest {
            clip: clip.into(),
            kind: kind.into(),
            options,
            ..SubjectRequest::default()
        };
        let analyze = |request: &SubjectRequest| {
            analyze_subject(&workspace, &jobs, &cache, &models, request).unwrap_err()
        };
        let video = video_clip(&workspace);
        let song = audio_clip(&workspace);

        let scenes = analyze(&ask(&song, "scenes", None));
        assert_eq!(scenes.code, codes::INVALID_ARGUMENTS);
        assert!(
            scenes
                .message
                .contains("only video is analyzed for shot cuts"),
            "{scenes:?}"
        );
        let silent = analyze(&ask(&video, "beats", None));
        assert_eq!(
            silent.code, "fosscut.analysis.no_audio",
            "a video without sound"
        );
        let modelled = analyze(&SubjectRequest {
            model: Some("subject".into()),
            ..ask(&song, "beats", None)
        });
        assert!(
            modelled.message.contains("a beats analysis takes no model"),
            "{modelled:?}"
        );
        let unknown = analyze(&ask(&song, "beats", Some(json!({ "tempo": 120 }))));
        assert!(unknown.message.contains("options of beats"), "{unknown:?}");
        let shapeless = analyze(&ask(&song, "levels", Some(json!(3))));
        assert!(
            shapeless.message.contains("give an object"),
            "{shapeless:?}"
        );
        let loud = analyze(&ask(&song, "loudness", Some(json!({ "x": 1 }))));
        assert!(loud.message.contains("takes no options"), "{loud:?}");
        assert!(
            jobs.list().is_empty(),
            "nothing started before these checks"
        );

        // Accepted: they start as jobs, which fail at once here because the files are
        // nowhere (or the models are not installed), naming what is missing.
        for (clip, kind, options) in [
            (&song, "beats", Some(json!({ "bpm": 120 }))),
            (&song, "transcript", None),
            (&song, "loudness", Some(json!({}))),
            (&video, "scenes", None),
        ] {
            let started = analyze(&ask(clip, kind, options));
            assert_ne!(
                started.code,
                codes::INVALID_ARGUMENTS,
                "{kind}: {started:?}"
            );
        }
        assert_eq!(jobs.list().len(), 4, "each ran as a job");
        // A matte's options from a missing-analysis error pass through; `model` wins.
        let subject = analyze(&ask(&video, "matte", Some(json!({ "model": "subject" }))));
        assert_eq!(subject.code, "fosscut.model.not_installed", "{subject:?}");
        assert!(
            subject.details["what"]
                .as_str()
                .unwrap()
                .contains("birefnet-lite")
        );
        let person = analyze(&SubjectRequest {
            model: Some("person".into()),
            ..ask(&video, "matte", Some(json!({ "model": "subject" })))
        });
        assert!(
            person.details["what"]
                .as_str()
                .unwrap()
                .contains("mediapipe-pose-full"),
            "{person:?}"
        );
        // The matte's frame step: as given, else the model's.
        let (bundle, project, _) = workspace.snapshot().unwrap();
        let planned = |request: &SubjectRequest| {
            subject_analysis(&project, &bundle, request, &cache, &models).unwrap()
        };
        let slow = planned(&ask(&video, "matte", Some(json!({ "model": "subject" }))));
        assert_eq!(
            (slow.options.matte.model, slow.options.matte.every),
            (MatteModel::Subject, 5)
        );
        let stepped = planned(&ask(
            &video,
            "matte",
            Some(json!({ "model": "subject", "every": 2 })),
        ));
        assert_eq!(stepped.options.matte.every, 2);
        let beats = planned(&ask(&song, "beats", Some(json!({ "bpm": 100 }))));
        assert_eq!(
            (beats.analyzers, beats.options.beats.bpm),
            (vec![Analyzer::Beats], Some(100.0))
        );
        done(workspace, runtime, work);
        let _ = std::fs::remove_dir_all(&models);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn beats_of_a_generated_sound_are_analyzed_as_a_job() {
        if !ffmpeg_programs() {
            eprintln!("skipped: ffmpeg or ffprobe is missing");
            return;
        }
        let (workspace, runtime, work) = opened("beatjob");
        let (models, cache) = (folder("beatjob-models"), folder("beatjob-cache"));
        // Clicks at 120 beats a minute for 6 seconds.
        let status = std::process::Command::new("ffmpeg")
            .current_dir(&work)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(
                "aevalsrc='0.6*sin(2*PI*2000*t)*exp(-mod(t+0.13,0.5)/0.0015)*gte(t,0.37)'\
                 :s=44100:d=6",
            )
            .args(["-c:a", "pcm_s16le", "click.wav"])
            .status()
            .unwrap();
        assert!(status.success());
        let (bundle, _, _) = workspace.snapshot().unwrap();
        let args =
            fosscut_engine::import::media_add_args(&work.join("click.wav"), None, &bundle).unwrap();
        let asset = workspace
            .execute(Request::single("media.add", args))
            .unwrap()
            .results[0]["asset"]
            .as_str()
            .unwrap()
            .to_string();
        let clip = workspace
            .execute(Request::single("clip.add", json!({ "asset": asset })))
            .unwrap()
            .results[0]["clip"]
            .as_str()
            .unwrap()
            .to_string();
        let jobs = Jobs::new();
        let request = SubjectRequest {
            clip,
            kind: "beats".into(),
            ..SubjectRequest::default()
        };
        let mut status = analyze_subject(&workspace, &jobs, &cache, &models, &request).unwrap();
        let begun = Instant::now();
        while status.state == JobState::Running && begun.elapsed() < Duration::from_secs(60) {
            std::thread::sleep(Duration::from_millis(50));
            status = subject_status(&jobs, &status.job, false).unwrap();
        }
        assert_eq!(status.state, JobState::Succeeded, "{status:?}");
        let report = status.analysis.unwrap();
        assert_eq!(report.computed, vec![Analyzer::Beats]);
        let beats = report.results.beats.unwrap();
        let tempo = beats.tempo_bpm.unwrap();
        assert!((tempo - 120.0).abs() < 2.0, "tempo {tempo}");
        assert!(beats.beats.len() >= 10, "{beats:?}");
        let again = analyze_subject(&workspace, &jobs, &cache, &models, &request).unwrap();
        assert_eq!(again.state, JobState::Succeeded);
        assert_eq!(again.analysis.unwrap().cached, vec![Analyzer::Beats]);
        done(workspace, runtime, work);
        let _ = std::fs::remove_dir_all(&models);
        let _ = std::fs::remove_dir_all(&cache);
    }
}
