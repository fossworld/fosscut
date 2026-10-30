//! The host (ADR-0022, items 1, 6 and 7): one bundle's session in memory, served to
//! clients over a local endpoint, with change notifications for the app and subscribers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Instant;

use fosscut_core::{ApiError, EntryKind, LogEntry, Request, Response, Session, codes};
use fosscut_model::{DirStore, MANIFEST_PATH, Project};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::auth::{self, Side};
use super::client::Connection;
use super::lock::{self, WriterLock};
use super::record::{self, HostName, Record};
use super::transport::{self, Closer, Listener, Stream};
use super::wire::{
    Call, FOSSCUT_ERROR, Hello, INVALID_PARAMS, INVALID_REQUEST, LineReader, METHOD_NOT_FOUND,
    Notification, PARSE_ERROR, Reply, Welcome, write_line,
};
use super::{
    ALREADY_HOSTED, DIVERGED, HANDSHAKE_TIMEOUT, MAX_CONNECTIONS, MAX_HANDSHAKE, MAX_MESSAGE,
    MAX_UNAUTHENTICATED, PROTOCOL, PROTOCOL_MISMATCH, UNAUTHORIZED, UNAVAILABLE, error,
};
use crate::project::now_ms;

/// Changes a subscriber may fall behind by before it is dropped.
const SUBSCRIBER_BACKLOG: usize = 1024;

/// Who hosts, and where records and sockets go.
#[derive(Clone, Debug)]
pub struct HostOptions {
    name: String,
    version: String,
    runtime: PathBuf,
}

impl HostOptions {
    /// A host called `name` at `version`, using the user's [`super::runtime_dir`].
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> HostOptions {
        HostOptions {
            name: name.into(),
            version: version.into(),
            runtime: super::runtime_dir(),
        }
    }

    /// Uses `dir` as the runtime directory instead.
    pub fn runtime(mut self, dir: impl Into<PathBuf>) -> HostOptions {
        self.runtime = dir.into();
        self
    }
}

/// A commit, as the app and subscribers learn of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// The project's revision after the change.
    pub revision: u64,
    /// The operation log's sequence number of the change.
    pub seq: u64,
    /// `commit`, `undo` or `redo`.
    pub kind: EntryKind,
    /// Who made it: `human`, `agent:<client>`, …
    pub actor: String,
    /// The names of the commands it ran.
    pub commands: Vec<String>,
}

impl Change {
    fn of(entry: &LogEntry) -> Change {
        Change {
            revision: entry.revision_after,
            seq: entry.seq,
            kind: entry.kind,
            actor: entry.actor.clone(),
            commands: entry.commands.iter().map(|c| c.command.clone()).collect(),
        }
    }
}

/// A client connected to a host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClientInfo {
    /// Unique within the host.
    pub id: u64,
    /// The name the client gave: `cli`, `app`, `agent:<client>`.
    pub name: String,
    /// When it connected, in milliseconds since the Unix epoch.
    pub since_ms: u64,
}

