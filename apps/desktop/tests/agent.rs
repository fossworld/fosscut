//! An AI edit from the window (ADR-0043) with fake agents: a shell script that prints a
//! canned conversation, or waits until it is killed, and one that drives FossCut's real MCP
//! server, started from the app's own executable in `mcp` mode as the agent's configuration
//! says, so its edit lands in the project the workspace hosts, labelled with the agent's
//! name. Also a project made from a generated video.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fosscut_desktop::agent::{self, AgentChoice, Places};
use fosscut_desktop::workspace::{NewProject, Workspace};
use fosscut_engine::jobs::{JobState, Jobs};
use serde_json::{Value, json};

/// A workspace with a new 1080 by 1920 project open, a short runtime folder (Unix socket
/// paths are limited to about 100 bytes) and a folder to work in.
fn opened(name: &str) -> (Arc<Workspace>, PathBuf, PathBuf) {
    let (workspace, runtime, work) = empty(name);
    workspace
        .create(&NewProject {
            path: work.join("Reel.fosscut"),
            name: None,
            width: 1080,
            height: 1920,
            rate: "30".into(),
        })
        .unwrap();
    (workspace, runtime, work)
}

fn empty(name: &str) -> (Arc<Workspace>, PathBuf, PathBuf) {
    let runtime = PathBuf::from("/tmp").join(format!("fca-{name}-{}", std::process::id()));
    let work =
        std::env::temp_dir().join(format!("fosscut-agent-desk-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&runtime);
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    (
        Arc::new(Workspace::new(runtime.clone(), None)),
        runtime,
        work,
    )
}

fn done(workspace: Arc<Workspace>, runtime: PathBuf, work: PathBuf) {
    workspace.close();
    let _ = std::fs::remove_dir_all(&runtime);
    let _ = std::fs::remove_dir_all(&work);
}

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake agent that records its arguments and the MCP configuration it was given beside
/// it, then prints a short conversation (`edit`) or waits until it is killed (`hang`).
fn canned_agent(dir: &Path, mode: &str) -> PathBuf {
    let script = dir.join(format!("claude-{mode}"));
    executable(
        &script,
        &format!(
            r#"#!/bin/sh
printf '%s\n' "$@" > "{dir}/args.txt"
while [ $# -gt 0 ]; do
  if [ "$1" = "--mcp-config" ]; then cp "$2" "{dir}/mcp-seen.json"; fi
  shift
done
echo '{{"type":"system","subtype":"init","model":"fake","session_id":"s","mcp_servers":[{{"name":"fosscut","status":"connected"}}]}}'
if [ "{mode}" = hang ]; then
  sleep 30 &
  echo $! > "{dir}/child.pid"
  wait
fi
echo '{{"type":"assistant","message":{{"id":"m1","content":[{{"type":"tool_use","name":"mcp__fosscut__timeline_read","input":{{}}}}]}}}}'
echo '{{"type":"result","subtype":"success","is_error":false,"num_turns":1,"total_cost_usd":0.01,"result":"Read the timeline."}}'
"#,
            dir = dir.display(),
        ),
    );
    script
}

fn places(work: &Path, agent: &Path) -> Places {
    Places {
        folder: work.join("cache").join("agent"),
        exe: PathBuf::from(env!("CARGO_BIN_EXE_fosscut-desktop")),
        agent_command: agent.display().to_string(),
    }
}

fn choice(confirmed: bool) -> AgentChoice {
    AgentChoice {
        brief: "Turn this into a reel.".into(),
        max_turns: Some(40),
        max_budget_usd: None,
        confirmed,
    }
}

#[test]
fn the_window_learns_the_agent_and_the_brief_then_nothing_runs_until_the_user_agrees() {
    let (workspace, runtime, work) = opened("confirm");
    let fake = canned_agent(&work, "edit");
    let places = places(&work, &fake);
    let setup = agent::setup(&workspace, &places.agent_command, &places.exe).unwrap();
    let found = setup.agent.clone().unwrap();
    assert_eq!(
        (found.path.as_path(), found.via.as_str()),
        (fake.as_path(), "preference")
    );
    assert!(
        setup.brief.contains("\"Reel\"") && setup.brief.contains("1080x1920"),
        "{}",
        setup.brief
    );
    assert!(setup.brief.contains(&setup.skill));
    assert_eq!(setup.max_turns, 200);
    assert_eq!(setup.notes.len(), 2);
    assert!(
        setup.server.ends_with("fosscut-desktop mcp"),
        "{}",
        setup.server
    );
    assert!(
        setup.setup[0].starts_with("claude mcp add fosscut -- "),
        "{:?}",
        setup.setup
    );

    let jobs = Jobs::new();
    let asked = agent::start(&workspace, &jobs, &choice(false), &places).unwrap_err();
    assert_eq!(asked.code, "fosscut.agent.confirmation_required");
    let command: Vec<&str> = asked.details["command"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        command[..3],
        [fake.to_str().unwrap(), "-p", "Turn this into a reel."]
    );
    assert!(command.contains(&"--max-turns") && command.contains(&"40"));
    assert_eq!(asked.details["notes"].as_array().unwrap().len(), 3);
    assert!(!work.join("args.txt").exists(), "the agent did not run");

    let started = agent::start(&workspace, &jobs, &choice(true), &places).unwrap();
    assert_eq!(started.kind, "agent");
    let finished = jobs.status(&started.job, Duration::from_secs(30)).unwrap();
    assert_eq!(finished.state, JobState::Succeeded, "{finished:?}");
    let report = finished.agent.unwrap();
    assert_eq!(report.summary.as_deref(), Some("Read the timeline."));
    assert_eq!(report.max_turns, 40);

    // The agent was given this app in mcp mode for the open project, and its live session.
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(work.join("mcp-seen.json")).unwrap())
            .unwrap();
    let server = &config["mcpServers"]["fosscut"];
    assert_eq!(server["command"], env!("CARGO_BIN_EXE_fosscut-desktop"));
    let bundle = workspace.current().unwrap().unwrap().bundle;
    assert_eq!(
        server["args"],
        json!(["mcp", "-p", bundle.display().to_string()])
    );
    assert_eq!(
        server["env"]["FOSSCUT_RUNTIME_DIR"],
        runtime.display().to_string()
    );
    assert!(server["env"].get("FOSSCUT_AGENT").is_none());
    done(workspace, runtime, work);
}

#[test]
fn without_an_agent_the_window_offers_the_brief_and_the_setup_line() {
    let (workspace, runtime, work) = opened("missing");
    let places = places(&work, &work.join("no-claude"));
    let setup = agent::setup(&workspace, &places.agent_command, &places.exe).unwrap();
    assert!(setup.agent.is_none());
    assert!(setup.tried[0].contains("no-claude"), "{:?}", setup.tried);
    assert!(!setup.brief.is_empty());
    let refused = agent::start(&workspace, &Jobs::new(), &choice(true), &places).unwrap_err();
    assert_eq!(refused.code, "fosscut.agent.not_found");
    assert!(refused.hint.unwrap().contains("never installs"));
    workspace.close();
    let closed = agent::setup(&workspace, &places.agent_command, &places.exe).unwrap_err();
    assert!(closed.message.contains("no project"), "{closed:?}");
    done(workspace, runtime, work);
}

#[test]
fn cancelling_stops_the_agent_from_the_window() {
    let (workspace, runtime, work) = opened("cancel");
    let places = places(&work, &canned_agent(&work, "hang"));
    let jobs = Jobs::new();
    let started = agent::start(&workspace, &jobs, &choice(true), &places).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !work.join("child.pid").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let running = jobs.status(&started.job, Duration::ZERO).unwrap();
    assert_eq!(running.state, JobState::Running);
    let cancelled = jobs.cancel(&started.job, Duration::from_secs(10)).unwrap();
    assert_eq!(cancelled.state, JobState::Cancelled, "{cancelled:?}");
    done(workspace, runtime, work);
}

/// True when the FFmpeg programs are here; in CI (`FOSSCUT_REQUIRE_FFMPEG`) they must be.
fn ffmpeg_programs() -> bool {
    let runs = |program: &str| {
        Command::new(program)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let found = runs("ffmpeg") && runs("ffprobe");
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        found || !required,
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg or ffprobe is missing"
    );
    found
}

#[test]
fn a_video_becomes_a_project_beside_it_with_the_video_on_its_timeline() {
    if !ffmpeg_programs() {
        eprintln!("skipped: ffmpeg or ffprobe is missing");
        return;
    }
    let (workspace, runtime, work) = empty("video");
    let video = work.join("raw take.mp4");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=180x320:rate=25",
        ])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"])
        .args(["-t", "1", "-c:v", "mpeg4", "-c:a", "aac", "-shortest"])
        .arg(&video)
        .status()
        .unwrap();
    assert!(status.success());
    let suggested = agent::suggested_project(&video);
    assert_eq!(suggested, work.join("raw take.fosscut"));
    let opened =
        agent::project_from_video(&workspace, &video, work.join("raw take"), 1080, 1920, "30")
            .unwrap();
    assert_eq!(opened.bundle.file_name().unwrap(), "raw take.fosscut");
    let timeline = workspace
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    assert_eq!(timeline["format"]["width"], 1080);
    assert_eq!(timeline["format"]["rate"], "25", "the video's rate");
    let clips: Vec<&Value> = timeline["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap())
        .collect();
    assert_eq!(clips.len(), 1, "{timeline}");
    let history = workspace
        .query("history.list", json!({ "limit": 2 }))
        .unwrap();
    assert_eq!(history["entries"][0]["reason"], "Start from a video");
    assert_eq!(history["entries"][0]["actor"], "human");
    assert_eq!(history["entries"][1]["reason"], "Import media");

    // A sound file is no video, and nothing is created for it.
    let sound = work.join("voice.wav");
    let status = Command::new("ffmpeg")
        .args([
            "-y", "-v", "error", "-f", "lavfi", "-i", "sine", "-t", "0.5",
        ])
        .arg(&sound)
        .status()
        .unwrap();
    assert!(status.success());
    let refused = agent::project_from_video(
        &workspace,
        &sound,
        work.join("voice.fosscut"),
        1080,
        1920,
        "30",
    )
    .unwrap_err();
    assert_eq!(refused.code, "fosscut.request.invalid_arguments");
    assert!(!work.join("voice.fosscut").exists());
    done(workspace, runtime, work);
}

/// The environment variable that turns [`fake_agent_drives_fosscuts_mcp_server`] into the
/// fake agent: the folder holding the arguments it was started with.
const DRIVER: &str = "FOSSCUT_TEST_FAKE_AGENT_DIR";

/// A fake agent that starts this test program again as
/// [`fake_agent_drives_fosscuts_mcp_server`], which talks MCP to the server the
/// configuration names.
fn driving_agent(dir: &Path) -> PathBuf {
    let script = dir.join("claude-drive");
    let test = std::env::current_exe().unwrap();
    // macOS drops library paths from the environment of system programs such as /bin/sh,
    // and this test program may need them to start.
    let libraries: String = ["DYLD_FALLBACK_LIBRARY_PATH", "DYLD_LIBRARY_PATH"]
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| format!("{name}='{value}' "))
        })
        .collect();
    executable(
        &script,
        &format!(
            r#"#!/bin/sh
printf '%s\n' "$@" > "{dir}/args.txt"
{libraries}{DRIVER}='{dir}' exec '{test}' fake_agent_drives_fosscuts_mcp_server --exact --ignored --nocapture --test-threads 1
"#,
            dir = dir.display(),
            test = test.display(),
        ),
    );
    script
}

