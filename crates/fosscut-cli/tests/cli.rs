//! End-to-end tests of the `fosscut` binary. The milestone 1 exit check: the round 1 edit
//! sequence (import, title, split at 00:00:03:28, undo, redo) entirely from the CLI.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fosscut(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fosscut"))
        .current_dir(dir)
        .env_remove("FOSSCUT_PROJECT")
        .env("FOSSCUT_ACTOR", "agent:test")
        .args(args)
        .output()
        .unwrap()
}

/// Runs a command with `--json` and returns the parsed stdout; panics on failure.
fn ok(dir: &Path, args: &[&str]) -> Value {
    let mut all = args.to_vec();
    all.push("--json");
    let output = fosscut(dir, &all);
    assert!(
        output.status.success(),
        "{args:?} failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Runs a command with `--json` that must fail and returns the error object.
fn fails(dir: &Path, args: &[&str]) -> Value {
    let mut all = args.to_vec();
    all.push("--json");
    let output = fosscut(dir, &all);
    assert_eq!(output.status.code(), Some(1), "{args:?} should fail");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    value["error"].clone()
}

fn has_ffmpeg() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|tool| {
        Command::new(tool)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// CI sets `FOSSCUT_REQUIRE_FFMPEG=1` so that a missing FFmpeg fails the test instead of
/// skipping it silently.
fn ffmpeg_required() -> bool {
    std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0")
}

#[test]
fn round_one_sequence_from_the_command_line() {
    if !has_ffmpeg() {
        assert!(
            !ffmpeg_required(),
            "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg and ffprobe are not installed"
        );
        eprintln!("skipping: ffmpeg and ffprobe are not installed");
        return;
    }
    let dir = workspace("round-one");
    let status = Command::new("ffmpeg")
        .current_dir(&dir)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30:duration=8",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=8",
        ])
        .args([
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
            "-c:a",
            "aac",
            "-shortest",
            "test-pattern.mp4",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg could not create the fixture");

    ok(
        &dir,
        &["project", "new", "Audit.fosscut", "--name", "FossCut audit"],
    );
    let asset = ok(&dir, &["media", "import", "test-pattern.mp4"]);
    let asset = asset["results"][0]["asset"].as_str().unwrap().to_string();
    let media = ok(&dir, &["media", "list"]);
    assert_eq!(media["assets"][0]["video"]["rate"], json!("30"));
    assert_eq!(
        media["assets"][0]["path"]["relative"],
        json!("../test-pattern.mp4")
    );

    let clip = ok(
        &dir,
        &["clip", "add", "--asset", &asset, "--expected-revision", "1"],
    );
    let clip_id = clip["results"][0]["clip"].as_str().unwrap().to_string();
    assert_eq!(clip["results"][0]["end"]["frames"], json!(240));

    let title = ok(
        &dir,
        &[
            "text",
            "add",
            "FossCut audit",
            "--start",
            "0f",
            "--duration",
            "3s",
            "--expected-revision",
            "2",
        ],
    );
    assert_eq!(title["results"][0]["end"]["timecode"], json!("00:00:03:00"));

    let split_args = [
        "clip",
        "split",
        "--clip",
        clip_id.as_str(),
        "--at",
        "00:00:03:28",
        "--expected-revision",
        "3",
        "--request-id",
        "split-1",
    ];
    let split = ok(&dir, &split_args);
    assert_eq!(split["revision"], json!(4));
    assert_eq!(split["results"][0]["left"]["end"]["frames"], json!(118));
    assert_eq!(
        split["results"][0]["right"]["duration"]["frames"],
        json!(122)
    );

    let retry = ok(&dir, &split_args);
    assert_eq!(retry["replayed"], json!(true));
    assert_eq!(retry["revision"], json!(4));

    let stale = fails(
        &dir,
        &[
            "clip",
            "move",
            "--clip",
            &clip_id,
            "--start",
            "1s",
            "--expected-revision",
            "1",
        ],
    );
    assert_eq!(stale["code"], json!("fosscut.revision.conflict"));
    assert_eq!(stale["details"]["current_revision"], json!(4));

    let guarded = fails(&dir, &["clip", "remove", "--clip", &clip_id]);
    assert_eq!(guarded["code"], json!("fosscut.guard.destructive"));

    ok(&dir, &["undo"]);
    let outline = ok(&dir, &["timeline", "read", "--detail", "outline"]);
    assert_eq!(outline["tracks"][0]["clip_count"], json!(1));
    ok(&dir, &["redo"]);
    let timeline = ok(&dir, &["timeline", "read"]);
    assert_eq!(timeline["revision"], json!(6));
    assert_eq!(
        timeline["tracks"][0]["clips"][1]["source_in"]["timecode"],
        json!("00:00:03:28")
    );
    assert_eq!(timeline["duration"]["frames"], json!(240));

    let history = ok(&dir, &["history"]);
    assert_eq!(history["entries"][0]["kind"], json!("redo"));
    assert_eq!(history["entries"][0]["actor"], json!("agent:test"));

    let human = |args: &[&str]| {
        let output = fosscut(&dir, args);
        assert!(output.status.success(), "{args:?}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let media = human(&["media", "list"]);
    assert!(
        media.contains("video 00:00:08:00 1280x720  test-pattern.mp4"),
        "{media}"
    );
    let timeline = human(&["timeline", "read"]);
    assert!(
        timeline.contains("(122 frames)  test-pattern.mp4 from 00:00:03:28"),
        "{timeline}"
    );
    let reimport = human(&["media", "import", "test-pattern.mp4"]);
    assert!(reimport.contains("(already imported)"), "{reimport}");

    std::fs::write(dir.join("notes.txt"), "not media").unwrap();
    let unreadable = fails(&dir, &["media", "import", "notes.txt"]);
    assert_eq!(unreadable["code"], json!("fosscut.media.probe_failed"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn works_without_media_tools() {
    let dir = workspace("basic");
    let capabilities = ok(&dir, &["capabilities"]);
    assert!(
        capabilities["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "clip.split")
    );

    ok(
        &dir,
        &["project", "new", "Promo.fosscut", "--rate", "30000/1001"],
    );
    let title = ok(&dir, &["text", "add", "Hello", "--duration", "60f"]);
    assert_eq!(title["results"][0]["end"]["timecode"], json!("00:00:02;00"));

    let bare = fosscut(&dir, &["text", "add", "Again", "--start", "12"]);
    assert_eq!(
        bare.status.code(),
        Some(2),
        "bare numbers are a usage error"
    );
    assert!(String::from_utf8_lossy(&bare.stderr).contains("explicit unit"));

    let batch = r#"{"commands": [{"command": "text.add", "args": {"text": "A", "start": "100f", "duration": "10f"}},
                                 {"command": "text.add", "args": {"text": "B", "start": "105f", "duration": "10f"}}]}"#;
    let error = fails(&dir, &["exec", batch]);
    assert_eq!(error["code"], json!("fosscut.clip.overlap"));
    assert_eq!(error["details"]["command_index"], json!(1));
    let info = ok(&dir, &["project", "info"]);
    assert_eq!(
        info["revision"],
        json!(1),
        "the failed batch changed nothing"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Runs each command without `--json` and records what a person sees, so changes to the
/// human-readable output show up in review.
fn transcript(dir: &Path, commands: &[&[&str]]) -> String {
    let mut text = String::new();
    for args in commands {
        let output = fosscut(dir, args);
        text.push_str(&format!("$ fosscut {}\n", args.join(" ")));
        text.push_str(&String::from_utf8_lossy(&output.stdout));
        if !output.status.success() {
            text.push_str(&format!("[exit {}]\n", output.status.code().unwrap_or(-1)));
            text.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        text.push('\n');
    }
    text
}

#[test]
fn human_output_is_stable() {
    let dir = workspace("human");
    let text = transcript(
        &dir,
        &[
            &["project", "new", "Demo.fosscut", "--name", "Demo"],
            &["text", "add", "Hello", "--duration", "2s"],
            &[
                "text",
                "add",
                "World",
                "--start",
                "3s",
                "--duration",
                "1s",
                "--dry-run",
            ],
            &[
                "text",
                "add",
                "World",
                "--start",
                "3s",
                "--duration",
                "1s",
                "--request-id",
                "world",
            ],
            &[
                "text",
                "add",
                "World",
                "--start",
                "3s",
                "--duration",
                "1s",
                "--request-id",
                "world",
            ],
            &[
                "text",
                "add",
                "Overlap",
                "--start",
                "3s",
                "--duration",
                "1s",
            ],
            &["track", "add", "--kind", "text", "--name", "Captions"],
            &["clip", "move", "--clip", "clip_0002", "--start", "5s"],
            &["clip", "move", "--clip", "clip_0002", "--start", "5s"],
            &["clip", "move", "--clip", "clip_0099", "--start", "1s"],
            &["clip", "split", "--clip", "clip_0002", "--at", "6s"],
            &["clip", "remove", "--clip", "clip_0005"],
            &["undo"],
            &["redo"],
            &["timeline", "read"],
            &["media", "list"],
            &["history"],
            &["project", "info"],
            &["capabilities"],
        ],
    );
    let info = ok(&dir, &["project", "info"]);
    let text = text.replace(info["id"].as_str().unwrap(), "project_<random>");
    insta::assert_snapshot!(text);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn errors_explain_what_to_do() {
    let dir = workspace("errors");

    let none = fosscut(&dir, &["project", "info"]);
    assert_eq!(none.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&none.stderr);
    assert!(
        stderr.contains("error: no project given [fosscut.project.not_found]"),
        "{stderr}"
    );
    assert!(stderr.contains("hint: Pass --project"), "{stderr}");

    let missing = fails(&dir, &["-p", "Missing.fosscut", "project", "info"]);
    assert_eq!(missing["code"], json!("fosscut.project.not_found"));
    assert!(missing["hint"].as_str().unwrap().contains("project new"));

    ok(&dir, &["project", "new", "A.fosscut"]);
    let again = fails(&dir, &["project", "new", "A.fosscut"]);
    assert_eq!(again["code"], json!("fosscut.request.invalid"));
    assert!(
        again["message"]
            .as_str()
            .unwrap()
            .contains("already exists")
    );
    let bad_rate = fails(&dir, &["project", "new", "C.fosscut", "--rate", "29.97"]);
    assert!(
        bad_rate["code"]
            .as_str()
            .unwrap()
            .starts_with("fosscut.time.")
    );

    ok(&dir, &["project", "new", "B.fosscut"]);
    let several = fails(&dir, &["project", "info"]);
    assert_eq!(several["code"], json!("fosscut.request.invalid"));
    assert!(several["hint"].as_str().unwrap().contains("--project"));

    let a = ["-p", "A.fosscut"];
    let with =
        |rest: &[&str]| -> Vec<String> { a.iter().chain(rest).map(|s| s.to_string()).collect() };
    let run_json = |rest: &[&str]| {
        let args = with(rest);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        fails(&dir, &args)
    };
    assert_eq!(
        run_json(&["exec", "not json"])["code"],
        json!("fosscut.request.invalid")
    );
    let unreadable = run_json(&["exec", "@missing-request.json"]);
    assert!(
        unreadable["message"]
            .as_str()
            .unwrap()
            .contains("cannot read")
    );
    assert_eq!(
        run_json(&["query", "timeline.read", "{bad"])["code"],
        json!("fosscut.request.invalid_arguments")
    );
    assert_eq!(
        run_json(&["query", "timeline.reed"])["code"],
        json!("fosscut.query.unknown")
    );

    std::fs::write(
        dir.join("request.json"),
        r#"{"command": "text.add", "args": {"text": "From a file", "duration": "1s"}}"#,
    )
    .unwrap();
    let from_file = ok(&dir, &["-p", "A.fosscut", "exec", "@request.json"]);
    assert_eq!(from_file["revision"], json!(1));

    let mut child = Command::new(env!("CARGO_BIN_EXE_fosscut"))
        .current_dir(&dir)
        .env_remove("FOSSCUT_PROJECT")
        .args(["-p", "A.fosscut", "--json", "exec", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(
        child.stdin.as_mut().unwrap(),
        br#"{"command": "text.add", "args": {"text": "From stdin", "start": "2s", "duration": "1s"}}"#,
    )
    .unwrap();
    let from_stdin = child.wait_with_output().unwrap();
    assert!(from_stdin.status.success());
    let from_stdin: Value = serde_json::from_slice(&from_stdin.stdout).unwrap();
    assert_eq!(from_stdin["revision"], json!(2));
    let query = ok(&dir, &["-p", "A.fosscut", "query", "project.info", "{}"]);
    assert_eq!(query["revision"], json!(2));

    std::fs::write(dir.join("clip.mp4"), b"not really a video").unwrap();
    let no_probe = Command::new(env!("CARGO_BIN_EXE_fosscut"))
        .current_dir(&dir)
        .env("FOSSCUT_FFPROBE", dir.join("no-such-ffprobe"))
        .args(["-p", "A.fosscut", "--json", "media", "import", "clip.mp4"])
        .output()
        .unwrap();
    let error: Value = serde_json::from_slice(&no_probe.stdout).unwrap();
    assert_eq!(
        error["error"]["code"],
        json!("fosscut.media.probe_unavailable")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