/// Serves one bundle until closed or dropped. While it runs it is the bundle's only
/// writer: it holds the writer lock, and its record tells clients where it listens.
pub struct Host {
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

struct Shared {
    bundle: PathBuf,
    key_name: String,
    secret: auth::Secret,
    runtime: PathBuf,
    endpoint: String,
    name: HostName,
    project: String,
    state: Mutex<State>,
    subscribers: Mutex<Vec<SyncSender<Change>>>,
    clients: Mutex<BTreeMap<u64, (ClientInfo, Closer)>>,
    next_client: AtomicU64,
    unauthenticated: AtomicUsize,
    connections: AtomicUsize,
    closing: AtomicBool,
}

struct State {
    session: Session<DirStore>,
    lock: Option<WriterLock>,
    closed: bool,
    diverged: bool,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Host {
    /// Starts hosting `bundle` (ADR-0022, item 7): takes the writer lock, waiting up to
    /// ten seconds for a local write in progress, loads the project, listens, and only
    /// then announces itself with a record. A bundle another host serves is
    /// `fosscut.session.already_hosted`.
    pub fn start(bundle: &Path, options: HostOptions) -> Result<Host, ApiError> {
        crate::project::require_project(bundle)?;
        let bundle = std::fs::canonicalize(bundle).map_err(|e| {
            ApiError::new(
                "fosscut.project.not_found",
                format!("{} cannot be read: {e}", bundle.display()),
            )
        })?;
        let key_name = record::bundle_key(&bundle)?;
        record::prepare(&options.runtime)?;
        let started = Instant::now();
        let lock = loop {
            if let Some(lock) = WriterLock::try_acquire(&bundle)? {
                break lock;
            }
            if let Some(existing) = record::read(&options.runtime, &key_name)?
                && Connection::open(&existing, &key_name, &bundle, "host").is_ok()
            {
                return Err(error(
                    ALREADY_HOSTED,
                    format!(
                        "{} is already open in {}",
                        bundle.display(),
                        existing.host.name
                    ),
                )
                .with_details(json!({
                    "host": existing.host,
                    "pid": existing.pid,
                    "started": existing.started,
                })));
            }
            if started.elapsed() >= lock::WAIT {
                return Err(lock::busy(&bundle));
            }
            std::thread::sleep(lock::RETRY);
        };
        let leftover = record::read(&options.runtime, &key_name).ok().flatten();
        record::remove(&options.runtime, &key_name, leftover.as_ref());
        let session = crate::project::open(&bundle)?;
        let listener = Listener::bind(&options.runtime)?;
        let secret = auth::random()?;
        let name = HostName {
            name: options.name,
            version: options.version,
        };
        let project = session.project().manifest.id.to_string();
        let announced = Record {
            protocol: PROTOCOL,
            endpoint: listener.endpoint().to_string(),
            key: auth::to_hex(&secret),
            pid: std::process::id(),
            host: name.clone(),
            bundle: bundle.clone(),
            project: project.clone(),
            started: now_ms().unwrap_or_default(),
        };
        if let Err(e) = record::write(&options.runtime, &key_name, &announced) {
            record::remove_socket(&options.runtime, listener.endpoint());
            return Err(e);
        }
        let shared = Arc::new(Shared {
            bundle,
            key_name,
            secret,
            runtime: options.runtime,
            endpoint: listener.endpoint().to_string(),
            name,
            project,
            state: Mutex::new(State {
                session,
                lock: Some(lock),
                closed: false,
                diverged: false,
            }),
            subscribers: Mutex::new(Vec::new()),
            clients: Mutex::new(BTreeMap::new()),
            next_client: AtomicU64::new(1),
            unauthenticated: AtomicUsize::new(0),
            connections: AtomicUsize::new(0),
            closing: AtomicBool::new(false),
        });
        let serving = Arc::clone(&shared);
        let accept = std::thread::Builder::new()
            .name("fosscut-host".into())
            .spawn(move || accept_loop(&serving, &listener))
            .map_err(|e| {
                shared.close();
                error(UNAVAILABLE, format!("cannot start the host thread: {e}"))
            })?;
        Ok(Host {
            shared,
            accept: Some(accept),
        })
    }

    /// Applies a write request, as a client's `execute` does. The host sets the commit time.
    pub fn execute(&self, request: Request) -> Result<Response, ApiError> {
        self.shared.execute(request)
    }

    /// Runs a query.
    pub fn query(&self, name: &str, args: Value) -> Result<Value, ApiError> {
        self.shared.query(name, args)
    }

    /// A copy of the project as it is now.
    pub fn project(&self) -> Project {
        guard(&self.shared.state).session.project().clone()
    }

    /// The project's current revision.
    pub fn revision(&self) -> u64 {
        guard(&self.shared.state).session.revision()
    }

    /// Every later commit, in order. A receiver that falls more than 1024 changes behind
    /// is dropped and sees the channel close; it never delays a commit.
    pub fn subscribe(&self) -> Receiver<Change> {
        self.shared.subscribe()
    }

    /// The clients connected now, oldest first.
    pub fn clients(&self) -> Vec<ClientInfo> {
        guard(&self.shared.clients)
            .values()
            .map(|(info, _)| info.clone())
            .collect()
    }

    /// The bundle's canonical path.
    pub fn bundle(&self) -> &Path {
        &self.shared.bundle
    }

    /// Where clients connect: a socket path or a pipe name.
    pub fn endpoint(&self) -> &str {
        &self.shared.endpoint
    }

    /// Stops hosting: refuses new requests, finishes the one in progress, removes the
    /// record and releases the writer lock.
    pub fn close(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.shared.close();
        if let Some(accept) = self.accept.take() {
            // A connection wakes the accept loop, which sees the host closing and ends. It
            // stays open until then: Windows discards a pipe client that leaves before the
            // host waits for it, and the loop would wait for another forever.
            let wake = transport::connect(&self.shared.endpoint);
            let _ = accept.join();
            drop(wake);
        }
        record::remove_socket(&self.shared.runtime, &self.shared.endpoint);
        for (_, closer) in guard(&self.shared.clients).values() {
            closer.close();
        }
        guard(&self.shared.state).lock.take();
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Shared {
    fn execute(&self, mut request: Request) -> Result<Response, ApiError> {
        request.at_ms = now_ms();
        let mut state = guard(&self.state);
        if state.closed {
            return Err(closed());
        }
        if !request.dry_run {
            self.check_disk(&mut state)?;
        }
        let response = state.session.execute(request)?;
        if response.committed
            && !response.replayed
            && let Some(entry) = state.session.log().last()
        {
            self.notify(&Change::of(entry));
        }
        Ok(response)
    }

    /// Refuses to write when files changed under the host: its lock file was replaced, or
    /// the project on disk is not at the session's revision.
    fn check_disk(&self, state: &mut State) -> Result<(), ApiError> {
        let in_place = state.lock.as_ref().is_some_and(WriterLock::still_in_place);
        let on_disk = std::fs::read(self.bundle.join(MANIFEST_PATH))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|manifest| manifest.get("revision").and_then(Value::as_u64));
        let revision = state.session.revision();
        if state.diverged || !in_place || on_disk != Some(revision) {
            state.diverged = true;
            return Err(error(
                DIVERGED,
                format!(
                    "files of {} changed outside FossCut while it was open",
                    self.bundle.display()
                ),
            )
            .with_details(json!({
                "revision": revision,
                "revision_on_disk": on_disk,
                "lock_in_place": in_place,
            })));
        }
        Ok(())
    }

    fn query(&self, name: &str, args: Value) -> Result<Value, ApiError> {
        let state = guard(&self.state);
        if state.closed {
            return Err(closed());
        }
        state.session.query(name, args)
    }

    /// The project's documents, keyed by their paths in the bundle, and their revision.
    fn snapshot(&self) -> Result<Value, ApiError> {
        let state = guard(&self.state);
        if state.closed {
            return Err(closed());
        }
        let project = state.session.project();
        let documents: serde_json::Map<String, Value> = project
            .document_keys()
            .iter()
            .filter_map(|key| project.document_value(key).map(|value| (key.path(), value)))
            .collect();
        Ok(json!({ "revision": project.manifest.revision, "documents": documents }))
    }

    fn subscribe(&self) -> Receiver<Change> {
        let (sender, receiver) = sync_channel(SUBSCRIBER_BACKLOG);
        guard(&self.subscribers).push(sender);
        receiver
    }

    /// Called with the state locked, so subscribers see commits in order.
    fn notify(&self, change: &Change) {
        guard(&self.subscribers).retain(|sender| match sender.try_send(change.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
        });
    }

    fn close(&self) {
        {
            // Taking the state waits for a request in progress.
            let mut state = guard(&self.state);
            if state.closed {
                return;
            }
            state.closed = true;
        }
        self.closing.store(true, Ordering::SeqCst);
        record::remove(&self.runtime, &self.key_name, None);
        guard(&self.subscribers).clear();
    }
}

fn closed() -> ApiError {
    error(UNAVAILABLE, "the host closed the project")
        .with_details(json!({ "outcome": "not_executed" }))
}

fn accept_loop(shared: &Arc<Shared>, listener: &Listener) {
    loop {
        let accepted = listener.accept();
        if shared.closing.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = accepted else {
            std::thread::sleep(lock::RETRY);
            continue;
        };
        if shared.connections.load(Ordering::SeqCst) >= MAX_CONNECTIONS
            || shared.unauthenticated.load(Ordering::SeqCst) >= MAX_UNAUTHENTICATED
        {
            continue;
        }
        shared.connections.fetch_add(1, Ordering::SeqCst);
        shared.unauthenticated.fetch_add(1, Ordering::SeqCst);
        let serving = Arc::clone(shared);
        let spawned = std::thread::Builder::new()
            .name("fosscut-host-client".into())
            .spawn(move || {
                serve(&serving, stream);
                serving.connections.fetch_sub(1, Ordering::SeqCst);
            });
        if spawned.is_err() {
            shared.connections.fetch_sub(1, Ordering::SeqCst);
            shared.unauthenticated.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn serve(shared: &Shared, mut stream: Stream) {
    let mut lines = LineReader::default();
    let _ = stream.set_timeout(Some(HANDSHAKE_TIMEOUT));
    let client = handshake(shared, &mut stream, &mut lines);
    shared.unauthenticated.fetch_sub(1, Ordering::SeqCst);
    let Some(name) = client else {
        return;
    };
    let _ = stream.set_timeout(None);
    let id = shared.next_client.fetch_add(1, Ordering::SeqCst);
    let info = ClientInfo {
        id,
        name,
        since_ms: now_ms().unwrap_or_default(),
    };
    guard(&shared.clients).insert(id, (info, stream.closer()));
    while !shared.closing.load(Ordering::SeqCst) {
        let call = match read_call(&mut stream, &mut lines, MAX_MESSAGE) {
            Ok(call) => call,
            Err(Some(reply)) => {
                if write_line(&mut stream, &reply).is_err() {
                    break;
                }
                continue;
            }
            Err(None) => break,
        };
        if call.method == "subscribe" {
            if write_line(&mut stream, &Reply::ok(call.id, json!({}))).is_ok() {
                stream_changes(shared, &mut stream);
            }
            break;
        }
        if write_line(&mut stream, &dispatch(shared, call)).is_err() {
            break;
        }
    }
    guard(&shared.clients).remove(&id);
}

/// The next call, or the error to answer with (`None` when the stream ended or broke).
fn read_call(
    stream: &mut Stream,
    lines: &mut LineReader,
    cap: usize,
) -> Result<Call, Option<Box<Reply>>> {
    let line = match lines.read_line(stream, cap) {
        Ok(Some(line)) => line,
        Ok(None) | Err(_) => return Err(None),
    };
    let value: Value = serde_json::from_slice(&line).map_err(|e| {
        Some(Box::new(Reply::failed(
            None,
            PARSE_ERROR,
            error(PROTOCOL_MISMATCH, format!("the message is not JSON: {e}")),
        )))
    })?;
    let call: Call = serde_json::from_value(value).map_err(|e| {
        Some(Box::new(Reply::failed(
            None,
            INVALID_REQUEST,
            error(
                PROTOCOL_MISMATCH,
                format!("the message is not a request: {e}"),
            ),
        )))
    })?;
    if call.jsonrpc != "2.0" {
        return Err(Some(Box::new(Reply::failed(
            Some(call.id),
            INVALID_REQUEST,
            error(PROTOCOL_MISMATCH, "jsonrpc must be \"2.0\""),
        ))));
    }
    Ok(call)
}

/// `hello` then `auth`; the client's name when both succeed.
fn handshake(shared: &Shared, stream: &mut Stream, lines: &mut LineReader) -> Option<String> {
    let call = match read_call(stream, lines, MAX_HANDSHAKE) {
        Ok(call) => call,
        Err(reply) => {
            if let Some(reply) = reply {
                let _ = write_line(stream, &reply);
            }
            return None;
        }
    };
    let refuse = |stream: &mut Stream, code: i64, api: ApiError| {
        let _ = write_line(stream, &Reply::failed(Some(call.id), code, api));
        None
    };
    if call.method != "hello" {
        return refuse(
            stream,
            FOSSCUT_ERROR,
            error(UNAUTHORIZED, "a connection starts with hello"),
        );
    }
    let hello: Hello = match serde_json::from_value(call.params) {
        Ok(hello) => hello,
        Err(e) => {
            return refuse(
                stream,
                INVALID_PARAMS,
                error(PROTOCOL_MISMATCH, format!("hello: {e}")),
            );
        }
    };
    if hello.protocol != PROTOCOL {
        return refuse(
            stream,
            FOSSCUT_ERROR,
            error(
                PROTOCOL_MISMATCH,
                format!(
                    "the host speaks protocol {PROTOCOL}, the client {}",
                    hello.protocol
                ),
            )
            .with_details(json!({ "host_protocol": PROTOCOL, "client_protocol": hello.protocol })),
        );
    }
    if record::bundle_key(&hello.bundle).ok().as_deref() != Some(shared.key_name.as_str()) {
        return refuse(
            stream,
            FOSSCUT_ERROR,
            error(UNAVAILABLE, "this host serves another project")
                .with_details(json!({ "outcome": "not_executed" })),
        );
    }
    let Some(client_nonce) = auth::from_hex(&hello.nonce) else {
        return refuse(
            stream,
            INVALID_PARAMS,
            error(PROTOCOL_MISMATCH, "hello: the nonce must be 64 hex digits"),
        );
    };
    let host_nonce = auth::random().ok()?;
    let welcome = Welcome {
        protocol: PROTOCOL,
        host: shared.name.clone(),
        project: shared.project.clone(),
        revision: guard(&shared.state).session.revision(),
        nonce: auth::to_hex(&host_nonce),
        proof: auth::proof(
            &shared.secret,
            Side::Host,
            &client_nonce,
            &host_nonce,
            &shared.key_name,
        )
        .to_hex()
        .to_string(),
    };
    write_line(stream, &Reply::ok(call.id, json!(welcome))).ok()?;

    let call = match read_call(stream, lines, MAX_HANDSHAKE) {
        Ok(call) => call,
        Err(_) => return None,
    };
    let expected = auth::proof(
        &shared.secret,
        Side::Client,
        &client_nonce,
        &host_nonce,
        &shared.key_name,
    );
    let proven = call.method == "auth"
        && call
            .params
            .get("proof")
            .and_then(Value::as_str)
            .is_some_and(|proof| auth::verify(&expected, proof));
    if !proven {
        let _ = write_line(
            stream,
            &Reply::failed(
                Some(call.id),
                FOSSCUT_ERROR,
                error(
                    UNAUTHORIZED,
                    "the client could not prove it knows the session key",
                ),
            ),
        );
        return None;
    }
    write_line(stream, &Reply::ok(call.id, json!({}))).ok()?;
    Some(hello.client)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryParams {
    name: String,
    #[serde(default)]
    args: Value,
}

fn dispatch(shared: &Shared, call: Call) -> Reply {
    let id = call.id;
    let invalid = |e: serde_json::Error| {
        Reply::failed(
            Some(id),
            INVALID_PARAMS,
            ApiError::new(codes::REQUEST_INVALID, format!("{}: {e}", call.method)),
        )
    };
    let result = match call.method.as_str() {
        "execute" => match serde_json::from_value::<Request>(call.params.clone()) {
            Ok(request) => shared
                .execute(request)
                .map(|response| serde_json::to_value(response).unwrap_or_default()),
            Err(e) => return invalid(e),
        },
        "query" => match serde_json::from_value::<QueryParams>(call.params.clone()) {
            Ok(params) => shared.query(&params.name, params.args),
            Err(e) => return invalid(e),
        },
        "snapshot" => shared.snapshot(),
        other => {
            return Reply::failed(
                Some(id),
                METHOD_NOT_FOUND,
                error(
                    PROTOCOL_MISMATCH,
                    format!("the host has no method {other:?}"),
                ),
            );
        }
    };
    match result {
        Ok(value) => Reply::ok(id, value),
        Err(e) => Reply::failed(Some(id), FOSSCUT_ERROR, e),
    }
}

/// After `subscribe`, the connection only carries `changed` notifications.
fn stream_changes(shared: &Shared, stream: &mut Stream) {
    let changes = shared.subscribe();
    while let Ok(change) = changes.recv() {
        let note = Notification {
            jsonrpc: "2.0".into(),
            method: "changed".into(),
            params: json!(change),
        };
        if write_line(stream, &note).is_err() {
            break;
        }
    }
}
