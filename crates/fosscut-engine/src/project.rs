//! Hosting projects on disk (ADR-0002, item 3) and reaching live hosts (ADR-0022): finding,
//! creating and opening bundles, and one writer at a time through the bundle's writer lock.
//! The CLI and the MCP server edit projects through [`Projects`] (or the functions of the
//! same names), which forward to the host when the desktop app or `fosscut host` has the
//! project open, and write on disk otherwise.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fosscut_core::{ApiError, Request, Response, Session, codes};
use fosscut_model::{DirStore, Id, IdKind, MemoryStore, Project, ProjectStore, TimelineFormat};
use serde::Serialize;
use serde_json::{Value, json};

use crate::live::{
    self, CallError, Connection, HostName, OpenError, PROTOCOL_MISMATCH, UNAVAILABLE, WriterLock,
};

/// The bundle path from `--project`, or a single `*.fosscut` directory in the working directory.
pub fn bundle_path(explicit: Option<&Path>) -> Result<PathBuf, ApiError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let candidates: Vec<PathBuf> = std::fs::read_dir(".")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.extension().is_some_and(|ext| ext == "fosscut"))
                .collect()
        })
        .unwrap_or_default();
    match candidates.as_slice() {
        [single] => Ok(single.clone()),
        [] => Err(ApiError::new("fosscut.project.not_found", "no project given")
            .with_hint("Pass --project <path.fosscut>, set FOSSCUT_PROJECT, or run inside a directory with one .fosscut bundle.")),
        _ => Err(ApiError::new(codes::REQUEST_INVALID, "several .fosscut bundles in this directory")
            .with_hint("Choose one with --project.")),
    }
}

/// Fails unless `bundle` holds a FossCut project.
pub(crate) fn require_project(bundle: &Path) -> Result<(), ApiError> {
    if bundle.join(fosscut_model::MANIFEST_PATH).exists() {
        return Ok(());
    }
    Err(ApiError::new(
        "fosscut.project.not_found",
        format!("{} is not a FossCut project", bundle.display()),
    )
    .with_hint("Create one with: fosscut project new <path.fosscut>"))
}

/// Opens a bundle's session from disk. Only for a writer holding the writer lock, or a
/// reader when no host runs.
pub(crate) fn open(bundle: &Path) -> Result<Session<DirStore>, ApiError> {
    require_project(bundle)?;
    Session::open(DirStore::new(bundle))
}

/// Opens a bundle's session to read it, writing nothing: a host or a local writer may be
/// writing it at the same time.
fn open_to_read(bundle: &Path) -> Result<Session<DirStore>, ApiError> {
    require_project(bundle)?;
    Session::open_read_only(DirStore::new(bundle))
}

/// Creates a new bundle directory with an empty primary timeline.
pub fn create(
    bundle: &Path,
    name: &str,
    format: TimelineFormat,
) -> Result<Session<DirStore>, ApiError> {
    if bundle.exists()
        && std::fs::read_dir(bundle)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        return Err(ApiError::new(
            codes::REQUEST_INVALID,
            format!("{} already exists and is not empty", bundle.display()),
        ));
    }
    std::fs::create_dir_all(bundle).map_err(|e| {
        ApiError::new(
            "fosscut.project.storage",
            format!("cannot create {}: {e}", bundle.display()),
        )
    })?;
    let project = Project::new(new_project_id(name), name, format);
    Session::create(DirStore::new(bundle), project)
}

fn new_project_id(name: &str) -> Id {
    let mut hasher = blake3::Hasher::new();
    hasher.update(name.as_bytes());
    hasher.update(&now_ms().unwrap_or_default().to_le_bytes());
    hasher.update(&std::process::id().to_le_bytes());
    let hash = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes[..5].copy_from_slice(&hash.as_bytes()[..5]);
    Id::new(IdKind::Project, u64::from_le_bytes(bytes))
}

