//! Running the user's own agent (ADR-0043) with a fake agent: a shell script that records
//! its command line, its working folder and the MCP configuration it was given, then prints
//! a canned `stream-json` conversation, fails, stops at its turn cap, or waits until it is
//! killed. No real agent runs: the confirmation, the command, the working folder, progress,
//! the final report, failures with the log, cancellation of the process group, and the
//! background job.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use fosscut_engine::EngineError;
use fosscut_engine::agent::{
    self, ACCOUNT_NOTE, AgentReport, AgentRequest, McpServer, SkillFiles, TOOLS_NOTE,
};
use fosscut_engine::jobs::{JobState, Jobs};
use serde_json::Value;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-agent-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The conversation the fake prints when it edits: it starts connected to FossCut, says
/// something, calls two tools (one fails), and ends with a summary.
const CONVERSATION: &str = r#"{"type":"system","subtype":"init","model":"fake-model","session_id":"sess-1","mcp_servers":[{"name":"fosscut","status":"connected"}],"tools":["mcp__fosscut__edit"]}
{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Reading the timeline first."}]}}
{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"mcp__fosscut__timeline_read","input":{}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"{\"revision\":3}"}]}}
{"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t2","name":"mcp__fosscut__edit","input":{"commands":[]}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","is_error":true,"content":[{"type":"text","text":"fosscut.revision.conflict: read again"}]}]}}
{"type":"assistant","message":{"id":"m3","content":[{"type":"text","text":"Cut two pauses and added captions."}]}}
{"type":"result","subtype":"success","is_error":false,"num_turns":3,"duration_ms":4200,"total_cost_usd":0.37,"result":"Cut two pauses and added captions.","session_id":"sess-1"}"#;

/// A fake agent that records `"$PWD"` and its arguments in `args.txt` and copies the MCP
/// configuration named after `--mcp-config` to `mcp-seen.json`, beside it, then behaves as
/// `mode` says.
fn fake_agent(dir: &Path, mode: &str) -> PathBuf {
    let script = dir.join(format!("claude-{mode}"));
    let conversation = dir.join("conversation.jsonl");
    std::fs::write(&conversation, format!("{CONVERSATION}\n")).unwrap();
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$PWD" "$@" > "{dir}/args.txt"
config=""
while [ $# -gt 0 ]; do
  if [ "$1" = "--mcp-config" ]; then config="$2"; fi
  shift
done
cp "$config" "{dir}/mcp-seen.json"
case "{mode}" in
  edit)
    cat "{conversation}"
    ;;
  fail)
    echo "Error: Invalid API key. Please run /login" >&2
    exit 1
    ;;
  limit)
    head -n 3 "{conversation}"
    echo '{{"type":"result","subtype":"error_max_turns","is_error":true,"num_turns":2,"total_cost_usd":0.11}}'
    exit 1
    ;;
  silent)
    exit 0
    ;;
  hang)
    head -n 2 "{conversation}"
    sleep 30 &
    echo $! > "{dir}/child.pid"
    wait
    ;;
esac
"#,
            dir = dir.display(),
            conversation = conversation.display(),
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn request(dir: &Path, agent: PathBuf, confirmed: bool) -> AgentRequest {
    let bundle = dir.join("Reel.fosscut");
    std::fs::create_dir_all(&bundle).unwrap();
    AgentRequest {
        bundle: bundle.clone(),
        brief: "Edit this into a reel with the talking-head-reel skill.".into(),
        max_turns: 50,
        max_budget_usd: None,
        agent: Some(agent),
        server: McpServer {
            command: PathBuf::from("/Applications/FossCut.app/Contents/MacOS/fosscut-desktop"),
            args: vec!["mcp".into(), "-p".into(), bundle.display().to_string()],
            env: BTreeMap::from([("FOSSCUT_RUNTIME_DIR".into(), "/tmp/fcrun".into())]),
        },
        skills: vec![SkillFiles {
            name: "talking-head-reel".into(),
            files: vec![
                (
                    "SKILL.md".into(),
                    "---\nname: talking-head-reel\n---\n".into(),
                ),
                ("references/visuals.md".into(), "# Visuals\n".into()),
            ],
        }],
        folder: dir.join("cache").join("agent"),
        confirmed,
    }
}

