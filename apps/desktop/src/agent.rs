//! The user's own agent from the window (ADR-0043): an AI edit of a video, made by the
//! agent command line the user installed (Claude Code first), headless, against the
//! project the app hosts, so its edits appear live and undo like any other.
//!
//! - **A project from a video** ([`project_from_video`]): a new project beside the video,
//!   at the video's frame rate and the canvas the user chose, with the video imported and
//!   on the timeline.
//! - **The agent** ([`setup`], [`start`]): found as the `agent.command` preference says, or
//!   by itself; given a brief the user may edit, FossCut's skills in a working folder, and
//!   an MCP configuration that starts this app's own executable in `mcp` mode for the open
//!   project. Asked without confirmation it refuses with
//!   `fosscut.agent.confirmation_required`, whose details carry the command and the notes
//!   for the window to show first. It runs as a background job that the window follows and
//!   can cancel.
//! - **`mcp` mode** ([`mcp_main`]): `fosscut-desktop mcp [-p <bundle>]` serves FossCut's
//!   MCP server on stdio without opening a window, so the agent needs no separate
//!   `fosscut` install.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fosscut_core::{ApiError, Request, codes};
use fosscut_engine::agent::{self, AgentRequest, BriefContext, FoundAgent, McpServer, SkillFiles};
use fosscut_engine::jobs::{JobStatus, Jobs};
use fosscut_model::AssetKind;
use fosscut_time::Rate;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;
use crate::commands::{blocking, invalid, open_project};
use crate::words::{Word, word};
use crate::workspace::{NewProject, Opened, Workspace};

type Answer<T> = Result<T, ApiError>;

/// The argument that starts the app as FossCut's MCP server.
pub const MCP_MODE: &str = "mcp";

/// How long `agent_cancel` waits for the agent to stop before answering.
const CANCEL_WAIT: Duration = Duration::from_secs(3);

/// The project `fosscut-desktop mcp [-p|--project <bundle>]` names, when the arguments ask
/// for the MCP server: `None` when they do not, and an error for arguments it does not
/// take.
pub fn mcp_arguments(
    args: impl IntoIterator<Item = OsString>,
) -> Option<Result<Option<PathBuf>, String>> {
    let mut args = args.into_iter();
    if args.next()? != MCP_MODE {
        return None;
    }
    let mut project = None;
    while let Some(arg) = args.next() {
        if arg == "-p" || arg == "--project" {
            match args.next() {
                Some(path) => project = Some(PathBuf::from(path)),
                None => return Some(Err(format!("{} needs a project", arg.to_string_lossy()))),
            }
        } else {
            return Some(Err(format!(
                "unexpected argument {}; use: fosscut-desktop mcp [-p <project.fosscut>]",
                arg.to_string_lossy()
            )));
        }
    }
    Some(Ok(project.map(|p| std::path::absolute(&p).unwrap_or(p))))
}

/// Serves FossCut's MCP server on stdio when the arguments (without the program's name)
/// ask for it, until the client disconnects, and answers the exit code; `None` when they do
/// not, and the app starts as usual. No window opens in this mode.
pub fn mcp_main(args: impl IntoIterator<Item = OsString>) -> Option<i32> {
    let project = match mcp_arguments(args)? {
        Ok(project) => project,
        Err(message) => {
            eprintln!("fosscut-desktop: {message}");
            return Some(2);
        }
    };
    match tauri::async_runtime::block_on(fosscut_mcp::serve_stdio(project)) {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("fosscut-desktop: the MCP session failed: {error}");
            Some(1)
        }
    }
}

/// The standard frame rates a video's rate is rounded to when it is within 1% of one.
const RATES: [&str; 8] = [
    "24000/1001",
    "24",
    "25",
    "30000/1001",
    "30",
    "50",
    "60000/1001",
    "60",
];

