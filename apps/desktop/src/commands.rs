//! What the window may ask of the app (ADR-0010, item 4): the engine client in `ui/` calls
//! only these. Errors are the typed FossCut errors, as the CLI and MCP return them.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fosscut_core::{ApiError, Request, Response, codes};
use fosscut_engine::jobs::{JobStatus, Jobs};
use fosscut_engine::live::ClientInfo;
use fosscut_engine::{ExportOptions, ExportRequest, Preset, SettingsChoice, SubtitleFormat};
use fosscut_model::{Id, Project};
use fosscut_time::Flicks;
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;
use crate::library::{ProjectSummary, TrashEntry};
use crate::preferences::{self as preferred, PreferencesView, UpdateAnswer};
use crate::preview::Job;
use crate::recording::{RecordChoice, RecordState};
use crate::words::{Word, word};
use crate::workspace::{NewProject, Opened, Workspace};
use fosscut_engine::preferences::Preferences;

type Answer<T> = Result<T, ApiError>;

/// How long `export_cancel` waits for the export to stop before answering.
const CANCEL_WAIT: Duration = Duration::from_secs(2);

pub(crate) fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// The open project's bundle, its state now and its primary timeline.
pub(crate) fn open_project(workspace: &Workspace) -> Answer<(PathBuf, Project, Id)> {
    workspace
        .snapshot()
        .ok_or_else(|| invalid("no project is open").with_hint("Open or create a project first."))
}

/// A text the window sent, or `None` when it sent nothing or only spaces.
pub(crate) fn given(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|t| !t.is_empty())
}

/// Runs blocking work (opening a project may wait for a writer) off the main thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Answer<T> + Send + 'static,
) -> Answer<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .unwrap_or_else(|e| {
            Err(ApiError::new(
                codes::INTERNAL,
                format!("the app stopped: {e}"),
            ))
        })
}

/// The app's name and version, and where live sessions are announced.
#[tauri::command]
pub fn app_info() -> Value {
    json!({
        "name": crate::workspace::HOST_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "runtime_dir": fosscut_engine::live::runtime_dir(),
    })
}

/// Recently opened projects that still exist, newest first.
#[tauri::command]
pub fn recent_projects(state: State<'_, AppState>) -> Vec<PathBuf> {
    state.workspace.recent()
}

/// The open project, if any.
#[tauri::command]
pub fn project_current(state: State<'_, AppState>) -> Answer<Option<Opened>> {
    state.workspace.current()
}

/// Opens the project at `path`.
#[tauri::command]
pub async fn project_open(
    app: AppHandle,
    state: State<'_, AppState>,
    path: PathBuf,
) -> Answer<Opened> {
    let workspace = Arc::clone(&state.workspace);
    let opened = blocking(move || workspace.open(&path)).await?;
    crate::follow(&app, &state);
    Ok(opened)
}

/// Asks for a project folder and opens it; `None` when the user cancels.
#[tauri::command]
pub async fn project_pick(app: AppHandle, state: State<'_, AppState>) -> Answer<Option<Opened>> {
    let dialog = app.clone();
    let workspace = Arc::clone(&state.workspace);
    let preferences = state.preferences.current();
    let opened = blocking(move || {
        let mut pick = dialog
            .dialog()
            .file()
            .set_title(word(&preferences.language, Word::OpenProject));
        if let Some(folder) = preferred::projects_folder(&preferences, preferred::home().as_deref())
        {
            pick = pick.set_directory(folder);
        }
        let Some(folder) = pick.blocking_pick_folder() else {
            return Ok(None);
        };
        let path = folder
            .into_path()
            .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, e.to_string()))?;
        workspace.open(&path).map(Some)
    })
    .await?;
    if opened.is_some() {
        crate::follow(&app, &state);
    }
    Ok(opened)
}

/// Asks where to save a new project (in the preferred projects folder), creates it with
/// `width`, `height` and `rate` (1920 by 1080 at the preferred frame rate when omitted),
/// and opens it; `None` when the user cancels.
#[tauri::command]
pub async fn project_new(
    app: AppHandle,
    state: State<'_, AppState>,
    width: Option<u32>,
    height: Option<u32>,
    rate: Option<String>,
) -> Answer<Option<Opened>> {
    let dialog = app.clone();
    let workspace = Arc::clone(&state.workspace);
    let preferences = state.preferences.current();
    let opened = blocking(move || {
        let mut save = dialog
            .dialog()
            .file()
            .set_title(word(&preferences.language, Word::CreateProject))
            .set_file_name("Untitled.fosscut");
        if let Some(folder) = preferred::projects_folder(&preferences, preferred::home().as_deref())
        {
            save = save.set_directory(folder);
        }
        let Some(file) = save.blocking_save_file() else {
            return Ok(None);
        };
        let path = file
            .into_path()
            .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, e.to_string()))?;
        let new = new_project(path, width, height, rate.as_deref(), &preferences);
        workspace.create(&new).map(Some)
    })
    .await?;
    if opened.is_some() {
        crate::follow(&app, &state);
    }
    Ok(opened)
}

/// A new project at `path` (with the `.fosscut` extension), `width` by `height` (1920 by
/// 1080 when omitted) at `rate`, or at the preferred frame rate when omitted.
fn new_project(
    mut path: PathBuf,
    width: Option<u32>,
    height: Option<u32>,
    rate: Option<&str>,
    preferences: &Preferences,
) -> NewProject {
    if path.extension().is_none_or(|ext| ext != "fosscut") {
        path.set_extension("fosscut");
    }
    NewProject {
        path,
        name: None,
        width: width.unwrap_or(1920),
        height: height.unwrap_or(1080),
        rate: preferred::new_project_rate(preferences, rate),
    }
}

/// Closes the open project; agents then edit it on disk again.
#[tauri::command]
pub fn project_close(app: AppHandle, state: State<'_, AppState>) {
    state.workspace.close();
    state.preview.send(Job::Scene(None));
    let _ = app.emit("project-closed", ());
}

/// Runs a query on the open project.
#[tauri::command]
pub async fn query(state: State<'_, AppState>, name: String, args: Option<Value>) -> Answer<Value> {
    let workspace: Arc<Workspace> = Arc::clone(&state.workspace);
    blocking(move || read(&workspace, &name, args.unwrap_or(Value::Null))).await
}

/// What `query` answers for the project open in `workspace`, with what the window adds.
fn read(workspace: &Workspace, name: &str, args: Value) -> Answer<Value> {
    let mut value = workspace.query(name, args)?;
    // The window offers to relink media whose files are gone.
    if name == "media.list"
        && let Some((bundle, _, _)) = workspace.snapshot()
    {
        fosscut_engine::import::mark_missing(&mut value, &bundle);
    }
    Ok(value)
}

/// Shows an asset's file in the system's file manager.
#[tauri::command]
pub async fn media_reveal(state: State<'_, AppState>, asset: String) -> Answer<()> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || {
        let (bundle, project, _) = workspace
            .snapshot()
            .ok_or_else(|| ApiError::new(codes::INVALID_ARGUMENTS, "no project is open"))?;
        let asset = asset
            .parse()
            .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("{asset}: {e}")))?;
        let file = fosscut_engine::import::asset_file(&project, &bundle, &asset)?;
        reveal(&file).map_err(|e| {
            ApiError::new(
                codes::INVALID_ARGUMENTS,
                format!("cannot show {}: {e}", file.display()),
            )
        })
    })
    .await
}

/// Opens the file manager at `file`, selected where the system allows.
fn reveal(file: &std::path::Path) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut open = std::process::Command::new("open");
        open.arg("-R").arg(file);
        open
    } else if cfg!(windows) {
        let mut explorer = std::process::Command::new("explorer");
        explorer.arg(format!("/select,{}", file.display()));
        explorer
    } else {
        let mut open = std::process::Command::new("xdg-open");
        open.arg(file.parent().unwrap_or(file));
        open
    };
    command.spawn().map(|_| ())
}

/// Asks where an asset's file is now and relinks it; `None` when the user cancels.
#[tauri::command]
pub async fn media_relink(
    app: AppHandle,
    state: State<'_, AppState>,
    asset: String,
) -> Answer<Option<Response>> {
    let dialog = app.clone();
    let workspace = Arc::clone(&state.workspace);
    let language = state.preferences.current().language;
    blocking(move || {
        let (bundle, _, _) = workspace
            .snapshot()
            .ok_or_else(|| ApiError::new(codes::INVALID_ARGUMENTS, "no project is open"))?;
        let Some(file) = dialog
            .dialog()
            .file()
            .set_title(word(&language, Word::FindMedia))
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let path = file
            .into_path()
            .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, e.to_string()))?;
        let id = asset
            .parse()
            .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("{asset}: {e}")))?;
        let args = fosscut_engine::import::media_relink_args(&id, &path, &bundle)?;
        workspace
            .execute(Request::single("media.relink", args))
            .map(Some)
    })
    .await
}

/// Applies the user's edit as one undoable step.
#[tauri::command]
pub async fn execute(state: State<'_, AppState>, request: Request) -> Answer<Response> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || workspace.execute(request)).await
}

/// Measures a clip (and a reference, for a match) and returns the `effect.add` arguments
/// of the color correction; the window applies them with `execute`, as any edit.
#[tauri::command]
pub async fn color_suggest(
    state: State<'_, AppState>,
    clip: String,
    reference: Option<String>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || {
        let parse = |text: &str| {
            fosscut_model::Id::parse(text).map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, e))
        };
        let clip = parse(&clip)?;
        let reference = reference.as_deref().map(parse).transpose()?;
        let (bundle, project, _) = workspace
            .snapshot()
            .ok_or_else(|| ApiError::new(codes::INVALID_ARGUMENTS, "no project is open"))?;
        let mut renderer = fosscut_render::Renderer::headless()
            .map_err(|e| ApiError::new(e.code(), e.to_string()))?;
        let suggestion = fosscut_engine::color::suggest(
            &project,
            &bundle,
            &clip,
            reference.as_ref(),
            &mut renderer,
        )
        .map_err(ApiError::from)?;
        Ok(json!({
            "effect_add": suggestion.effect_add,
            "measured": suggestion.stats,
            "reference": suggestion.reference,
        }))
    })
    .await
}

/// Measures the loudness of `clips` and returns the volumes that bring each to
/// `target_lufs` (the preferred `editing.loudness_target` when omitted, -14 LUFS unless
/// changed) with peaks under -1 dBFS, with the `clip.set` calls that apply them; the
/// window applies those with `execute`, as any edit.
#[tauri::command]
pub async fn audio_normalize(
    state: State<'_, AppState>,
    clips: Vec<String>,
    target_lufs: Option<f64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    let target = preferred::loudness_target(&state.preferences.current(), target_lufs);
    blocking(move || normalize(&workspace, &clips, target)).await
}

/// What `audio_normalize` answers for the project open in `workspace`.
fn normalize(workspace: &Workspace, clips: &[String], target_lufs: f64) -> Answer<Value> {
    let clips = clips
        .iter()
        .map(|text| {
            fosscut_model::Id::parse(text).map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, e))
        })
        .collect::<Answer<Vec<_>>>()?;
    let (bundle, project, _) = workspace
        .snapshot()
        .ok_or_else(|| ApiError::new(codes::INVALID_ARGUMENTS, "no project is open"))?;
    let normalization = fosscut_engine::loudness::normalize(
        &project,
        &bundle,
        &clips,
        target_lufs,
        fosscut_engine::loudness::DEFAULT_CEILING_DBFS,
        true,
    )
    .map_err(ApiError::from)?;
    serde_json::to_value(normalization).map_err(|e| ApiError::new(codes::INTERNAL, e.to_string()))
}

/// An id the window sent, named `what` in the error when it is not one.
pub(crate) fn parse_id(text: &str, what: &str) -> Answer<Id> {
    Id::parse(text).map_err(|e| invalid(format!("{what}: {e}")))
}

/// Refuses before measuring when the project moved on from the revision the window shows,
/// as the write that follows would.
pub(crate) fn ensure_revision(project: &Project, expected: Option<u64>) -> Answer<()> {
    let current = project.manifest.revision;
    match expected {
        Some(expected) if expected != current => Err(ApiError::new(
            codes::REVISION_CONFLICT,
            format!("the project is at revision {current}, not {expected}"),
        )
        .with_details(json!({ "expected_revision": expected, "current_revision": current }))),
        _ => Ok(()),
    }
}

