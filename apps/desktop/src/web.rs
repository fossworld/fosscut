//! Web compositions and Remotion renders from the window (ADR-0039).
//!
//! - **Compositions:** a page or a folder with an `index.html` is read first ([`inspect`]),
//!   so the window can show what its `fosscut:composition` meta tag says and ask for what it
//!   leaves out; then it is copied into the bundle and added as the user's edit ([`import`]).
//! - **Remotion:** a composition of the user's own Remotion project renders in the
//!   background into the bundle's `renders/` folder and is imported when done ([`start`]).
//!   Asked without confirmation, it refuses with `fosscut.web.confirmation_required`, whose
//!   details carry the licence note and the command, for the window to show before the user
//!   agrees. FossCut never installs Remotion.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fosscut_core::{ApiError, Request};
use fosscut_engine::EngineError;
use fosscut_engine::jobs::{JobStatus, Jobs, RemotionImport};
use fosscut_engine::remotion::{self, RemotionRequest};
use fosscut_engine::web::{self, CompositionOptions};
use fosscut_time::TimeInput;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::{given, invalid, open_project};
use crate::workspace::Workspace;

/// A page or folder chosen for import, with what its meta tag says (`web_pick`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WebPage {
    /// The page, or the folder holding `index.html`.
    pub path: String,
    /// The name the asset takes unless the user gives another: the folder's or the file's.
    pub name: String,
    /// What the meta tag says, each `None` when it says nothing of it.
    pub width: Option<u32>,
    /// Canvas height in pixels.
    pub height: Option<u32>,
    /// Frames per second as written, such as `30` or `30000/1001`.
    pub fps: Option<String>,
    /// Length in seconds.
    pub duration_seconds: Option<f64>,
    /// Why the meta tag could not be read, when it could not.
    pub problem: Option<String>,
    /// Addresses the page loads from the network, which keep it from importing.
    pub remote: Vec<String>,
    /// The browser that will render it, or `None` when this machine has none.
    pub browser: Option<String>,
}

/// What `web_import` imports: the page or folder and the values that override its meta tag.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebImport {
    /// The page, or a folder with an `index.html`.
    pub path: String,
    /// The asset's name; the folder's or file's when empty.
    pub name: Option<String>,
    /// Canvas width in pixels.
    pub width: Option<u32>,
    /// Canvas height in pixels.
    pub height: Option<u32>,
    /// Frames per second, such as `30` or `30000/1001`; 30 when neither here nor in the page.
    pub fps: Option<String>,
    /// Length in seconds.
    pub duration_seconds: Option<f64>,
}

/// A composition added to the project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WebImported {
    /// The asset it became.
    pub asset: String,
    /// The project's revision after the import.
    pub revision: u64,
}

/// A render of the user's own Remotion project (`remotion_start`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemotionChoice {
    /// The Remotion project: a folder whose `package.json` depends on `remotion`.
    pub project: String,
    /// The composition id, as the project registers it.
    pub composition: String,
    /// Input props for the composition, as JSON.
    pub props: Option<Value>,
    /// The entry point relative to the project; found among Remotion's usual places when
    /// empty.
    pub entry: Option<String>,
    /// True once the user agreed to run Remotion under its licence.
    pub confirmed: bool,
}

/// Seconds as the engine takes a length, exactly as written to the millisecond.
fn seconds_input(seconds: f64) -> Result<TimeInput, ApiError> {
    if !(seconds.is_finite() && seconds > 0.0) {
        return Err(invalid(format!(
            "a composition lasts more than zero seconds, not {seconds}"
        )));
    }
    Ok(TimeInput::Seconds(format!("{:.3}", seconds)))
}

/// Seconds of `options`' duration at its frame rate, if it gives one that reads.
fn duration_seconds(options: &CompositionOptions) -> Option<f64> {
    // Width and height stand in when missing: only the length and rate are wanted here.
    let probe = CompositionOptions {
        width: Some(options.width.unwrap_or(1).clamp(1, web::MAX_SIDE)),
        height: Some(options.height.unwrap_or(1).clamp(1, web::MAX_SIDE)),
        ..options.clone()
    };
    let composition = probe.resolve().ok()?;
    Some(composition.duration.get() as f64 / fosscut_time::FLICKS_PER_SECOND as f64)
}

