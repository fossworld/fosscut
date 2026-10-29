//! FossCut's MCP server (ADR-0002): a small set of coarse tools over the engine, so any MCP
//! client (Claude Code, Codex, Cursor and others) can edit, check and export projects.
//! `fosscut mcp` serves it on stdio.
//!
//! - One generic `edit` tool runs any command from `capabilities` as an atomic batch with
//!   the agent safety contract (ADR-0006); a few dedicated tools cover what most sessions
//!   need: projects, timeline reads, media import, frame grabs, contact sheets, export jobs.
//! - Every write takes the bundle's lock for one request and reopens the project, so the
//!   CLI and this server can work on one project in turn. Writes are logged as made by
//!   `agent:<client name>`.
//! - FossCut errors are tool results with `isError` set and the typed
//!   `{"error": {code, message, hint, details}}`, so agents can recover.

#![forbid(unsafe_code)]

mod params;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine;
use fosscut_core::{ApiError, CommandCall, REDO, Request, UNDO, codes};
use fosscut_engine::jobs::Jobs;
use fosscut_engine::{ExportRequest, Preset, project};
use fosscut_model::TimelineFormat;
use fosscut_render::Renderer;
use fosscut_time::Rate;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde_json::{Value, json};

use params::{
    CapabilitiesArgs, Detail, EditArgs, ExportArgs, FrameArgs, HistoryArgs, HistoryStepArgs,
    JobArgs, MediaImportArgs, ProjectArgs, ProjectNewArgs, ProjectOpenArgs, QueryArgs, SheetArgs,
    TimelineReadArgs,
};

/// What the server tells agents when they connect.
const INSTRUCTIONS: &str = "\
FossCut edits video projects (.fosscut bundles). A safe session:
1. project_open (or project_new) once; later tools use that project.
2. timeline_read shows tracks and clips in frames, timecode and seconds, and the revision.
3. edit applies commands from capabilities as one atomic, undoable step. Pass expected_revision from your last read, use dry_run first for large changes and a request_id so retries are safe. Ask the user before setting allow_destructive.
4. Check the result with frame_grab or contact_sheet before exporting.
5. export starts a background job; call job_status with wait_seconds until it succeeds, then read its verification report.
Times need a unit: {\"frames\": n}, {\"seconds\": \"1.5\"}, {\"timecode\": \"HH:MM:SS:FF\"} or {\"flicks\": n}.
Errors come back as {\"error\": {code, message, hint, details}}: follow the hint.";

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
    jobs: Jobs,
    renderer: Mutex<Option<Renderer>>,
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

impl FosscutServer {
    /// A server whose tools use `project` when they are given none.
    pub fn new(project: Option<PathBuf>) -> FosscutServer {
        FosscutServer {
            state: Arc::new(State {
                project: Mutex::new(project),
                jobs: Jobs::new(),
                renderer: Mutex::new(None),
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
        blocking(move || project::query(&bundle, name, args)).await
    }

    async fn execute(&self, project: Option<String>, request: Request) -> Result<Value, ApiError> {
        let bundle = self.bundle(project)?;
        blocking(move || project::execute(&bundle, request).map(|response| as_value(&response)))
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
        description = "List every editing command and query with a summary (detail: full adds each command's JSON Schema and all error codes), the engine operations, and the export presets this machine can encode. Read it before your first edit.",
        annotations(read_only_hint = true)
    )]
    async fn capabilities(
        &self,
        params: Parameters<CapabilitiesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let detail = params.0.detail;
        let bundle = lock(&self.state.project).clone();
        reply(
            blocking(move || fosscut_engine::capabilities(detail.as_str(), bundle.as_deref()))
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
        let result = blocking(move || project::query(&opened, "project.info", Value::Null)).await;
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
        reply(
            self.read(params.0.project, "project.info", Value::Null)
                .await,
        )
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
            project::execute(&bundle, request).map(|response| as_value(&response))
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
        reply(blocking(move || project::query(&bundle, &name, query_args)).await)
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
            let session = project::open(&bundle)?;
            let project = session.project();
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
            let session = project::open(&bundle)?;
            let project = session.project();
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
            let session = project::open(&bundle)?;
            let project = session.project().clone();
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
                cache: fosscut_engine::cache_dir(),
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
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("fosscut", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}
