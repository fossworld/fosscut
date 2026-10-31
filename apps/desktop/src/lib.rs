//! FossCut's desktop app (ADR-0010): a Tauri window over the same engine as the CLI and
//! the MCP server. The app hosts the open project live (ADR-0022), so an agent working
//! through `fosscut mcp` edits the session the window shows: its changes appear as they
//! commit and undo here. The preview renders with wgpu behind the webview (path A).
//!
//! - [`workspace`]: the open project, hosted live, and recent projects.
//! - [`preview`] and [`surface`]: the preview thread and the window's GPU surface;
//!   [`audio`]: the sound playback follows.
//! - [`recording`]: recordings from a microphone, camera or screen, imported and placed
//!   once they succeed.
//! - [`commands`]: what the UI may ask, including the built-in library, exports run as
//!   background jobs, and edits measured from media (scene cuts, stabilization, tracking
//!   and multicam sync); the engine client in `ui/` is the only caller.
//! - [`preferences`]: the person's preferences (ADR-0031), read at start, changed from the
//!   Settings window and written into the requests the app builds; [`announce`]: the
//!   sound and notifications for finished exports and recordings; [`words`]: the texts the
//!   app shows itself, translated.
//! - [`models`]: local models (ADR-0033): installing them once the person agreed, the
//!   analyses background removal and body effects need, text to speech, and translating
//!   and dubbing captions.
//! - [`performance`]: hardware acceleration, what the preview decodes and keeps, the cache
//!   policy, and the proxies and remuxed copies made in the background (ADR-0032).
//! - [`auto_edit`]: edits planned from the media's analyses (auto reframe, camera tracking,
//!   vertical versions, beat markers and cuts, shorts), applied as the user's one step.
//! - [`media`]: thumbnails and waveforms of the open project's media for the media bin and
//!   the timeline, cached by content, and importing files as the user's edits.

#![forbid(unsafe_code)]

pub mod announce;
pub mod audio;
pub mod auto_edit;
pub mod commands;
pub mod library;
pub mod media;
pub mod models;
pub mod performance;
pub mod preferences;
pub mod preview;
pub mod recording;
pub mod surface;
pub mod words;
pub mod workspace;

use std::sync::{Arc, Mutex};

use fosscut_engine::jobs::Jobs;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};

use crate::audio::Speaker;
use crate::library::Library;
use crate::media::MediaViews;
use crate::models::ModelInstalls;
use crate::performance::MediaJobs;
use crate::preferences::PreferenceStore;
use crate::preview::{Job, Preview, Scene};
use crate::recording::Recordings;
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
    /// Exports and recordings running in the background, followed by id.
    pub jobs: Arc<Jobs>,
    /// Recordings started from the window, imported once they succeed.
    pub recordings: Arc<Recordings>,
    /// The person's preferences.
    pub preferences: Arc<PreferenceStore>,
    /// The sound output, shared with the preview, for the app's own sounds.
    pub speaker: Option<Speaker>,
    /// The page of the newer release the last update check found.
    pub release: Mutex<Option<String>>,
    /// FossCut's cache folder: analysis, proxies, remuxed copies and rendered frames.
    pub cache: std::path::PathBuf,
    /// The proxies and remuxed copies being made.
    pub media_jobs: Arc<MediaJobs>,
    /// The local models being installed (ADR-0033).
    pub installs: Arc<ModelInstalls>,
    /// Thumbnails and waveforms of the open project's media, cached in the cache folder.
    pub media_views: Arc<MediaViews>,
    /// The app menu's Settings item, renamed when the language changes (macOS).
    pub settings_item: Option<tauri::menu::MenuItem<tauri::Wry>>,
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
    plan_media(app);
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
                // Imports may bring media that wants a proxy or a remuxed copy.
                plan_media(&app);
            }
        })
        .ok();
    if let Ok(mut slot) = state.follower.lock() {
        // The previous follower ends by itself once its project closes.
        *slot = follower;
    }
}

/// Asks for the proxies and remuxed copies the open project's media should have, as the
/// preferences say, and tells the window when that starts work.
pub(crate) fn plan_media(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some((bundle, project, _)) = state.workspace.snapshot() else {
        return;
    };
    let wanted = performance::wanted(
        &project,
        &bundle,
        &state.preferences.current(),
        &state.cache,
    );
    if state.media_jobs.ask(&bundle, &project, wanted) {
        let _ = app.emit("media-work", state.media_jobs.status());
    }
}

/// Applies the preferences that change speed and disk use: hardware acceleration, what
/// the preview decodes and keeps, and the cache policy jobs follow; then asks for the
/// copies of media they now want.
pub(crate) fn apply_preferences(app: &AppHandle) {
    let state = app.state::<AppState>();
    let preferences = state.preferences.current();
    performance::apply_hardware(&preferences);
    state.preview.send(Job::Options {
        options: performance::preview_options(&preferences, &state.cache),
        auto_render: preferences.performance.auto_render,
    });
    state
        .jobs
        .set_cache_policy(Some(performance::cache_policy(&preferences, &state.cache)));
    if let Some(item) = &state.settings_item {
        let _ = item.set_text(words::word(&preferences.language, words::Word::Settings));
    }
    plan_media(app);
}

