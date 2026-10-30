//! End-to-end tests of live sessions (ADR-0022): `fosscut host` keeps a project open while
//! other `fosscut` processes edit it, as the desktop app will.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use common::workspace;
use serde_json::{Value, json};

/// A short runtime directory: Unix socket paths are limited to about 100 bytes.
fn runtime(name: &str) -> PathBuf {
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let dir = base.join(format!("fcc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn command(dir: &Path, runtime: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fosscut"));
    command
        .current_dir(dir)
        .env_remove("FOSSCUT_PROJECT")
        .env("FOSSCUT_ACTOR", "agent:test")
        .env("FOSSCUT_CACHE_DIR", dir.join("cache"))
        .env("FOSSCUT_RUNTIME_DIR", runtime);
    command
}

fn run(dir: &Path, runtime: &Path, args: &[&str]) -> Output {
    command(dir, runtime)
        .args(["-p", "Demo.fosscut"])
        .args(args)
        .arg("--json")
        .output()
        .unwrap()
}

fn ok(dir: &Path, runtime: &Path, args: &[&str]) -> Value {
    let output = run(dir, runtime, args);
    assert!(
        output.status.success(),
        "{args:?} failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fails(dir: &Path, runtime: &Path, args: &[&str]) -> Value {
    let output = run(dir, runtime, args);
    assert!(!output.status.success(), "{args:?} should fail");
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"].clone()
}

/// A title request at `start`, as raw JSON for `fosscut exec`.
fn title(start: &str, expected_revision: Option<u64>) -> String {
    let mut request = json!({
        "command": "text.add",
        "args": { "text": "Hello", "start": start, "duration": "1s" },
    });
    if let Some(revision) = expected_revision {
        request["expected_revision"] = json!(revision);
    }
    request.to_string()
}

/// `fosscut host` running as a child process, with its JSON lines collected.
struct Hosted {
    child: Child,
    stdin: Option<ChildStdin>,
    events: Receiver<Value>,
}

impl Hosted {
    /// Starts the host from another folder than the clients, which name the bundle
    /// relative to their own: a relative path must reach the host as the same bundle.
    fn start(dir: &Path, runtime: &Path) -> Hosted {
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let bundle = dir.join("Demo.fosscut");
        let mut child = command(&elsewhere, runtime)
            .arg("-p")
            .arg(&bundle)
            .args(["host", "--events", "--exit-on-eof", "--json"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, events) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender
                    .send(serde_json::from_str::<Value>(&line).unwrap())
                    .is_err()
                {
                    break;
                }
            }
        });
        let stdin = child.stdin.take();
        let hosted = Hosted {
            child,
            stdin,
            events,
        };
        let ready = hosted.next();
        assert_eq!(ready["event"], "ready", "{ready}");
        hosted
    }

    fn next(&self) -> Value {
        self.events
            .recv_timeout(Duration::from_secs(60))
            .expect("the host printed nothing")
    }

    /// Closes the host's standard input, its stop signal, and waits for it to exit.
    fn stop(mut self) -> Value {
        drop(self.stdin.take());
        let stopped = self.next();
        assert!(self.child.wait().unwrap().success());
        stopped
    }
}

