//! The project the app has open (ADR-0022): the app hosts it live, so the CLI and agents
//! connecting to it edit the same session the window shows, and the user's own edits run
//! there too, logged as `human`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Mutex, MutexGuard};

use fosscut_core::{ApiError, Request, Response};
use fosscut_engine::live::{Change, ClientInfo, Host, HostOptions};
use fosscut_engine::project;
use fosscut_model::{Id, Project, TimelineFormat};
use fosscut_time::Rate;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How many recent projects the app remembers.
const RECENT: usize = 10;

/// The name the app gives itself to clients and in records.
pub const HOST_NAME: &str = "FossCut";

/// What a new project looks like.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct NewProject {
    /// The bundle to create, a folder ending in `.fosscut`.
    pub path: PathBuf,
    /// Shown in the app; the folder's name when omitted.
    #[serde(default)]
    pub name: Option<String>,
    /// Canvas width in pixels.
    #[serde(default = "default_width")]
    pub width: u32,
    /// Canvas height in pixels.
    #[serde(default = "default_height")]
    pub height: u32,
    /// Frame rate, for example `30` or `30000/1001`.
    #[serde(default = "default_rate")]
    pub rate: String,
}

fn default_width() -> u32 {
    1920
}

fn default_height() -> u32 {
    1080
}

fn default_rate() -> String {
    "30".into()
}

/// The open project as the window needs it: where it is and `project.info`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Opened {
    /// The bundle's path.
    pub bundle: PathBuf,
    /// The core's `project.info`.
    pub info: Value,
}

