//! Generates the Tauri context, with the app's commands listed so that the window may call
//! only these (capabilities/default.json grants them).

fn main() {
    let manifest = tauri_build::AppManifest::new().commands(&[
        "app_info",
        "recent_projects",
        "project_current",
        "project_open",
        "project_pick",
        "project_new",
        "project_close",
        "query",
        "execute",
        "clients",
        "color_suggest",
        "audio_normalize",
        "scenes_split",
        "clip_stabilize",
        "clip_track",
        "multicam_create",
        "preview_viewport",
        "preview_show",
        "preview_play",
        "preview_pause",
        "preview_shuttle",
        "window_fullscreen",
        "media_reveal",
        "media_relink",
        "project_summaries",
        "project_trash",
        "trash_list",
        "trash_restore",
        "trash_delete",
        "library_list",
        "library_add",
        "export_presets",
        "export_pick",
        "export_start",
        "export_status",
        "export_cancel",
        "record_devices",
        "record_start",
        "record_status",
        "record_stop",
    ]);
    if let Err(error) =
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
    {
        eprintln!("the Tauri build step failed: {error:#}");
        std::process::exit(1);
    }
}