/// The frame rate of a project made from a video of rate `probed`: the standard rate it is
/// close to, else `preferred`.
pub fn project_rate(probed: Option<Rate>, preferred: &str) -> String {
    let fps = |rate: Rate| f64::from(rate.num()) / f64::from(rate.den().max(1));
    probed
        .and_then(|probed| {
            RATES
                .iter()
                .filter_map(|text| {
                    let rate = text.parse::<Rate>().ok()?;
                    let off = (fps(rate) - fps(probed)).abs();
                    (off <= fps(rate) * 0.01).then_some((off, *text))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
        })
        .map_or_else(|| preferred.to_string(), |(_, rate)| rate.to_string())
}

/// Where a project made from `video` is suggested: beside it, named after it.
pub fn suggested_project(video: &Path) -> PathBuf {
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Untitled".into());
    video
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{stem}.fosscut"))
}

/// Creates a project at `path` with a `width` by `height` canvas at `video`'s frame rate (a
/// standard one near it, else `preferred_rate`), opens it in `workspace`, imports the video
/// and places it at the start of the timeline: two of the user's undoable steps. A file
/// that is not a video is refused before anything is created.
pub fn project_from_video(
    workspace: &Workspace,
    video: &Path,
    path: PathBuf,
    width: u32,
    height: u32,
    preferred_rate: &str,
) -> Answer<Opened> {
    let probed = fosscut_engine::import::probe(video)?;
    if probed.kind != AssetKind::Video {
        return Err(invalid(format!("{} is not a video", video.display()))
            .with_hint("Choose a video file with a picture, such as an .mp4 or .mov."));
    }
    let rate = project_rate(
        probed.info.video.as_ref().and_then(|v| v.rate),
        preferred_rate,
    );
    let mut path = path;
    if path.extension().is_none_or(|ext| ext != "fosscut") {
        path.set_extension("fosscut");
    }
    workspace.create(&NewProject {
        path,
        name: None,
        width,
        height,
        rate,
    })?;
    let imported = crate::media::import(workspace, &[video.to_path_buf()])?;
    let Some(asset) = imported.assets.first() else {
        let reason = imported
            .failed
            .first()
            .map(|f| f.error.clone())
            .unwrap_or_else(|| "it was not imported".into());
        return Err(ApiError::new(
            codes::INVALID_ARGUMENTS,
            format!("{} could not be imported: {reason}", video.display()),
        ));
    };
    let mut place = Request::single(
        "clip.add",
        json!({ "asset": asset, "start": { "flicks": 0 } }),
    );
    place.reason = Some("Start from a video".into());
    workspace.execute(place)?;
    workspace
        .current()?
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "the new project is not open"))
}

/// What the window shows before an agent runs (`agent_find`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentSetup {
    /// The agent found, or `None` when there is none.
    pub agent: Option<FoundAgent>,
    /// Where it was looked for, when it was not found.
    pub tried: Vec<String>,
    /// The brief FossCut suggests for the open project.
    pub brief: String,
    /// The turn cap suggested.
    pub max_turns: u32,
    /// The highest turn cap.
    pub max_turns_limit: u32,
    /// What the user is told before agreeing.
    pub notes: Vec<String>,
    /// The skill the brief follows.
    pub skill: String,
    /// The command that serves FossCut's MCP server from this app, for running an agent by
    /// hand.
    pub server: String,
    /// Lines that set up an agent by hand.
    pub setup: Vec<String>,
}

/// How the agent starts FossCut's MCP server for `bundle`: `exe` (this app) in `mcp` mode,
/// finding the app's live session in `workspace`'s runtime folder.
pub fn server_for(workspace: &Workspace, bundle: &Path, exe: &Path) -> McpServer {
    let mut env = agent::server_environment();
    env.insert(
        "FOSSCUT_RUNTIME_DIR".into(),
        workspace.runtime().display().to_string(),
    );
    McpServer {
        command: exe.to_path_buf(),
        args: vec![MCP_MODE.into(), "-p".into(), bundle.display().to_string()],
        env,
    }
}

/// The user-facing skills built into FossCut, for the agent's working folder.
pub fn skills() -> Vec<SkillFiles> {
    fosscut_mcp::skills::SKILLS
        .iter()
        .map(|skill| SkillFiles {
            name: skill.name.to_string(),
            files: skill
                .files
                .iter()
                .map(|(path, text)| ((*path).to_string(), (*text).to_string()))
                .collect(),
        })
        .collect()
}

/// The agent configured as an absolute path, if the preference names one.
fn configured(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    (!command.is_empty()).then(|| PathBuf::from(command))
}

