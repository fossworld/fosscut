//! FossCut's desktop app (ADR-0010): a Tauri window over the same engine as the CLI and
//! the MCP server. The app hosts the open project live (ADR-0022), so an agent working
//! through `fosscut mcp` edits the session the window shows: its changes appear as they
//! commit and undo here. The preview renders with wgpu behind the webview (path A).
//!
//! - [`workspace`]: the open project, hosted live, and recent projects.
//! - [`preview`] and [`surface`]: the preview thread and the window's GPU surface;
//!   [`audio`]: the sound playback follows.
//! - [`commands`]: what the UI may ask; the engine client in `ui/` is the only caller.

#![forbid(unsafe_code)]

pub mod audio;
pub mod commands;
pub mod library;
pub mod preview;
pub mod surface;
pub mod workspace;

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};

use crate::library::Library;
use crate::preview::{Job, Preview, Scene};
use crate::surface::Surface;
use crate::workspace::Workspace;

/// What the app keeps while it runs.
pub struct AppState {
    /// The open project.
    pub workspace: Arc<Workspace>,
    /// The preview thread.
    pub preview: Preview,
    /// Recent projects' summaries and the trash.
    pub library: Arc<Library>,
    /// The thread that forwards the open project's changes, while one is open.
    follower: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// Shows the open project in the preview and forwards every later change to the preview
/// and the window (`project-changed`), whoever made it, until the project closes.
pub(crate) fn follow(app: &AppHandle, state: &State<'_, AppState>) {
    let workspace = Arc::clone(&state.workspace);
    let preview = state.preview.clone();
    let scene = |workspace: &Workspace| {
        workspace.snapshot().map(|(bundle, project, timeline)| {
            Arc::new(Scene {
                bundle,
                project,
                timeline,
            })
        })
    };
    preview.send(Job::Scene(scene(&workspace)));
    let Some(changes) = workspace.subscribe() else {
        return;
    };
    let app = app.clone();
    let follower = std::thread::Builder::new()
        .name("fosscut-follow".into())
        .spawn(move || {
            for change in changes {
                preview.send(Job::Scene(scene(&workspace)));
                let _ = app.emit("project-changed", change);
            }
        })
        .ok();
    if let Ok(mut slot) = state.follower.lock() {
        // The previous follower ends by itself once its project closes.
        *slot = follower;
    }
}

/// Starts the app. A path on the command line opens that project.
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let recent = app
                .path()
                .app_data_dir()
                .ok()
                .map(|dir| dir.join("recent.json"));
            let workspace = Arc::new(Workspace::new(fosscut_engine::live::runtime_dir(), recent));
            let window = app
                .get_webview_window("main")
                .ok_or("the main window is missing")?;
            let preview = match Surface::create(window.clone()) {
                Ok((surface, renderer)) => {
                    let (heads, fails) = (app.handle().clone(), app.handle().clone());
                    Preview::start(
                        surface,
                        renderer,
                        audio::Speaker::open(),
                        move |playhead| {
                            let _ = heads.emit("playhead", playhead);
                        },
                        move |message| {
                            eprintln!("fosscut-desktop: preview: {message}");
                            let _ = fails.emit("preview-error", message);
                        },
                    )?
                }
                Err(message) => {
                    eprintln!("fosscut-desktop: no preview: {message}");
                    Preview::off()
                }
            };
            let resized = preview.clone();
            window.on_window_event(move |event| {
                if matches!(
                    event,
                    WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
                ) {
                    resized.send(Job::Resized);
                }
            });
            let data = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir());
            let cache = app
                .path()
                .app_cache_dir()
                .unwrap_or_else(|_| std::env::temp_dir());
            app.manage(AppState {
                workspace: Arc::clone(&workspace),
                preview,
                library: Arc::new(Library::new(&data, &cache)),
                follower: Mutex::new(None),
            });
            if let Some(path) = std::env::args_os().nth(1) {
                let path = std::path::PathBuf::from(path);
                match workspace.open(&path) {
                    Ok(_) => follow(app.handle(), &app.state::<AppState>()),
                    Err(error) => eprintln!("fosscut-desktop: {}: {error}", path.display()),
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::recent_projects,
            commands::project_current,
            commands::project_open,
            commands::project_pick,
            commands::project_new,
            commands::project_close,
            commands::query,
            commands::execute,
            commands::clients,
            commands::color_suggest,
            commands::preview_viewport,
            commands::preview_show,
            commands::preview_play,
            commands::preview_pause,
            commands::preview_shuttle,
            commands::window_fullscreen,
            commands::media_reveal,
            commands::media_relink,
            commands::project_summaries,
            commands::project_trash,
            commands::trash_list,
            commands::trash_restore,
            commands::trash_delete,
        ])
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|app, event| {
            if let RunEvent::Exit = event {
                // Hand the project back to the disk, so agents keep working without the app.
                app.state::<AppState>().workspace.close();
            }
        }),
        Err(error) => eprintln!("fosscut-desktop could not start: {error}"),
    }
}
