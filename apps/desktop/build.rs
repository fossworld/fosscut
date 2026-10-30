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
        "preview_viewport",
        "preview_show",
        "preview_play",
        "preview_pause",
    ]);
    if let Err(error) =
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
    {
        eprintln!("the Tauri build step failed: {error:#}");
        std::process::exit(1);
    }
}