/// The page a composition at `source` loads first.
fn entry_of(source: &Path) -> PathBuf {
    if source.is_dir() {
        source.join("index.html")
    } else {
        source.to_path_buf()
    }
}

/// What the page or folder at `source` is, for the window to show before importing it:
/// refused with `fosscut.web.not_composition` when it is neither a page nor a folder with
/// an `index.html`.
pub fn inspect(source: &Path) -> Result<WebPage, ApiError> {
    if !web::is_composition(source) {
        let message = if source.is_dir() {
            "the folder has no index.html"
        } else {
            "it is not an .html file"
        };
        return Err(ApiError::from(web::WebError::NotComposition {
            path: source.display().to_string(),
            message: message.into(),
        }));
    }
    let page = entry_of(source);
    let html = std::fs::read_to_string(&page).map_err(|e| {
        ApiError::from(web::WebError::NotComposition {
            path: source.display().to_string(),
            message: format!("cannot read {}: {e}", page.display()),
        })
    })?;
    let (options, problem) = match web::parse_meta(&html) {
        Ok(options) => (options, None),
        Err(error) => (CompositionOptions::default(), Some(error.to_string())),
    };
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "composition".into());
    Ok(WebPage {
        path: source.display().to_string(),
        name,
        width: options.width,
        height: options.height,
        fps: options.fps.clone(),
        duration_seconds: duration_seconds(&options),
        problem,
        remote: web::remote_references(&html),
        browser: web::find_browser().map(|found| found.path.display().to_string()),
    })
}

/// Copies the composition `request` names into the bundle of the project open in
/// `workspace` and adds it as the user's one undoable edit, its values overriding the meta
/// tag's. Refused as the engine refuses compositions: `fosscut.web.composition_unknown`
/// names what is still missing, `fosscut.web.remote_reference` the network addresses.
pub fn import(workspace: &Workspace, request: &WebImport) -> Result<WebImported, ApiError> {
    let (bundle, ..) = open_project(workspace)?;
    let source = PathBuf::from(request.path.trim());
    if source.as_os_str().is_empty() {
        return Err(invalid("choose a page or a folder to import"));
    }
    let options = CompositionOptions {
        width: request.width,
        height: request.height,
        fps: given(request.fps.as_deref()).map(str::to_string),
        duration: request.duration_seconds.map(seconds_input).transpose()?,
    };
    let args = web::media_add_args(&source, given(request.name.as_deref()), &bundle, &options)?;
    let mut edit = Request::single("media.add", args);
    edit.reason = Some("Import a web composition".into());
    let response = workspace.execute_on(&bundle, edit)?;
    let asset = response
        .results
        .first()
        .and_then(|result| result["asset"].as_str())
        .ok_or_else(|| {
            ApiError::new(fosscut_core::codes::INTERNAL, "media.add returned no asset")
        })?;
    Ok(WebImported {
        asset: asset.to_string(),
        revision: response.revision,
    })
}

