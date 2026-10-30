//! FossCut's MCP server (ADR-0002): a small set of coarse tools over the engine, so any MCP
//! client (Claude Code, Codex, Cursor and others) can edit, check and export projects.
//! `fosscut mcp` serves it on stdio.
//!
//! - One generic `edit` tool runs any command from `capabilities` as an atomic batch with
//!   the agent safety contract (ADR-0006); a few dedicated tools cover what most sessions
//!   need: projects, timeline reads, media import, frame grabs, contact sheets, export jobs.
//! - Writes go to the project's live host when the desktop app or `fosscut host` has it
//!   open, so the user sees them at once (ADR-0022); otherwise each write takes the
//!   bundle's writer lock for one request. Writes are logged as made by
//!   `agent:<client name>`.
//! - FossCut errors are tool results with `isError` set and the typed
//!   `{"error": {code, message, hint, details}}`, so agents can recover.
//! - The built-in skills ([`skills`]) are MCP prompts and resources too, for clients
//!   without skill support (ADR-0013, item 3).

#![forbid(unsafe_code)]

mod params;
pub mod skills;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine;
use fosscut_core::{ApiError, CommandCall, REDO, Request, UNDO, codes};
use fosscut_engine::jobs::Jobs;
use fosscut_engine::project::Projects;
use fosscut_engine::{EngineError, ExportRequest, Preset, project};
use fosscut_model::TimelineFormat;
use fosscut_render::Renderer;
use fosscut_time::Rate;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, GetPromptRequestParams, GetPromptResponse, GetPromptResult,
    Implementation, ListPromptsResult, ListResourcesResult, PaginatedRequestParams, Prompt,
    PromptMessage, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, Role, ServerCapabilities, ServerConfig,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde_json::{Value, json};

use params::{
    CapabilitiesArgs, CaptionsAction, CaptionsArgs, ColorAction, ColorArgs, Detail, EditArgs,
    ExportArgs, FrameArgs, HistoryArgs, HistoryStepArgs, JobArgs, MediaAnalyzeArgs,
    MediaImportArgs, ModelsArgs, ProjectArgs, ProjectNewArgs, ProjectOpenArgs, QueryArgs,
    SheetArgs, TimelineReadArgs,
};

/// What the server tells agents when they connect.
const INSTRUCTIONS: &str = "\
FossCut edits video projects (.fosscut bundles). A safe session:
1. project_open (or project_new) once; later tools use that project.
2. timeline_read shows tracks and clips in frames, timecode and seconds, and the revision.
3. edit applies commands as one atomic, undoable step; get a command's arguments first with capabilities and names, for example [\"clip.add\"]. Pass expected_revision from your last read, use dry_run first for large changes and a request_id so retries are safe. Ask the user before setting allow_destructive.
4. Check the result with frame_grab or contact_sheet before exporting.
5. export starts a background job; call job_status with wait_seconds until it succeeds, then read its verification report.
Times need a unit: {\"frames\": n}, {\"seconds\": \"1.5\"}, {\"timecode\": \"HH:MM:SS:FF\"} or {\"flicks\": n}.
Errors come back as {\"error\": {code, message, hint, details}}: follow the hint.
When project_info or project_open shows session.live, the user has the project open in the FossCut app: your edits appear there live and the user may edit too, so after a fosscut.revision.conflict read again and re-plan.
The fosscut-basics prompt (also the resource fosscut://skills/fosscut-basics) is the full guide.";

/// Longest `job_status` wait, in seconds.
const MAX_WAIT_SECONDS: u64 = 60;

/// Serves FossCut over stdio until the client disconnects. `project` is the project tools
/// use when they are given none.
pub async fn serve_stdio(
    project: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let service = FosscutServer::new(project)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

/// The MCP server: the open project, the session's export jobs and a renderer for frames.
#[derive(Clone)]
pub struct FosscutServer {
    state: Arc<State>,
    tool_router: ToolRouter<FosscutServer>,
}

struct State {
    project: Mutex<Option<PathBuf>>,
    /// Reaches projects on disk or through their live host (ADR-0022).
    projects: Projects,
    jobs: Jobs,
    renderer: Mutex<Option<Renderer>>,
    /// Where analysis results and export request records are kept.
    cache: PathBuf,
    /// Where models are installed.
    models: PathBuf,
}

/// Locks `mutex`, recovering the data if a tool panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs blocking work (disk, GPU, FFmpeg) off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work).await.unwrap_or_else(|e| {
        Err(ApiError::new(
            codes::INTERNAL,
            format!("the tool stopped: {e}"),
        ))
    })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// A tool result: JSON text, or a FossCut error with `isError` set.
fn reply(result: Result<Value, ApiError>) -> Result<CallToolResult, ErrorData> {
    Ok(match result {
        Ok(value) => CallToolResult::success(vec![ContentBlock::text(pretty(&value))]),
        Err(error) => failure(&error),
    })
}

fn failure(error: &ApiError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(pretty(&json!({ "error": error })))])
}