/// Applies the calls a measurement proposed as the user's one undoable step, against the
/// revision the window shows when it gives one; nothing when it proposed none.
pub(crate) fn apply_measured(
    workspace: &Workspace,
    commands: Vec<Value>,
    expected_revision: Option<u64>,
    reason: &str,
) -> Answer<Option<Response>> {
    if commands.is_empty() {
        return Ok(None);
    }
    let mut request: Request = serde_json::from_value(json!({ "commands": commands }))
        .map_err(|e| ApiError::new(codes::INTERNAL, e.to_string()))?;
    request.expected_revision = expected_revision;
    request.reason = Some(reason.to_string());
    workspace.execute(request).map(Some)
}

/// The measurements take no progress reports from the window; they run to the end.
pub(crate) fn no_progress(_: fosscut_engine::Progress) -> bool {
    true
}

/// Splits a video clip at its shot cuts (measured once, then cached) as one undoable
/// step. Answers `{scenes: {clip, cuts}, response}`: the cuts in timeline flicks, and the
/// response, null when the clip has no cuts.
#[tauri::command]
pub async fn scenes_split(
    state: State<'_, AppState>,
    clip: String,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || split_scenes(&workspace, &clip, expected_revision)).await
}

/// What `scenes_split` does for the project open in `workspace`.
fn split_scenes(
    workspace: &Workspace,
    clip: &str,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let clip = parse_id(clip, "clip")?;
    let (bundle, project, _) = open_project(workspace)?;
    ensure_revision(&project, expected_revision)?;
    let found = fosscut_engine::scenes::split_at_scenes(
        &project,
        bundle,
        &clip,
        fosscut_engine::cache_dir(),
        fosscut_engine::models_dir(),
        &mut no_progress,
    )?;
    let response = apply_measured(
        workspace,
        found.commands,
        expected_revision,
        "Split at scenes",
    )?;
    Ok(json!({
        "scenes": { "clip": found.clip, "cuts": found.cuts },
        "response": response,
    }))
}

/// A motion plan without its calls, which the response shows applied.
fn motion_report(plan: &fosscut_engine::motion::MotionPlan) -> Value {
    let mut report = json!(plan);
    if let Some(object) = report.as_object_mut() {
        object.remove("commands");
    }
    report
}

/// Measures how the camera moved through a video clip and keys its position, rotation and
/// scale to steady it, as one undoable step. `strength` runs from 0 (no change) to 1 (very
/// smooth), 0.5 when omitted; up to 3 is taken for a locked-off look. Answers `{motion:
/// {clip, frames, zoom}, response}`: the zoom that hides the moving edges, from 1 to 1.5.
#[tauri::command]
pub async fn clip_stabilize(
    state: State<'_, AppState>,
    clip: String,
    strength: Option<f64>,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || stabilize(&workspace, &clip, strength, expected_revision)).await
}

/// What `clip_stabilize` does for the project open in `workspace`.
fn stabilize(
    workspace: &Workspace,
    clip: &str,
    strength: Option<f64>,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let clip = parse_id(clip, "clip")?;
    let (bundle, project, _) = open_project(workspace)?;
    ensure_revision(&project, expected_revision)?;
    let plan = fosscut_engine::motion::stabilize(
        &project,
        &bundle,
        &clip,
        strength.unwrap_or(fosscut_engine::motion::DEFAULT_STRENGTH),
        &mut no_progress,
    )?;
    let report = motion_report(&plan);
    let response = apply_measured(workspace, plan.commands, expected_revision, "Stabilize")?;
    Ok(json!({ "motion": report, "response": response }))
}

/// What to track and what follows it (`clip_track`).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackRequest {
    /// The video clip whose picture is followed.
    pub clip: String,
    /// The region to follow: centre and size as fractions of the picture as displayed.
    pub region: fosscut_engine::motion::Region,
    /// The timeline time the region was drawn at, in flicks, inside the clip.
    pub at: i64,
    /// A mask of the clip, whose centre then follows the region.
    #[serde(default)]
    pub mask: Option<String>,
    /// Another clip, such as a title or sticker, which then moves as the region does.
    #[serde(default)]
    pub follow: Option<String>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    #[serde(default)]
    pub expected_revision: Option<u64>,
}

/// Follows a region of a video clip's picture from `at` to the clip's end and keys one of
/// its masks, or another clip, to it as one undoable step. Answers `{motion: {clip,
/// frames, lost}, response}`: `clip` is the one keyed, and `lost` the timeline ranges
/// (start and end in flicks) where the region was lost and its last place held.
#[tauri::command]
pub async fn clip_track(state: State<'_, AppState>, request: TrackRequest) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || track(&workspace, &request)).await
}

/// What `clip_track` does for the project open in `workspace`.
fn track(workspace: &Workspace, request: &TrackRequest) -> Answer<Value> {
    use fosscut_engine::motion::TrackTarget;
    let clip = parse_id(&request.clip, "clip")?;
    let target = match (
        given(request.mask.as_deref()),
        given(request.follow.as_deref()),
    ) {
        (Some(mask), None) => TrackTarget::Mask(parse_id(mask, "mask")?),
        (None, Some(other)) => TrackTarget::Clip(parse_id(other, "follow")?),
        _ => {
            return Err(invalid(
                "tracking needs either a mask of the clip or another clip to follow the region",
            )
            .with_hint("Choose one of the clip's masks, or a title, sticker or clip to follow."));
        }
    };
    let (bundle, project, _) = open_project(workspace)?;
    ensure_revision(&project, request.expected_revision)?;
    let plan = fosscut_engine::motion::track(
        &project,
        &bundle,
        &clip,
        request.region,
        Flicks::new(request.at),
        &target,
        &mut no_progress,
    )?;
    let report = motion_report(&plan);
    let response = apply_measured(workspace, plan.commands, request.expected_revision, "Track")?;
    Ok(json!({ "motion": report, "response": response }))
}

/// Makes a multicam clip (ADR-0029) of `angles`, video assets recorded at the same time, as
/// one undoable step: lined up by offsets measured from their sound unless `sync` is false
/// (then they all start together), with angle `sound`'s sound (from 1; the first when
/// omitted), named `name`, starting at `start` flicks (after the track's last clip when
/// omitted). Answers `{sync: {offsets, confidence} | null, response}`: each angle's offset
/// in flicks and how sure each match is, from 0 to 1 (below 0.3 is doubtful).
#[tauri::command]
pub async fn multicam_create(
    state: State<'_, AppState>,
    angles: Vec<String>,
    sync: Option<bool>,
    sound: Option<usize>,
    name: Option<String>,
    start: Option<i64>,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    let request = MulticamRequest {
        angles,
        sync: sync.unwrap_or(true),
        sound,
        name,
        start,
        expected_revision,
    };
    blocking(move || create_multicam(&workspace, &request)).await
}

/// What `multicam_create` is asked.
#[derive(Clone, Debug, Default)]
struct MulticamRequest {
    angles: Vec<String>,
    sync: bool,
    sound: Option<usize>,
    name: Option<String>,
    start: Option<i64>,
    expected_revision: Option<u64>,
}

/// What `multicam_create` does for the project open in `workspace`.
fn create_multicam(workspace: &Workspace, request: &MulticamRequest) -> Answer<Value> {
    let ids = request
        .angles
        .iter()
        .map(|angle| parse_id(angle, "angle"))
        .collect::<Answer<Vec<_>>>()?;
    let (bundle, project, _) = open_project(workspace)?;
    let mut args = json!({ "angles": ids });
    if let Some(sound) = request.sound {
        args["sound"] = json!(sound);
    }
    if let Some(name) = given(request.name.as_deref()) {
        args["name"] = json!(name);
    }
    if let Some(start) = request.start {
        args["start"] = json!({ "flicks": start });
    }
    let call = |args: &Value| json!({ "command": "multicam.create", "args": args });
    // A dry run first: the core refuses what it would refuse (too few angles, assets that
    // are not video, a stale revision, no room at the start) before any sound is read.
    let mut check: Request = serde_json::from_value(json!({ "commands": [call(&args)] }))
        .map_err(|e| ApiError::new(codes::INTERNAL, e.to_string()))?;
    check.expected_revision = request.expected_revision;
    check.dry_run = true;
    workspace.execute(check)?;
    let measured = if request.sync {
        let found = fosscut_engine::motion::sync_offsets(&project, &bundle, &ids, 0.0)?;
        args["offsets"] = found
            .offsets
            .iter()
            .map(|offset| json!({ "flicks": offset.get() }))
            .collect();
        Some(found)
    } else {
        None
    };
    let response = apply_measured(
        workspace,
        vec![call(&args)],
        request.expected_revision,
        "Create multicam",
    )?;
    Ok(json!({ "sync": measured, "response": response }))
}

/// How long `record_stop` waits for the recording's file before answering.
const RECORD_STOP_WAIT: Duration = Duration::from_secs(5);

/// The capture devices on this machine: cameras, screens and microphones, each with its
/// `kind`, `name`, input `format` and `input`.
#[tauri::command]
pub async fn record_devices() -> Answer<Vec<fosscut_engine::record::Device>> {
    blocking(|| Ok(fosscut_engine::record::devices()?)).await
}

/// Starts recording a microphone, camera or screen into the preferred recordings folder,
/// or the open project's `recordings/` folder, in the background, after choosing the
/// devices and settings, and answers the job at once. Once it succeeds, `record_status`
/// or `record_stop` imports the file and places it at `choice.at` (the playhead it
/// started from), once, and a notification says so when the preferences allow.
#[tauri::command]
pub async fn record_start(
    app: AppHandle,
    state: State<'_, AppState>,
    choice: RecordChoice,
) -> Answer<RecordState> {
    let workspace = Arc::clone(&state.workspace);
    let jobs = Arc::clone(&state.jobs);
    let recordings = Arc::clone(&state.recordings);
    let folder = preferred::recordings_folder(&state.preferences.current());
    let watched = Arc::clone(&jobs);
    let started =
        blocking(move || recordings.start_in(&workspace, &jobs, &choice, folder.as_deref()))
            .await?;
    crate::announce::watch(&app, &watched, started.status.job.clone());
    Ok(started)
}

/// A recording's status (`progress.done` is the whole seconds recorded) and, once it
/// succeeded, the report in `recording` and the `asset` and `clip` it became.
#[tauri::command]
pub async fn record_status(state: State<'_, AppState>, job: String) -> Answer<RecordState> {
    let workspace = Arc::clone(&state.workspace);
    let jobs = Arc::clone(&state.jobs);
    let recordings = Arc::clone(&state.recordings);
    blocking(move || recordings.status(&workspace, &jobs, &job)).await
}

/// Stops a recording, keeping what it recorded: the job succeeds with its file, which is
/// imported and placed as `record_status` does. Answers once the file is ready, or after
/// a few seconds while it is still being finished.
#[tauri::command]
pub async fn record_stop(state: State<'_, AppState>, job: String) -> Answer<RecordState> {
    let workspace = Arc::clone(&state.workspace);
    let jobs = Arc::clone(&state.jobs);
    let recordings = Arc::clone(&state.recordings);
    blocking(move || recordings.stop(&workspace, &jobs, &job, RECORD_STOP_WAIT)).await
}

/// The agents and other front ends connected to the open project.
#[tauri::command]
pub fn clients(state: State<'_, AppState>) -> Vec<ClientInfo> {
    state.workspace.clients()
}

/// Where the preview shows through the page, in CSS pixels; scaled to the window's pixels
/// here.
#[tauri::command]
pub fn preview_viewport(
    app: AppHandle,
    state: State<'_, AppState>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    let scale = tauri::Manager::get_webview_window(&app, "main")
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(1.0) as f32;
    state.preview.send(Job::Viewport([
        x * scale,
        y * scale,
        width * scale,
        height * scale,
    ]));
}

/// Shows the frame at `flicks`, pausing playback. With `scrub` (the window scrubs or
/// skims), a moment of its sound plays too while `timeline.scrub_audio` is on.
#[tauri::command]
pub fn preview_show(state: State<'_, AppState>, flicks: i64, scrub: Option<bool>) {
    state.preview.send(show_job(
        Flicks::new(flicks),
        scrub.unwrap_or(false),
        &state.preferences.current(),
    ));
}

/// What the preview is asked to show the frame at `time` with.
fn show_job(time: Flicks, scrub: bool, preferences: &Preferences) -> Job {
    if scrub && preferences.timeline.scrub_audio {
        Job::Scrub(time)
    } else {
        Job::Show(time)
    }
}