/// What the window needs before an agent runs on the open project: the agent found (or
/// where it was looked for), the suggested brief and cap, the notes, and the lines that
/// set an agent up by hand.
pub fn setup(workspace: &Workspace, agent_command: &str, exe: &Path) -> Answer<AgentSetup> {
    let (bundle, project, timeline) = open_project(workspace)?;
    let format = project
        .timelines
        .get(&timeline)
        .map(|t| (t.format.width, t.format.height))
        .unwrap_or((1080, 1920));
    let video = project
        .media
        .assets
        .iter()
        .find(|asset| asset.kind == AssetKind::Video)
        .map(|asset| asset.name.clone());
    let brief = agent::default_brief(&BriefContext {
        project: project.manifest.name.clone(),
        bundle: bundle.clone(),
        width: format.0,
        height: format.1,
        video,
    });
    let (found, tried) = match agent::find(configured(agent_command).as_deref()) {
        Ok(found) => (Some(found), Vec::new()),
        Err(agent::AgentError::NotFound { tried }) => (None, tried),
        Err(other) => return Err(other.into()),
    };
    let server = server_for(workspace, &bundle, exe);
    Ok(AgentSetup {
        agent: found,
        tried,
        brief,
        max_turns: agent::DEFAULT_MAX_TURNS,
        max_turns_limit: agent::MAX_TURNS_LIMIT,
        notes: vec![agent::ACCOUNT_NOTE.into(), agent::TOOLS_NOTE.into()],
        skill: agent::REEL_SKILL.into(),
        server: format!("{} {MCP_MODE}", server.command.display()),
        setup: vec![server.setup_line()],
    })
}

/// What the window asks to run (`agent_start`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentChoice {
    /// What the agent is asked to do.
    pub brief: String,
    /// The turn cap; [`agent::DEFAULT_MAX_TURNS`] when omitted.
    pub max_turns: Option<u32>,
    /// The spending cap in US dollars, when the user set one.
    pub max_budget_usd: Option<f64>,
    /// True once the user agreed to the command and the notes.
    pub confirmed: bool,
}

/// Where the agent runs from: its working folder, and the program serving FossCut's MCP
/// server.
#[derive(Clone, Debug)]
pub struct Places {
    /// The folder that holds the agents' working folders.
    pub folder: PathBuf,
    /// This app's executable, which serves FossCut's MCP server in `mcp` mode.
    pub exe: PathBuf,
    /// The agent the preference names, as an absolute path, or empty.
    pub agent_command: String,
}

/// Starts the user's agent on the project open in `workspace` as a background job of
/// `jobs`. Without `confirmed` nothing runs: the refusal
/// `fosscut.agent.confirmation_required` carries the command and the notes in its details.
pub fn start(
    workspace: &Workspace,
    jobs: &Jobs,
    choice: &AgentChoice,
    places: &Places,
) -> Answer<JobStatus> {
    let (bundle, ..) = open_project(workspace)?;
    let request = AgentRequest {
        bundle: bundle.clone(),
        brief: choice.brief.clone(),
        max_turns: choice.max_turns.unwrap_or(agent::DEFAULT_MAX_TURNS),
        max_budget_usd: choice.max_budget_usd,
        agent: configured(&places.agent_command),
        server: server_for(workspace, &bundle, &places.exe),
        skills: skills(),
        folder: places.folder.clone(),
        confirmed: choice.confirmed,
    };
    let plan = agent::plan(&request)?;
    Ok(jobs.start_agent(plan)?)
}

/// Where this app runs agents from.
fn places(state: &State<'_, AppState>) -> Answer<Places> {
    let exe = std::env::current_exe().map_err(|e| {
        ApiError::new(
            codes::INTERNAL,
            format!("cannot tell where FossCut runs from: {e}"),
        )
    })?;
    Ok(Places {
        folder: state.cache.join("agent"),
        exe,
        agent_command: state.preferences.current().agent.command,
    })
}

// ---- Commands the window calls ----

