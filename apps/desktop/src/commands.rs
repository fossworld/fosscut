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
use crate::preview::Job;
use crate::workspace::{NewProject, Opened, Workspace};

type Answer<T> = Result<T, ApiError>;

/// How long `export_cancel` waits for the export to stop before answering.
const CANCEL_WAIT: Duration = Duration::from_secs(2);

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// The open project's bundle, its state now and its primary timeline.
fn open_project(workspace: &Workspace) -> Answer<(PathBuf, Project, Id)> {
    workspace
        .snapshot()
        .ok_or_else(|| invalid("no project is open").with_hint("Open or create a project first."))
}

/// A text the window sent, or `None` when it sent nothing or only spaces.
fn given(text: Option<&str>) -> Option<&str> {
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
    let opened = blocking(move || {
        let Some(folder) = dialog
            .dialog()
            .file()
            .set_title("Open a FossCut project")
            .blocking_pick_folder()
        else {
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

/// Asks where to save a new project, creates it with `width`, `height` and `rate`, and
/// opens it; `None` when the user cancels.
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
    let opened = blocking(move || {
        let Some(file) = dialog
            .dialog()
            .file()
            .set_title("Create a FossCut project")
            .set_file_name("Untitled.fosscut")
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let mut path = file
            .into_path()
            .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, e.to_string()))?;
        if path.extension().is_none_or(|ext| ext != "fosscut") {
            path.set_extension("fosscut");
        }
        let new = NewProject {
            path,
            name: None,
            width: width.unwrap_or(1920),
            height: height.unwrap_or(1080),
            rate: rate.unwrap_or_else(|| "30".into()),
        };
        workspace.create(&new).map(Some)
    })
    .await?;
    if opened.is_some() {
        crate::follow(&app, &state);
    }
    Ok(opened)
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
    blocking(move || {
        let mut value = workspace.query(&name, args.unwrap_or(Value::Null))?;
        // The window offers to relink media whose files are gone.
        if name == "media.list"
            && let Some((bundle, _, _)) = workspace.snapshot()
        {
            fosscut_engine::import::mark_missing(&mut value, &bundle);
        }
        Ok(value)
    })
    .await
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
    blocking(move || {
        let (bundle, _, _) = workspace
            .snapshot()
            .ok_or_else(|| ApiError::new(codes::INVALID_ARGUMENTS, "no project is open"))?;
        let Some(file) = dialog
            .dialog()
            .file()
            .set_title("Find the media file")
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
/// `target_lufs` (-14 LUFS when omitted) with peaks under -1 dBFS, with the `clip.set`
/// calls that apply them; the window applies those with `execute`, as any edit.
#[tauri::command]
pub async fn audio_normalize(
    state: State<'_, AppState>,
    clips: Vec<String>,
    target_lufs: Option<f64>,
) -> Answer<Value> {
    let workspace = Arc::clone(&state.workspace);
    blocking(move || normalize(&workspace, &clips, target_lufs)).await
}

/// What `audio_normalize` answers for the project open in `workspace`.
fn normalize(workspace: &Workspace, clips: &[String], target_lufs: Option<f64>) -> Answer<Value> {
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
        target_lufs.unwrap_or(fosscut_engine::loudness::DEFAULT_TARGET_LUFS),
        fosscut_engine::loudness::DEFAULT_CEILING_DBFS,
        true,
    )
    .map_err(ApiError::from)?;
    serde_json::to_value(normalization).map_err(|e| ApiError::new(codes::INTERNAL, e.to_string()))
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

/// Shows the frame at `flicks`, pausing playback.
#[tauri::command]
pub fn preview_show(state: State<'_, AppState>, flicks: i64) {
    state.preview.send(Job::Show(Flicks::new(flicks)));
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
    // The revision on screen guards the first write; a sound's placement follows its import.
    let mut request = if matches!(item.kind, "sound" | "music") {
        let mut import = Request::single("media.add", library::sound_media(&bundle, &item.name)?);
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
                // A free audio track, so a sound never lands on another.
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
    blocking(move || {
        let mut file = dialog
            .dialog()
            .file()
            .set_title("Export to")
            .set_file_name(name);
        if let Some(folder) = workspace
            .snapshot()
            .and_then(|(bundle, _, _)| bundle.parent().map(PathBuf::from))
        {
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
/// everything an export checks before rendering, and returns the job at once.
#[tauri::command]
pub async fn export_start(state: State<'_, AppState>, choice: ExportChoice) -> Answer<JobStatus> {
    let workspace = Arc::clone(&state.workspace);
    let jobs = Arc::clone(&state.jobs);
    blocking(move || start_export(&workspace, &jobs, &choice)).await
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
        let closed = normalize(&workspace, &["clip_0001".into()], None).unwrap_err();
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
        let unparsed = normalize(&workspace, &["not an id".into()], None).unwrap_err();
        assert_eq!(unparsed.code, codes::INVALID_ARGUMENTS);
        let none = normalize(&workspace, &[], None).unwrap_err();
        assert!(none.message.contains("at least one clip"), "{none:?}");

        let added = workspace
            .execute(Request::single(
                "text.add",
                json!({ "text": "Quiet", "duration": "1s" }),
            ))
            .unwrap();
        let title = added.results[0]["clip"].as_str().unwrap().to_string();
        let silent = normalize(&workspace, std::slice::from_ref(&title), None).unwrap_err();
        assert_eq!(silent.code, codes::INVALID_ARGUMENTS);
        assert!(silent.message.contains("without sound"), "{silent:?}");
        let target = normalize(&workspace, &[title], Some(-60.0)).unwrap_err();
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