impl Drop for Hosted {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn project(name: &str) -> (PathBuf, PathBuf) {
    let dir = workspace(&format!("live-{name}"));
    let runtime = runtime(name);
    ok(&dir, &runtime, &["project", "new", "Demo.fosscut"]);
    (dir, runtime)
}

#[test]
fn edits_from_other_processes_run_in_the_host_and_appear_as_events() {
    let (dir, runtime) = project("events");
    let host = Hosted::start(&dir, &runtime);

    let info = ok(&dir, &runtime, &["project", "info"]);
    assert_eq!(info["session"]["live"], true);
    assert_eq!(info["session"]["host"]["name"], "fosscut host");

    let added = ok(
        &dir,
        &runtime,
        &["text", "add", "Hello", "--start", "0f", "--duration", "1s"],
    );
    assert_eq!(added["revision"], 1);
    let change = host.next();
    assert_eq!(change["event"], "changed");
    assert_eq!(
        (&change["revision"], &change["kind"], &change["actor"]),
        (&json!(1), &json!("commit"), &json!("agent:test"))
    );
    assert_eq!(change["commands"], json!(["text.add"]));

    ok(&dir, &runtime, &["undo"]);
    ok(&dir, &runtime, &["redo"]);
    let kinds: Vec<Value> = (0..2).map(|_| host.next()["kind"].clone()).collect();
    assert_eq!(kinds, [json!("undo"), json!("redo")]);

    let captions = json!({
        "command": "captions.add",
        "args": { "words": [
            { "text": "Live", "start": { "seconds": "0.1" }, "end": { "seconds": "0.5" } },
            { "text": "edits.", "start": { "seconds": "0.5" }, "end": { "seconds": "0.9" } },
        ]},
    });
    ok(&dir, &runtime, &["exec", &captions.to_string()]);
    assert_eq!(host.next()["commands"], json!(["captions.add"]));
    ok(&dir, &runtime, &["captions", "export", "out.srt"]);
    let srt = std::fs::read_to_string(dir.join("out.srt")).unwrap();
    assert!(
        srt.contains("Live edits."),
        "exports read the host's snapshot: {srt}"
    );

    let stopped = host.stop();
    assert_eq!(stopped, json!({ "event": "stopped", "revision": 4 }));
    let info = ok(&dir, &runtime, &["project", "info"]);
    assert_eq!(info["session"], json!({ "live": false }));
    assert_eq!(info["revision"], 4, "every commit reached the disk");
}

#[test]
fn racing_writers_get_one_commit_and_one_conflict() {
    let (dir, runtime) = project("race");
    let host = Hosted::start(&dir, &runtime);
    let writers: Vec<_> = ["0f", "2s"]
        .into_iter()
        .map(|start| {
            command(&dir, &runtime)
                .args([
                    "-p",
                    "Demo.fosscut",
                    "exec",
                    &title(start, Some(0)),
                    "--json",
                ])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let outputs: Vec<Output> = writers
        .into_iter()
        .map(|writer| writer.wait_with_output().unwrap())
        .collect();
    let committed = outputs.iter().filter(|o| o.status.success()).count();
    assert_eq!(committed, 1, "exactly one writer wins");
    let loser = outputs.iter().find(|o| !o.status.success()).unwrap();
    let error: Value = serde_json::from_slice(&loser.stdout).unwrap();
    assert_eq!(error["error"]["code"], "fosscut.revision.conflict");
    assert_eq!(host.next()["revision"], 1);
    assert_eq!(host.stop()["revision"], 1);
}

#[test]
fn a_killed_host_frees_the_project_and_a_second_host_is_refused() {
    let (dir, runtime) = project("killed");
    let mut host = Hosted::start(&dir, &runtime);
    let refused = fails(&dir, &runtime, &["host"]);
    assert_eq!(refused["code"], "fosscut.session.already_hosted");
    assert_eq!(refused["details"]["host"]["name"], "fosscut host");

    ok(&dir, &runtime, &["exec", &title("0f", None)]);
    assert_eq!(host.next()["revision"], 1);
    host.child.kill().unwrap();
    host.child.wait().unwrap();

    let local = ok(&dir, &runtime, &["exec", &title("2s", None)]);
    assert_eq!(
        local["revision"], 2,
        "the kernel freed the lock; the write ran locally"
    );
    let records = std::fs::read_dir(runtime.join("sessions")).unwrap().count();
    assert_eq!(records, 0, "the writer removed the dead host's record");
    let history = ok(&dir, &runtime, &["history"]);
    let seqs: Vec<&Value> = history["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["seq"])
        .collect();
    assert_eq!(seqs, [&json!(2), &json!(1)]);

    let again = Hosted::start(&dir, &runtime);
    assert_eq!(again.stop()["revision"], 2);
}