/// Runs `request` against the bundle as one transaction, through its live host when one
/// runs. The client is named `cli` to the host.
pub fn execute(bundle: &Path, request: Request) -> Result<Response, ApiError> {
    Projects::new("cli").execute(bundle, request)
}

/// Runs a read-only query, through the live host when one runs.
pub fn query(bundle: &Path, name: &str, args: Value) -> Result<Value, ApiError> {
    Projects::new("cli").query(bundle, name, args)
}

/// The project as it is now: from its live host, or from disk.
pub fn snapshot(bundle: &Path) -> Result<Project, ApiError> {
    Projects::new("cli").snapshot(bundle)
}

/// Whether a host serves a bundle, as `project_info` reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionStatus {
    /// True when a host (the desktop app or `fosscut host`) has the project open, so edits
    /// appear there live and the user may be editing too.
    pub live: bool,
    /// The host program, when live.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<HostName>,
}

/// How a front end reaches projects (ADR-0022, item 8). Each call decides afresh: when the
/// bundle's writer lock is free no host runs, and the call works on disk under the lock;
/// when a host holds it, the call goes to the host its record names. Connections to hosts
/// are kept for later calls, so a long-lived front end shows as one connected client.
pub struct Projects {
    runtime: PathBuf,
    wait: Duration,
    client: Mutex<String>,
    links: Mutex<HashMap<String, Connection>>,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a call through a host came to.
enum Routed {
    /// The host answered, or the request may have run: report it.
    Done(Result<Value, ApiError>),
    /// The request did not run; route it again.
    Retry(ApiError),
}

impl Projects {
    /// A front end called `client` (`cli`, `agent:<name>`, …), with records in the user's
    /// [`live::runtime_dir`].
    pub fn new(client: impl Into<String>) -> Projects {
        Projects::with_runtime(client, live::runtime_dir())
    }

    /// The same, with records in `runtime`.
    pub fn with_runtime(client: impl Into<String>, runtime: impl Into<PathBuf>) -> Projects {
        Projects {
            runtime: runtime.into(),
            wait: live::WAIT,
            client: Mutex::new(client.into()),
            links: Mutex::new(HashMap::new()),
        }
    }

    /// Waits at most `wait` for other writers (ten seconds by default).
    pub fn with_wait(mut self, wait: Duration) -> Projects {
        self.wait = wait;
        self
    }

    /// Renames the front end for connections opened from now on.
    pub fn set_client(&self, client: impl Into<String>) {
        *guard(&self.client) = client.into();
    }

    /// Where records are looked up.
    pub fn runtime(&self) -> &Path {
        &self.runtime
    }

    /// Runs `request` as one transaction: under the writer lock on disk, or on the host.
    /// Waits up to ten seconds for another writer, then reports `fosscut.project.busy`.
    /// A host lost after the request was sent is `fosscut.session.unavailable` with
    /// `details.outcome` `unknown`; it is never retried here.
    pub fn execute(&self, bundle: &Path, request: Request) -> Result<Response, ApiError> {
        require_project(bundle)?;
        let key = live::bundle_key(bundle)?;
        let params = serde_json::to_value(&request)
            .map_err(|e| ApiError::new(codes::INTERNAL, format!("request: {e}")))?;
        let client = if request.actor == "unknown" {
            guard(&self.client).clone()
        } else {
            request.actor.clone()
        };
        let request_id = request.request_id.clone();
        let response = |value: Value| {
            serde_json::from_value::<Response>(value)
                .map_err(|e| live::error(PROTOCOL_MISMATCH, format!("response: {e}")))
        };
        let started = Instant::now();
        let mut last = None;
        loop {
            // The writer lock decides: free means no host runs, whatever connections
            // were kept.
            if let Some(_lock) = WriterLock::try_acquire(bundle)? {
                guard(&self.links).remove(&key);
                if let Ok(Some(leftover)) = live::read_record(&self.runtime, &key) {
                    live::remove_record(&self.runtime, &key, Some(&leftover));
                }
                return open(bundle)?.execute(request);
            }
            match self
                .linked(&key)
                .map_or_else(|| self.connect(bundle, &key, &client), |c| Ok(Some(c)))
            {
                Ok(Some(connection)) => {
                    match self.send(&key, connection, "execute", &params, request_id.as_deref()) {
                        Routed::Done(done) => return done.and_then(response),
                        Routed::Retry(error) => last = Some(error),
                    }
                }
                Ok(None) => {}
                Err(error) if error.code == PROTOCOL_MISMATCH => return Err(error),
                Err(error) => last = Some(error),
            }
            if started.elapsed() >= self.wait {
                return Err(self.busy(bundle, last));
            }
            std::thread::sleep(live::RETRY);
        }
    }

