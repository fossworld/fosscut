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
    AutoEditAction, AutoEditArgs, CapabilitiesArgs, CaptionsAction, CaptionsArgs, ColorAction,
    ColorArgs, Detail, EditArgs, ExportArgs, FrameArgs, HistoryArgs, HistoryStepArgs, JobArgs,
    LibraryAction, LibraryArgs, MediaAnalyzeArgs, MediaImportArgs, ModelsArgs, NormalizeArgs,
    ProjectArgs, ProjectNewArgs, ProjectOpenArgs, QueryArgs, RecordAction, RecordArgs, SheetArgs,
    TimelineReadArgs,
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

/// A motion plan without its calls, which the response shows applied.
fn motion_summary(plan: &fosscut_engine::motion::MotionPlan) -> Value {
    let mut report = json!(plan);
    if let Some(object) = report.as_object_mut() {
        object.remove("commands");
    }
    report
}

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

fn parse_id(text: &str, what: &str) -> Result<fosscut_model::Id, ApiError> {
    fosscut_model::Id::parse(text)
        .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("{what}: {e}")))
}

fn required<T>(value: Option<T>, message: &str) -> Result<T, ApiError> {
    value.ok_or_else(|| ApiError::new(codes::INVALID_ARGUMENTS, message))
}

/// Where `clip` is: its timeline and the clip.
fn located<'a>(
    project: &'a fosscut_model::Project,
    clip: &fosscut_model::Id,
) -> Result<(&'a fosscut_model::Timeline, &'a fosscut_model::Clip), ApiError> {
    project
        .timelines
        .values()
        .find_map(|t| {
            t.tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|c| &c.id == clip)
                .map(|c| (t, c))
        })
        .ok_or_else(|| ApiError::new(codes::CLIP_NOT_FOUND, format!("clip {clip} does not exist")))
}

/// The envelope of a write a tool makes from proposed calls.
struct WriteOptions {
    expected_revision: Option<u64>,
    dry_run: bool,
    request_id: Option<String>,
    reason: Option<String>,
    actor: String,
}