/// Plays from the playhead.
#[tauri::command]
pub fn preview_play(state: State<'_, AppState>) {
    state.preview.send(Job::Play);
}

/// Pauses playback.
#[tauri::command]
pub fn preview_pause(state: State<'_, AppState>) {
    state.preview.send(Job::Pause);
}

/// Shuttles at `rate` from the playhead: 2 is twice as fast, -1 backwards, 0 pauses.
#[tauri::command]
pub fn preview_shuttle(state: State<'_, AppState>, rate: f64) {
    state.preview.send(Job::Shuttle(rate));
}

/// Puts the window in full screen, or back.
#[tauri::command]
pub fn window_fullscreen(app: AppHandle, on: bool) {
    if let Some(window) = tauri::Manager::get_webview_window(&app, "main") {
        let _ = window.set_fullscreen(on);
    }
}

/// Summaries of the recent projects for the browser, with thumbnails when `thumbnails`.
#[tauri::command]
pub async fn project_summaries(
    state: State<'_, AppState>,
    thumbnails: bool,
) -> Answer<Vec<ProjectSummary>> {
    let workspace = Arc::clone(&state.workspace);
    let library = Arc::clone(&state.library);
    blocking(move || {
        Ok(workspace
            .recent()
            .iter()
            .filter_map(|path| library.summary(path, thumbnails).ok())
            .collect())
    })
    .await
}

/// Moves a project to the trash; never the open one.
#[tauri::command]
pub async fn project_trash(state: State<'_, AppState>, path: PathBuf) -> Answer<TrashEntry> {
    let workspace = Arc::clone(&state.workspace);
    let library = Arc::clone(&state.library);
    blocking(move || {
        if workspace
            .snapshot()
            .is_some_and(|(bundle, _, _)| bundle == path)
        {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "close the project before moving it to the trash",
            ));
        }
        library.trash(&path)
    })
    .await
}

/// The projects in the trash, newest first.
#[tauri::command]
pub fn trash_list(state: State<'_, AppState>) -> Vec<TrashEntry> {
    state.library.trashed()
}

/// Moves a project back from the trash and returns where it is.
#[tauri::command]
pub async fn trash_restore(state: State<'_, AppState>, id: String) -> Answer<PathBuf> {
    let library = Arc::clone(&state.library);
    blocking(move || library.restore(&id)).await
}

/// Deletes a project in the trash for good.
#[tauri::command]
pub async fn trash_delete(state: State<'_, AppState>, id: String) -> Answer<()> {
    let library = Arc::clone(&state.library);
    blocking(move || library.delete(&id)).await
}

/// The built-in library's items (ADR-0028), of `kind` when given, with every word of
/// `search` in their name, title, description or tags.
#[tauri::command]
pub fn library_list(kind: Option<String>, search: Option<String>) -> Answer<Value> {
    Ok(library_items(kind.as_deref(), search.as_deref()))
}

/// What `library_list` answers.
fn library_items(kind: Option<&str>, search: Option<&str>) -> Value {
    json!(fosscut_engine::library::search(given(kind), given(search)))
}

/// What to add from the library, and where (`library_add`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LibraryAdd {
    /// The item's name.
    pub name: String,
    /// Its kind, for names that repeat across kinds (such as `highlight`).
    pub kind: Option<String>,
    /// Where it starts on the timeline, in flicks; at the start when omitted.
    pub start_flicks: Option<i64>,
    /// The track to put it on; the engine picks one when omitted.
    pub track: Option<String>,
    /// The clip a look applies to.
    pub clip: Option<String>,
    /// Lines for titles and templates; a text style's text.
    pub lines: Vec<String>,
    /// Video or image assets for a template's slots, in order.
    pub media: Vec<String>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    pub expected_revision: Option<u64>,
}

/// Adds a library item to the open project as the user's edit and returns the response of
/// the last step. Sounds and music are written into the bundle and imported first, as a
/// step of their own, then placed; templates build a new timeline that becomes the one
/// shown.
#[tauri::command]
pub async fn library_add(state: State<'_, AppState>, request: LibraryAdd) -> Answer<Response> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || add_from_library(&workspace, &request)).await
}

/// What `library_add` does for the project open in `workspace`.
fn add_from_library(workspace: &Workspace, add: &LibraryAdd) -> Answer<Response> {
    use fosscut_engine::library;
    let (bundle, _, _) = open_project(workspace)?;
    let item = library::find(&add.name, given(add.kind.as_deref()))?;
    let reason = format!("Add {} from the library", item.title);
    let start = add.start_flicks.map(|flicks| json!({ "flicks": flicks }));
    let track = given(add.track.as_deref()).map(str::to_string);
    // The revision on screen guards the first write; media items (sounds, music, device
    // frames, drawings and web compositions) are placed once they are imported.
    let mut request = if library::imports_media(&item) {
        let mut import = Request::single(
            "media.add",
            library::media_args(&bundle, &item, &add.lines)?,
        );
        import.expected_revision = add.expected_revision;
        import.reason = Some(reason.clone());
        let imported = workspace.execute(import)?;
        let asset = imported
            .results
            .first()
            .and_then(|result| result.get("asset"))
            .cloned()
            .ok_or_else(|| ApiError::new(codes::INTERNAL, "media.add returned no asset"))?;
        let mut placed = json!({ "asset": asset });
        match (start, track) {
            (Some(start), None) => {
                // A free track of its kind, so it never lands on another clip.
                placed["start"] = start;
                placed["free_track"] = json!(true);
            }
            (start, track) => {
                if let Some(start) = start {
                    placed["start"] = start;
                }
                if let Some(track) = track {
                    placed["track"] = json!(track);
                }
            }
        }
        Request::single("clip.add", placed)
    } else {
        let placement = library::Placement {
            start,
            duration: None,
            track,
            // The window adds to the timeline it shows, the primary one.
            timeline: None,
            clip: given(add.clip.as_deref()).map(str::to_string),
            lines: add.lines.clone(),
            media: add.media.clone(),
        };
        let call = library::command_for(&item, &placement)?;
        let command = call["command"].as_str().unwrap_or_default().to_string();
        let mut args = call["args"].clone();
        if command == "template.apply" {
            // The window shows the primary timeline, so the new one is made primary.
            args["primary"] = json!(true);
        }
        let mut request = Request::single(command, args);
        request.expected_revision = add.expected_revision;
        request
    };
    request.reason = Some(reason);
    workspace.execute(request)
}

/// The export presets with whether this machine can encode each, and the resolutions
/// export offers by name.
#[tauri::command]
pub async fn export_presets() -> Answer<Value> {
    blocking(|| Ok(presets_listing())).await
}

/// What `export_presets` answers.
fn presets_listing() -> Value {
    let resolutions: Vec<Value> = fosscut_engine::RESOLUTIONS
        .iter()
        .map(|(name, pixels)| json!({ "name": name, "pixels": pixels }))
        .collect();
    json!({ "presets": fosscut_engine::presets(), "resolutions": resolutions })
}

/// Asks where to save an export, suggesting `name` beside the open project; `None` when
/// the user cancels.
#[tauri::command]
pub async fn export_pick(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    extension: Option<String>,
) -> Answer<Option<PathBuf>> {
    let dialog = app.clone();
    let workspace = Arc::clone(&state.workspace);
    let preferences = state.preferences.current();
    blocking(move || {
        let mut file = dialog
            .dialog()
            .file()
            .set_title(word(&preferences.language, Word::ExportTo))
            .set_file_name(name);
        let bundle = workspace.snapshot().map(|(bundle, _, _)| bundle);
        if let Some(folder) = preferred::exports_folder(&preferences, bundle.as_deref()) {
            file = file.set_directory(folder);
        }
        if let Some(extension) = given(extension.as_deref()) {
            file = file.add_filter(extension.to_uppercase(), &[extension]);
        }
        let Some(path) = file.blocking_save_file() else {
            return Ok(None);
        };
        path.into_path()
            .map(Some)
            .map_err(|e| invalid(e.to_string()))
    })
    .await
}

/// How to export the open project's primary timeline (`export_start`). Texts are as the
/// CLI takes them: `720p`, `29.97`, `8M`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExportChoice {
    /// The file to write, as a whole path; the preset's extension is added when it has none.
    pub output: String,
    /// The preset (`export_presets` lists them); the output's extension implies one when
    /// omitted.
    pub preset: Option<String>,
    /// The shorter side, by name (`720p`, `4k`) or in pixels; the timeline's when omitted.
    pub resolution: Option<String>,
    /// The frame rate (`30`, `29.97`, `30000/1001`); the timeline's when omitted.
    pub fps: Option<String>,
    /// The video bit rate (`8M`); the encoder's choice for the size when omitted.
    pub bitrate: Option<String>,
    /// Only this part of the timeline, start and end in flicks.
    pub range_flicks: Option<(i64, i64)>,
    /// A cover image of the frame this long after the export's start, in flicks.
    pub cover_flicks: Option<i64>,
    /// `srt` or `vtt` subtitles from the first caption track; none when omitted or `none`.
    pub subtitles: Option<String>,
    /// Replace the output if it exists.
    pub overwrite: bool,
}

/// Starts exporting the open project's primary timeline in the background, after checking
/// everything an export checks before rendering, and returns the job at once. When it
/// finishes, the export sound plays and a notification shows, as the preferences allow.
#[tauri::command]
pub async fn export_start(
    app: AppHandle,
    state: State<'_, AppState>,
    choice: ExportChoice,
) -> Answer<JobStatus> {
    let workspace = Arc::clone(&state.workspace);
    let jobs = Arc::clone(&state.jobs);
    let watched = Arc::clone(&jobs);
    let started = blocking(move || start_export(&workspace, &jobs, &choice)).await?;
    crate::announce::watch(&app, &watched, started.job.clone());
    Ok(started)
}

/// What `export_start` does for the project open in `workspace`.
fn start_export(workspace: &Workspace, jobs: &Jobs, choice: &ExportChoice) -> Answer<JobStatus> {
    let (project, request) = export_request(workspace, choice)?;
    fosscut_engine::plan(&project, &request)?;
    Ok(jobs.start_export(project, request)?)
}

/// The project to export and the request `choice` makes of it.
fn export_request(
    workspace: &Workspace,
    choice: &ExportChoice,
) -> Answer<(Project, ExportRequest)> {
    let (bundle, project, timeline) = open_project(workspace)?;
    let mut output = PathBuf::from(choice.output.trim());
    if output.as_os_str().is_empty() {
        return Err(invalid("choose where to save the export"));
    }
    if !output.is_absolute() {
        return Err(invalid(format!(
            "{}: give the whole path of the file to write",
            output.display()
        )));
    }
    let preset = match given(choice.preset.as_deref()) {
        Some(name) => Preset::parse(name).ok_or_else(|| {
            let names: Vec<&str> = Preset::ALL.iter().map(|p| p.name()).collect();
            invalid(format!("unknown preset {name}"))
                .with_hint(format!("Presets: {}.", names.join(", ")))
        })?,
        None => Preset::for_output(&output).ok_or_else(|| {
            ApiError::new(
                "fosscut.export.codec_unavailable",
                "this machine cannot encode video",
            )
        })?,
    };
    if output.extension().is_none() {
        output.set_extension(preset.extension());
    }
    let format = project
        .timelines
        .get(&timeline)
        .map(|doc| doc.format.clone())
        .ok_or_else(|| {
            ApiError::new(
                codes::TIMELINE_NOT_FOUND,
                format!("timeline {timeline} is missing"),
            )
        })?;
    let settings = SettingsChoice {
        resolution: given(choice.resolution.as_deref())
            .map(fosscut_engine::resolution)
            .transpose()?,
        rate: given(choice.fps.as_deref())
            .map(fosscut_engine::frame_rate)
            .transpose()?,
        video_bit_rate: given(choice.bitrate.as_deref())
            .map(fosscut_engine::bit_rate)
            .transpose()?,
        audio_bit_rate: None,
    };
    let options = ExportOptions {
        range: choice
            .range_flicks
            .map(|(start, end)| (Flicks::new(start), Flicks::new(end))),
        cover: choice.cover_flicks.map(Flicks::new),
        subtitles: given(choice.subtitles.as_deref())
            .filter(|name| *name != "none")
            .map(SubtitleFormat::named)
            .transpose()?,
    };
    let request = ExportRequest {
        bundle,
        settings: fosscut_engine::settings_for(preset, &format, &settings),
        timeline,
        output,
        allow_overwrite: choice.overwrite,
        expected_revision: None,
        request_id: None,
        cache: fosscut_engine::cache_dir(),
        options,
    };
    Ok((project, request))
}

