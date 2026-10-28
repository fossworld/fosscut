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

#[test]
fn round_one_sequence_from_the_command_line() {
    if !has_ffmpeg() {
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