/// Asks for a video to edit; `None` when the user cancels.
#[tauri::command]
pub async fn agent_video_pick(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Answer<Option<String>> {
    let language = state.preferences.current().language;
    blocking(move || {
        let mut pick = app
            .dialog()
            .file()
            .set_title(word(&language, Word::ChooseVideo));
        if let Some((name, extensions)) = crate::media::IMPORT_FILTERS
            .iter()
            .find(|(name, _)| *name == Word::VideoFiles)
        {
            pick = pick.add_filter(word(&language, *name), extensions);
        }
        let Some(picked) = pick.blocking_pick_file() else {
            return Ok(None);
        };
        let path = picked.into_path().map_err(|e| invalid(e.to_string()))?;
        Ok(Some(path.display().to_string()))
    })
    .await
}

/// Asks where to save a project made from `video` (beside it, named after it), creates it
/// with a `width` by `height` canvas at the video's frame rate, imports the video, places
/// it on the timeline and opens the project; `None` when the user cancels.
#[tauri::command]
pub async fn agent_project(
    app: AppHandle,
    state: State<'_, AppState>,
    video: String,
    width: u32,
    height: u32,
) -> Answer<Option<Opened>> {
    let dialog = app.clone();
    let workspace = Arc::clone(&state.workspace);
    let preferences = state.preferences.current();
    let opened = blocking(move || {
        let video = PathBuf::from(video.trim());
        if !video.is_file() {
            return Err(invalid(format!("{} is not a file", video.display())));
        }
        let suggested = suggested_project(&video);
        let mut save = dialog
            .dialog()
            .file()
            .set_title(word(&preferences.language, Word::CreateProject));
        if let Some(name) = suggested.file_name() {
            save = save.set_file_name(name.to_string_lossy());
        }
        if let Some(folder) = suggested.parent() {
            save = save.set_directory(folder);
        }
        let Some(file) = save.blocking_save_file() else {
            return Ok(None);
        };
        let path = file.into_path().map_err(|e| invalid(e.to_string()))?;
        let rate = crate::preferences::new_project_rate(&preferences, None);
        project_from_video(&workspace, &video, path, width, height, &rate).map(Some)
    })
    .await?;
    if opened.is_some() {
        crate::follow(&app, &state);
    }
    Ok(opened)
}

/// The agent found for the open project, the suggested brief, cap and notes, and the lines
/// that set an agent up by hand.
#[tauri::command]
pub async fn agent_find(state: State<'_, AppState>) -> Answer<AgentSetup> {
    let workspace = Arc::clone(&state.workspace);
    let places = places(&state)?;
    blocking(move || setup(&workspace, &places.agent_command, &places.exe)).await
}

/// Starts the user's agent on the open project in the background. Without `confirmed` it
/// refuses with `fosscut.agent.confirmation_required`, whose details carry the command and
/// the notes for the window to show first.
#[tauri::command]
pub async fn agent_start(state: State<'_, AppState>, choice: AgentChoice) -> Answer<JobStatus> {
    let workspace = Arc::clone(&state.workspace);
    let jobs = Arc::clone(&state.jobs);
    let places = places(&state)?;
    blocking(move || start(&workspace, &jobs, &choice, &places)).await
}

/// An agent run's state and what it did so far.
#[tauri::command]
pub fn agent_status(state: State<'_, AppState>, job: String) -> Answer<JobStatus> {
    Ok(state.jobs.status(&job, Duration::ZERO)?)
}

/// Stops the agent and everything it started; its edits so far stay and can be undone.
#[tauri::command]
pub async fn agent_cancel(state: State<'_, AppState>, job: String) -> Answer<JobStatus> {
    let jobs = Arc::clone(&state.jobs);
    blocking(move || Ok(jobs.cancel(&job, CANCEL_WAIT)?)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn mcp_mode_is_asked_for_by_its_first_argument() {
        assert!(mcp_arguments(args(&[])).is_none());
        assert!(mcp_arguments(args(&["/work/Show.fosscut"])).is_none());
        assert_eq!(mcp_arguments(args(&["mcp"])), Some(Ok(None)));
        let project = mcp_arguments(args(&["mcp", "-p", "/work/Show.fosscut"]));
        assert_eq!(project, Some(Ok(Some(PathBuf::from("/work/Show.fosscut")))));
        let long = mcp_arguments(args(&["mcp", "--project", "Show.fosscut"]));
        let Some(Ok(Some(path))) = long else {
            panic!("{long:?}")
        };
        assert!(path.is_absolute() && path.ends_with("Show.fosscut"));
        assert!(matches!(mcp_arguments(args(&["mcp", "-p"])), Some(Err(_))));
        assert!(matches!(
            mcp_arguments(args(&["mcp", "--verbose"])),
            Some(Err(_))
        ));
    }

    #[test]
    fn projects_take_a_standard_rate_near_the_videos() {
        let rate = |text: &str| text.parse::<Rate>().ok();
        assert_eq!(project_rate(rate("30000/1001"), "25"), "30000/1001");
        assert_eq!(project_rate(rate("2997/100"), "25"), "30000/1001");
        assert_eq!(project_rate(rate("60"), "30"), "60");
        assert_eq!(project_rate(rate("25"), "30"), "25");
        assert_eq!(project_rate(rate("15"), "30"), "30");
        assert_eq!(project_rate(None, "24"), "24");
    }

    #[test]
    fn projects_are_suggested_beside_their_video() {
        assert_eq!(
            suggested_project(Path::new("/Users/me/Downloads/test_raw.mp4")),
            PathBuf::from("/Users/me/Downloads/test_raw.fosscut")
        );
    }

    #[test]
    fn the_skills_installed_for_the_agent_are_fosscuts_own() {
        let skills = skills();
        assert_eq!(skills.len(), fosscut_mcp::skills::SKILLS.len());
        let basics = skills.iter().find(|s| s.name == "fosscut-basics").unwrap();
        assert_eq!(basics.files[0].0, "SKILL.md");
        assert!(basics.files[0].1.contains("fosscut-basics"));
    }
}