/// An export's state, progress and, once it finished, its verified report or its error.
#[tauri::command]
pub fn export_status(state: State<'_, AppState>, job: String) -> Answer<JobStatus> {
    Ok(state.jobs.status(&job, Duration::ZERO)?)
}

/// Stops an export (nothing is written) and answers once it stopped, or after a moment.
#[tauri::command]
pub async fn export_cancel(state: State<'_, AppState>, job: String) -> Answer<JobStatus> {
    let jobs = Arc::clone(&state.jobs);
    blocking(move || Ok(jobs.cancel(&job, CANCEL_WAIT)?)).await
}

/// Every preference, the defaults and where the preferences file is.
#[tauri::command]
pub fn preferences_get(state: State<'_, AppState>) -> PreferencesView {
    state.preferences.view()
}

/// Sets the dotted preference `key` (`editing.frame_step`, `shortcuts.overrides.timeline.split`)
/// to the JSON `value`, `null` returning it to its default, saves the file and answers
/// every preference. A value out of range changes nothing and answers
/// `fosscut.preferences.invalid` with the reason.
#[tauri::command]
pub async fn preferences_set(
    app: AppHandle,
    state: State<'_, AppState>,
    key: String,
    value: Value,
) -> Answer<PreferencesView> {
    let store = Arc::clone(&state.preferences);
    let view = blocking(move || store.set(&key, value)).await?;
    crate::apply_preferences(&app);
    Ok(view)
}

/// Returns the dotted preference `key` to its default, saves the file and answers every
/// preference.
#[tauri::command]
pub async fn preferences_reset(
    app: AppHandle,
    state: State<'_, AppState>,
    key: String,
) -> Answer<PreferencesView> {
    let store = Arc::clone(&state.preferences);
    let view = blocking(move || store.reset(&key)).await?;
    crate::apply_preferences(&app);
    Ok(view)
}

/// Asks the release server for the latest release, off the main thread, unless the
/// `updates` preference is off; then it asks nothing and says so. Answers `{policy,
/// checked, current, latest, newer, url, published}` once asked, `{policy, checked:
/// false, message}` otherwise.
#[tauri::command]
pub async fn update_check(state: State<'_, AppState>) -> Answer<UpdateAnswer> {
    let policy = state.preferences.current().updates;
    let answer =
        blocking(move || preferred::check_for_updates(policy, env!("CARGO_PKG_VERSION"), None))
            .await?;
    if let Some(found) = &answer.found
        && found.newer
        && let Ok(mut release) = state.release.lock()
    {
        release.clone_from(&found.url);
    }
    Ok(answer)
}

/// Opens the page of the newer release the last `update_check` found in the browser.
#[tauri::command]
pub fn update_open(state: State<'_, AppState>) -> Answer<()> {
    let page = state
        .release
        .lock()
        .ok()
        .and_then(|release| release.clone())
        .ok_or_else(|| invalid("no newer release was found; check for updates first"))?;
    if !page.starts_with("https://") {
        return Err(invalid(format!("{page} is not a web page")));
    }
    browse(&page).map_err(|e| invalid(format!("cannot open {page}: {e}")))
}

/// Opens a web page in the system's browser.
fn browse(page: &str) -> std::io::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(page)
        .spawn()
        .map(|_| ())
}

/// Plays the library sound `name` once, as an export plays it when it finishes.
#[tauri::command]
pub fn sound_play(state: State<'_, AppState>, name: String) -> Answer<()> {
    let item = fosscut_engine::library::find(&name, Some("sound"))?;
    let speaker = state.speaker.as_ref().ok_or_else(|| {
        ApiError::new(codes::INVALID_ARGUMENTS, "there is no sound output device")
    })?;
    if speaker.play_sound(&item.name) {
        Ok(())
    } else {
        Err(invalid(format!("the library has no sound {name}")))
    }
}

/// Asks for a folder, starting in `directory` when given, with the window's `title`;
/// `None` when the user cancels.
#[tauri::command]
pub async fn folder_pick(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
    directory: Option<PathBuf>,
) -> Answer<Option<PathBuf>> {
    let language = state.preferences.current().language;
    blocking(move || {
        let title = given(title.as_deref()).map_or_else(
            || word(&language, Word::ChooseFolder).to_string(),
            str::to_string,
        );
        let mut pick = app.dialog().file().set_title(title);
        if let Some(directory) = directory.filter(|d| d.is_dir()) {
            pick = pick.set_directory(directory);
        }
        let Some(folder) = pick.blocking_pick_folder() else {
            return Ok(None);
        };
        folder
            .into_path()
            .map(Some)
            .map_err(|e| invalid(e.to_string()))
    })
    .await
}

/// Each cache's folder, size in bytes and number of entries: analysis, proxies, remux and
/// frames.
#[tauri::command]
pub async fn cache_info(
    state: State<'_, AppState>,
) -> Answer<Vec<fosscut_engine::cache::CacheUsage>> {
    let cache = state.cache.clone();
    blocking(move || Ok(crate::performance::cache_listing(&cache))).await
}

/// Empties the caches in `kinds` (`analysis`, `proxies`, `remux`, `frames`; every cache
/// when empty) and answers `{freed_bytes, caches}`. Media thumbnails count as frames and
/// waveforms as analyses. The preview opens its media again, thumbnails and waveforms kept
/// in memory are forgotten, and copies the preferences want are made again.
#[tauri::command]
pub async fn cache_clear(
    app: AppHandle,
    state: State<'_, AppState>,
    kinds: Vec<String>,
) -> Answer<Value> {
    let cache = state.cache.clone();
    let cleared = blocking(move || crate::performance::clear_caches(&cache, &kinds)).await?;
    state.preview.send(Job::ResetCaches);
    state.media_views.forget();
    crate::plan_media(&app);
    Ok(cleared)
}

/// Thumbnails of video or image asset `asset` of the open project, `height` pixels high
/// (72 when omitted, kept from 16 to 360) and as wide as the asset's aspect ratio makes
/// them: at source times `times` in flicks (at most 48), or else at `count` times spread
/// evenly over its duration (the middle of each of `count` equal parts; 1 when omitted, at
/// most 48). An image gives the same picture at every time. Answers `{asset, width,
/// height, frames: [{flicks, image}]}` with each image a PNG data URL. Refuses sound,
/// LUT and font assets with `fosscut.media.no_picture`, and missing files with
/// `fosscut.export.missing_media` unless the thumbnails were made before.
#[tauri::command]
pub async fn media_thumbnails(
    state: State<'_, AppState>,
    asset: String,
    times: Option<Vec<i64>>,
    count: Option<u32>,
    height: Option<u32>,
) -> Answer<crate::media::ThumbnailStrip> {
    let workspace = Arc::clone(&state.workspace);
    let views = Arc::clone(&state.media_views);
    blocking(move || views.thumbnails(&workspace, &asset, times.as_deref(), count, height)).await
}

/// The waveform of audio asset `asset`, or of video asset `asset`'s sound, of the open
/// project: its duration split into `buckets` equal spans (kept from 1 to 65536), each the
/// lowest and highest sample of any channel in it. Answers `{asset, duration, buckets,
/// peaks: [[min, max]]}` with `duration` in flicks. Refuses images, LUTs, fonts and video
/// without sound with `fosscut.media.no_sound`.
#[tauri::command]
pub async fn media_waveform(
    state: State<'_, AppState>,
    asset: String,
    buckets: u32,
) -> Answer<crate::media::Waveform> {
    let workspace = Arc::clone(&state.workspace);
    let views = Arc::clone(&state.media_views);
    blocking(move || views.waveform(&workspace, &asset, buckets)).await
}

/// Imports files into the open project as the user's one undoable edit: `paths` when
/// given (dropped files, say), else the files the user picks in a dialog; `None` when they
/// cancel it. Answers `{assets, failed: [{path, error, code}], revision}`: files that
/// cannot be imported are reported and skipped, and the rest are added together.
#[tauri::command]
pub async fn media_import(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Option<Vec<String>>,
) -> Answer<Option<crate::media::ImportResult>> {
    let workspace = Arc::clone(&state.workspace);
    let language = state.preferences.current().language;
    blocking(move || {
        // Refused before the dialog asks for anything.
        open_project(&workspace)?;
        let files: Vec<PathBuf> = match paths {
            Some(paths) => paths.into_iter().map(PathBuf::from).collect(),
            None => {
                let mut pick = app
                    .dialog()
                    .file()
                    .set_title(word(&language, Word::ImportMedia));
                for (name, extensions) in crate::media::IMPORT_FILTERS {
                    pick = pick.add_filter(word(&language, name), extensions);
                }
                let Some(picked) = pick.blocking_pick_files() else {
                    return Ok(None);
                };
                picked
                    .into_iter()
                    .map(|file| file.into_path().map_err(|e| invalid(e.to_string())))
                    .collect::<Answer<_>>()?
            }
        };
        crate::media::import(&workspace, &files).map(Some)
    })
    .await
}

// ---- Web compositions and Remotion (ADR-0039) ----

/// Asks for a web composition to import: a folder with an `index.html` when `folder`,
/// else an `.html` page. Answers what it is and what its meta tag says (`None` when the
/// user cancels), for the window to ask for what the tag leaves out before `web_import`.
#[tauri::command]
pub async fn web_pick(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: bool,
) -> Answer<Option<crate::web::WebPage>> {
    let workspace = Arc::clone(&state.workspace);
    let language = state.preferences.current().language;
    blocking(move || {
        // Refused before the dialog asks for anything.
        open_project(&workspace)?;
        let pick = app
            .dialog()
            .file()
            .set_title(word(&language, Word::ImportComposition));
        let picked = if folder {
            pick.blocking_pick_folder()
        } else {
            pick.add_filter(word(&language, Word::WebFiles), &["html", "htm"])
                .blocking_pick_file()
        };
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked.into_path().map_err(|e| invalid(e.to_string()))?;
        crate::web::inspect(&path).map(Some)
    })
    .await
}

/// Copies a web composition into the open project's bundle and adds it as the user's one
/// undoable edit, with the size, frame rate and length given overriding its meta tag.
#[tauri::command]
pub async fn web_import(
    state: State<'_, AppState>,
    request: crate::web::WebImport,
) -> Answer<crate::web::WebImported> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || crate::web::import(&workspace, &request)).await
}

/// Starts rendering a composition of the user's own Remotion project into the open
/// project in the background, imported once it is done. Without `confirmed` it refuses
/// with `fosscut.web.confirmation_required`, whose details carry the licence note and the
/// command for the window to show first.
#[tauri::command]
pub async fn remotion_start(
    state: State<'_, AppState>,
    choice: crate::web::RemotionChoice,
) -> Answer<JobStatus> {
    let workspace = Arc::clone(&state.workspace);
    let jobs = Arc::clone(&state.jobs);
    blocking(move || crate::web::start(&workspace, &jobs, &choice, None)).await
}

/// A Remotion render's state, the frames done of the total, and once it succeeded the file
/// and the asset it became.
#[tauri::command]
pub fn remotion_status(state: State<'_, AppState>, job: String) -> Answer<JobStatus> {
    Ok(state.jobs.status(&job, Duration::ZERO)?)
}

/// Stops a Remotion render (Remotion's whole process group ends and no file is kept), and
/// answers once it stopped, or after a moment.
#[tauri::command]
pub async fn remotion_cancel(state: State<'_, AppState>, job: String) -> Answer<JobStatus> {
    let jobs = Arc::clone(&state.jobs);
    blocking(move || Ok(jobs.cancel(&job, CANCEL_WAIT)?)).await
}

// ---- Edits planned from analyses ----

/// Where the planners read analyses (the cache folder) and models.
fn places(state: &State<'_, AppState>) -> crate::auto_edit::Places {
    crate::auto_edit::Places {
        cache: state.cache.clone(),
        models: fosscut_engine::models_dir(),
    }
}