    /// Runs a read-only query: on the host when one answers, otherwise on disk. Reads take
    /// no lock; the disk holds every commit.
    pub fn query(&self, bundle: &Path, name: &str, args: Value) -> Result<Value, ApiError> {
        require_project(bundle)?;
        let params = json!({ "name": name, "args": args });
        if let Some(done) = self.read_remote(bundle, "query", &params) {
            return done;
        }
        open_to_read(bundle)?.query(name, args)
    }

    /// The project as it is now: the host's documents, loaded and validated like a bundle,
    /// or the bundle on disk.
    pub fn snapshot(&self, bundle: &Path) -> Result<Project, ApiError> {
        require_project(bundle)?;
        if let Some(done) = self.read_remote(bundle, "snapshot", &Value::Null) {
            return from_snapshot(&done?);
        }
        Ok(open_to_read(bundle)?.project().clone())
    }

    /// Whether a host serves the bundle now: the writer lock is held and its host answers.
    pub fn status(&self, bundle: &Path) -> Result<SessionStatus, ApiError> {
        require_project(bundle)?;
        let key = live::bundle_key(bundle)?;
        let linked = self.linked(&key);
        if WriterLock::try_acquire(bundle)?.is_some() {
            return Ok(SessionStatus {
                live: false,
                host: None,
            });
        }
        let client = guard(&self.client).clone();
        let connection = match linked {
            Some(connection) => Some(connection),
            None => self.connect(bundle, &key, &client).ok().flatten(),
        };
        let host = connection.and_then(|mut connection| {
            let ping = json!({ "name": "project.info", "args": null });
            connection.call("query", ping).ok()?;
            let host = connection.welcome().host.clone();
            guard(&self.links).insert(key, connection);
            Some(host)
        });
        Ok(SessionStatus {
            live: host.is_some(),
            host,
        })
    }

    /// Opens a connection to the host named by the bundle's record, if there is one.
    fn connect(
        &self,
        bundle: &Path,
        key: &str,
        client: &str,
    ) -> Result<Option<Connection>, ApiError> {
        let Some(record) = live::read_record(&self.runtime, key)? else {
            return Ok(None);
        };
        match Connection::open(&record, key, bundle, client) {
            Ok(connection) => Ok(Some(connection)),
            Err(OpenError::Refused(error)) => Err(error),
            Err(OpenError::Unreachable(message)) => Err(live::error(UNAVAILABLE, message)),
        }
    }

    /// Calls the host through `connection`, keeping the connection when it still works.
    fn send(
        &self,
        key: &str,
        mut connection: Connection,
        method: &str,
        params: &Value,
        request_id: Option<&str>,
    ) -> Routed {
        match connection.call(method, params.clone()) {
            Ok(value) => {
                guard(&self.links).insert(key.to_string(), connection);
                Routed::Done(Ok(value))
            }
            Err(CallError::Api(error)) if not_executed(&error) => Routed::Retry(error),
            Err(CallError::Api(error)) => {
                guard(&self.links).insert(key.to_string(), connection);
                Routed::Done(Err(error))
            }
            Err(CallError::NotSent(e)) => Routed::Retry(live::error(
                UNAVAILABLE,
                format!("the host connection broke: {e}"),
            )),
            Err(CallError::Lost(e)) => Routed::Done(Err(lost(&e, request_id))),
        }
    }