/// A tool result with a PNG image followed by its JSON description.
fn image_reply(result: Result<(Vec<u8>, Value), ApiError>) -> Result<CallToolResult, ErrorData> {
    Ok(match result {
        Ok((png, value)) => CallToolResult::success(vec![
            ContentBlock::image(
                base64::engine::general_purpose::STANDARD.encode(png),
                "image/png",
            ),
            ContentBlock::text(pretty(&value)),
        ]),
        Err(error) => failure(&error),
    })
}

/// `agent:<client name>`, as the operation log records writes (ADR-0006, item 9).
fn actor(context: &RequestContext<RoleServer>) -> String {
    context
        .client_info()
        .map(|client| format!("agent:{}", client.name))
        .unwrap_or_else(|| "agent:mcp".into())
}

fn as_value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or_default()
}

/// Project info with `session`: whether the desktop app or `fosscut host` has the project
/// open, so the user may be watching and editing too (ADR-0022).
fn info(projects: &Projects, bundle: &Path) -> Result<Value, ApiError> {
    let mut value = projects.query(bundle, "project.info", Value::Null)?;
    value["session"] = as_value(&projects.status(bundle)?);
    Ok(value)
}

impl FosscutServer {
    /// A server whose tools use `project` when they are given none, caching in the user's
    /// cache directory ([`fosscut_engine::cache_dir`]).
    pub fn new(project: Option<PathBuf>) -> FosscutServer {
        FosscutServer::with_cache(project, fosscut_engine::cache_dir())
    }

    /// A server that caches analysis results and export request records in `cache`, with
    /// models in [`fosscut_engine::models_dir`].
    pub fn with_cache(project: Option<PathBuf>, cache: PathBuf) -> FosscutServer {
        FosscutServer::with_dirs(project, cache, fosscut_engine::models_dir())
    }

    /// A server that caches in `cache` and installs and reads models in `models`.
    pub fn with_dirs(project: Option<PathBuf>, cache: PathBuf, models: PathBuf) -> FosscutServer {
        FosscutServer::with_runtime(project, cache, models, fosscut_engine::live::runtime_dir())
    }

    /// The same, finding live hosts (the desktop app, `fosscut host`) through the records
    /// in `runtime` (ADR-0022).
    pub fn with_runtime(
        project: Option<PathBuf>,
        cache: PathBuf,
        models: PathBuf,
        runtime: PathBuf,
    ) -> FosscutServer {
        FosscutServer {
            state: Arc::new(State {
                project: Mutex::new(project),
                projects: Projects::with_runtime("agent:mcp", runtime),
                jobs: Jobs::new(),
                renderer: Mutex::new(None),
                cache,
                models,
            }),
            tool_router: FosscutServer::tool_router(),
        }
    }

    /// The bundle a tool works on: `given`, or the open project.
    fn bundle(&self, given: Option<String>) -> Result<PathBuf, ApiError> {
        given
            .map(PathBuf::from)
            .or_else(|| lock(&self.state.project).clone())
            .ok_or_else(|| {
                ApiError::new("fosscut.project.not_found", "no project is open").with_hint(
                    "Call project_open with the path of a .fosscut bundle or project_new to create one, or pass project to this tool.",
                )
            })
    }

    async fn read(
        &self,
        project: Option<String>,
        name: &'static str,
        args: Value,
    ) -> Result<Value, ApiError> {
        let bundle = self.bundle(project)?;
        let state = Arc::clone(&self.state);
        blocking(move || state.projects.query(&bundle, name, args)).await
    }

    async fn execute(&self, project: Option<String>, request: Request) -> Result<Value, ApiError> {
        let bundle = self.bundle(project)?;
        let state = Arc::clone(&self.state);
        blocking(move || {
            state
                .projects
                .execute(&bundle, request)
                .map(|response| as_value(&response))
        })
        .await
    }