/// The app menu with a Settings item (Command and comma) that asks the window to open
/// Settings, on macOS, where every app has one; elsewhere the window's button and
/// Control and comma open them.
fn settings_menu(
    app: &tauri::App,
    language: &str,
) -> tauri::Result<Option<tauri::menu::MenuItem<tauri::Wry>>> {
    if !cfg!(target_os = "macos") {
        return Ok(None);
    }
    use tauri::menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
    let handle = app.handle();
    let menu = Menu::default(handle)?;
    let item = MenuItem::with_id(
        handle,
        "settings",
        words::word(language, words::Word::Settings),
        true,
        Some("CmdOrCtrl+,"),
    )?;
    if let Some(MenuItemKind::Submenu(first)) = menu.items()?.into_iter().next() {
        first.insert(&PredefinedMenuItem::separator(handle)?, 1)?;
        first.insert(&item, 2)?;
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id() == "settings" {
            let _ = app.emit("open-settings", ());
        }
    });
    Ok(Some(item))
}

/// Starts the app. A path on the command line opens that project.
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let preferences = Arc::new(PreferenceStore::open(fosscut_engine::preferences::path()));
            if let Some(problem) = preferences.view().problem {
                eprintln!("fosscut-desktop: preferences: {problem}; using the defaults");
            }
            let recent = app
                .path()
                .app_data_dir()
                .ok()
                .map(|dir| dir.join("recent.json"));
            let workspace = Arc::new(Workspace::new(fosscut_engine::live::runtime_dir(), recent));
            let window = app
                .get_webview_window("main")
                .ok_or("the main window is missing")?;
            let speaker = Speaker::open();
            let preview = match Surface::create(window.clone()) {
                Ok((surface, renderer)) => {
                    let (heads, fails) = (app.handle().clone(), app.handle().clone());
                    Preview::start(
                        surface,
                        renderer,
                        speaker.clone(),
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
            let jobs = Arc::new(Jobs::new());
            let media_jobs = Arc::new(MediaJobs::default());
            let caches = fosscut_engine::cache_dir();
            let (told, reopen) = (app.handle().clone(), preview.clone());
            media_jobs.run(Arc::clone(&jobs), caches.clone(), move |work, made| {
                if made {
                    reopen.send(Job::ReopenMedia);
                }
                let _ = told.emit("media-work", work);
            })?;
            // The cache policy applies once now, and after every job from here on.
            let policy = performance::cache_policy(&preferences.current(), &caches);
            let _ = std::thread::Builder::new()
                .name("fosscut-cache-policy".into())
                .spawn(move || policy.enforce());
            let settings_item = settings_menu(app, &preferences.current().language)?;
            let media_views = Arc::new(MediaViews::new(&caches));
            app.manage(AppState {
                workspace: Arc::clone(&workspace),
                preview,
                library: Arc::new(Library::new(&data, &cache)),
                jobs,
                recordings: Arc::new(Recordings::default()),
                preferences,
                speaker,
                release: Mutex::new(None),
                cache: caches,
                media_jobs,
                installs: Arc::new(ModelInstalls::default()),
                media_views,
                settings_item,
                follower: Mutex::new(None),
            });
            apply_preferences(app.handle());
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
            commands::audio_normalize,
            commands::scenes_split,
            commands::clip_stabilize,
            commands::clip_track,
            commands::multicam_create,
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
            commands::library_list,
            commands::library_add,
            commands::export_presets,
            commands::export_pick,
            commands::export_start,
            commands::export_status,
            commands::export_cancel,
            commands::record_devices,
            commands::record_start,
            commands::record_status,
            commands::record_stop,
            commands::preferences_get,
            commands::preferences_set,
            commands::preferences_reset,
            commands::update_check,
            commands::update_open,
            commands::sound_play,
            commands::folder_pick,
            commands::cache_info,
            commands::cache_clear,
            commands::media_sources,
            commands::media_work,
            commands::models_list,
            commands::models_install,
            commands::models_install_status,
            commands::subject_analyze,
            commands::subject_status,
            commands::subject_cancel,
            commands::voices_list,
            commands::speech_preview,
            commands::speech_add,
            commands::captions_translate,
            commands::captions_dub,
            commands::media_thumbnails,
            commands::media_waveform,
            commands::media_import,
            commands::clip_reframe,
            commands::clip_camera_track,
            commands::timeline_vertical,
            commands::beats_markers,
            commands::beats_cut,
            commands::shorts_candidates,
            commands::shorts_make,
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