/// Makes video clip `clip` follow its subject on its timeline's canvas, as one undoable
/// step: `source` `person` (its cached pose), `subject` (its cached matte) or `region`
/// (tracked from `region`, centre and size as fractions of the picture, drawn at timeline
/// time `at` in flicks, the clip's start when omitted), shot by shot from the cached scene
/// cuts when `shots`. Answers `{reframe: {clip, frames, subject_frames, shots: [{start,
/// end, zoom, subject_frames, inside_frames}]}, response}`. A missing analysis refuses
/// with `fosscut.analysis.missing`, its details naming the analyzer and the model it runs.
#[tauri::command]
pub async fn clip_reframe(
    state: State<'_, AppState>,
    clip: String,
    source: String,
    region: Option<fosscut_engine::reframe::Region>,
    at: Option<i64>,
    shots: Option<bool>,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    let places = places(&state);
    let follow = crate::auto_edit::Follow {
        clip,
        source,
        region,
        at,
        shots: shots.unwrap_or(false),
        expected_revision,
    };
    blocking(move || crate::auto_edit::reframe(&workspace, &places, &follow)).await
}

/// Zooms video clip `clip` by `zoom` (1 to 4) on its own canvas and makes the zoomed crop
/// follow `source` (as `clip_reframe` takes it), as one undoable step; answered as
/// `clip_reframe` answers.
#[tauri::command]
pub async fn clip_camera_track(
    state: State<'_, AppState>,
    clip: String,
    source: String,
    region: Option<fosscut_engine::reframe::Region>,
    at: Option<i64>,
    zoom: f64,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    let places = places(&state);
    let follow = crate::auto_edit::Follow {
        clip,
        source,
        region,
        at,
        shots: false,
        expected_revision,
    };
    blocking(move || crate::auto_edit::camera_track(&workspace, &places, &follow, zoom)).await
}

/// Copies `timeline` (the primary one when omitted) onto a `width` by `height` canvas
/// (1080 by 1920 when omitted) named `name`, and makes every video clip of the copy follow
/// `source` (`person` or `subject`), shot by shot when `shots`, as one undoable step.
/// Answers `{vertical: {source, timeline, clips, centred: [{clip, reason}]}, response}`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn timeline_vertical(
    state: State<'_, AppState>,
    timeline: Option<String>,
    source: String,
    width: Option<u32>,
    height: Option<u32>,
    name: Option<String>,
    shots: Option<bool>,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    let places = places(&state);
    let request = crate::auto_edit::Vertical {
        timeline,
        source,
        width,
        height,
        name,
        shots: shots.unwrap_or(false),
        expected_revision,
    };
    blocking(move || crate::auto_edit::vertical(&workspace, &places, &request)).await
}

/// Measures the beats of music clip `clip` (unless cached) and marks every `every`-th
/// (1 to 16, 1 when omitted) as one undoable step. Answers `{beats: {timeline, tempo_bpm,
/// beats_per_bar, confidence, markers, existing}, response}`, `response` null when every
/// beat has a marker already.
#[tauri::command]
pub async fn beats_markers(
    state: State<'_, AppState>,
    clip: String,
    every: Option<u32>,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    let places = places(&state);
    blocking(move || {
        crate::auto_edit::beat_markers(&workspace, &places, &clip, every, expected_revision)
    })
    .await
}

/// Measures the beats of music clip `music` (unless cached) and moves the cuts between the
/// clips of video track `track` onto them (every `every`-th beat, 1 when omitted), as one
/// undoable step. Answers `{cuts: {track, music, tempo_bpm, beats, tolerance, snapped:
/// [{before, after, from, to}], unsnapped: [{before, after, at, reason}]}, response}`,
/// `response` null when no cut moved.
#[tauri::command]
pub async fn beats_cut(
    state: State<'_, AppState>,
    track: String,
    music: String,
    every: Option<u32>,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    let places = places(&state);
    blocking(move || {
        crate::auto_edit::beat_cuts(
            &workspace,
            &places,
            &track,
            &music,
            every,
            expected_revision,
        )
    })
    .await
}

/// The candidate shorts in the speech of clip `clip`, from its cached transcript, best
/// first: from `min_seconds` to `max_seconds` long (15 to 60 when omitted), at most `count`
/// (10 when omitted). Reads only. Answers `{clip, asset, measured, missing, sentences,
/// considered, candidates: [{span, seconds, score, text, sentences, words, reasons,
/// timeline_start, timeline_end}]}`.
#[tauri::command]
pub async fn shorts_candidates(
    state: State<'_, AppState>,
    clip: String,
    min_seconds: Option<f64>,
    max_seconds: Option<f64>,
    count: Option<u32>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    let places = places(&state);
    let request = crate::auto_edit::Candidates {
        clip,
        min_seconds,
        max_seconds,
        count,
    };
    blocking(move || crate::auto_edit::short_candidates(&workspace, &places, &request)).await
}

/// Builds a short from `start` to `end` (flicks in the media's own time, a candidate's
/// `span`) of clip `clip`'s media as a new timeline named `name`, `width` by `height`
/// (1080 by 1920 when omitted), following `follow` (`person` or `subject`, shot by shot
/// unless `shots` is false; centred when omitted), with captions of its words when
/// `captions` (in `caption_preset`) and `title` over its first three seconds, as one
/// undoable step. Answers `{short: {timeline, clip, name, start, duration, centred,
/// caption_words}, response}`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn shorts_make(
    state: State<'_, AppState>,
    clip: String,
    start: i64,
    end: i64,
    name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    follow: Option<String>,
    shots: Option<bool>,
    captions: Option<bool>,
    caption_preset: Option<String>,
    title: Option<String>,
    expected_revision: Option<u64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    let places = places(&state);
    let request = crate::auto_edit::Short {
        clip,
        start,
        end,
        name,
        width,
        height,
        follow,
        shots,
        captions: captions.unwrap_or(false),
        caption_preset,
        title,
        expected_revision,
    };
    blocking(move || crate::auto_edit::make_short(&workspace, &places, &request)).await
}

/// The assets of the open project the preview decodes from a proxy (`{"proxy": "half"}`)
/// or a remuxed copy (`"remux"`): those it has shown so, and those with such a copy made
/// that the preferences let it use.
#[tauri::command]
pub async fn media_sources(state: State<'_, AppState>) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    let shown = state.preview.sources();
    let preferences = state.preferences.current();
    let cache = state.cache.clone();
    blocking(move || {
        let Some((bundle, project, _)) = workspace.snapshot() else {
            return Ok(json!({}));
        };
        Ok(json!(media_sources_of(
            &project,
            &bundle,
            &preferences,
            &cache,
            &shown
        )))
    })
    .await
}

/// What `media_sources` answers.
fn media_sources_of(
    project: &Project,
    bundle: &std::path::Path,
    preferences: &Preferences,
    cache: &std::path::Path,
    shown: &std::collections::BTreeMap<Id, fosscut_engine::preview::PreviewSource>,
) -> std::collections::BTreeMap<String, fosscut_engine::preview::PreviewSource> {
    use fosscut_engine::preview::PreviewSource;
    let options = crate::performance::preview_options(preferences, cache);
    let mut sources = std::collections::BTreeMap::new();
    for id in project.media.assets.iter().map(|asset| &asset.id) {
        let source = shown.get(id).copied().or_else(|| {
            let proxy = options.proxies.filter(|size| {
                matches!(
                    fosscut_engine::proxy::existing_proxy(project, bundle, id, *size, cache),
                    Ok(Some(_))
                )
            });
            match proxy {
                Some(size) => Some(PreviewSource::Proxy(size)),
                None => (options.remuxed
                    && matches!(
                        fosscut_engine::remux::existing_remux(project, bundle, id, cache),
                        Ok(Some(_))
                    ))
                .then_some(PreviewSource::Remux),
            }
        });
        if let Some(source) = source.filter(|s| *s != PreviewSource::Original) {
            sources.insert(id.to_string(), source);
        }
    }
    sources
}

/// The proxies and remuxed copies being made: `{proxies: {done, total}, remux: {done,
/// total}, failed}`; the window also hears it as the `media-work` event.
#[tauri::command]
pub fn media_work(state: State<'_, AppState>) -> crate::performance::MediaWork {
    state.media_jobs.status()
}

// ---- Local models (ADR-0033) ----

/// Every model with whether it is installed, the models folder, and the ONNX Runtime
/// library the models will load (`runtime`, null when it must be installed first):
/// `{dir, models: [{name, task, size_bytes, license, attribution, languages, url,
/// installed, files, note}], runtime}`.
#[tauri::command]
pub async fn models_list() -> Answer<crate::models::ModelListing> {
    blocking(|| Ok(crate::models::listing(&fosscut_engine::models_dir()))).await
}

/// Starts downloading and installing model `name` in the background and answers its
/// install at once: `{name, state, done_bytes, total_bytes, elapsed_ms, install, error}`.
/// The window passes `confirmed` only after the person agreed to the size and license it
/// showed; without it nothing downloads and the answer is
/// `fosscut.model.confirmation_required`. An installed model answers `succeeded` with
/// `install.status` `unchanged`.
#[tauri::command]
pub async fn models_install(
    state: State<'_, AppState>,
    name: String,
    confirmed: Option<bool>,
) -> Answer<crate::models::InstallState> {
    let installs = Arc::clone(&state.installs);
    blocking(move || {
        installs.start(
            &fosscut_engine::models_dir(),
            name.trim(),
            fosscut_engine::analysis::ModelSource::Download(None),
            confirmed.unwrap_or(false),
        )
    })
    .await
}

/// The install of model `name` started last, as `models_install` answers it.
#[tauri::command]
pub fn models_install_status(
    state: State<'_, AppState>,
    name: String,
) -> Answer<crate::models::InstallState> {
    state.installs.status(name.trim())
}

/// Analyzes a clip's media as a background job: `kind` `matte` for background removal
/// (`model` `person`, the default, or `subject`, any subject and slow) or `pose` for body
/// effects, or any other analyzer a plan may name in `fosscut.analysis.missing` (`scenes`
/// of a video clip; `transcript`, `beats`, `levels`, `loudness` or `silences` of a video or
/// audio clip), with `options` its options as that error's `details.options` holds them
/// under its name. Answers the job (`{job, kind, state, progress, elapsed_ms, asset,
/// analysis, error}`) once it finished when the result was cached, or while it runs;
/// `subject_status` follows it. A model that is missing refuses at once with
/// `fosscut.model.not_installed`, naming it in `details.what`.
#[tauri::command]
pub async fn subject_analyze(
    state: State<'_, AppState>,
    request: crate::models::SubjectRequest,
) -> Answer<JobStatus> {
    let workspace = Arc::clone(&state.workspace);
    let jobs = Arc::clone(&state.jobs);
    let cache = state.cache.clone();
    blocking(move || {
        crate::models::analyze_subject(
            &workspace,
            &jobs,
            &cache,
            &fosscut_engine::models_dir(),
            &request,
        )
    })
    .await
}

/// An analysis job's state, progress and, once it finished, its result or error.
#[tauri::command]
pub fn subject_status(state: State<'_, AppState>, job: String) -> Answer<JobStatus> {
    crate::models::subject_status(&state.jobs, &job, false)
}

/// Stops an analysis job; nothing is cached.
#[tauri::command]
pub async fn subject_cancel(state: State<'_, AppState>, job: String) -> Answer<JobStatus> {
    let jobs = Arc::clone(&state.jobs);
    blocking(move || crate::models::subject_status(&jobs, &job, true)).await
}

/// The voices of the voice model: `[{name, accent, gender, installed}]`.
#[tauri::command]
pub fn voices_list() -> Vec<fosscut_engine::voice::VoiceInfo> {
    crate::models::voices(&fosscut_engine::models_dir())
}

/// Speaks `text` with `voice` at `speed` into a temporary file and plays it through the
/// app's sound output, adding nothing: `{file: {relative, reused, duration, seconds, voice,
/// speed}, played}`. Inserting the same speech afterwards uses this file.
#[tauri::command]
pub async fn speech_preview(
    state: State<'_, AppState>,
    text: String,
    voice: String,
    speed: Option<f32>,
) -> Answer<Value> {
    let speaker = state.speaker.clone();
    blocking(move || {
        let preview = crate::models::preview_speech(
            &fosscut_engine::models_dir(),
            &crate::models::preview_folder(),
            &text,
            voice.trim(),
            speed,
        )?;
        let played = speaker
            .as_ref()
            .is_some_and(|speaker| speaker.play_mono(&preview.samples, preview.rate));
        Ok(json!({ "file": preview.file, "played": played }))
    })
    .await
}