fn arguments(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("args.txt"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn nothing_runs_until_the_user_agrees_to_the_command_and_the_notes() {
    let dir = folder("confirm");
    let mut asked = request(&dir, fake_agent(&dir, "edit"), false);
    asked.max_budget_usd = Some(4.0);
    let error = agent::plan(&asked).unwrap_err();
    assert_eq!(error.code(), "fosscut.agent.confirmation_required");
    let details = error.details();
    assert_eq!(
        details["agent"],
        dir.join("claude-edit").display().to_string()
    );
    let command: Vec<&str> = details["command"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(command[1], "-p");
    assert_eq!(command[2], asked.brief);
    let after = |flag: &str| {
        let at = command.iter().position(|a| *a == flag).unwrap();
        command[at + 1]
    };
    assert!(after("--mcp-config").ends_with("mcp.json"));
    assert!(command.contains(&"--strict-mcp-config"));
    assert_eq!(after("--allowedTools"), "mcp__fosscut__*,Skill");
    assert!(after("--disallowedTools").contains("Bash"));
    assert_eq!(after("--max-turns"), "50");
    assert_eq!(after("--max-budget-usd"), "4.00");
    assert_eq!(after("--output-format"), "stream-json");
    let notes: Vec<&str> = details["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert_eq!(notes[..2], [ACCOUNT_NOTE, TOOLS_NOTE]);
    assert!(notes[2].contains("50 turns") && notes[2].contains("US$4.00"));
    assert!(error.to_string().contains("own account"), "{error}");
    assert!(!dir.join("args.txt").exists(), "the agent did not run");
    assert!(!dir.join("cache").exists(), "nothing was written");

    // The command agreed to is the command that runs.
    asked.confirmed = true;
    assert_eq!(
        agent::plan(&asked).unwrap().command,
        details["command"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn bad_requests_and_missing_agents_are_refused_before_anything_runs() {
    let dir = folder("refused");
    let agent_path = fake_agent(&dir, "edit");
    let code = |request: &AgentRequest| agent::plan(request).unwrap_err().code().to_string();
    let mut empty = request(&dir, agent_path.clone(), true);
    empty.brief = "  \n".into();
    assert_eq!(code(&empty), "fosscut.request.invalid_arguments");
    let mut turns = request(&dir, agent_path.clone(), true);
    turns.max_turns = 0;
    assert_eq!(code(&turns), "fosscut.request.invalid_arguments");
    turns.max_turns = 5000;
    assert_eq!(code(&turns), "fosscut.request.invalid_arguments");
    let mut budget = request(&dir, agent_path.clone(), true);
    budget.max_budget_usd = Some(-1.0);
    assert_eq!(code(&budget), "fosscut.request.invalid_arguments");
    let mut nowhere = request(&dir, agent_path.clone(), true);
    nowhere.bundle = dir.join("Missing.fosscut");
    assert_eq!(code(&nowhere), "fosscut.request.invalid_arguments");
    let mut missing = request(&dir, dir.join("no-claude"), true);
    assert_eq!(code(&missing), "fosscut.agent.not_found");
    missing.confirmed = false;
    assert_eq!(
        code(&missing),
        "fosscut.agent.not_found",
        "found before asked"
    );

    // A brief that starts like an option is not read as one.
    let mut dashed = request(&dir, agent_path, true);
    dashed.brief = "--help me".into();
    assert_eq!(agent::plan(&dashed).unwrap().command[2], "Brief: --help me");
}

#[test]
fn a_run_reports_the_agents_progress_and_its_summary() {
    let dir = folder("run");
    let request = request(&dir, fake_agent(&dir, "edit"), true);
    let plan = agent::plan(&request).unwrap();
    let mut seen: Vec<AgentReport> = Vec::new();
    let report = agent::run(&plan, &AtomicBool::new(false), &mut |now| {
        seen.push(now.clone());
        true
    })
    .unwrap();
    assert_eq!(report.outcome.as_deref(), Some("success"));
    assert_eq!(
        report.summary.as_deref(),
        Some("Cut two pauses and added captions.")
    );
    assert_eq!(
        (report.turns, report.tool_calls, report.tool_errors),
        (3, 2, 1)
    );
    assert_eq!(report.max_turns, 50);
    assert_eq!(report.last_tool.as_deref(), Some("edit"));
    assert_eq!(report.model.as_deref(), Some("fake-model"));
    assert_eq!(report.session.as_deref(), Some("sess-1"));
    assert_eq!(report.connected, Some(true));
    assert_eq!(report.cost_usd, Some(0.37));
    assert_eq!(report.command, plan.command);
    assert_eq!(seen.last(), Some(&report), "the last tick sees the end");

    // It ran in its working folder with the command as shown.
    let args = arguments(&dir);
    assert_eq!(
        Path::new(&args[0]).canonicalize().unwrap(),
        plan.folder.canonicalize().unwrap()
    );
    assert_eq!(args[1..], plan.command[1..]);

    // The configuration names FossCut's server only, as the request gave it.
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("mcp-seen.json")).unwrap()).unwrap();
    let servers = config["mcpServers"].as_object().unwrap();
    assert_eq!(servers.keys().collect::<Vec<_>>(), ["fosscut"]);
    assert_eq!(
        servers["fosscut"]["command"],
        "/Applications/FossCut.app/Contents/MacOS/fosscut-desktop"
    );
    assert_eq!(servers["fosscut"]["args"][0], "mcp");
    assert_eq!(
        servers["fosscut"]["args"][2],
        request.bundle.display().to_string()
    );
    assert_eq!(
        servers["fosscut"]["env"]["FOSSCUT_RUNTIME_DIR"],
        "/tmp/fcrun"
    );

    // The skills are where the agent finds project skills, and a run starts them afresh.
    let skill = plan.folder.join(".claude/skills/talking-head-reel");
    assert!(skill.join("SKILL.md").is_file());
    assert!(skill.join("references/visuals.md").is_file());
    std::fs::write(plan.folder.join(".claude/skills/stale.md"), "old").unwrap();
    agent::run(&plan, &AtomicBool::new(false), &mut |_| true).unwrap();
    assert!(!plan.folder.join(".claude/skills/stale.md").exists());
}

#[test]
fn failures_keep_what_the_agent_printed() {
    let dir = folder("fail");
    let plan = agent::plan(&request(&dir, fake_agent(&dir, "fail"), true)).unwrap();
    let error = agent::run(&plan, &AtomicBool::new(false), &mut |_| true).unwrap_err();
    assert_eq!(error.code(), "fosscut.agent.failed");
    assert!(error.to_string().contains("exited with"), "{error}");
    let log = error.details()["log"].clone();
    assert!(
        log.as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().contains("Invalid API key")),
        "{log}"
    );

    let dir = folder("silent");
    let plan = agent::plan(&request(&dir, fake_agent(&dir, "silent"), true)).unwrap();
    let error = agent::run(&plan, &AtomicBool::new(false), &mut |_| true).unwrap_err();
    assert_eq!(error.code(), "fosscut.agent.failed");
    assert!(error.to_string().contains("without an answer"), "{error}");
}

#[test]
fn a_run_that_reaches_its_cap_says_which() {
    let dir = folder("limit");
    let plan = agent::plan(&request(&dir, fake_agent(&dir, "limit"), true)).unwrap();
    let error = agent::run(&plan, &AtomicBool::new(false), &mut |_| true).unwrap_err();
    assert_eq!(error.code(), "fosscut.agent.limit_reached");
    assert_eq!(error.details()["limit"], "turns");
    assert_eq!(error.details()["turns"], 2);
    assert!(error.hint().unwrap().contains("undone"));
}

#[test]
fn cancelling_stops_the_agent_and_everything_it_started() {
    let dir = folder("cancel");
    let plan = agent::plan(&request(&dir, fake_agent(&dir, "hang"), true)).unwrap();
    let pid_file = dir.join("child.pid");
    let started = Instant::now();
    let mut last = AgentReport::default();
    // Cancel once the fake has started its own child and its first line was read.
    let error = agent::run(&plan, &AtomicBool::new(false), &mut |now| {
        last = now.clone();
        let ready = pid_file.exists() && now.connected.is_some();
        !ready && started.elapsed() < Duration::from_secs(20)
    })
    .unwrap_err();
    assert!(matches!(error, EngineError::JobCancelled { .. }));
    assert_eq!(error.code(), "fosscut.job.cancelled");
    assert!(started.elapsed() < Duration::from_secs(25));
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    let alive = || {
        Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap()
            .success()
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(), "the process group was stopped");
    assert_eq!(
        last.connected,
        Some(true),
        "progress came in before the cancel"
    );
}

#[test]
fn a_job_runs_the_agent_in_the_background_and_keeps_its_report() {
    let dir = folder("job");
    let jobs = Jobs::new();
    let plan = agent::plan(&request(&dir, fake_agent(&dir, "edit"), true)).unwrap();
    let status = jobs.start_agent(plan).unwrap();
    assert_eq!(status.kind, "agent");
    assert_eq!(status.progress.total, 50);
    assert_eq!(status.agent.as_ref().unwrap().max_turns, 50);
    let done = jobs.status(&status.job, Duration::from_secs(30)).unwrap();
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    let report = done.agent.unwrap();
    assert_eq!(report.tool_calls, 2);
    assert_eq!(
        report.summary.as_deref(),
        Some("Cut two pauses and added captions.")
    );
    assert_eq!(done.progress.done, 3);

    // A failed run keeps what it did so far.
    let dir = folder("jobfail");
    let plan = agent::plan(&request(&dir, fake_agent(&dir, "limit"), true)).unwrap();
    let status = jobs.start_agent(plan).unwrap();
    let failed = jobs.status(&status.job, Duration::from_secs(30)).unwrap();
    assert_eq!(failed.state, JobState::Failed);
    assert_eq!(
        failed.error.as_ref().unwrap().code,
        "fosscut.agent.limit_reached"
    );
    assert_eq!(failed.agent.unwrap().tool_calls, 1);
}

#[test]
fn one_agent_runs_on_a_project_at_a_time_and_cancelling_keeps_its_report() {
    let dir = folder("jobcancel");
    let jobs = Jobs::new();
    let request = request(&dir, fake_agent(&dir, "hang"), true);
    let first = jobs.start_agent(agent::plan(&request).unwrap()).unwrap();
    let busy = jobs
        .start_agent(agent::plan(&request).unwrap())
        .unwrap_err();
    assert_eq!(busy.code(), "fosscut.export.output_busy");
    // Wait for the fake to start its child, so the cancel stops a running agent.
    let deadline = Instant::now() + Duration::from_secs(20);
    while !dir.join("child.pid").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let cancelled = jobs.cancel(&first.job, Duration::from_secs(10)).unwrap();
    assert_eq!(cancelled.state, JobState::Cancelled, "{cancelled:?}");
    assert_eq!(cancelled.error.unwrap().code, "fosscut.job.cancelled");
    assert!(cancelled.agent.is_some());
    // Once it stopped, another may start.
    let again = jobs.start_agent(agent::plan(&request).unwrap()).unwrap();
    jobs.cancel(&again.job, Duration::from_secs(10)).unwrap();
}
