//! Live sessions (ADR-0022): a host serves a bundle, and `Projects` reaches it the way the
//! CLI and the MCP server do. Each test uses its own bundle and runtime directory.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use fosscut_core::{ApiError, EntryKind, Request, codes};
use fosscut_engine::live::{self, Host, HostOptions, LOCK_FILE, WriterLock};
use fosscut_engine::project::{self, Projects};
use fosscut_model::{DirStore, Project, TimelineFormat};
use fosscut_time::Rate;
use serde_json::{Value, json};

/// A short runtime directory: Unix socket paths are limited to about 100 bytes.
fn runtime(name: &str) -> PathBuf {
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let dir = base.join(format!("fcl-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A new, empty 1080p30 project.
fn bundle(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Demo.fosscut");
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::new(30, 1).unwrap(),
        sample_rate: 48_000,
    };
    project::create(&path, "Demo", format).unwrap();
    path
}

fn host(bundle: &Path, runtime: &Path) -> Host {
    Host::start(
        bundle,
        HostOptions::new("test host", "1.0").runtime(runtime),
    )
    .unwrap()
}

/// A title in a free spot: every call places it 2 s after the last.
fn title(actor: &str) -> Request {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let start = NEXT.fetch_add(1, Ordering::SeqCst) * 2;
    let mut request = Request::single(
        "text.add",
        json!({ "text": "Hello", "start": format!("{start}s"), "duration": "1.5s" }),
    );
    request.actor = actor.into();
    request
}

fn undo(actor: &str) -> Request {
    let mut request = Request::single(fosscut_core::UNDO, Value::Null);
    request.actor = actor.into();
    request
}

fn on_disk(bundle: &Path) -> Project {
    Project::load(&DirStore::new(bundle)).unwrap()
}

fn records(runtime: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(runtime.join("sessions"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn edits_through_a_host_run_in_its_session_and_reach_the_disk() {
    let (bundle, runtime) = (bundle("edits"), runtime("edits"));
    let host = host(&bundle, &runtime);
    let changes = host.subscribe();
    let agent = Projects::with_runtime("agent:test", &runtime);

    let added = agent.execute(&bundle, title("agent:test")).unwrap();
    assert!(added.committed);
    assert_eq!(added.revision, 1);
    assert_eq!(host.revision(), 1, "the host's session made the change");
    assert_eq!(
        on_disk(&bundle).manifest.revision,
        1,
        "every commit is on disk"
    );
    let change = changes.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        (
            change.revision,
            change.seq,
            change.kind,
            change.actor.as_str()
        ),
        (1, 1, EntryKind::Commit, "agent:test")
    );
    assert_eq!(change.commands, ["text.add"]);
    let names: Vec<String> = host.clients().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["agent:test"], "the connection is kept and listed");

    let history = agent
        .query(&bundle, "history.list", json!({ "limit": 5 }))
        .unwrap();
    let entry = &history["entries"][0];
    assert_eq!(entry["actor"], "agent:test");
    assert!(
        entry["at_ms"].as_u64().unwrap() > 0,
        "the host sets the commit time"
    );

    let snapshot = agent.snapshot(&bundle).unwrap();
    assert_eq!(snapshot, host.project());
    assert_eq!(snapshot.manifest.revision, 1);

    let mut dry = title("agent:test");
    dry.dry_run = true;
    let predicted = agent.execute(&bundle, dry).unwrap();
    assert!(!predicted.committed && predicted.dry_run);
    let mut keyed = title("agent:test");
    keyed.request_id = Some("r-1".into());
    let first = agent.execute(&bundle, keyed.clone()).unwrap();
    let again = agent.execute(&bundle, keyed).unwrap();
    assert!(!first.replayed && again.replayed);
    assert_eq!(again.revision, first.revision);

    let undone = agent.execute(&bundle, undo("agent:test")).unwrap();
    assert_eq!(undone.revision, 3);
    let kinds: Vec<(u64, EntryKind)> = std::iter::from_fn(|| changes.try_recv().ok())
        .map(|change| (change.revision, change.kind))
        .collect();
    assert_eq!(
        kinds,
        [(2, EntryKind::Commit), (3, EntryKind::Undo)],
        "dry runs and replays send no change"
    );
    host.close();
    assert!(records(&runtime).is_empty(), "closing removes the record");
}

#[test]
fn errors_through_a_host_are_the_errors_of_a_local_session() {
    let (bundle, runtime) = (bundle("errors"), runtime("errors"));
    let projects = Projects::with_runtime("cli", &runtime);
    let failing = || {
        let mut stale = title("cli");
        stale.expected_revision = Some(7);
        let unknown = Request::single("clip.fly", json!({}));
        let mut empty = title("cli");
        empty.commands[0].args = json!({ "text": "" });
        [stale, unknown, empty]
    };
    let local: Vec<ApiError> = failing()
        .into_iter()
        .map(|request| projects.execute(&bundle, request).unwrap_err())
        .collect();
    let local_query = projects
        .query(&bundle, "timeline.read", json!({ "timeline": "tl_9999" }))
        .unwrap_err();

    let host = host(&bundle, &runtime);
    let remote: Vec<ApiError> = failing()
        .into_iter()
        .map(|request| projects.execute(&bundle, request).unwrap_err())
        .collect();
    let remote_query = projects
        .query(&bundle, "timeline.read", json!({ "timeline": "tl_9999" }))
        .unwrap_err();
    assert_eq!(remote, local);
    assert_eq!(remote_query, local_query);
    assert_eq!(remote[0].code, codes::REVISION_CONFLICT);
    assert_eq!(host.revision(), 0, "nothing was committed");
}

#[test]
fn a_second_host_is_refused_and_a_closed_host_frees_the_project() {
    let (bundle, runtime) = (bundle("second"), runtime("second"));
    let first = host(&bundle, &runtime);
    let Err(refused) = Host::start(&bundle, HostOptions::new("other", "2").runtime(&runtime))
    else {
        panic!("a bundle has one host");
    };
    assert_eq!(refused.code, live::ALREADY_HOSTED);
    assert_eq!(refused.details["host"]["name"], "test host");
    assert!(refused.hint.is_some());

    let cli = Projects::with_runtime("cli", &runtime);
    cli.execute(&bundle, title("cli")).unwrap();
    first.close();
    let local = cli.execute(&bundle, title("cli")).unwrap();
    assert_eq!(
        local.revision, 2,
        "with the host gone, the kept connection gives way to a local write"
    );
    assert!(
        WriterLock::try_acquire(&bundle).unwrap().is_some(),
        "the lock is free"
    );

    let second = host(&bundle, &runtime);
    assert_eq!(second.revision(), 2);
    let log = second
        .query("history.list", json!({ "limit": 10 }))
        .unwrap();
    let seqs: Vec<u64> = log["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(
        seqs,
        [2, 1],
        "sequence numbers continue across hosts and local writes"
    );
}

#[test]
fn a_record_left_by_a_crashed_host_is_cleaned_up_by_the_next_writer() {
    let (bundle, runtime) = (bundle("leftover"), runtime("leftover"));
    let host = host(&bundle, &runtime);
    let record = records(&runtime).pop().unwrap();
    let contents = std::fs::read(&record).unwrap();
    host.close();
    // A crash leaves the record, but the kernel frees the lock.
    std::fs::write(&record, &contents).unwrap();
    let response = Projects::with_runtime("cli", &runtime)
        .execute(&bundle, title("cli"))
        .unwrap();
    assert_eq!(response.revision, 1);
    assert!(!record.exists(), "the writer removed the leftover record");
}

#[test]
fn a_host_waits_for_a_local_write_and_then_loads_it() {
    let (bundle, runtime) = (bundle("wait"), runtime("wait"));
    let lock = WriterLock::acquire(&bundle).unwrap();
    let starting = {
        let (bundle, runtime) = (bundle.clone(), runtime.clone());
        std::thread::spawn(move || host(&bundle, &runtime))
    };
    // The local write in progress: the host cannot load before it is done.
    let mut session = fosscut_core::Session::open(DirStore::new(&bundle)).unwrap();
    session.execute(title("cli")).unwrap();
    drop(lock);
    let host = starting.join().unwrap();
    assert_eq!(host.revision(), 1, "the host loaded after the write");
    assert!(bundle.join(LOCK_FILE).exists());
}

#[test]
fn a_host_closed_as_soon_as_it_starts_shuts_down() {
    // Closing wakes the host's accept loop with a connection; on Windows a client that
    // left before the loop waited for it was discarded, and closing hung.
    let (bundle, runtime) = (bundle("quick"), runtime("quick"));
    for _ in 0..20 {
        host(&bundle, &runtime).close();
    }
    assert!(records(&runtime).is_empty());
    assert!(WriterLock::try_acquire(&bundle).unwrap().is_some());
}

#[test]
fn a_held_lock_without_a_host_is_busy_and_names_the_runtime_directory() {
    let (bundle, runtime) = (bundle("busy"), runtime("busy"));
    let _lock = WriterLock::acquire(&bundle).unwrap();
    let projects = Projects::with_runtime("cli", &runtime).with_wait(Duration::from_millis(150));
    let error = projects.execute(&bundle, title("cli")).unwrap_err();
    assert_eq!(error.code, codes::PROJECT_BUSY);
    assert_eq!(error.details["runtime_dir"], json!(runtime));
    assert_eq!(on_disk(&bundle).manifest.revision, 0, "nothing was written");
    assert_eq!(
        projects
            .query(&bundle, "project.info", Value::Null)
            .unwrap()["revision"],
        0,
        "reads need no lock"
    );
}

#[test]
fn a_wrong_key_or_protocol_is_refused() {
    let (bundle, runtime) = (bundle("refused"), runtime("refused"));
    let host = host(&bundle, &runtime);
    let path = records(&runtime).pop().unwrap();
    let original: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let rewrite = |change: &dyn Fn(&mut Value)| {
        let mut record = original.clone();
        change(&mut record);
        std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    };
    let projects = Projects::with_runtime("cli", &runtime).with_wait(Duration::from_millis(200));

    rewrite(&|record| record["key"] = json!("ab".repeat(32)));
    let error = projects.execute(&bundle, title("cli")).unwrap_err();
    assert_eq!(error.code, codes::PROJECT_BUSY);
    assert_eq!(error.details["last_error"]["code"], live::UNAUTHORIZED);

    rewrite(&|record| record["protocol"] = json!(99));
    let error = projects.execute(&bundle, title("cli")).unwrap_err();
    assert_eq!(
        error.code,
        live::PROTOCOL_MISMATCH,
        "waiting cannot help, so it is reported at once"
    );
    assert_eq!(error.details["host_protocol"], 99);
    let error = projects
        .query(&bundle, "project.info", Value::Null)
        .unwrap_err();
    assert_eq!(error.code, live::PROTOCOL_MISMATCH);

    rewrite(&|_| {});
    assert_eq!(projects.execute(&bundle, title("cli")).unwrap().revision, 1);
    assert_eq!(host.revision(), 1);
}

#[test]
fn many_clients_at_once_are_all_served() {
    let (bundle, runtime) = (bundle("many"), runtime("many"));
    let host = host(&bundle, &runtime);
    let shared = Arc::new((bundle.clone(), runtime.clone()));
    let workers: Vec<_> = (0..200)
        .map(|i| {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let (bundle, runtime) = &*shared;
                let projects = Projects::with_runtime(format!("client-{i}"), runtime);
                if i % 2 == 0 {
                    projects
                        .query(bundle, "project.info", Value::Null)
                        .map(|_| ())
                } else {
                    projects.execute(bundle, title("cli")).map(|_| ())
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    assert_eq!(host.revision(), 100);
    assert_eq!(records(&runtime).len(), 1, "the record survives a crowd");
}

#[test]
fn files_changed_under_a_host_stop_its_writes() {
    let (bundle, runtime) = (bundle("diverged"), runtime("diverged"));
    let host = host(&bundle, &runtime);
    host.execute(title("human")).unwrap();
    let manifest = bundle.join(fosscut_model::MANIFEST_PATH);
    let mut on_disk: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    on_disk["revision"] = json!(40);
    std::fs::write(&manifest, serde_json::to_vec(&on_disk).unwrap()).unwrap();
    for _ in 0..2 {
        let error = host.execute(title("human")).unwrap_err();
        assert_eq!(error.code, live::DIVERGED);
        assert_eq!(error.details["revision_on_disk"], 40);
    }
    let mut dry = title("human");
    dry.dry_run = true;
    assert!(host.execute(dry).is_ok(), "dry runs write nothing");
    assert_eq!(
        host.query("project.info", Value::Null).unwrap()["revision"],
        1
    );
}

#[test]
fn watchers_follow_commits_until_the_host_closes() {
    let (bundle, runtime) = (bundle("watch"), runtime("watch"));
    assert_eq!(
        live::watch(&bundle, &runtime, "watcher")
            .err()
            .unwrap()
            .code,
        live::UNAVAILABLE
    );
    let host = host(&bundle, &runtime);
    let mut watcher = live::watch(&bundle, &runtime, "watcher").unwrap();
    host.execute(title("human")).unwrap();
    host.execute(undo("human")).unwrap();
    let first = watcher.next_change().unwrap().unwrap();
    let second = watcher.next_change().unwrap().unwrap();
    assert_eq!(
        (first.kind, first.actor.as_str()),
        (EntryKind::Commit, "human")
    );
    assert_eq!((second.revision, second.kind), (2, EntryKind::Undo));
    host.close();
    assert_eq!(watcher.next_change().unwrap(), None);
}

#[test]
fn project_status_reports_the_host() {
    let (bundle, runtime) = (bundle("status"), runtime("status"));
    let projects = Projects::with_runtime("agent:test", &runtime);
    assert!(!projects.status(&bundle).unwrap().live);
    let host = host(&bundle, &runtime);
    let status = projects.status(&bundle).unwrap();
    assert!(status.live);
    assert_eq!(status.host.unwrap().name, "test host");
    assert_eq!(
        serde_json::to_value(projects.status(&bundle).unwrap()).unwrap()["live"],
        true
    );
    host.close();
    assert_eq!(
        serde_json::to_value(projects.status(&bundle).unwrap()).unwrap(),
        json!({ "live": false })
    );
}

#[test]
fn missing_projects_are_not_found_everywhere() {
    let runtime = runtime("missing");
    let missing =
        std::env::temp_dir().join(format!("fosscut-live-none-{}.fosscut", std::process::id()));
    let projects = Projects::with_runtime("cli", &runtime);
    for error in [
        projects.execute(&missing, title("cli")).unwrap_err(),
        projects
            .query(&missing, "project.info", Value::Null)
            .unwrap_err(),
        projects.snapshot(&missing).unwrap_err(),
        projects.status(&missing).unwrap_err(),
        Host::start(&missing, HostOptions::new("h", "1").runtime(&runtime))
            .err()
            .unwrap(),
    ] {
        assert_eq!(error.code, "fosscut.project.not_found");
    }
}

#[cfg(unix)]
mod raw {
    //! The host as a stranger sees it: bytes on its socket.

    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};

    use super::*;

    fn endpoint(host: &Host) -> UnixStream {
        let stream = UnixStream::connect(host.endpoint()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
    }

    fn exchange(stream: &mut UnixStream, message: &Value) -> Value {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        stream.write_all(&line).unwrap();
        let mut reply = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut reply)
            .unwrap();
        serde_json::from_str(&reply).unwrap()
    }

    fn hello(bundle: &Path, protocol: u32) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": "hello", "params": {
            "protocol": protocol, "client": "stranger", "bundle": bundle, "nonce": "00".repeat(32),
        }})
    }

    #[test]
    fn strangers_get_typed_refusals_and_nothing_else() {
        let (bundle, runtime) = (bundle("raw"), runtime("raw"));
        let host = host(&bundle, &runtime);

        let reply = exchange(&mut endpoint(&host), &hello(&bundle, 2));
        assert_eq!(reply["error"]["data"]["code"], live::PROTOCOL_MISMATCH);
        assert_eq!(
            reply["error"]["data"]["details"]["host_protocol"],
            live::PROTOCOL
        );

        let query = json!({ "jsonrpc": "2.0", "id": 1, "method": "query", "params": { "name": "project.info" } });
        let reply = exchange(&mut endpoint(&host), &query);
        assert_eq!(
            reply["error"]["data"]["code"],
            live::UNAUTHORIZED,
            "hello comes first"
        );

        let mut stream = endpoint(&host);
        let welcome = exchange(&mut stream, &hello(&bundle, live::PROTOCOL));
        assert_eq!(welcome["result"]["host"]["name"], "test host");
        let reply = exchange(
            &mut stream,
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "auth", "params": { "proof": "00".repeat(32) } }),
        );
        assert_eq!(reply["error"]["data"]["code"], live::UNAUTHORIZED);
        let mut rest = Vec::new();
        assert_eq!(
            stream.read_to_end(&mut rest).unwrap_or(0),
            0,
            "the host hangs up"
        );

        let other = super::bundle("raw-other");
        let reply = exchange(&mut endpoint(&host), &hello(&other, live::PROTOCOL));
        assert_eq!(
            reply["error"]["data"]["code"],
            live::UNAVAILABLE,
            "a host serves one bundle"
        );

        let reply = exchange(&mut endpoint(&host), &json!({ "id": 1 }));
        assert_eq!(reply["error"]["code"], -32600);
        let mut stream = endpoint(&host);
        stream.write_all(b"not json\n").unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap()["error"]["code"],
            -32700
        );

        let mut stream = endpoint(&host);
        let _ = stream.write_all(&vec![b'x'; 8192]);
        let mut rest = Vec::new();
        assert_eq!(
            stream.read_to_end(&mut rest).unwrap_or(0),
            0,
            "an oversized first message ends the connection"
        );
        assert_eq!(host.revision(), 0);
        assert!(host.clients().is_empty());
    }

    #[test]
    fn a_fake_host_never_receives_the_clients_proof_or_request() {
        let (bundle, runtime) = (bundle("fake"), runtime("fake"));
        let real = host(&bundle, &runtime);
        let path = records(&runtime).pop().unwrap();
        let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let socket = runtime.join("fake.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        record["endpoint"] = json!(socket);
        std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        // The impostor answers every hello with a made-up proof and records what else
        // arrives, until the test sends "stop".
        let impostor = std::thread::spawn(move || {
            let mut received = Vec::new();
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                if first.starts_with("stop") {
                    break;
                }
                let welcome = json!({ "jsonrpc": "2.0", "id": 1, "result": {
                    "protocol": live::PROTOCOL, "host": { "name": "fake", "version": "0" },
                    "project": "proj_x", "revision": 0, "nonce": "11".repeat(32), "proof": "22".repeat(32),
                }});
                let mut line = serde_json::to_vec(&welcome).unwrap();
                line.push(b'\n');
                stream.write_all(&line).unwrap();
                let mut rest = String::new();
                let _ = reader.read_to_string(&mut rest);
                received.push((first, rest));
            }
            received
        });
        let projects =
            Projects::with_runtime("cli", &runtime).with_wait(Duration::from_millis(100));
        let error = projects.execute(&bundle, title("cli")).unwrap_err();
        assert_eq!(error.details["last_error"]["code"], live::UNAUTHORIZED);
        UnixStream::connect(&socket)
            .unwrap()
            .write_all(b"stop\n")
            .unwrap();
        let received = impostor.join().unwrap();
        assert!(!received.is_empty());
        for (first, rest) in &received {
            assert!(first.contains("\"hello\""));
            assert!(
                rest.is_empty(),
                "nothing after hello reached the impostor: {rest}"
            );
        }
        assert_eq!(real.revision(), 0);
    }
}