impl WriteOptions {
    fn request(self, commands: Vec<Value>) -> Result<Request, ApiError> {
        let mut write: Request = serde_json::from_value(json!({ "commands": commands }))
            .map_err(|e| ApiError::new(codes::REQUEST_INVALID, e.to_string()))?;
        write.expected_revision = self.expected_revision;
        write.dry_run = self.dry_run;
        write.request_id = self.request_id;
        write.reason = self.reason;
        write.actor = self.actor;
        write.at_ms = project::now_ms();
        Ok(write)
    }
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
        description = "Import video, audio or image files into the project's media table in one revision. Returns each new asset id (asset_...), which clip.add uses. Importing a file that is already in the project returns its existing asset. With relink set to an asset id, the one file given becomes that asset's new place, for media that was moved or renamed (media_list marks missing files).",
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
        let relink = args.relink;
        let state = Arc::clone(&self.state);
        let envelope = (
            args.expected_revision,
            args.dry_run,
            args.request_id,
            args.reason,
        );
        let result = blocking(move || {
            let commands = match relink {
                Some(asset) => {
                    let [file] = files.as_slice() else {
                        return Err(ApiError::new(
                            codes::INVALID_ARGUMENTS,
                            "media_import: relink takes exactly one file",
                        ));
                    };
                    let asset = asset.parse().map_err(|e| {
                        ApiError::new(codes::INVALID_ARGUMENTS, format!("relink: {e}"))
                    })?;
                    vec![CommandCall {
                        command: "media.relink".into(),
                        args: fosscut_engine::import::media_relink_args(
                            &asset,
                            Path::new(file),
                            &bundle,
                        )?,
                    }]
                }
                None => files
                    .iter()
                    .map(|file| {
                        fosscut_engine::import::media_add_args(Path::new(file), None, &bundle).map(
                            |args| CommandCall {
                                command: "media.add".into(),
                                args,
                            },
                        )
                    })
                    .collect::<Result<Vec<_>, ApiError>>()?,
            };
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
        description = "List the project's media: asset ids, names, kinds, durations, stream details, bin folders, the clips that use each asset, and missing: true for files that cannot be found (relink them with media_import).",
        annotations(read_only_hint = true)
    )]
    async fn media_list(
        &self,
        params: Parameters<ProjectArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let bundle = self.bundle(params.0.project.clone());
        let listed = self.read(params.0.project, "media.list", Value::Null).await;
        reply(listed.map(|mut listing| {
            if let Ok(bundle) = bundle {
                fosscut_engine::import::mark_missing(&mut listing, &bundle);
            }
            listing
        }))
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
        description = "List the models FossCut can install, with their task (speech for media_analyze transcript; matte, pose, voice and translate for local vision, voice and translation models; runtime for the ONNX Runtime library those run on), size, license, attribution, languages and whether each is installed, or install one. Installing downloads it and checks its SHA-256, and happens only after the user agrees: tell them the size and license first, then pass confirm: true. Larger speech models are more accurate and slower; English-only models (.en) suit English speech.",
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
                        let text = match args.subtitles.as_deref() {
                            Some(name) => fosscut_engine::captions::SrtText::named(name)?,
                            None => fosscut_engine::captions::SrtText::Original,
                        };
                        let srt = fosscut_engine::captions::export_srt(
                            &snapshot,
                            timeline.as_ref(),
                            track.as_ref(),
                            text,
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
                                skip_fillers: args.skip_fillers,
                                lyrics: args.lyrics,
                            },
                        )?;
                        let paging = match (&args.paging, args.lyrics) {
                            (None, true) => Some(fosscut_engine::captions::lyrics_paging()),
                            (paging, _) => paging.clone(),
                        };
                        (generated.words, paging)
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
                    (
                        "preset",
                        args.preset
                            .map(|p| json!(p))
                            .or_else(|| args.lyrics.then(|| json!("karaoke"))),
                    ),
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