/// The app's projects: at most one open, hosted live.
pub struct Workspace {
    runtime: PathBuf,
    recent_file: Option<PathBuf>,
    open: Mutex<Option<Host>>,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Workspace {
    /// A workspace whose hosts announce themselves in `runtime` and that remembers recent
    /// projects in `recent_file`.
    pub fn new(runtime: PathBuf, recent_file: Option<PathBuf>) -> Workspace {
        Workspace {
            runtime,
            recent_file,
            open: Mutex::new(None),
        }
    }

    /// Opens `bundle`, closing the project open before. Fails when another program hosts
    /// it (`fosscut.session.already_hosted`).
    pub fn open(&self, bundle: &Path) -> Result<Opened, ApiError> {
        let mut open = guard(&self.open);
        if let Some(previous) = open.take() {
            previous.close();
        }
        let options = HostOptions::new(HOST_NAME, env!("CARGO_PKG_VERSION")).runtime(&self.runtime);
        let host = Host::start(bundle, options)?;
        let opened = Opened {
            bundle: host.bundle().to_path_buf(),
            info: host.query("project.info", Value::Null)?,
        };
        *open = Some(host);
        drop(open);
        self.remember(&opened.bundle);
        Ok(opened)
    }

    /// Creates a project and opens it.
    pub fn create(&self, new: &NewProject) -> Result<Opened, ApiError> {
        let rate: Rate = new.rate.parse()?;
        let name = new.name.clone().unwrap_or_else(|| {
            new.path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".into())
        });
        let format = TimelineFormat {
            width: new.width,
            height: new.height,
            rate,
            sample_rate: 48_000,
        };
        project::create(&new.path, &name, format)?;
        self.open(&new.path)
    }

    /// Closes the open project, if any.
    pub fn close(&self) {
        if let Some(host) = guard(&self.open).take() {
            host.close();
        }
    }

    /// The open project, if any.
    pub fn current(&self) -> Result<Option<Opened>, ApiError> {
        let open = guard(&self.open);
        let Some(host) = open.as_ref() else {
            return Ok(None);
        };
        Ok(Some(Opened {
            bundle: host.bundle().to_path_buf(),
            info: host.query("project.info", Value::Null)?,
        }))
    }

    /// Runs a query on the open project.
    pub fn query(&self, name: &str, args: Value) -> Result<Value, ApiError> {
        self.with_host(|host| host.query(name, args))
    }

    /// Applies the user's edit as one transaction, logged as `human` unless the request
    /// names someone else.
    pub fn execute(&self, mut request: Request) -> Result<Response, ApiError> {
        if request.actor == "unknown" {
            request.actor = "human".into();
        }
        self.with_host(|host| host.execute(request))
    }

    /// The clients connected to the open project: agents and other front ends.
    pub fn clients(&self) -> Vec<ClientInfo> {
        guard(&self.open)
            .as_ref()
            .map(Host::clients)
            .unwrap_or_default()
    }

    /// Every later commit to the open project, from anyone.
    pub fn subscribe(&self) -> Option<Receiver<Change>> {
        guard(&self.open).as_ref().map(Host::subscribe)
    }

    /// The open project's bundle, its state now and its primary timeline, for the preview.
    pub fn snapshot(&self) -> Option<(PathBuf, Project, Id)> {
        guard(&self.open).as_ref().map(|host| {
            let project = host.project();
            let timeline = project.manifest.primary_timeline.clone();
            (host.bundle().to_path_buf(), project, timeline)
        })
    }

    /// Recently opened projects that still exist, newest first.
    pub fn recent(&self) -> Vec<PathBuf> {
        self.read_recent()
            .into_iter()
            .filter(|path| path.join(fosscut_model::MANIFEST_PATH).exists())
            .collect()
    }

    fn with_host<T>(&self, work: impl FnOnce(&Host) -> Result<T, ApiError>) -> Result<T, ApiError> {
        let open = guard(&self.open);
        let host = open.as_ref().ok_or_else(|| {
            ApiError::new("fosscut.project.not_found", "no project is open")
                .with_hint("Open or create a project first.")
        })?;
        work(host)
    }

    fn read_recent(&self) -> Vec<PathBuf> {
        self.recent_file
            .as_ref()
            .and_then(|file| std::fs::read(file).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn remember(&self, bundle: &Path) {
        let Some(file) = &self.recent_file else {
            return;
        };
        let mut recent = self.read_recent();
        recent.retain(|path| path != bundle);
        recent.insert(0, bundle.to_path_buf());
        recent.truncate(RECENT);
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(file, serde_json::to_vec_pretty(&recent).unwrap_or_default());
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_engine::project::Projects;
    use serde_json::json;

    /// A short runtime directory: Unix socket paths are limited to about 100 bytes.
    fn dirs(name: &str) -> (PathBuf, PathBuf) {
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
        (runtime, work)
    }

    fn new_project(work: &Path, name: &str) -> NewProject {
        NewProject {
            path: work.join(format!("{name}.fosscut")),
            name: None,
            width: 1280,
            height: 720,
            rate: "25".into(),
        }
    }

    fn title(start: &str) -> Request {
        Request::single(
            "text.add",
            json!({ "text": "Hello", "start": start, "duration": "1s" }),
        )
    }

    #[test]
    fn the_open_project_is_hosted_so_agents_edit_the_same_session() {
        let (runtime, work) = dirs("hosted");
        let workspace = Workspace::new(runtime.clone(), None);
        let opened = workspace.create(&new_project(&work, "Demo")).unwrap();
        assert_eq!(opened.info["name"], "Demo");
        assert_eq!(opened.info["timelines"][0]["format"]["width"], 1280);
        let changes = workspace.subscribe().unwrap();

        let mine = workspace.execute(title("0f")).unwrap();
        assert_eq!(mine.revision, 1);
        let agent = Projects::with_runtime("agent:test", &runtime);
        let mut theirs = title("2s");
        theirs.actor = "agent:test".into();
        agent.execute(&opened.bundle, theirs).unwrap();

        let actors: Vec<String> = std::iter::from_fn(|| changes.try_recv().ok())
            .map(|change| change.actor)
            .collect();
        assert_eq!(actors, ["human", "agent:test"]);
        let names: Vec<String> = workspace.clients().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["agent:test"]);
        assert_eq!(
            workspace.query("project.info", Value::Null).unwrap()["revision"],
            2
        );
        let (bundle, project, timeline) = workspace.snapshot().unwrap();
        assert_eq!(bundle, opened.bundle);
        assert_eq!(project.manifest.revision, 2);
        assert_eq!(timeline, project.manifest.primary_timeline);

        let undone = workspace
            .execute(Request::single(fosscut_core::UNDO, Value::Null))
            .unwrap();
        assert_eq!(undone.revision, 3);
        assert_eq!(workspace.current().unwrap().unwrap().info["revision"], 3);

        workspace.close();
        assert!(workspace.current().unwrap().is_none());
        let error = workspace.query("project.info", Value::Null).unwrap_err();
        assert_eq!(error.code, "fosscut.project.not_found");
        assert!(workspace.clients().is_empty());
        assert!(workspace.subscribe().is_none() && workspace.snapshot().is_none());
        let after = agent.execute(&opened.bundle, title("4s")).unwrap();
        assert_eq!(
            after.revision, 4,
            "with the app closed, agents write on disk"
        );
    }

    #[test]
    fn opening_another_project_closes_the_first_and_both_are_remembered() {
        let (runtime, work) = dirs("recent");
        let recent_file = work.join("state").join("recent.json");
        let workspace = Workspace::new(runtime.clone(), Some(recent_file.clone()));
        let first = workspace.create(&new_project(&work, "First")).unwrap();
        let second = workspace.create(&new_project(&work, "Second")).unwrap();
        assert_eq!(
            workspace.recent(),
            [second.bundle.clone(), first.bundle.clone()]
        );
        assert!(
            !Projects::with_runtime("cli", &runtime)
                .status(&first.bundle)
                .unwrap()
                .live,
            "the first project is no longer hosted"
        );
        workspace.open(&first.bundle).unwrap();
        assert_eq!(
            workspace.recent(),
            [first.bundle.clone(), second.bundle.clone()]
        );

        std::fs::remove_dir_all(&second.bundle).unwrap();
        assert_eq!(
            workspace.recent(),
            std::slice::from_ref(&first.bundle),
            "missing projects are skipped"
        );
        std::fs::write(&recent_file, b"not json").unwrap();
        assert!(workspace.recent().is_empty());
        assert!(Workspace::new(runtime, None).recent().is_empty());
    }

    #[test]
    fn projects_hosted_elsewhere_or_invalid_are_refused() {
        let (runtime, work) = dirs("refused");
        let workspace = Workspace::new(runtime.clone(), None);
        let opened = workspace.create(&new_project(&work, "Busy")).unwrap();
        let other = Workspace::new(runtime, None);
        let error = other.open(&opened.bundle).unwrap_err();
        assert_eq!(error.code, "fosscut.session.already_hosted");

        let mut bad_rate = new_project(&work, "Bad");
        bad_rate.rate = "fast".into();
        assert!(workspace.create(&bad_rate).is_err());
        let error = workspace.create(&new_project(&work, "Busy")).unwrap_err();
        assert_eq!(error.code, "fosscut.request.invalid", "the bundle exists");
        let missing = workspace.open(&work.join("Missing.fosscut")).unwrap_err();
        assert_eq!(missing.code, "fosscut.project.not_found");
    }

    #[test]
    fn new_projects_default_to_1080p30() {
        let new: NewProject = serde_json::from_value(json!({ "path": "/x/A.fosscut" })).unwrap();
        assert_eq!(
            (new.width, new.height, new.rate.as_str()),
            (1920, 1080, "30")
        );
        assert_eq!(new.name, None);
    }
}