    /// Runs `work` with the session's renderer, creating it on first use.
    fn with_renderer<T>(
        state: &State,
        work: impl FnOnce(&mut Renderer) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let mut renderer = lock(&state.renderer);
        if renderer.is_none() {
            *renderer =
                Some(Renderer::headless().map_err(|e| ApiError::new(e.code(), e.to_string()))?);
        }
        match renderer.as_mut() {
            Some(renderer) => work(renderer),
            None => Err(ApiError::new(codes::INTERNAL, "no renderer")),
        }
    }
}

/// A write request with the envelope fields an agent passed.
fn request(
    commands: Vec<CommandCall>,
    context: &RequestContext<RoleServer>,
    expected_revision: Option<u64>,
    dry_run: bool,
    request_id: Option<String>,
    reason: Option<String>,
) -> Request {
    let mut request = Request::single("", Value::Null);
    request.commands = commands;
    request.expected_revision = expected_revision;
    request.dry_run = dry_run;
    request.request_id = request_id;
    request.reason = reason;
    request.actor = actor(context);
    request.at_ms = project::now_ms();
    request
}

#[tool_router]
impl FosscutServer {
    /// Lists what FossCut can do.
    #[tool(
        description = "List what FossCut can do: every editing command and query with a summary, the engine operations, the analyzers, the models and whether each is installed, and the export presets this machine can encode. Before using a command the first time, call it with names (for example [\"clip.add\", \"text.add\"]) to get just those argument schemas; detail: full returns every schema and error code at once and is large.",
        annotations(read_only_hint = true)
    )]
    async fn capabilities(
        &self,
        params: Parameters<CapabilitiesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let CapabilitiesArgs { detail, names } = params.0;
        let bundle = lock(&self.state.project).clone();
        let models = self.state.models.clone();
        reply(
            blocking(move || {
                let mut value =
                    fosscut_engine::capabilities(detail.as_str(), &names, bundle.as_deref())?;
                if value.get("models").is_some() {
                    value["models"] = json!(fosscut_engine::analysis::models(&models));
                }
                Ok(value)
            })
            .await,
        )
    }

    /// Creates a project and opens it.
    #[tool(
        description = "Create a new project bundle (a directory ending in .fosscut) with an empty timeline, and make it the open project. Defaults: 1920x1080 at 30 fps; use width 1080 and height 1920 for vertical video. Returns project info.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn project_new(
        &self,
        params: Parameters<ProjectNewArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let path = PathBuf::from(&args.path);
        let created = path.clone();
        let result = blocking(move || {
            let rate: Rate = args.rate.as_deref().unwrap_or("30").parse()?;
            let name = args.name.unwrap_or_else(|| {
                created
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled".into())
            });
            let format = TimelineFormat {
                width: args.width.unwrap_or(1920),
                height: args.height.unwrap_or(1080),
                rate,
                sample_rate: 48_000,
            };
            project::create(&created, &name, format)?.query("project.info", Value::Null)
        })
        .await;
        if result.is_ok() {
            *lock(&self.state.project) = Some(path);
        }
        reply(result)
    }

    /// Opens a project.
    #[tool(
        description = "Open an existing .fosscut project; later tools use it when they get no project argument. Returns project info: name, revision, timelines and undo state.",
        annotations(read_only_hint = true)
    )]
    async fn project_open(
        &self,
        params: Parameters<ProjectOpenArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let path = PathBuf::from(params.0.path);
        let opened = path.clone();
        let state = Arc::clone(&self.state);
        let result = blocking(move || info(&state.projects, &opened)).await;
        if result.is_ok() {
            *lock(&self.state.project) = Some(path);
        }
        reply(result)
    }

    /// Reads project info.
    #[tool(
        description = "Project name, current revision, timelines with their formats, and whether undo or redo is possible.",
        annotations(read_only_hint = true)
    )]
    async fn project_info(
        &self,
        params: Parameters<ProjectArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let bundle = match self.bundle(params.0.project) {
            Ok(bundle) => bundle,
            Err(error) => return Ok(failure(&error)),
        };
        let state = Arc::clone(&self.state);
        reply(blocking(move || info(&state.projects, &bundle)).await)
    }

    /// Reads a timeline.
    #[tool(
        description = "Read a timeline: its format, duration and revision, and every track with its clips (ids, start, duration, source range, text) in frames, timecode and seconds. Use the revision as expected_revision in edit.",
        annotations(read_only_hint = true)
    )]
    async fn timeline_read(
        &self,
        params: Parameters<TimelineReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let mut query = json!({ "detail": args.detail.unwrap_or(Detail::Normal).as_str() });
        if let Some(timeline) = args.timeline {
            query["timeline"] = json!(timeline);
        }
        reply(self.read(args.project, "timeline.read", query).await)
    }

    /// Imports media files.
    #[tool(
        description = "Import video, audio or image files into the project's media table in one revision. Returns each new asset id (asset_...), which clip.add uses. Importing a file that is already in the project returns its existing asset.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn media_import(
        &self,
        params: Parameters<MediaImportArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let bundle = match self.bundle(args.project) {
            Ok(bundle) => bundle,
            Err(error) => return Ok(failure(&error)),
        };
        let files = args.files;
        let state = Arc::clone(&self.state);
        let envelope = (
            args.expected_revision,
            args.dry_run,
            args.request_id,
            args.reason,
        );
        let result = blocking(move || {
            let commands = files
                .iter()
                .map(|file| {
                    fosscut_engine::import::media_add_args(Path::new(file), None, &bundle).map(
                        |args| CommandCall {
                            command: "media.add".into(),
                            args,
                        },
                    )
                })
                .collect::<Result<Vec<_>, ApiError>>()?;
            let (expected_revision, dry_run, request_id, reason) = envelope;
            let request = request(
                commands,
                &context,
                expected_revision,
                dry_run,
                request_id,
                reason,
            );
            state
                .projects
                .execute(&bundle, request)
                .map(|response| as_value(&response))
        })
        .await;
        reply(result)
    }

    /// Lists media.
    #[tool(
        description = "List the project's media: asset ids, names, kinds, durations and stream details.",
        annotations(read_only_hint = true)
    )]
    async fn media_list(
        &self,
        params: Parameters<ProjectArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        reply(self.read(params.0.project, "media.list", Value::Null).await)
    }

    /// Analyzes media.
    #[tool(
        description = "Measure an asset's loudness (LUFS), silences (pauses to cut or keep) and shot cuts, or transcribe its speech (analyzers: [\"transcript\"]) into segments and words with start and end times, filler words such as um and uh, and pauses between words, all on this machine. Cached results come back at once; otherwise the measurement runs as a job and this waits up to wait_seconds (default 30) for it: if state is still running, follow it with job_status. Times are from the start of the asset's file, in flicks and seconds; map them to the timeline through the clip's source_in (and its speed, when timeline_read shows one). options tunes thresholds, for example {\"silences\": {\"threshold_dbfs\": -35}}. A transcript needs a speech model: if none is installed, ask the user before installing one with models.",
        annotations(read_only_hint = true)
    )]
    async fn media_analyze(
        &self,
        params: Parameters<MediaAnalyzeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let bundle = match self.bundle(args.project) {
            Ok(bundle) => bundle,
            Err(error) => return Ok(failure(&error)),
        };
        let wait = Duration::from_secs(args.wait_seconds.unwrap_or(30).min(MAX_WAIT_SECONDS));
        let state = Arc::clone(&self.state);
        let result = blocking(move || {
            let snapshot = state.projects.snapshot(&bundle)?;
            let asset = fosscut_model::Id::parse(&args.asset)
                .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("asset: {e}")))?;
            let request = fosscut_engine::analysis::AnalysisRequest {
                bundle: bundle.clone(),
                asset,
                analyzers: fosscut_engine::analysis::analyzers(&args.analyzers)?,
                options: args.options,
                refresh: args.refresh,
                cache: state.cache.clone(),
                models: state.models.clone(),
            };
            let project = &snapshot;
            if let Some(report) = fosscut_engine::analysis::cached(project, &request)? {
                return Ok(json!({
                    "state": "succeeded",
                    "kind": "analysis",
                    "asset": report.asset,
                    "analysis": report,
                }));
            }
            let started = state.jobs.start_analysis(project.clone(), request);
            let status = state.jobs.status(&started.job, wait)?;
            let mut value = as_value(&status);
            if status.state == fosscut_engine::jobs::JobState::Running {
                value["next"] = json!("Call job_status with this job and wait_seconds.");
            }
            Ok(value)
        })
        .await;
        reply(result)
    }

    /// Lists or installs models.
    #[tool(
        description = "List the models analyzers can use (speech recognition for media_analyze transcript) with size, license, languages and whether each is installed, or install one. Installing downloads it and checks its SHA-256: tell the user the size and license first, and pass confirm: true only after they agree. Larger models are more accurate and slower; English-only models (.en) suit English speech.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn models(&self, params: Parameters<ModelsArgs>) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let dir = self.state.models.clone();
        reply(
            blocking(move || {
                let Some(name) = args.install else {
                    return Ok(json!({
                        "dir": dir.display().to_string(),
                        "models": fosscut_engine::analysis::models(&dir),
                    }));
                };
                let source = match args.from {
                    Some(file) => fosscut_engine::analysis::ModelSource::File(file.into()),
                    None => fosscut_engine::analysis::ModelSource::Download(None),
                };
                let report = fosscut_engine::analysis::install_model(
                    &dir,
                    &name,
                    &source,
                    args.confirm,
                    false,
                )?;
                Ok(as_value(&report))
            })
            .await,
        )
    }

    /// Applies commands.
    #[tool(
        description = "Change the project: apply one or more commands (clip.add, clip.split, clip.move, clip.remove, text.add, track.add and others from capabilities) in order as one atomic transaction and one undo step. Pass expected_revision from your last read; dry_run: true reports the changes without committing; a request_id makes retries safe. Returns the new revision, each command's result and a summary of changes.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn edit(
        &self,
        params: Parameters<EditArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let mut request = request(
            args.commands,
            &context,
            args.expected_revision,
            args.dry_run,
            args.request_id,
            args.reason,
        );
        request.allow_destructive = args.allow_destructive;
        reply(self.execute(args.project, request).await)
    }

    /// Captions from transcripts and SRT files.
    #[tool(
        description = "Word-timed captions. action generate captions the clips of a track from their cached transcripts (run media_analyze with analyzers [\"transcript\"] first): words become pages of a few words on a caption track, styled by preset (bold-pop, highlight, karaoke or boxed) and style. action import adds pages from an SRT file, one per cue; action export writes a caption track to an SRT file. generate and import are one undoable step with expected_revision, dry_run and request_id. Then change pages with edit (captions.edit) and the look with captions.style, and check frame_grab at a spoken word.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn captions(
        &self,
        params: Parameters<CaptionsArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let bundle = match self.bundle(args.project.clone()) {
            Ok(bundle) => bundle,
            Err(error) => return Ok(failure(&error)),
        };
        let actor = actor(&context);
        let state = Arc::clone(&self.state);
        reply(
            blocking(move || {
                let snapshot = state.projects.snapshot(&bundle)?;
                let id = |value: &Option<String>, what: &str| {
                    value
                        .as_deref()
                        .map(fosscut_model::Id::parse)
                        .transpose()
                        .map_err(|e| {
                            ApiError::new(codes::INVALID_ARGUMENTS, format!("{what}: {e}"))
                        })
                };
                let timeline = id(&args.timeline, "timeline")?;
                let track = id(&args.track, "track")?;
                let file = |what: &str| {
                    args.file.clone().map(PathBuf::from).ok_or_else(|| {
                        ApiError::new(codes::INVALID_ARGUMENTS, format!("{what} needs file"))
                    })
                };
                let words = match args.action {
                    CaptionsAction::Export => {
                        let path = file("export")?;
                        let srt = fosscut_engine::captions::export_srt(
                            &snapshot,
                            timeline.as_ref(),
                            track.as_ref(),
                        )?;
                        if path.exists() && !args.allow_destructive {
                            return Err(EngineError::OutputExists {
                                path: path.display().to_string(),
                            }
                            .into());
                        }
                        if !args.dry_run {
                            std::fs::write(&path, &srt).map_err(|e| {
                                ApiError::new(
                                    "fosscut.media.write_failed",
                                    format!("cannot write {}: {e}", path.display()),
                                )
                            })?;
                        }
                        return Ok(json!({
                            "file": path.display().to_string(),
                            "cues": srt.matches(" --> ").count(),
                            "dry_run": args.dry_run,
                            "preview": fosscut_engine::captions::srt_preview(&srt, 5),
                        }));
                    }
                    CaptionsAction::Generate => {
                        let generated = fosscut_engine::captions::generate(
                            &snapshot,
                            &fosscut_engine::captions::GenerateRequest {
                                bundle: bundle.clone(),
                                timeline: timeline.clone(),
                                source_track: id(&args.source_track, "source_track")?,
                                transcript: args.transcript.clone(),
                                cache: state.cache.clone(),
                                models: state.models.clone(),
                            },
                        )?;
                        (generated.words, args.paging.clone())
                    }
                    CaptionsAction::Import => {
                        let path = file("import")?;
                        let text = std::fs::read_to_string(&path).map_err(|e| {
                            ApiError::new(
                                codes::INVALID_ARGUMENTS,
                                format!("cannot read {}: {e}", path.display()),
                            )
                        })?;
                        let cues = fosscut_engine::captions::parse_srt(&text)?;
                        let timeline_id = timeline
                            .clone()
                            .unwrap_or_else(|| snapshot.manifest.primary_timeline.clone());
                        let rate = snapshot
                            .timelines
                            .get(&timeline_id)
                            .map(|t| t.format.rate)
                            .ok_or_else(|| {
                                ApiError::new(
                                    "fosscut.timeline.not_found",
                                    format!("timeline {timeline_id} does not exist"),
                                )
                            })?;
                        (
                            fosscut_engine::captions::cue_words(&cues, rate),
                            Some(fosscut_engine::captions::cue_paging()),
                        )
                    }
                };
                let (words, paging) = words;
                let mut add = json!({ "words": words });
                for (key, value) in [
                    ("timeline", timeline.map(|t| json!(t))),
                    ("track", track.map(|t| json!(t))),
                    ("preset", args.preset.map(|p| json!(p))),
                    ("style", args.style),
                    ("paging", paging),
                    ("replace", args.replace.then_some(json!(true))),
                ] {
                    if let Some(value) = value {
                        add[key] = value;
                    }
                }
                let mut write = Request::single("captions.add", add);
                write.expected_revision = args.expected_revision;
                write.dry_run = args.dry_run;
                write.request_id = args.request_id;
                write.allow_destructive = args.allow_destructive;
                write.reason = args.reason;
                write.actor = actor;
                write.at_ms = project::now_ms();
                state
                    .projects
                    .execute(&bundle, write)
                    .map(|response| as_value(&response))
            })
            .await,
        )
    }

    /// Corrects a clip's color, or matches it to another.
    #[tool(
        description = "Automatic color for a video or image clip, measured from its frames on this machine. action auto adds a color.adjust effect (first in the clip's effects) that neutralizes the white balance, sets the brightness and stretches blacks and whites; action match adds one that brings the clip's balance, brightness, contrast and saturation close to the reference clip. One undoable step with expected_revision, dry_run and request_id; returns the measurements and the effect. Adjust the effect afterwards with edit (effect.set) and check frame_grab."
    )]
    async fn color(
        &self,
        params: Parameters<ColorArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let bundle = match self.bundle(args.project.clone()) {
            Ok(bundle) => bundle,
            Err(error) => return Ok(failure(&error)),
        };
        let actor = actor(&context);
        let state = Arc::clone(&self.state);
        reply(
            blocking(move || {
                let snapshot = state.projects.snapshot(&bundle)?;
                let id = |text: &str, what: &str| {
                    fosscut_model::Id::parse(text).map_err(|e| {
                        ApiError::new(codes::INVALID_ARGUMENTS, format!("{what}: {e}"))
                    })
                };
                let clip = id(&args.clip, "clip")?;
                let reference = match (args.action, &args.reference) {
                    (ColorAction::Match, Some(reference)) => Some(id(reference, "reference")?),
                    (ColorAction::Match, None) => {
                        return Err(ApiError::new(
                            codes::INVALID_ARGUMENTS,
                            "color match needs reference: the clip to match",
                        ));
                    }
                    (ColorAction::Auto, _) => None,
                };
                let suggestion = FosscutServer::with_renderer(&state, |renderer| {
                    fosscut_engine::color::suggest(
                        &snapshot,
                        &bundle,
                        &clip,
                        reference.as_ref(),
                        renderer,
                    )
                    .map_err(ApiError::from)
                })?;
                let mut write = Request::single("effect.add", suggestion.effect_add.clone());
                write.expected_revision = args.expected_revision;
                write.dry_run = args.dry_run;
                write.request_id = args.request_id;
                write.reason = args.reason;
                write.actor = actor;
                write.at_ms = project::now_ms();
                let response = state.projects.execute(&bundle, write)?;
                let mut value = as_value(&response);
                value["measured"] = json!(suggestion.stats);
                if let Some(reference) = suggestion.reference {
                    value["reference"] = json!(reference);
                }
                Ok(value)
            })
            .await,
        )
    }

    /// Undoes the last change.
    #[tool(
        description = "Undo the most recent change to the project (one edit, import or redo), whoever made it. Returns the new revision.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn undo(
        &self,
        params: Parameters<HistoryStepArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let call = CommandCall {
            command: UNDO.into(),
            args: json!({}),
        };
        let request = request(
            vec![call],
            &context,
            args.expected_revision,
            false,
            None,
            args.reason,
        );
        reply(self.execute(args.project, request).await)
    }

    /// Redoes the last undone change.
    #[tool(
        description = "Redo the most recently undone change. Returns the new revision.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn redo(
        &self,
        params: Parameters<HistoryStepArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let call = CommandCall {
            command: REDO.into(),
            args: json!({}),
        };
        let request = request(
            vec![call],
            &context,
            args.expected_revision,
            false,
            None,
            args.reason,
        );
        reply(self.execute(args.project, request).await)
    }

    /// Lists recent changes.
    #[tool(
        description = "List the project's recent changes, newest first: revision, who made it (human or agent:<name>), the commands and the reason.",
        annotations(read_only_hint = true)
    )]
    async fn history(&self, params: Parameters<HistoryArgs>) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let query = json!({ "limit": args.limit.unwrap_or(20) });
        reply(self.read(args.project, "history.list", query).await)
    }

    /// Runs any query.
    #[tool(
        description = "Run any read-only query from capabilities by name with its arguments, for queries no other tool covers.",
        annotations(read_only_hint = true)
    )]
    async fn query(&self, params: Parameters<QueryArgs>) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let bundle = match self.bundle(args.project) {
            Ok(bundle) => bundle,
            Err(error) => return Ok(failure(&error)),
        };
        let query_args = args.args.unwrap_or(Value::Null);
        let name = args.name;
        let state = Arc::clone(&self.state);
        reply(blocking(move || state.projects.query(&bundle, &name, query_args)).await)
    }

    /// Renders one frame.
    #[tool(
        description = "Render the frame of the timeline at a time as a PNG image, exactly as the export will show it (default 640 pixels wide). Use it to check titles, placement and cuts. Returns the image and the frame's time in every unit.",
        annotations(read_only_hint = true)
    )]
    async fn frame_grab(&self, params: Parameters<FrameArgs>) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let bundle = match self.bundle(args.project) {
            Ok(bundle) => bundle,
            Err(error) => return Ok(failure(&error)),
        };
        let state = Arc::clone(&self.state);
        let result = blocking(move || {
            let project = state.projects.snapshot(&bundle)?;
            let project = &project;
            let timeline = project::timeline(project, args.timeline.as_deref())?;
            let time = args
                .time
                .resolve(project.timelines[&timeline].format.rate)?;
            let still = FosscutServer::with_renderer(&state, |renderer| {
                fosscut_engine::frame::frame(
                    project,
                    &bundle,
                    &timeline,
                    time,
                    Some(args.width.unwrap_or(640)),
                    renderer,
                )
                .map_err(ApiError::from)
            })?;
            let png = fosscut_engine::frame::png(&still.image)?;
            let value = json!({
                "timeline": timeline,
                "at": still.at,
                "width": still.image.width,
                "height": still.image.height,
            });
            Ok((png, value))
        })
        .await;
        image_reply(result)
    }

    /// Renders a contact sheet.
    #[tool(
        description = "Render frames spread evenly over the timeline into one PNG grid (default 12 frames, 240 pixels wide each), to check a whole edit at a glance. Returns the image and each tile's time and position.",
        annotations(read_only_hint = true)
    )]
    async fn contact_sheet(
        &self,
        params: Parameters<SheetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let bundle = match self.bundle(args.project) {
            Ok(bundle) => bundle,
            Err(error) => return Ok(failure(&error)),
        };
        let state = Arc::clone(&self.state);
        let result = blocking(move || {
            let project = state.projects.snapshot(&bundle)?;
            let project = &project;
            let timeline = project::timeline(project, args.timeline.as_deref())?;
            let sheet = FosscutServer::with_renderer(&state, |renderer| {
                fosscut_engine::frame::contact_sheet(
                    project,
                    &bundle,
                    &timeline,
                    args.count.unwrap_or(12),
                    args.width.unwrap_or(240),
                    renderer,
                )
                .map_err(ApiError::from)
            })?;
            let png = fosscut_engine::frame::png(&sheet.image)?;
            let value = json!({
                "timeline": timeline,
                "width": sheet.image.width,
                "height": sheet.image.height,
                "columns": sheet.columns,
                "tiles": sheet.tiles,
            });
            Ok((png, value))
        })
        .await;
        image_reply(result)
    }

    /// Starts an export.
    #[tool(
        description = "Export the timeline to a video or audio file in the background and return a job id at once; then call job_status with wait_seconds until the job succeeds. The file is probed and checked against the request (codec, size, rate, frame count, duration, audio); a codec this machine cannot encode fails with the codecs that work instead, never a substitute. dry_run: true checks everything and reports frames, duration and encoders without writing.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn export(&self, params: Parameters<ExportArgs>) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let bundle = match self.bundle(args.project) {
            Ok(bundle) => bundle,
            Err(error) => return Ok(failure(&error)),
        };
        let state = Arc::clone(&self.state);
        let result = blocking(move || {
            let project = state.projects.snapshot(&bundle)?;
            let timeline = project::timeline(&project, args.timeline.as_deref())?;
            let output = PathBuf::from(&args.output);
            let preset = match args.preset.as_deref() {
                Some(name) => Preset::parse(name).ok_or_else(|| {
                    let names: Vec<&str> = Preset::ALL.iter().map(|p| p.name()).collect();
                    ApiError::new(codes::INVALID_ARGUMENTS, format!("unknown preset {name}"))
                        .with_hint(format!("Presets: {}.", names.join(", ")))
                })?,
                None => Preset::for_output(&output).ok_or_else(|| {
                    ApiError::new(
                        "fosscut.export.codec_unavailable",
                        "this machine cannot encode video",
                    )
                })?,
            };
            let request = ExportRequest {
                bundle: bundle.clone(),
                settings: preset.settings(&project.timelines[&timeline].format),
                timeline,
                output,
                allow_overwrite: args.allow_destructive,
                expected_revision: args.expected_revision,
                request_id: args.request_id,
                cache: state.cache.clone(),
            };
            if args.dry_run {
                let plan = fosscut_engine::plan(&project, &request)?;
                return Ok(json!({ "dry_run": true, "preset": preset.name(), "plan": plan }));
            }
            fosscut_engine::plan(&project, &request)?;
            let status = state.jobs.start_export(project, request)?;
            let mut value = as_value(&status);
            value["preset"] = json!(preset.name());
            value["next"] =
                json!("Call job_status with this job and wait_seconds until state is succeeded.");
            Ok(value)
        })
        .await;
        reply(result)
    }

    /// Reports an export job.
    #[tool(
        description = "The state of an export job (running, succeeded, failed or cancelled) with progress; when it succeeded, the verified export report; when it failed, the typed error. wait_seconds (up to 60) waits for the job to finish before answering.",
        annotations(read_only_hint = true)
    )]
    async fn job_status(&self, params: Parameters<JobArgs>) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let wait = Duration::from_secs(args.wait_seconds.unwrap_or(0).min(MAX_WAIT_SECONDS));
        let state = Arc::clone(&self.state);
        reply(
            blocking(move || {
                state
                    .jobs
                    .status(&args.job, wait)
                    .map(|status| as_value(&status))
                    .map_err(ApiError::from)
            })
            .await,
        )
    }

    /// Cancels an export job.
    #[tool(
        description = "Cancel a running export job; nothing is written. A finished job is not affected. wait_seconds (up to 60) waits for it to stop.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn job_cancel(&self, params: Parameters<JobArgs>) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let wait = Duration::from_secs(args.wait_seconds.unwrap_or(0).min(MAX_WAIT_SECONDS));
        let state = Arc::clone(&self.state);
        reply(
            blocking(move || {
                state
                    .jobs
                    .cancel(&args.job, wait)
                    .map(|status| as_value(&status))
                    .map_err(ApiError::from)
            })
            .await,
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FosscutServer {
    /// Names this server to live hosts after the client that connected (ADR-0022), so the
    /// app lists it as `agent:<client name>`.
    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        if let Some(peer) = context.peer.peer_info() {
            self.state
                .projects
                .set_client(format!("agent:{}", peer.client_info.name));
        }
    }

    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("fosscut", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        let prompts = skills::SKILLS
            .iter()
            .map(|skill| Prompt::new(skill.name, Some(skill.description()), None))
            .collect();
        Ok(ListPromptsResult::with_all_items(prompts))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let skill = skills::skill(&request.name).ok_or_else(|| {
            ErrorData::invalid_params(format!("no FossCut skill {}", request.name), None)
        })?;
        Ok(
            GetPromptResult::new(vec![PromptMessage::new_text(Role::User, skill.text())])
                .with_description(skill.description())
                .into(),
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let resources = skills::SKILLS
            .iter()
            .map(|skill| {
                Resource::new(skill.uri(), skill.name)
                    .with_description(skill.description())
                    .with_mime_type("text/markdown")
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let skill = skills::SKILLS
            .iter()
            .find(|skill| skill.uri() == request.uri)
            .ok_or_else(|| ErrorData::resource_not_found(request.uri.clone(), None))?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(skill.text(), request.uri)]).into())
    }
}
