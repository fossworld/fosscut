//! What the window may ask of the app (ADR-0010, item 4): the engine client in `ui/` calls
//! only these. Errors are the typed FossCut errors, as the CLI and MCP return them.

use std::path::PathBuf;
use std::sync::Arc;

use fosscut_core::{ApiError, Request, Response, codes};
use fosscut_engine::live::ClientInfo;
use fosscut_time::Flicks;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;
use crate::library::{ProjectSummary, TrashEntry};
use crate::preview::Job;
use crate::workspace::{NewProject, Opened, Workspace};

type Answer<T> = Result<T, ApiError>;

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
}