    /// The connection kept for `key`, unless the host has hung up on it.
    fn linked(&self, key: &str) -> Option<Connection> {
        guard(&self.links)
            .remove(key)
            .filter(|connection| !connection.hung_up())
    }

    /// A read through the host. `None` sends the read to disk: no host, or one that cannot
    /// be reached (reads are safe to repeat).
    fn read_remote(
        &self,
        bundle: &Path,
        method: &str,
        params: &Value,
    ) -> Option<Result<Value, ApiError>> {
        let key = match live::bundle_key(bundle) {
            Ok(key) => key,
            Err(error) => return Some(Err(error)),
        };
        if let Some(connection) = self.linked(&key) {
            match self.send(&key, connection, method, params, None) {
                Routed::Done(Ok(value)) => return Some(Ok(value)),
                Routed::Done(Err(error)) if error.code != UNAVAILABLE => return Some(Err(error)),
                _ => {}
            }
        }
        let client = guard(&self.client).clone();
        match self.connect(bundle, &key, &client) {
            Ok(Some(connection)) => match self.send(&key, connection, method, params, None) {
                Routed::Done(Err(error)) if error.code == UNAVAILABLE => None,
                Routed::Done(done) => Some(done),
                Routed::Retry(_) => None,
            },
            Err(error) if error.code == PROTOCOL_MISMATCH => Some(Err(error)),
            _ => None,
        }
    }

    fn busy(&self, bundle: &Path, last: Option<ApiError>) -> ApiError {
        let mut details = json!({ "runtime_dir": self.runtime });
        if let Some(last) = last {
            details["last_error"] = json!(last);
        }
        live::busy(bundle).with_details(details)
    }
}

/// A host's refusal that guarantees the request did not run.
fn not_executed(error: &ApiError) -> bool {
    error.code == UNAVAILABLE && error.details["outcome"] == "not_executed"
}

fn lost(error: &std::io::Error, request_id: Option<&str>) -> ApiError {
    live::error(
        UNAVAILABLE,
        format!("the host was lost before it answered: {error}"),
    )
    .with_details(json!({ "outcome": "unknown", "request_id": request_id }))
}

/// Loads a host's snapshot as a bundle would load: format checks and validation included.
fn from_snapshot(snapshot: &Value) -> Result<Project, ApiError> {
    let mut store = MemoryStore::new();
    let documents = snapshot
        .get("documents")
        .and_then(Value::as_object)
        .ok_or_else(|| live::error(PROTOCOL_MISMATCH, "snapshot: no documents"))?;
    for (path, document) in documents {
        let bytes = serde_json::to_vec(document)
            .map_err(|e| ApiError::new(codes::INTERNAL, format!("{path}: {e}")))?;
        store
            .write_atomic(path, &bytes)
            .map_err(|e| ApiError::new(codes::INTERNAL, format!("{path}: {e}")))?;
    }
    Ok(Project::load(&store)?)
}

/// The timeline with id `id`, or the primary timeline when `id` is `None`.
pub fn timeline(project: &Project, id: Option<&str>) -> Result<Id, ApiError> {
    let timeline = match id {
        Some(text) => Id::parse(text)
            .map_err(|e| ApiError::new(codes::INVALID_ARGUMENTS, format!("timeline: {e}")))?,
        None => project.manifest.primary_timeline.clone(),
    };
    if !project.timelines.contains_key(&timeline) {
        return Err(ApiError::new(
            codes::TIMELINE_NOT_FOUND,
            format!("timeline {timeline} does not exist"),
        ));
    }
    Ok(timeline)
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}