    /// Edits measured from media.
    #[tool(
        description = "Edits measured from media on this machine, applied as one undoable step with expected_revision, dry_run and request_id. action split_scenes splits clip at its shot cuts (measured once and cached), so each shot is a clip of its own. action stabilize measures how the camera moved in clip and keys its transform.position, rotation and scale to cancel the shake (strength 0 to 1, default 0.5; up to 3 for a locked-off look), with one zoom up to 1.5 that hides the moving edges; it replaces those keyframes. action track follows region of clip (centre and size as fractions of the picture as it shows at at, found with frame_grab) from at to the clip's end, keying either mask (a mask of the clip: its centre follows) or follow (another clip such as a title or sticker: it moves as the region does from where it is); where the region is lost its last position holds and lost lists the ranges. action multicam makes a multicam clip of angles (video assets recorded at the same time, at least two): a compound timeline with each angle on its own track, lined up by offsets measured from their sound (unless offsets are given or sync is false; max_offset_seconds limits the search), one angle's sound (sound, from 1), placed at start; cut between angles afterwards with edit and the command multicam.switch. action multicam_sync only measures the offsets and their confidence (below 0.3 is doubtful). Returns the measurement and the response; check with frame_grab or contact_sheet."
    )]
    async fn auto_edit(
        &self,
        params: Parameters<AutoEditArgs>,
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
                use fosscut_engine::motion::{self, Region, TrackTarget};
                let snapshot = state.projects.snapshot(&bundle)?;
                let clip = || {
                    let clip = required(args.clip.as_deref(), "this action needs clip")?;
                    parse_id(clip, "clip")
                };
                let angles = || {
                    args.angles
                        .iter()
                        .map(|a| parse_id(a, "angle"))
                        .collect::<Result<Vec<_>, _>>()
                };
                let max_offset = args.max_offset_seconds.unwrap_or(0.0);
                let mut progress = |_: fosscut_engine::Progress| true;
                let (key, report, commands) = match args.action {
                    AutoEditAction::SplitScenes => {
                        let found = fosscut_engine::scenes::split_at_scenes(
                            &snapshot,
                            bundle.clone(),
                            &clip()?,
                            fosscut_engine::cache_dir(),
                            fosscut_engine::models_dir(),
                            &mut progress,
                        )?;
                        let commands = found.commands.clone();
                        ("scenes", json!({ "clip": found.clip, "cuts": found.cuts }), commands)
                    }
                    AutoEditAction::Stabilize => {
                        let plan = motion::stabilize(
                            &snapshot,
                            &bundle,
                            &clip()?,
                            args.strength.unwrap_or(motion::DEFAULT_STRENGTH),
                            &mut progress,
                        )?;
                        let commands = plan.commands.clone();
                        ("motion", motion_summary(&plan), commands)
                    }
                    AutoEditAction::Track => {
                        let clip = clip()?;
                        let region = required(
                            args.region,
                            "track needs region: the centre and size to follow",
                        )?;
                        let target = match (&args.mask, &args.follow) {
                            (Some(mask), None) => TrackTarget::Mask(parse_id(mask, "mask")?),
                            (None, Some(other)) => TrackTarget::Clip(parse_id(other, "follow")?),
                            _ => {
                                return Err(ApiError::new(
                                    codes::INVALID_ARGUMENTS,
                                    "track needs either mask (a mask of the clip) or follow (another clip)",
                                ));
                            }
                        };
                        let (timeline, found) = located(&snapshot, &clip)?;
                        let at = match &args.at {
                            Some(at) => at.resolve(timeline.format.rate).map_err(|e| {
                                ApiError::new(codes::INVALID_ARGUMENTS, format!("at: {e}"))
                            })?,
                            None => found.start,
                        };
                        let plan = motion::track(
                            &snapshot,
                            &bundle,
                            &clip,
                            Region {
                                x: region.x,
                                y: region.y,
                                width: region.width,
                                height: region.height,
                            },
                            at,
                            &target,
                            &mut progress,
                        )?;
                        let commands = plan.commands.clone();
                        ("motion", motion_summary(&plan), commands)
                    }
                    AutoEditAction::MulticamSync => {
                        let found = motion::sync_offsets(&snapshot, &bundle, &angles()?, max_offset)?;
                        return Ok(json!({ "sync": found }));
                    }
                    AutoEditAction::Multicam => {
                        let mut measured = Value::Null;
                        let offsets: Vec<Value> = if !args.offsets.is_empty() {
                            args.offsets.iter().map(|o| json!(o)).collect()
                        } else if args.sync == Some(false) {
                            Vec::new()
                        } else {
                            let found =
                                motion::sync_offsets(&snapshot, &bundle, &angles()?, max_offset)?;
                            let offsets = found
                                .offsets
                                .iter()
                                .map(|o| json!({ "flicks": o.get() }))
                                .collect();
                            measured = json!(found);
                            offsets
                        };
                        let mut call = json!({ "angles": args.angles });
                        if !offsets.is_empty() {
                            call["offsets"] = json!(offsets);
                        }
                        for (name, value) in [
                            ("sound", args.sound.map(|s| json!(s))),
                            ("name", args.name.as_ref().map(|n| json!(n))),
                            ("start", args.start.as_ref().map(|t| json!(t))),
                            ("track", args.track.as_ref().map(|t| json!(t))),
                            ("timeline", args.timeline.as_ref().map(|t| json!(t))),
                        ] {
                            if let Some(value) = value {
                                call[name] = value;
                            }
                        }
                        (
                            "sync",
                            measured,
                            vec![json!({ "command": "multicam.create", "args": call })],
                        )
                    }
                };
                if commands.is_empty() {
                    return Ok(json!({ key: report, "committed": false }));
                }
                let write = WriteOptions {
                    expected_revision: args.expected_revision,
                    dry_run: args.dry_run,
                    request_id: args.request_id,
                    reason: args.reason,
                    actor,
                }
                .request(commands)?;
                let response = state.projects.execute(&bundle, write)?;
                let mut value = as_value(&response);
                if !report.is_null() {
                    value[key] = report;
                }
                Ok(value)
            })
            .await,
        )
    }

    /// Lists and adds library items.
    #[tool(
        description = "FossCut's own library, made by FossCut and free to use, also commercially: shapes, stickers (badges, buttons, arrows, bubbles), backgrounds (solid and gradient), project templates, title templates, text and caption styles, looks, sound effects and music loops. action list (with kind and search) lists items; action add puts one in the project: sounds and music are imported and placed at start, shapes, stickers and backgrounds drawn, titles and templates added with lines and media, styles and looks applied (looks need clip). One undoable step with expected_revision, dry_run and request_id (sounds are imported in a step of their own first)."
    )]
    async fn library(
        &self,
        params: Parameters<LibraryArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        // Listing needs no project.
        if args.action == LibraryAction::List {
            let items =
                fosscut_engine::library::search(args.kind.as_deref(), args.search.as_deref());
            return reply(Ok(json!({ "items": items })));
        }
        let bundle = match self.bundle(args.project.clone()) {
            Ok(bundle) => bundle,
            Err(error) => return Ok(failure(&error)),
        };
        let actor = actor(&context);
        let state = Arc::clone(&self.state);
        reply(
            blocking(move || {
                use fosscut_engine::library;
                let name = args.name.clone().ok_or_else(|| {
                    ApiError::new(codes::INVALID_ARGUMENTS, "library add needs name")
                })?;
                let item = library::find(&name, args.kind.as_deref())?;
                let envelope = |mut request: Request, expected: Option<u64>| {
                    request.expected_revision = expected;
                    request.dry_run = args.dry_run;
                    request.request_id = args.request_id.clone();
                    request.reason = args.reason.clone();
                    request.actor = actor.clone();
                    request.at_ms = project::now_ms();
                    request
                };
                let mut expected = args.expected_revision;
                let (command, call_args) = if matches!(item.kind, "sound" | "music") {
                    // The import is the first write and takes the revision check; placing
                    // follows on the revision it made.
                    let media_add = library::sound_media(&bundle, &item.name)?;
                    let mut import = Request::single("media.add", media_add);
                    import.expected_revision = args.expected_revision;
                    import.dry_run = args.dry_run;
                    import.actor = actor.clone();
                    import.at_ms = project::now_ms();
                    import.reason = args.reason.clone();
                    let imported = state.projects.execute(&bundle, import)?;
                    if args.dry_run {
                        let mut value = as_value(&imported);
                        value["item"] = json!(item);
                        value["note"] = json!("dry run: the sound would be imported, then placed");
                        return Ok(value);
                    }
                    expected = Some(imported.revision);
                    let mut placed = json!({ "asset": imported.results[0]["asset"] });
                    if let Some(start) = &args.start {
                        placed["start"] = json!(start);
                        if args.track.is_none() {
                            placed["free_track"] = json!(true);
                        }
                    }
                    if let Some(duration) = &args.duration {
                        placed["duration"] = json!(duration);
                    }
                    if let Some(track) = &args.track {
                        placed["track"] = json!(track);
                    }
                    ("clip.add".to_string(), placed)
                } else {
                    let call = library::command_for(
                        &item,
                        &library::Placement {
                            start: args.start.as_ref().map(|t| json!(t)),
                            duration: args.duration.as_ref().map(|t| json!(t)),
                            track: args.track.clone(),
                            clip: args.clip.clone(),
                            lines: args.lines.clone(),
                            media: args.media.clone(),
                        },
                    )?;
                    (
                        call["command"].as_str().unwrap_or_default().to_string(),
                        call["args"].clone(),
                    )
                };
                let request = envelope(Request::single(&command, call_args), expected);
                let response = state.projects.execute(&bundle, request)?;
                let mut value = as_value(&response);
                value["item"] = json!(item);
                Ok(value)
            })
            .await,
        )
    }

    /// Levels clips' loudness.
    #[tool(
        description = "Level the loudness of clips with sound: each clip is measured as it plays (its speed and sound effects applied) with the ITU-R BS.1770 meter on this machine, and its volume is set so it reaches target_lufs (default -14, the short-form platform level) with peaks under ceiling_dbfs (default -1). Where a clip's peaks leave too little room, or it needs more than the volume's +20 dB, an audio.limiter (gain and ceiling) makes up the rest unless limit is false. Silent clips keep their volume; clips with volume keyframes are refused. One undoable step with expected_revision, dry_run and request_id; returns each clip's measured loudness, peak, gain, new volume, limiter and the loudness it reaches."
    )]
    async fn audio_normalize(
        &self,
        params: Parameters<NormalizeArgs>,
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
                use fosscut_engine::loudness;
                let snapshot = state.projects.snapshot(&bundle)?;
                let clips = args
                    .clips
                    .iter()
                    .map(|c| {
                        fosscut_model::Id::parse(c).map_err(|e| {
                            ApiError::new(codes::INVALID_ARGUMENTS, format!("clip: {e}"))
                        })
                    })
                    .collect::<Result<Vec<_>, ApiError>>()?;
                let measured = loudness::normalize(
                    &snapshot,
                    &bundle,
                    &clips,
                    args.target_lufs.unwrap_or(loudness::DEFAULT_TARGET_LUFS),
                    args.ceiling_dbfs.unwrap_or(loudness::DEFAULT_CEILING_DBFS),
                    args.limit.unwrap_or(true),
                )?;
                if measured.commands.is_empty() {
                    return Ok(json!({ "normalization": measured, "committed": false }));
                }
                let mut write: Request =
                    serde_json::from_value(json!({ "commands": measured.commands }))
                        .map_err(|e| ApiError::new(codes::REQUEST_INVALID, e.to_string()))?;
                write.expected_revision = args.expected_revision;
                write.dry_run = args.dry_run;
                write.request_id = args.request_id;
                write.reason = args.reason;
                write.actor = actor;
                write.at_ms = project::now_ms();
                let response = state.projects.execute(&bundle, write)?;
                let mut value = as_value(&response);
                value["normalization"] = json!(measured);
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
        description = "Run any read-only query from capabilities by name with its arguments, for queries no other tool covers. preferences.read returns the user's preferences (editing and timeline defaults such as loudness_target, image_duration and frame_rate), so your requests can follow them; it needs no project, and only the user changes them.",
        annotations(read_only_hint = true)
    )]
    async fn query(&self, params: Parameters<QueryArgs>) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        if args.name == "preferences.read" {
            return reply(
                blocking(|| {
                    let file = fosscut_engine::preferences::path();
                    let preferences = fosscut_engine::preferences::Preferences::load(&file)?;
                    Ok(json!({ "path": file, "preferences": preferences }))
                })
                .await,
            );
        }
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
            let format = project.timelines[&timeline].format.clone();
            let rate = format.rate;
            let choice = fosscut_engine::SettingsChoice {
                resolution: args
                    .resolution
                    .as_deref()
                    .map(fosscut_engine::resolution)
                    .transpose()?,
                rate: args
                    .fps
                    .as_deref()
                    .map(fosscut_engine::frame_rate)
                    .transpose()?,
                video_bit_rate: args
                    .bitrate
                    .as_deref()
                    .map(fosscut_engine::bit_rate)
                    .transpose()?,
                audio_bit_rate: args
                    .audio_bitrate
                    .as_deref()
                    .map(fosscut_engine::bit_rate)
                    .transpose()?,
            };
            let time = |input: &fosscut_time::TimeInput| {
                input
                    .resolve(rate)
                    .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("time: {e}")))
            };
            let options = fosscut_engine::ExportOptions {
                range: match &args.range {
                    Some(range) => Some((time(&range.start)?, time(&range.end)?)),
                    None => None,
                },
                cover: args.cover.as_ref().map(time).transpose()?,
                subtitles: args
                    .subtitles
                    .as_deref()
                    .map(fosscut_engine::SubtitleFormat::named)
                    .transpose()?,
            };
            let request = ExportRequest {
                bundle: bundle.clone(),
                settings: fosscut_engine::settings_for(preset, &format, &choice),
                timeline,
                output,
                allow_overwrite: args.allow_destructive,
                expected_revision: args.expected_revision,
                request_id: args.request_id,
                cache: state.cache.clone(),
                options,
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

    /// Records a voiceover, a camera or the screen.
    #[tool(
        description = "Record a voiceover, a camera or the screen of this machine into the project's recordings folder. Only when the user asked you to record: start needs confirm true. action devices lists the microphones, cameras and screens. action start (what: microphone, camera or screen; device and microphone by name, part of a name or number from devices; duration, size, fps) starts a background job and returns it: follow it with job_status, stop it with job_cancel (the recording is kept), and when it succeeded import recording.output with media_import, then place it with edit (clip.add). A microphone records 48 kHz stereo WAV; camera and screen the most compatible video this machine encodes."
    )]
    async fn record(&self, params: Parameters<RecordArgs>) -> Result<CallToolResult, ErrorData> {
        let args = params.0;
        let state = Arc::clone(&self.state);
        let bundle = match args.action {
            RecordAction::Devices => None,
            RecordAction::Start => match self.bundle(args.project.clone()) {
                Ok(bundle) => Some(bundle),
                Err(error) => return Ok(failure(&error)),
            },
        };
        reply(
            blocking(move || {
                use fosscut_engine::record::{self, CaptureInput, RecordKind, RecordRequest};
                let Some(bundle) = bundle else {
                    return Ok(json!({ "devices": record::devices()? }));
                };
                if !args.confirm {
                    return Err(ApiError::new(
                        "fosscut.record.confirmation_required",
                        "recording uses the user's microphone, camera or screen: start it only when they asked, with confirm true",
                    ));
                }
                let what = args
                    .what
                    .as_deref()
                    .and_then(RecordKind::parse)
                    .ok_or_else(|| {
                        ApiError::new(
                            codes::INVALID_ARGUMENTS,
                            "start needs what: microphone, camera or screen",
                        )
                    })?;
                let output = record::default_output(&bundle, what, std::time::SystemTime::now());
                let mut request = RecordRequest::new(what, output);
                request.device = args.device;
                request.microphone = args.microphone;
                request.input = args
                    .input
                    .as_deref()
                    .map(|text| {
                        let (format, input) = text.split_once(':').ok_or_else(|| {
                            ApiError::new(
                                codes::INVALID_ARGUMENTS,
                                "input: give FORMAT:INPUT, for example lavfi:sine",
                            )
                        })?;
                        Ok::<_, ApiError>(CaptureInput {
                            format: format.into(),
                            input: input.into(),
                            options: Vec::new(),
                        })
                    })
                    .transpose()?;
                request.duration = args
                    .duration
                    .as_ref()
                    .map(|d| d.resolve(Rate::FPS_30))
                    .transpose()
                    .map_err(|e| {
                        ApiError::new(codes::INVALID_ARGUMENTS, format!("duration: {e}"))
                    })?;
                request.size = args.size.map(|[w, h]| (w, h));
                request.rate = args
                    .fps
                    .as_deref()
                    .map(fosscut_engine::frame_rate)
                    .transpose()?;
                let status = state.jobs.start_recording(request)?;
                Ok(as_value(&status))
            })
            .await,
        )
    }

    /// Reports an export job.
    #[tool(
        description = "The state of a job from export, media_analyze or record (running, succeeded, failed or cancelled) with progress; when it succeeded, the verified export report, the analysis or the recording; when it failed, the typed error. wait_seconds (up to 60) waits for the job to finish before answering.",
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
        description = "Cancel a running export or analysis job (nothing is written), or stop a recording (what was recorded is kept and the job succeeds). A finished job is not affected. wait_seconds (up to 60) waits for it to stop.",
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