/// Starts rendering `choice` into the bundle of the project open in `workspace` as a
/// background job of `jobs`, which imports the file into that project once it is done (the
/// job's report names the asset). Without `confirmed` nothing runs: the refusal
/// `fosscut.web.confirmation_required` carries the licence note and the command in its
/// details. Projects that are not Remotion projects, or without Remotion or `npx`
/// installed, are refused before anything runs.
pub fn start(
    workspace: &Arc<Workspace>,
    jobs: &Jobs,
    choice: &RemotionChoice,
    npx: Option<PathBuf>,
) -> Result<JobStatus, ApiError> {
    let (bundle, ..) = open_project(workspace)?;
    let project = PathBuf::from(choice.project.trim());
    if project.as_os_str().is_empty() {
        return Err(invalid("choose the Remotion project's folder"));
    }
    let request = RemotionRequest {
        project,
        composition: choice.composition.trim().to_string(),
        props: choice.props.clone().filter(|props| !props.is_null()),
        entry: given(choice.entry.as_deref()).map(str::to_string),
        bundle: bundle.clone(),
        confirmed: choice.confirmed,
        npx,
    };
    let plan = remotion::plan(&request)?;
    let importer = Arc::clone(workspace);
    let import: RemotionImport = Box::new(move |file: &Path| {
        let failed = |e: ApiError| EngineError::Failed {
            code: e.code,
            message: e.message,
        };
        let args = fosscut_engine::import::media_add_args(file, None, &bundle).map_err(failed)?;
        let mut edit = Request::single("media.add", args);
        edit.reason = Some("Render a Remotion composition".into());
        let response = importer.execute_on(&bundle, edit).map_err(failed)?;
        response
            .results
            .first()
            .and_then(|result| result["asset"].as_str())
            .map(str::to_string)
            .ok_or_else(|| EngineError::Failed {
                code: fosscut_core::codes::INTERNAL.into(),
                message: "media.add returned no asset".into(),
            })
    });
    Ok(jobs.start_remotion(plan, Some(import))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::NewProject;
    use serde_json::json;
    use std::time::Duration;

    /// A workspace with a new 640 by 360 project open, a short runtime directory (Unix
    /// socket paths are limited to about 100 bytes) and a folder to work in.
    fn opened(name: &str) -> (Arc<Workspace>, PathBuf, PathBuf) {
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
        let workspace = Arc::new(Workspace::new(runtime.clone(), None));
        workspace
            .create(&NewProject {
                path: work.join("Web.fosscut"),
                name: None,
                width: 640,
                height: 360,
                rate: "30".into(),
            })
            .unwrap();
        (workspace, runtime, work)
    }

    fn done(workspace: Arc<Workspace>, runtime: PathBuf, work: PathBuf) {
        workspace.close();
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
    }

    const TAGGED: &str = r#"<!doctype html><html><head>
<meta name="fosscut:composition" content="width=1080, height=1920, fps=25, duration=4">
</head><body><div id="card">Hello</div></body></html>"#;

    const BARE: &str = "<!doctype html><html><body><p>No tag</p></body></html>";

    #[test]
    fn pages_and_folders_read_their_meta_tag_and_name_what_they_load() {
        let (workspace, runtime, work) = opened("webread");
        let folder = work.join("Phone mockup");
        std::fs::create_dir_all(folder.join("assets")).unwrap();
        std::fs::write(folder.join("index.html"), TAGGED).unwrap();
        std::fs::write(folder.join("assets").join("app.css"), "body{}").unwrap();
        let page = inspect(&folder).unwrap();
        assert_eq!(page.name, "Phone mockup");
        assert_eq!(
            (page.width, page.height, page.fps.as_deref()),
            (Some(1080), Some(1920), Some("25"))
        );
        assert_eq!(page.duration_seconds, Some(4.0));
        assert!(page.problem.is_none() && page.remote.is_empty(), "{page:?}");

        let bare = work.join("bare.html");
        std::fs::write(&bare, BARE).unwrap();
        let unknown = inspect(&bare).unwrap();
        assert_eq!(unknown.name, "bare.html");
        assert_eq!(
            (unknown.width, unknown.height, unknown.duration_seconds),
            (None, None, None)
        );

        let remote = work.join("remote.html");
        std::fs::write(
            &remote,
            r#"<html><head><script src="https://cdn.example.com/gsap.js"></script></head></html>"#,
        )
        .unwrap();
        assert_eq!(
            inspect(&remote).unwrap().remote,
            ["https://cdn.example.com/gsap.js"]
        );

        let empty = work.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let refused = inspect(&empty).unwrap_err();
        assert_eq!(refused.code, "fosscut.web.not_composition");
        assert!(refused.message.contains("index.html"), "{refused:?}");
        let text = work.join("notes.txt");
        std::fs::write(&text, "x").unwrap();
        assert_eq!(
            inspect(&text).unwrap_err().code,
            "fosscut.web.not_composition"
        );
        done(workspace, runtime, work);
    }

    #[test]
    fn compositions_import_as_one_edit_with_the_values_the_tag_leaves_out() {
        let (workspace, runtime, work) = opened("webimport");
        let bare = work.join("card.html");
        std::fs::write(&bare, BARE).unwrap();
        let request = |extra: Value| -> WebImport {
            let mut value = json!({ "path": bare.display().to_string() });
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().cloned().unwrap_or_default());
            serde_json::from_value(value).unwrap()
        };
        let unknown = import(&workspace, &request(json!({}))).unwrap_err();
        assert_eq!(unknown.code, "fosscut.web.composition_unknown");
        let zero = import(
            &workspace,
            &request(json!({ "width": 320, "height": 180, "duration_seconds": 0.0 })),
        )
        .unwrap_err();
        assert_eq!(zero.code, fosscut_core::codes::INVALID_ARGUMENTS);

        let imported = import(
            &workspace,
            &request(json!({
                "name": "Search bar",
                "width": 320,
                "height": 180,
                "fps": "24",
                "duration_seconds": 2.5,
            })),
        )
        .unwrap();
        let listed = workspace.query("media.list", Value::Null).unwrap();
        let asset = &listed["assets"][0];
        assert_eq!(asset["id"], imported.asset.as_str());
        assert_eq!(
            (asset["kind"].as_str(), asset["name"].as_str()),
            (Some("web"), Some("Search bar"))
        );
        assert_eq!(asset["video"]["width"], 320);
        assert_eq!(asset["duration"]["seconds"], "2.5");
        let history = workspace
            .query("history.list", json!({ "limit": 1 }))
            .unwrap();
        let entry = &history["entries"][0];
        assert_eq!(
            (entry["actor"].as_str(), entry["reason"].as_str()),
            (Some("human"), Some("Import a web composition"))
        );
        assert_eq!(entry["revision_after"], imported.revision);

        // The page's own tag needs nothing more; the name comes from the folder.
        let folder = work.join("Tagged");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("index.html"), TAGGED).unwrap();
        let tagged = import(
            &workspace,
            &WebImport {
                path: folder.display().to_string(),
                ..WebImport::default()
            },
        )
        .unwrap();
        let listed = workspace.query("media.list", Value::Null).unwrap();
        let second = listed["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == tagged.asset.as_str())
            .unwrap();
        assert_eq!(second["name"], "Tagged");
        assert_eq!(second["video"]["height"], 1920);

        let nothing = import(&workspace, &WebImport::default()).unwrap_err();
        assert_eq!(nothing.code, fosscut_core::codes::INVALID_ARGUMENTS);
        workspace.close();
        let closed = import(&workspace, &request(json!({}))).unwrap_err();
        assert!(closed.message.contains("no project"), "{closed:?}");
        done(workspace, runtime, work);
    }

    /// A Remotion project with Remotion "installed" and a stand-in `npx` that prints
    /// progress and writes the output file (an MP4 made by FFmpeg, when there is one).
    #[cfg(unix)]
    fn remotion_project(work: &Path) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let project = work.join("ad");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::create_dir_all(project.join("node_modules").join("@remotion").join("cli"))
            .unwrap();
        std::fs::write(
            project.join("package.json"),
            r#"{"dependencies":{"remotion":"4.0.0","@remotion/cli":"4.0.0"}}"#,
        )
        .unwrap();
        std::fs::write(project.join("src").join("index.ts"), "// entry").unwrap();
        let npx = work.join("npx");
        // The output is the sixth argument; a missing ffmpeg leaves a file that fails to
        // import, which the test tells apart.
        std::fs::write(
            &npx,
            "#!/bin/sh\nout=\"$6\"\necho \"Rendered 1/2\"\necho \"Rendered 2/2\"\n\
             ffmpeg -y -v error -f lavfi -i testsrc2=size=64x36:rate=30 -frames:v 6 \
             -c:v mpeg4 -f mov \"$out\" 2>/dev/null || printf 'x' > \"$out\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&npx, std::fs::Permissions::from_mode(0o755)).unwrap();
        (project, npx)
    }

    #[cfg(unix)]
    #[test]
    fn remotion_asks_for_confirmation_then_renders_and_imports_in_the_background() {
        let (workspace, runtime, work) = opened("remotion");
        let (project, npx) = remotion_project(&work);
        let jobs = Jobs::new();
        let choice = |confirmed: bool| RemotionChoice {
            project: project.display().to_string(),
            composition: "Promo".into(),
            props: Some(json!({ "title": "Hello" })),
            entry: None,
            confirmed,
        };
        let asked = start(&workspace, &jobs, &choice(false), Some(npx.clone())).unwrap_err();
        assert_eq!(asked.code, "fosscut.web.confirmation_required");
        let details = asked.details.clone();
        assert!(
            details["licence"]
                .as_str()
                .is_some_and(|l| l.contains("licence")),
            "{asked:?}"
        );
        let command: Vec<&str> = details["command"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            command.contains(&"Promo") && command.contains(&"--no-install"),
            "{command:?}"
        );
        assert!(
            !work.join("Web.fosscut").join("renders").exists(),
            "nothing ran before the user agreed"
        );

        let not_remotion = RemotionChoice {
            project: work.display().to_string(),
            ..choice(true)
        };
        let refused = start(&workspace, &jobs, &not_remotion, Some(npx.clone())).unwrap_err();
        assert_eq!(refused.code, "fosscut.web.not_remotion");
        let nameless = RemotionChoice {
            project: String::new(),
            ..choice(true)
        };
        assert_eq!(
            start(&workspace, &jobs, &nameless, Some(npx.clone()))
                .unwrap_err()
                .code,
            fosscut_core::codes::INVALID_ARGUMENTS
        );

        let started = start(&workspace, &jobs, &choice(true), Some(npx)).unwrap();
        assert_eq!(started.kind, "remotion");
        let finished = jobs.status(&started.job, Duration::from_secs(60)).unwrap();
        let listed = workspace.query("media.list", Value::Null).unwrap();
        match finished.state {
            fosscut_engine::jobs::JobState::Succeeded => {
                let report = finished.remotion.expect("a report");
                let asset = report.asset.expect("the render was imported");
                assert_eq!(listed["assets"][0]["id"], asset.as_str());
                assert_eq!(listed["assets"][0]["kind"], "video");
                let history = workspace
                    .query("history.list", json!({ "limit": 1 }))
                    .unwrap();
                assert_eq!(
                    history["entries"][0]["reason"],
                    "Render a Remotion composition"
                );
            }
            state => {
                // Without FFmpeg the stand-in writes a file that is not video.
                assert_eq!(
                    state,
                    fosscut_engine::jobs::JobState::Failed,
                    "{finished:?}"
                );
                assert!(listed["assets"].as_array().unwrap().is_empty());
            }
        }
        done(workspace, runtime, work);
    }

    #[test]
    fn renders_land_only_in_the_project_they_were_started_for() {
        let (workspace, runtime, work) = opened("webonly");
        let (bundle, ..) = open_project(&workspace).unwrap();
        workspace
            .create(&NewProject {
                path: work.join("Other.fosscut"),
                name: None,
                width: 640,
                height: 360,
                rate: "30".into(),
            })
            .unwrap();
        let refused = workspace
            .execute_on(
                &bundle,
                Request::single("text.add", json!({ "text": "Late", "duration": "1s" })),
            )
            .unwrap_err();
        assert_eq!(refused.code, "fosscut.project.not_found");
        assert!(refused.message.contains("no longer"), "{refused:?}");
        done(workspace, runtime, work);
    }
}