#[test]
fn an_agent_edits_the_open_project_live_through_the_apps_mcp_mode() {
    let (workspace, runtime, work) = opened("live");
    let places = places(&work, &driving_agent(&work));
    let jobs = Jobs::new();
    let started = agent::start(&workspace, &jobs, &choice(true), &places).unwrap();
    let finished = jobs.status(&started.job, Duration::from_secs(120)).unwrap();
    assert_eq!(finished.state, JobState::Succeeded, "{finished:#?}");
    let report = finished.agent.unwrap();
    assert_eq!(report.connected, Some(true));
    assert_eq!(
        (report.tool_calls, report.tool_errors),
        (1, 0),
        "{report:?}"
    );
    assert_eq!(report.summary.as_deref(), Some("Added a title."));

    // The edit is in the hosted project, made by the agent, and undoes like any other.
    let history = workspace
        .query("history.list", json!({ "limit": 1 }))
        .unwrap();
    let entry = &history["entries"][0];
    assert_eq!(entry["actor"], "agent:fake-agent", "{history}");
    assert_eq!(entry["reason"], "Add a title");
    let revision = entry["revision_after"].as_u64().unwrap();
    let undone = workspace
        .execute(fosscut_core::Request::single("history.undo", Value::Null))
        .unwrap();
    assert!(undone.revision > revision);
    done(workspace, runtime, work);
}