/// Speaks `request.text` and places it on the primary timeline at `request.at` as the
/// user's one undoable step: `{speech: {revision, asset, file}, response}`.
#[tauri::command]
pub async fn speech_add(
    state: State<'_, AppState>,
    request: crate::models::SpeechAdd,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || {
        crate::models::add_speech(
            &workspace,
            &fosscut_engine::models_dir(),
            &crate::models::preview_folder(),
            &request,
        )
    })
    .await
}

/// Translates a caption track into `request.to` (`hu` or `en`), into each page's
/// translation (`bilingual`) or instead of its words (`replace`), as the user's one
/// undoable step: `{translation: {pair, mode, model, attribution, pages, skipped, ...},
/// response}`.
#[tauri::command]
pub async fn captions_translate(
    state: State<'_, AppState>,
    request: crate::models::CaptionsTranslate,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || {
        crate::models::translate_captions(&workspace, &fosscut_engine::models_dir(), &request)
    })
    .await
}

/// Dubs a caption track with text to speech, one clip per page at its start, speaking its
/// words or its translations, as the user's one undoable step: `{dub: {voice, pages,
/// skipped, ...}, response}`.
#[tauri::command]
pub async fn captions_dub(
    state: State<'_, AppState>,
    request: crate::models::CaptionsDub,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || {
        crate::models::dub_captions(&workspace, &fosscut_engine::models_dir(), &request)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizing_measures_clips_of_the_open_project_and_refuses_the_rest() {
        // A short runtime directory: Unix socket paths are limited to about 100 bytes.
        let base = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let runtime = base.join(format!("fcd-loud-{}", std::process::id()));
        let work =
            std::env::temp_dir().join(format!("fosscut-desktop-loud-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).unwrap();
        let workspace = Workspace::new(runtime.clone(), None);
        let default = fosscut_engine::loudness::DEFAULT_TARGET_LUFS;
        let closed = normalize(&workspace, &["clip_0001".into()], default).unwrap_err();
        assert_eq!(closed.code, codes::INVALID_ARGUMENTS);
        assert!(closed.message.contains("no project"), "{closed:?}");

        workspace
            .create(&NewProject {
                path: work.join("Loud.fosscut"),
                name: None,
                width: 1280,
                height: 720,
                rate: "25".into(),
            })
            .unwrap();
        let unparsed = normalize(&workspace, &["not an id".into()], default).unwrap_err();
        assert_eq!(unparsed.code, codes::INVALID_ARGUMENTS);
        let none = normalize(&workspace, &[], default).unwrap_err();
        assert!(none.message.contains("at least one clip"), "{none:?}");

        let added = workspace
            .execute(Request::single(
                "text.add",
                json!({ "text": "Quiet", "duration": "1s" }),
            ))
            .unwrap();
        let title = added.results[0]["clip"].as_str().unwrap().to_string();
        let silent = normalize(&workspace, std::slice::from_ref(&title), default).unwrap_err();
        assert_eq!(silent.code, codes::INVALID_ARGUMENTS);
        assert!(silent.message.contains("without sound"), "{silent:?}");
        let target = normalize(&workspace, &[title], -60.0).unwrap_err();
        assert!(target.message.contains("target"), "{target:?}");

        workspace.close();
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
    }

    const SECOND: i64 = fosscut_time::FLICKS_PER_SECOND;

    /// A workspace with nothing open, a short runtime directory (Unix socket paths are
    /// limited to about 100 bytes) and a folder to work in.
    fn workspace(name: &str) -> (Workspace, PathBuf, PathBuf) {
        let base = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let runtime = base.join(format!("fcd-{name}-{}", std::process::id()));
        let work =
            std::env::temp_dir().join(format!("fosscut-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).unwrap();
        (Workspace::new(runtime.clone(), None), runtime, work)
    }

    fn open_new(workspace: &Workspace, work: &std::path::Path, width: u32, height: u32) -> Opened {
        workspace
            .create(&NewProject {
                path: work.join("Demo.fosscut"),
                name: None,
                width,
                height,
                rate: "30".into(),
            })
            .unwrap()
    }

    fn done(workspace: Workspace, runtime: PathBuf, work: PathBuf) {
        workspace.close();
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
    }

    fn add(name: &str) -> LibraryAdd {
        LibraryAdd {
            name: name.into(),
            ..LibraryAdd::default()
        }
    }

    /// The clips of the primary timeline, by id.
    fn clips(workspace: &Workspace) -> Vec<Value> {
        let timeline = workspace
            .query("timeline.read", json!({ "detail": "full" }))
            .unwrap();
        timeline["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["clips"].as_array().unwrap().clone())
            .collect()
    }

    fn clip<'a>(clips: &'a [Value], id: &Value) -> &'a Value {
        clips.iter().find(|c| &c["id"] == id).unwrap()
    }

    #[test]
    fn new_projects_take_the_preferred_frame_rate_unless_asked_for_another() {
        let (workspace, runtime, work) = workspace("newrate");
        let mut preferences = Preferences::default();
        preferences.set("timeline.frame_rate", "25").unwrap();
        let new = new_project(work.join("Trip"), None, None, None, &preferences);
        assert_eq!(
            new.path,
            work.join("Trip.fosscut"),
            "the extension is added"
        );
        assert_eq!(
            (new.width, new.height, new.rate.as_str()),
            (1920, 1080, "25")
        );
        let opened = workspace.create(&new).unwrap();
        let timeline = workspace
            .query(
                "timeline.read",
                json!({ "timeline": opened.info["primary_timeline"] }),
            )
            .unwrap();
        assert_eq!(timeline["format"]["rate"], "25", "{timeline}");
        workspace.close();
        let asked = new_project(
            work.join("Tall.fosscut"),
            Some(1080),
            Some(1920),
            Some("60"),
            &preferences,
        );
        assert_eq!(
            (asked.width, asked.height, asked.rate.as_str()),
            (1080, 1920, "60")
        );
        done(workspace, runtime, work);
    }

    #[test]
    fn media_marks_show_the_copies_the_preview_decodes() {
        use fosscut_engine::preview::PreviewSource;
        use fosscut_engine::proxy::ProxySize;
        let (workspace, runtime, work) = workspace("sources");
        open_new(&workspace, &work, 1280, 720);
        let cache = work.join("cache");
        // Media whose files exist, so their copies can be found.
        let real = |name: &str, size: u64| {
            let file = work.join(name);
            std::fs::write(&file, b"media").unwrap();
            let mut args = unplayable_video(name, size);
            args["path"] = json!({ "absolute": file });
            args
        };
        let a = import(&workspace, real("a.mov", 11));
        let b = import(&workspace, real("b.ts", 12));
        let (bundle, project, _) = workspace.snapshot().unwrap();
        let mut preferences = Preferences::default();
        let none = std::collections::BTreeMap::new();
        assert!(media_sources_of(&project, &bundle, &preferences, &cache, &none).is_empty());
        let identity = |id: &str| {
            project
                .media
                .get(&Id::parse(id).unwrap())
                .unwrap()
                .identity
                .clone()
                .unwrap()
        };
        let proxy = fosscut_engine::proxy::proxy_path(&cache, &identity(&a), ProxySize::Half);
        let remuxed = fosscut_engine::remux::remux_path(&cache, &identity(&b));
        for file in [&proxy, &remuxed] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"copy").unwrap();
        }
        let marks = media_sources_of(&project, &bundle, &preferences, &cache, &none);
        assert_eq!(marks.len(), 1, "proxies are off: {marks:?}");
        assert_eq!(marks[&b], PreviewSource::Remux);
        preferences.set("performance.proxies", "on").unwrap();
        let marks = media_sources_of(&project, &bundle, &preferences, &cache, &none);
        assert_eq!(marks[&a], PreviewSource::Proxy(ProxySize::Half));
        let shown =
            std::collections::BTreeMap::from([(Id::parse(&a).unwrap(), PreviewSource::Original)]);
        let marks = media_sources_of(&project, &bundle, &preferences, &cache, &shown);
        assert!(
            !marks.contains_key(&a),
            "what the preview decodes wins: {marks:?}"
        );
        assert_eq!(
            serde_json::to_value(media_sources_of(
                &project,
                &bundle,
                &preferences,
                &cache,
                &none
            ))
            .unwrap(),
            json!({ a.clone(): { "proxy": "half" }, b.clone(): "remux" })
        );
        done(workspace, runtime, work);
    }

    #[test]
    fn scrubbing_plays_sound_only_while_the_preferences_say_so() {
        let at = Flicks::new(SECOND);
        let mut preferences = Preferences::default();
        assert!(matches!(show_job(at, true, &preferences), Job::Scrub(t) if t == at));
        assert!(matches!(show_job(at, false, &preferences), Job::Show(t) if t == at));
        preferences.set("timeline.scrub_audio", "false").unwrap();
        assert!(matches!(show_job(at, true, &preferences), Job::Show(t) if t == at));
    }

    #[test]
    fn loudness_is_levelled_to_the_preferred_target() {
        let (workspace, runtime, work) = workspace("loudpref");
        open_new(&workspace, &work, 1280, 720);
        let ding = add_from_library(&workspace, &add("ding")).unwrap();
        let clip = ding.results[0]["clip"].as_str().unwrap().to_string();
        let mut preferences = Preferences::default();
        preferences.set("editing.loudness_target", "-20").unwrap();
        let target = preferred::loudness_target(&preferences, None);
        let levelled = normalize(&workspace, std::slice::from_ref(&clip), target).unwrap();
        assert_eq!(levelled["target_lufs"], -20.0, "{levelled}");
        let asked = preferred::loudness_target(&preferences, Some(-16.0));
        let levelled = normalize(&workspace, &[clip], asked).unwrap();
        assert_eq!(levelled["target_lufs"], -16.0);
        done(workspace, runtime, work);
    }

    #[test]
    fn the_library_lists_items_by_kind_and_words() {
        let all = library_items(None, None);
        let all = all.as_array().unwrap();
        assert!(all.len() > 40, "{}", all.len());
        assert!(
            all.iter().all(|i| i["commercial"] == true),
            "all free to use"
        );
        let stickers = library_items(Some("sticker"), Some("  "));
        let stickers = stickers.as_array().unwrap();
        assert!(!stickers.is_empty());
        assert!(
            stickers.iter().all(|i| i["kind"] == "sticker"),
            "a blank search is no search"
        );
        assert_eq!(
            library_items(Some(""), Some("SALE badge")),
            library_items(None, Some("sale BADGE")),
            "a blank kind is any kind, and case does not matter"
        );
        let sale = library_items(Some("sticker"), Some("SALE badge"));
        let names: Vec<&str> = sale
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|i| i["name"].as_str())
            .collect();
        assert_eq!(names, ["badge-sale"]);
        let music = library_items(Some("music"), None);
        assert!(
            music
                .as_array()
                .unwrap()
                .iter()
                .all(|i| i["bpm"].is_number() && i["seconds"].is_number())
        );
        assert_eq!(library_items(Some("nothing"), None), json!([]));
    }

    #[test]
    fn drawings_particles_and_3d_objects_from_the_library_are_placed_at_the_playhead() {
        let (workspace, runtime, work) = workspace("libmotion");
        open_new(&workspace, &work, 1920, 1080);
        let at = |kind: &str, name: &str| LibraryAdd {
            kind: Some(kind.into()),
            start_flicks: Some(SECOND),
            ..add(name)
        };
        let phone = add_from_library(&workspace, &at("device", "phone")).unwrap();
        let sparks = add_from_library(&workspace, &at("particles", "sparks")).unwrap();
        let podium = add_from_library(&workspace, &at("model", "podium")).unwrap();
        let now = clips(&workspace);
        let kind = |response: &Response| clip(&now, &response.results[0]["clip"])["kind"].clone();
        assert_eq!(
            kind(&phone),
            "image",
            "a device frame is a drawing shown as an image"
        );
        assert_eq!(kind(&sparks), "particles");
        assert_eq!(kind(&podium), "model");
        for response in [&phone, &sparks, &podium] {
            assert_eq!(
                clip(&now, &response.results[0]["clip"])["start"]["flicks"],
                SECOND
            );
        }
        let media = workspace.query("media.list", Value::Null).unwrap();
        assert_eq!(media["assets"][0]["kind"], "vector", "{media}");
        done(workspace, runtime, work);
    }

    #[test]
    fn library_items_are_added_as_the_users_edits() {
        let (workspace, runtime, work) = workspace("libadd");
        let closed = add_from_library(&workspace, &add("badge-sale")).unwrap_err();
        assert_eq!(closed.code, codes::INVALID_ARGUMENTS);
        assert!(closed.message.contains("no project"), "{closed:?}");
        open_new(&workspace, &work, 1920, 1080);

        let sticker = add_from_library(
            &workspace,
            &LibraryAdd {
                kind: Some("sticker".into()),
                start_flicks: Some(SECOND),
                expected_revision: Some(0),
                ..add("badge-sale")
            },
        )
        .unwrap();
        let badge = sticker.results[0]["clip"].clone();
        let now = clips(&workspace);
        assert_eq!(clip(&now, &badge)["kind"], "shape");
        assert_eq!(clip(&now, &badge)["text"], "SALE");
        assert_eq!(clip(&now, &badge)["start"]["flicks"], SECOND);

        let stale = add_from_library(
            &workspace,
            &LibraryAdd {
                expected_revision: Some(0),
                ..add("circle")
            },
        )
        .unwrap_err();
        assert_eq!(stale.code, codes::REVISION_CONFLICT);

        let look = library_items(Some("look"), None)[0]["name"]
            .as_str()
            .unwrap()
            .to_string();
        let bare = add_from_library(&workspace, &add(&look)).unwrap_err();
        assert!(bare.message.contains("needs the clip"), "{bare:?}");
        add_from_library(
            &workspace,
            &LibraryAdd {
                clip: badge.as_str().map(str::to_string),
                ..add(&look)
            },
        )
        .unwrap();
        let now = clips(&workspace);
        let effects = &clip(&now, &badge)["effects"];
        assert!(
            !effects.as_array().unwrap().is_empty(),
            "the look's effects: {effects}"
        );

        let title = add_from_library(
            &workspace,
            &LibraryAdd {
                kind: Some("title".into()),
                start_flicks: Some(4 * SECOND),
                lines: vec!["Sam Lee".into(), "Host".into()],
                ..add("lower-third")
            },
        )
        .unwrap();
        let shown: Vec<String> = clips(&workspace)
            .iter()
            .filter(|c| c["start"]["flicks"] == 4 * SECOND)
            .filter_map(|c| c["text"].as_str().map(str::to_string))
            .collect();
        assert!(
            shown.iter().any(|t| t.contains("Sam Lee")),
            "{shown:?} {title:?}"
        );

        let ambiguous = add_from_library(&workspace, &add("highlight")).unwrap_err();
        assert!(
            ambiguous.message.contains("say which kind"),
            "{ambiguous:?}"
        );
        let styled = add_from_library(
            &workspace,
            &LibraryAdd {
                kind: Some("text_style".into()),
                start_flicks: Some(8 * SECOND),
                ..add("highlight")
            },
        )
        .unwrap();
        let text = styled.results[0]["clip"].clone();
        assert_eq!(clip(&clips(&workspace), &text)["text"], "Title");

        let history = workspace
            .query("history.list", json!({ "limit": 1 }))
            .unwrap();
        assert_eq!(history["entries"][0]["actor"], "human");
        assert!(
            history["entries"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("from the library")
        );

        let before = workspace.query("project.info", Value::Null).unwrap();
        let template = add_from_library(
            &workspace,
            &LibraryAdd {
                lines: vec!["Less, but better.".into()],
                ..add("quote-card")
            },
        )
        .unwrap();
        let info = workspace.query("project.info", Value::Null).unwrap();
        assert_ne!(info["primary_timeline"], before["primary_timeline"]);
        assert_eq!(
            info["primary_timeline"], template.results[0]["timeline"],
            "the new timeline is the one shown"
        );
        done(workspace, runtime, work);
    }

    #[test]
    fn sounds_are_imported_into_the_bundle_once_and_placed() {
        let (workspace, runtime, work) = workspace("libsound");
        let opened = open_new(&workspace, &work, 1280, 720);
        let sound = library_items(Some("sound"), None)[0]["name"]
            .as_str()
            .unwrap()
            .to_string();
        let first = add_from_library(
            &workspace,
            &LibraryAdd {
                start_flicks: Some(2 * SECOND),
                expected_revision: Some(0),
                ..add(&sound)
            },
        )
        .unwrap();
        let again = add_from_library(
            &workspace,
            &LibraryAdd {
                start_flicks: Some(5 * SECOND),
                ..add(&sound)
            },
        )
        .unwrap();
        assert!(opened.bundle.join(format!("library/{sound}.wav")).is_file());
        let media = workspace.query("media.list", Value::Null).unwrap();
        let assets = media["assets"].as_array().unwrap();
        assert_eq!(assets.len(), 1, "imported once: {assets:?}");
        let now = clips(&workspace);
        for (response, start) in [(&first, 2 * SECOND), (&again, 5 * SECOND)] {
            let placed = clip(&now, &response.results[0]["clip"]);
            assert_eq!(placed["kind"], "audio");
            assert_eq!(placed["asset"], assets[0]["id"]);
            assert_eq!(placed["start"]["flicks"], start);
        }
        done(workspace, runtime, work);
    }

    fn export(output: &std::path::Path) -> ExportChoice {
        ExportChoice {
            output: output.display().to_string(),
            ..ExportChoice::default()
        }
    }

    #[test]
    fn export_choices_become_the_request() {
        let (workspace, runtime, work) = workspace("exportreq");
        let opened = open_new(&workspace, &work, 1920, 1080);
        let (_, request) = export_request(
            &workspace,
            &ExportChoice {
                preset: Some("mov-prores".into()),
                resolution: Some("720p".into()),
                fps: Some("25".into()),
                bitrate: Some(" ".into()),
                range_flicks: Some((0, SECOND)),
                cover_flicks: Some(SECOND / 2),
                subtitles: Some("vtt".into()),
                overwrite: true,
                ..export(&work.join("Promo"))
            },
        )
        .unwrap();
        assert_eq!(
            request.output,
            work.join("Promo.mov"),
            "the preset's extension"
        );
        assert_eq!(request.bundle, opened.bundle);
        assert_eq!(
            request.timeline.to_string(),
            opened.info["primary_timeline"]
        );
        assert_eq!(request.settings.container.extension(), "mov");
        let video = request.settings.video.as_ref().unwrap();
        assert_eq!((video.width, video.height), (1280, 720));
        assert_eq!(video.rate.to_string(), "25");
        assert_eq!(
            video.bit_rate, None,
            "a blank bit rate is the encoder's choice"
        );
        assert_eq!(
            request.options,
            ExportOptions {
                range: Some((Flicks::ZERO, Flicks::new(SECOND))),
                cover: Some(Flicks::new(SECOND / 2)),
                subtitles: Some(SubtitleFormat::Vtt),
            }
        );
        assert!(request.allow_overwrite);

        let (_, web) = export_request(
            &workspace,
            &ExportChoice {
                subtitles: Some("none".into()),
                fps: Some("29.97".into()),
                ..export(&work.join("Web.webm"))
            },
        )
        .unwrap();
        assert_eq!(
            web.settings.container.extension(),
            "webm",
            "the extension's preset"
        );
        let video = web.settings.video.as_ref().unwrap();
        assert_eq!(
            (video.width, video.height),
            (1920, 1080),
            "the timeline's size"
        );
        assert_eq!(video.rate.to_string(), "30000/1001");
        assert_eq!(web.options, ExportOptions::default());
        assert!(!web.allow_overwrite);
        done(workspace, runtime, work);
    }

    #[test]
    fn exports_are_refused_before_they_start() {
        let (workspace, runtime, work) = workspace("exportno");
        let jobs = Jobs::new();
        // WebM implies its preset without asking which encoders this machine has, and
        // every refusal here comes before the encoders are checked.
        let out = work.join("Out.webm");
        let closed = start_export(&workspace, &jobs, &export(&out)).unwrap_err();
        assert!(closed.message.contains("no project"), "{closed:?}");
        open_new(&workspace, &work, 1280, 720);
        let empty = start_export(&workspace, &jobs, &export(&out)).unwrap_err();
        assert_eq!(empty.code, "fosscut.export.empty_timeline");
        workspace
            .execute(Request::single(
                "text.add",
                json!({ "text": "Hello", "duration": "2s" }),
            ))
            .unwrap();

        let refused = |choice: ExportChoice| start_export(&workspace, &jobs, &choice).unwrap_err();
        let blank = refused(export(std::path::Path::new(" ")));
        assert!(blank.message.contains("where to save"), "{blank:?}");
        let relative = refused(export(std::path::Path::new("Out.mp4")));
        assert!(relative.message.contains("whole path"), "{relative:?}");
        let preset = refused(ExportChoice {
            preset: Some("vhs".into()),
            ..export(&out)
        });
        assert!(preset.hint.unwrap_or_default().contains("mp4-h264"));
        for bad in [
            ExportChoice {
                resolution: Some("huge".into()),
                ..export(&out)
            },
            ExportChoice {
                fps: Some("fast".into()),
                ..export(&out)
            },
            ExportChoice {
                bitrate: Some("lots".into()),
                ..export(&out)
            },
            ExportChoice {
                subtitles: Some("ass".into()),
                ..export(&out)
            },
            ExportChoice {
                range_flicks: Some((5 * SECOND, 6 * SECOND)),
                ..export(&out)
            },
            ExportChoice {
                range_flicks: Some((0, SECOND)),
                cover_flicks: Some(SECOND),
                ..export(&out)
            },
        ] {
            let error = refused(bad.clone());
            assert_eq!(
                error.code, "fosscut.export.invalid_settings",
                "{bad:?}: {error:?}"
            );
        }
        assert!(jobs.list().is_empty(), "nothing started");
        assert!(!out.exists());
        done(workspace, runtime, work);
    }

    /// `media.add` arguments for a video with sound that is nowhere on disk.
    fn unplayable_video(name: &str, size: u64) -> Value {
        json!({
            "name": name,
            "kind": "video",
            "path": { "absolute": format!("/nowhere/{name}") },
            "identity": { "size": size, "sample_blake3": "ab".repeat(32) },
            "info": {
                "duration": 4 * SECOND,
                "container": "mov,mp4,m4a,3gp,3g2,mj2",
                "video": { "width": 320, "height": 180, "rate": "30", "codec": "h264" },
                "audio": { "sample_rate": 48000, "channels": 1, "codec": "aac" }
            }
        })
    }

    /// Imports media as the user would and returns the asset's id.
    fn import(workspace: &Workspace, args: Value) -> String {
        let added = workspace
            .execute(Request::single("media.add", args))
            .unwrap();
        added.results[0]["asset"].as_str().unwrap().to_string()
    }

    /// Places an asset at `seconds` and returns the clip's id.
    fn place(workspace: &Workspace, asset: &str, seconds: i64) -> String {
        let placed = workspace
            .execute(Request::single(
                "clip.add",
                json!({ "asset": asset, "start": { "flicks": seconds * SECOND } }),
            ))
            .unwrap();
        placed.results[0]["clip"].as_str().unwrap().to_string()
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

    fn tracking(clip: &str, region: [f64; 4], at: i64) -> TrackRequest {
        let [x, y, width, height] = region;
        TrackRequest {
            clip: clip.into(),
            region: fosscut_engine::motion::Region {
                x,
                y,
                width,
                height,
            },
            at,
            mask: None,
            follow: None,
            expected_revision: None,
        }
    }

    fn multicam(angles: &[&str], sync: bool) -> MulticamRequest {
        MulticamRequest {
            angles: angles.iter().map(|a| a.to_string()).collect(),
            sync,
            ..MulticamRequest::default()
        }
    }

    #[test]
    fn measurements_refuse_what_they_cannot_measure_before_measuring() {
        let (workspace, runtime, work) = workspace("measureno");
        let closed = [
            split_scenes(&workspace, "clip_0001", None).unwrap_err(),
            stabilize(&workspace, "clip_0001", None, None).unwrap_err(),
            track(
                &workspace,
                &TrackRequest {
                    mask: Some("mask_0002".into()),
                    ..tracking("clip_0001", [0.5, 0.5, 0.2, 0.2], 0)
                },
            )
            .unwrap_err(),
            create_multicam(&workspace, &multicam(&["asset_0001", "asset_0002"], true))
                .unwrap_err(),
        ];
        for error in closed {
            assert!(error.message.contains("no project"), "{error:?}");
        }
        open_new(&workspace, &work, 1280, 720);
        let video = import(&workspace, unplayable_video("cam-a.mp4", 11));
        let clip = place(&workspace, &video, 0);
        let title = workspace
            .execute(Request::single(
                "text.add",
                json!({ "text": "Hello", "duration": "2s" }),
            ))
            .unwrap()
            .results[0]["clip"]
            .as_str()
            .unwrap()
            .to_string();
        let before = revision(&workspace);

        let text = stabilize(&workspace, &title, None, None).unwrap_err();
        assert_eq!(text.code, codes::INVALID_ARGUMENTS);
        assert!(
            text.message.contains("text clip") && text.message.contains("video clips"),
            "a title cannot be stabilized: {text:?}"
        );
        let scenes = split_scenes(&workspace, &title, None).unwrap_err();
        assert!(scenes.message.contains("only video clips"), "{scenes:?}");
        let strong = stabilize(&workspace, &clip, Some(5.0), None).unwrap_err();
        assert!(strong.message.contains("outside 0 to 3"), "{strong:?}");
        let unparsed = stabilize(&workspace, "not an id", None, None).unwrap_err();
        assert_eq!(unparsed.code, codes::INVALID_ARGUMENTS);
        assert!(unparsed.message.starts_with("clip:"), "{unparsed:?}");
        // The revision is checked first, so a stale window measures nothing.
        let stale = stabilize(&workspace, &clip, None, Some(before - 1)).unwrap_err();
        assert_eq!(stale.code, codes::REVISION_CONFLICT);
        assert_eq!(stale.details["current_revision"], before);
        let stale = split_scenes(&workspace, &clip, Some(before + 1)).unwrap_err();
        assert_eq!(stale.code, codes::REVISION_CONFLICT);

        let region = [0.5, 0.5, 0.2, 0.2];
        for (mask, follow) in [(None, None), (Some("mask_0009"), Some(title.as_str()))] {
            let neither = track(
                &workspace,
                &TrackRequest {
                    mask: mask.map(str::to_string),
                    follow: follow.map(str::to_string),
                    ..tracking(&clip, region, 0)
                },
            )
            .unwrap_err();
            assert_eq!(neither.code, codes::INVALID_ARGUMENTS);
            assert!(
                neither.message.contains("either a mask"),
                "a mask or a follower, not {mask:?} and {follow:?}: {neither:?}"
            );
        }
        let follows = |clip: &str, region: [f64; 4], at: i64, follow: &str| {
            track(
                &workspace,
                &TrackRequest {
                    follow: Some(follow.into()),
                    ..tracking(clip, region, at)
                },
            )
            .unwrap_err()
        };
        let text = follows(&title, region, 0, &clip);
        assert!(text.message.contains("video clips"), "{text:?}");
        let itself = follows(&clip, region, 0, &clip);
        assert!(
            itself.message.contains("cannot follow itself"),
            "{itself:?}"
        );
        let flat = follows(&clip, [0.5, 0.5, 0.0, 0.2], 0, &title);
        assert!(
            flat.message.contains("fractions of the picture"),
            "{flat:?}"
        );
        let outside = follows(&clip, region, 9 * SECOND, &title);
        assert!(outside.message.contains("outside"), "{outside:?}");
        let no_mask = track(
            &workspace,
            &TrackRequest {
                mask: Some("mask_0009".into()),
                ..tracking(&clip, region, 0)
            },
        )
        .unwrap_err();
        assert!(no_mask.message.contains("has no mask"), "{no_mask:?}");

        // Multicam needs two video assets; the dry run refuses before any sound is read.
        let sound = import(
            &workspace,
            json!({
                "name": "voice.wav",
                "kind": "audio",
                "path": { "absolute": "/nowhere/voice.wav" },
                "identity": { "size": 12, "sample_blake3": "cd".repeat(32) },
                "info": { "duration": 4 * SECOND, "audio": { "sample_rate": 48000, "channels": 1 } }
            }),
        );
        let after_import = revision(&workspace);
        let one = create_multicam(&workspace, &multicam(&[&video], true)).unwrap_err();
        assert_eq!(one.code, codes::MULTICAM_INVALID);
        assert!(one.message.contains("at least two"), "{one:?}");
        let audio = create_multicam(&workspace, &multicam(&[&video, &sound], true)).unwrap_err();
        assert_eq!(audio.code, codes::MULTICAM_INVALID);
        assert!(audio.message.contains("not a video asset"), "{audio:?}");
        let missing =
            create_multicam(&workspace, &multicam(&[&video, "asset_0099"], true)).unwrap_err();
        assert_eq!(missing.code, codes::ASSET_NOT_FOUND);
        let unparsed =
            create_multicam(&workspace, &multicam(&[&video, "cam b"], false)).unwrap_err();
        assert!(unparsed.message.starts_with("angle:"), "{unparsed:?}");
        let stale = create_multicam(
            &workspace,
            &MulticamRequest {
                expected_revision: Some(before),
                ..multicam(&[&video, &video], true)
            },
        )
        .unwrap_err();
        assert_eq!(stale.code, codes::REVISION_CONFLICT);
        assert_eq!(revision(&workspace), after_import, "nothing was applied");
        done(workspace, runtime, work);
    }

    /// The clips of `timeline` (the primary one when null) as the window reads them.
    fn window_clips(workspace: &Workspace, timeline: &Value) -> Vec<Value> {
        let mut args = json!({ "detail": "full" });
        if !timeline.is_null() {
            args["timeline"] = timeline.clone();
        }
        let view = read(workspace, "timeline.read", args).unwrap();
        view["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["clips"].as_array().unwrap().clone())
            .collect()
    }

    #[test]
    fn multicam_clips_show_their_angle_and_cut_between_angles_by_number() {
        let (workspace, runtime, work) = workspace("multicam");
        open_new(&workspace, &work, 1280, 720);
        let a = import(&workspace, unplayable_video("cam-a.mp4", 21));
        let b = import(&workspace, unplayable_video("cam-b.mp4", 22));
        let shown = window_clips(&workspace, &Value::Null);
        assert!(shown.is_empty());
        let made = create_multicam(
            &workspace,
            &MulticamRequest {
                sound: Some(2),
                name: Some("Interview".into()),
                start: Some(SECOND),
                expected_revision: Some(revision(&workspace)),
                ..multicam(&[&a, &b], false)
            },
        )
        .unwrap();
        assert_eq!(made["sync"], Value::Null, "nothing measured without sync");
        let created = &made["response"]["results"][0];
        let id = created["clip"]["clip"].clone();
        let entry = last_edit(&workspace);
        assert_eq!(entry["actor"], "human");
        assert_eq!(entry["reason"], "Create multicam");

        let clips = window_clips(&workspace, &Value::Null);
        let placed = clip(&clips, &id);
        assert_eq!(placed["kind"], "timeline");
        assert_eq!(placed["start"]["flicks"], SECOND, "at the start asked");
        assert_eq!(placed["timeline_name"], "Interview");
        assert_eq!(
            placed["angle"], created["angles"][0],
            "a new multicam clip shows its first angle"
        );
        assert_eq!(
            placed["angles"],
            json!([
                { "number": 1, "id": created["angles"][0], "name": "Angle 1: cam-a.mp4" },
                { "number": 2, "id": created["angles"][1], "name": "Angle 2: cam-b.mp4" },
            ]),
            "numbered as multicam.switch numbers them"
        );
        let inner = window_clips(&workspace, &created["timeline"]);
        assert!(
            inner.iter().all(|c| c.get("angles").is_none()),
            "only multicam clips are marked"
        );

        // Cutting to angle 2 at 3 s: the part after the cut shows it.
        workspace
            .execute(Request::single(
                "multicam.switch",
                json!({ "clip": id, "at": { "flicks": 3 * SECOND }, "angle": 2 }),
            ))
            .unwrap();
        let clips = window_clips(&workspace, &Value::Null);
        let parts: Vec<(i64, Value)> = clips
            .iter()
            .map(|c| (c["start"]["flicks"].as_i64().unwrap(), c["angle"].clone()))
            .collect();
        assert_eq!(
            parts,
            [
                (SECOND, created["angles"][0].clone()),
                (3 * SECOND, created["angles"][1].clone())
            ]
        );
        assert!(
            clips
                .iter()
                .all(|c| c["angles"].as_array().unwrap().len() == 2)
        );
        done(workspace, runtime, work);
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

    fn ffmpeg(folder: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("ffmpeg")
            .current_dir(folder)
            .args(["-y", "-v", "error"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg {args:?}");
    }

    #[test]
    fn stabilizing_and_multicam_sync_measure_generated_media() {
        if !ffmpeg_programs() {
            eprintln!("skipped: ffmpeg or ffprobe is missing");
            return;
        }
        let (workspace, runtime, work) = workspace("measured");
        // Two angles of one scene: a moving test picture over a noisy room, the second
        // started recording a second after the first.
        ffmpeg(
            &work,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=30:duration=4",
                "-f",
                "lavfi",
                "-i",
                "anoisesrc=duration=4:color=pink:seed=7:sample_rate=48000",
                "-af",
                "volume='0.15+0.85*gt(sin(2*PI*1.3*t)*sin(2*PI*0.37*t+0.5),0.3)':eval=frame",
                "-c:v",
                "mpeg4",
                "-q:v",
                "3",
                "-c:a",
                "pcm_s16le",
                "-shortest",
                "cam-a.mov",
            ],
        );
        ffmpeg(
            &work,
            &[
                "-ss",
                "1",
                "-i",
                "cam-a.mov",
                "-c:v",
                "mpeg4",
                "-q:v",
                "3",
                "-c:a",
                "pcm_s16le",
                "cam-b.mov",
            ],
        );
        let opened = open_new(&workspace, &work, 320, 180);
        let media = |file: &str| {
            fosscut_engine::import::media_add_args(&work.join(file), None, &opened.bundle).unwrap()
        };
        let a = import(&workspace, media("cam-a.mov"));
        let b = import(&workspace, media("cam-b.mov"));
        let clip = place(&workspace, &a, 0);

        let steady = stabilize(&workspace, &clip, Some(0.5), Some(revision(&workspace))).unwrap();
        let zoom = steady["motion"]["zoom"].as_f64().unwrap();
        assert!((1.0..=1.5).contains(&zoom), "zoom {zoom}");
        assert!(steady["motion"]["frames"].as_u64().unwrap() >= 2);
        assert!(
            steady["motion"].get("commands").is_none(),
            "the response shows them"
        );
        assert_eq!(steady["response"]["committed"], true);
        let clips = window_clips(&workspace, &Value::Null);
        let keyed = &clip_of(&clips, &clip)["keyframes"];
        for property in [
            "transform.position",
            "transform.rotation",
            "transform.scale",
        ] {
            assert!(
                keyed[property]
                    .as_array()
                    .is_some_and(|keys| !keys.is_empty()),
                "{property}: {keyed}"
            );
        }
        let entry = last_edit(&workspace);
        assert_eq!(
            (entry["actor"].as_str(), entry["reason"].as_str()),
            (Some("human"), Some("Stabilize"))
        );

        let made = create_multicam(
            &workspace,
            &MulticamRequest {
                start: Some(10 * SECOND),
                ..multicam(&[&a, &b], true)
            },
        )
        .unwrap();
        let offsets = made["sync"]["offsets"].as_array().unwrap();
        assert_eq!(offsets[0], 0);
        let later = offsets[1].as_i64().unwrap() as f64 / SECOND as f64;
        assert!(
            (later - 1.0).abs() < 0.01,
            "the second angle started 1 s later: {later}"
        );
        assert!(made["sync"]["confidence"][1].as_f64().unwrap() > 0.3);
        let created = &made["response"]["results"][0];
        let inner = window_clips(&workspace, &created["timeline"]);
        let starts: Vec<i64> = inner
            .iter()
            .map(|c| c["start"]["flicks"].as_i64().unwrap())
            .collect();
        assert_eq!(starts, [SECOND, 0], "angle 2 (the lower track) a second in");
        done(workspace, runtime, work);
    }

    fn clip_of<'a>(clips: &'a [Value], id: &str) -> &'a Value {
        clips.iter().find(|c| c["id"] == id).unwrap()
    }

    #[test]
    fn export_presets_are_listed_with_the_resolutions() {
        let listing = presets_listing();
        let presets = listing["presets"].as_array().unwrap();
        assert!(presets.iter().any(|p| p["name"] == "mp4-h264"));
        assert!(
            presets
                .iter()
                .all(|p| p["available"].is_boolean() && p["extension"].is_string())
        );
        assert!(
            listing["resolutions"]
                .as_array()
                .unwrap()
                .contains(&json!({ "name": "720p", "pixels": 720 }))
        );
    }
}