/// One JSON-RPC message to the server.
fn send(input: &mut impl Write, message: &Value) {
    writeln!(input, "{message}").unwrap();
    input.flush().unwrap();
}

/// The server's answer to request `id`, skipping notifications.
fn answer(output: &mut impl BufRead, id: u64) -> Value {
    let mut line = String::new();
    loop {
        line.clear();
        assert!(output.read_line(&mut line).unwrap() > 0, "the server ended");
        let message: Value = serde_json::from_str(line.trim()).unwrap();
        if message["id"] == id {
            return message;
        }
    }
}

/// The fake agent of [`an_agent_edits_the_open_project_live_through_the_apps_mcp_mode`]:
/// does nothing unless started by [`driving_agent`]. It reads the MCP configuration it was
/// given, starts the server as the configuration says, adds a title through the `edit`
/// tool, and prints the conversation as Claude Code's `stream-json` does.
#[test]
#[ignore = "the fake agent of an_agent_edits_the_open_project_live_through_the_apps_mcp_mode"]
fn fake_agent_drives_fosscuts_mcp_server() {
    let Some(dir) = std::env::var_os(DRIVER) else {
        return;
    };
    let args = std::fs::read_to_string(Path::new(&dir).join("args.txt")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    let at = args.iter().position(|a| *a == "--mcp-config").unwrap();
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(args[at + 1]).unwrap()).unwrap();
    let server = &config["mcpServers"]["fosscut"];
    let mut command = Command::new(server["command"].as_str().unwrap());
    for arg in server["args"].as_array().unwrap() {
        command.arg(arg.as_str().unwrap());
    }
    for (name, value) in server["env"].as_object().unwrap() {
        command.env(name, value.as_str().unwrap());
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut input,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "fake-agent", "version": "1.0"},
        }}),
    );
    let initialized = answer(&mut output, 1);
    assert!(initialized["result"].is_object(), "{initialized}");
    send(
        &mut input,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    // The test harness has printed the test's name without ending the line.
    println!();
    println!(
        "{}",
        json!({"type": "system", "subtype": "init", "model": "fake", "session_id": "drive",
               "mcp_servers": [{"name": "fosscut", "status": "connected"}]})
    );
    let edit = json!({
        "commands": [{"command": "text.add", "args": {"text": "Hello", "duration": "1s"}}],
        "reason": "Add a title",
    });
    println!(
        "{}",
        json!({"type": "assistant", "message": {"id": "m1", "content": [
            {"type": "tool_use", "id": "t1", "name": "mcp__fosscut__edit", "input": edit}]}})
    );
    send(
        &mut input,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {"name": "edit", "arguments": edit}}),
    );
    let called = answer(&mut output, 2);
    let failed = called["result"]["isError"] == true || called.get("error").is_some();
    println!(
        "{}",
        json!({"type": "user", "message": {"content": [{"type": "tool_result",
               "tool_use_id": "t1", "is_error": failed, "content": called.to_string()}]}})
    );
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let result = if failed {
        json!({"type": "result", "subtype": "error_during_execution", "is_error": true,
               "num_turns": 2, "result": called.to_string()})
    } else {
        json!({"type": "result", "subtype": "success", "is_error": false, "num_turns": 2,
               "total_cost_usd": 0.0, "result": "Added a title."})
    };
    println!("{result}");
}
