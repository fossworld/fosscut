//! End-to-end tests of the `fosscut` binary. The milestone 1 exit check: the round 1 edit
//! sequence (import, title, split at 00:00:03:28, undo, redo) entirely from the CLI.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};

use common::{can_render, ffmpeg_required, has_ffmpeg, test_pattern, workspace};
use serde_json::{Value, json};

fn command(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fosscut"));
    command
        .current_dir(dir)
        .env_remove("FOSSCUT_PROJECT")
        .env("FOSSCUT_ACTOR", "agent:test")
        .env("FOSSCUT_CACHE_DIR", dir.join("cache"))
        // Models live in the cache folder above unless a test names another.
        .env_remove("FOSSCUT_MODELS_DIR");
    command
}

fn fosscut(dir: &Path, args: &[&str]) -> Output {
    command(dir).args(args).output().unwrap()
}

/// Like [`ok`], but the command's progress goes to the test's own stderr, so a slow or
/// stuck export shows how far it got when the test fails or times out.
fn ok_with_progress(dir: &Path, args: &[&str]) -> Value {
    let output = command(dir)
        .args(args)
        .arg("--json")
        .stderr(Stdio::inherit())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
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

/// The size of a PNG file, from its header.
fn png_size(path: &Path) -> (u32, u32) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(
        &bytes[..8],
        b"\x89PNG\r\n\x1a\n",
        "{} is a PNG",
        path.display()
    );
    let number = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (number(16), number(20))
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
    test_pattern(&dir);

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

    // Trimming moves an edge; a title has no media to run out of.
    let clip = title["results"][0]["clip"].as_str().unwrap().to_string();
    let trimmed = ok(
        &dir,
        &[
            "clip",
            "trim",
            "--clip",
            &clip,
            "--start",
            "30f",
            "--end",
            "00:00:05;00",
        ],
    );
    assert_eq!(trimmed["results"][0]["start"]["frames"], json!(30));
    assert_eq!(
        trimmed["results"][0]["end"]["timecode"],
        json!("00:00:05;00")
    );
    let nothing = fails(&dir, &["clip", "trim", "--clip", &clip]);
    assert_eq!(nothing["code"], json!("fosscut.request.invalid_arguments"));
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

/// Mean luma of a 16x9 grid of blocks over a frame, so frames of different sizes compare.
fn blocks(frame: &fosscut_media::VideoFrame) -> Vec<f64> {
    let plane = &frame.planes[0];
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut means = Vec::with_capacity(144);
    for by in 0..9 {
        for bx in 0..16 {
            let (x0, x1) = (bx * w / 16, (bx + 1) * w / 16);
            let (y0, y1) = (by * h / 9, (by + 1) * h / 9);
            let mut sum = 0u64;
            for y in y0..y1 {
                sum += plane.row(y)[x0..x1]
                    .iter()
                    .map(|&v| u64::from(v))
                    .sum::<u64>();
            }
            means.push(sum as f64 / ((x1 - x0) * (y1 - y0)) as f64);
        }
    }
    means
}

/// The milestone 3 exit check: the round 1 baseline (import an 8 s clip, add a 3 s title,
/// split at 00:00:03:28) exported from the command line and verified by probing, with the
/// title in the picture for its 3 seconds and the clip's audio in the file.
#[test]
fn round_one_baseline_exports_and_verifies() {
    use fosscut_media::VideoDecoder;
    if !can_render() {
        return;
    }
    let dir = workspace("baseline");
    test_pattern(&dir);
    ok(
        &dir,
        &["project", "new", "Audit.fosscut", "--name", "FossCut audit"],
    );
    let asset = ok(&dir, &["media", "import", "test-pattern.mp4"])["results"][0]["asset"]
        .as_str()
        .unwrap()
        .to_string();
    let clip = ok(&dir, &["clip", "add", "--asset", &asset])["results"][0]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &dir,
        &[
            "text",
            "add",
            "FossCut audit",
            "--duration",
            "3s",
            "--size",
            "0.1",
        ],
    );
    ok(
        &dir,
        &["clip", "split", "--clip", &clip, "--at", "00:00:03:28"],
    );

    let presets = ok(&dir, &["export", "--presets"]);
    let available = |name: &str| {
        presets["presets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == name && p["available"] == json!(true))
    };
    // The round 1 regression: H.264 requested must be H.264 or a typed refusal, never HEVC.
    if !available("mp4-h264") {
        let refused = fails(&dir, &["export", "H264.mp4", "--preset", "mp4-h264"]);
        assert_eq!(refused["code"], json!("fosscut.export.codec_unavailable"));
        assert!(
            !refused["details"]["available"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(!dir.join("H264.mp4").exists());
    }
    let preset = if available("mp4-h264") {
        "mp4-h264"
    } else {
        "mp4-av1"
    };
    // A dry run checks everything and writes nothing; a stale revision is refused.
    let planned = ok(
        &dir,
        &["export", "Baseline.mp4", "--preset", preset, "--dry-run"],
    );
    assert_eq!(planned["dry_run"], json!(true));
    assert_eq!(planned["frames"], json!(240));
    assert_eq!(planned["duration"], json!(8 * 705_600_000_i64));
    assert!(!dir.join("Baseline.mp4").exists());
    let stale = fails(
        &dir,
        &["export", "Baseline.mp4", "--expected-revision", "1"],
    );
    assert_eq!(stale["code"], json!("fosscut.revision.conflict"));
    assert!(!dir.join("Baseline.mp4").exists());
    let revision = ok(&dir, &["project", "info"])["revision"].to_string();
    assert_eq!(stale["details"]["current_revision"].to_string(), revision);

    let report = ok_with_progress(
        &dir,
        &[
            "export",
            "Baseline.mp4",
            "--preset",
            preset,
            "--expected-revision",
            &revision,
            "--request-id",
            "baseline",
        ],
    );
    let video = &report["verified"]["video"];
    assert_eq!(video["codec"], json!(preset.trim_start_matches("mp4-")));
    assert_eq!(
        (video["width"].clone(), video["height"].clone()),
        (json!(1920), json!(1080))
    );
    assert_eq!(video["rate"], json!("30"));
    assert_eq!(video["frame_count"], json!(240));
    assert_eq!(video["color"]["matrix"], json!("bt709"));
    assert_eq!(report["frames"], json!(240));
    assert_eq!(report["duration"], json!(8 * 705_600_000i64));
    let audio = &report["verified"]["audio"];
    assert_eq!(
        (
            audio["codec"].clone(),
            audio["sample_rate"].clone(),
            audio["channels"].clone()
        ),
        (json!("aac"), json!(48000), json!(2))
    );
    assert!(
        !report["encoders"]["video"]
            .as_str()
            .unwrap()
            .contains("x264")
    );

    // The title is in the picture for its 3 seconds: compare blocks of the export with the
    // source at the same moments.
    let software = fosscut_media::DecoderOptions {
        hardware: fosscut_media::HardwareDecoding::Off,
    };
    let mut output =
        fosscut_media::ffmpeg::FfmpegVideoDecoder::open(&dir.join("Baseline.mp4"), software)
            .unwrap();
    let mut source =
        fosscut_media::ffmpeg::FfmpegVideoDecoder::open(&dir.join("test-pattern.mp4"), software)
            .unwrap();
    let difference = |output: &mut fosscut_media::ffmpeg::FfmpegVideoDecoder,
                      source: &mut fosscut_media::ffmpeg::FfmpegVideoDecoder,
                      seconds: i64| {
        let time = fosscut_time::Flicks::from_seconds(seconds).unwrap();
        let a = blocks(&output.frame_at(time).unwrap());
        let b = blocks(&source.frame_at(time).unwrap());
        a.iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f64::max)
    };
    let with_title = difference(&mut output, &mut source, 1);
    let without_title = difference(&mut output, &mut source, 4);
    assert!(
        with_title > 30.0,
        "title visible at 1 s: largest block difference {with_title}"
    );
    assert!(
        without_title < 12.0,
        "no title at 4 s: largest block difference {without_title}"
    );

    // The clip's 440 Hz tone (amplitude 1/8) is in the export.
    let mut sound =
        fosscut_media::ffmpeg::FfmpegAudioDecoder::open(&dir.join("Baseline.mp4"), 48_000, 2)
            .unwrap();
    let samples = sound
        .read(
            fosscut_time::Flicks::from_seconds(2).unwrap(),
            fosscut_time::Flicks::from_seconds(1).unwrap(),
        )
        .unwrap()
        .samples;
    let rms =
        (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len() as f64).sqrt();
    assert!(
        (rms - 0.125 / 2f64.sqrt()).abs() < 0.02,
        "audio level {rms}"
    );
    // Close the export before replacing it: Windows cannot replace a file that is open.
    drop((output, source, sound));

    // Repeating the request replays it; a new export over the file needs permission.
    let replay = ok(
        &dir,
        &[
            "export",
            "Baseline.mp4",
            "--preset",
            preset,
            "--request-id",
            "baseline",
        ],
    );
    assert_eq!(replay["replayed"], json!(true));
    let records = std::fs::read_dir(dir.join("cache").join("jobs")).unwrap();
    assert_eq!(records.count(), 1, "one project's records");
    assert!(!dir.join("Audit.fosscut").join("cache").exists());
    let guarded = fails(&dir, &["export", "Baseline.mp4", "--preset", preset]);
    assert_eq!(guarded["code"], json!("fosscut.guard.destructive"));
    // Overwriting with permission writes a new, verified file. Audio only, so this check
    // does not render the 1080p timeline a second time (slow on software GPUs in CI).
    let human = fosscut(
        &dir,
        &[
            "export",
            "Baseline.mp4",
            "--preset",
            "m4a-aac",
            "--allow-destructive",
        ],
    );
    assert!(
        human.status.success(),
        "overwrite failed: {}",
        String::from_utf8_lossy(&human.stderr)
    );
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("exported Baseline.mp4") && text.contains("aac 48000 Hz x 2"),
        "{text}"
    );
    assert!(
        text.contains("verified against the request; rendered in"),
        "{text}"
    );
    let rewritten = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(dir.join("Baseline.mp4"))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&rewritten.stdout).trim(), "audio");
    let dry = fosscut(
        &dir,
        &["export", "Copy.mp4", "--preset", preset, "--dry-run"],
    );
    let text = String::from_utf8_lossy(&dry.stdout);
    assert!(
        text.contains("would export Copy.mp4") && text.contains("240 frames"),
        "{text}"
    );
    assert!(text.contains("nothing was written"), "{text}");
    assert!(!dir.join("Copy.mp4").exists());
    let listing =
        String::from_utf8_lossy(&fosscut(&dir, &["export", "--presets"]).stdout).into_owned();
    assert!(listing.contains("mov-prores"), "{listing}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn export_needs_an_output_and_a_known_preset() {
    let dir = workspace("export-errors");
    ok(&dir, &["project", "new", "Empty.fosscut"]);
    let missing = fails(&dir, &["export"]);
    assert_eq!(missing["code"], json!("fosscut.request.invalid_arguments"));
    assert!(missing["hint"].as_str().unwrap().contains("--presets"));
    let unknown = fails(&dir, &["export", "x.mp4", "--preset", "gif"]);
    assert!(unknown["hint"].as_str().unwrap().contains("mp4-h264"));
    let timeline = fails(&dir, &["export", "x.mp4", "--timeline", "timeline_0099"]);
    assert_eq!(timeline["code"], json!("fosscut.timeline.not_found"));
    let bad_id = fails(&dir, &["export", "x.mp4", "--timeline", "nonsense"]);
    assert_eq!(bad_id["code"], json!("fosscut.request.invalid_arguments"));
    if fosscut_render::Renderer::headless().is_ok() {
        let empty = fails(&dir, &["export", "x.m4a"]);
        assert_eq!(empty["code"], json!("fosscut.export.empty_timeline"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn frames_and_contact_sheets_show_the_edit() {
    if !can_render() {
        return;
    }
    let dir = workspace("frames");
    let status = Command::new("ffmpeg")
        .current_dir(&dir)
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=320x180:rate=30:duration=2")
        .args(["-c:v", "mpeg4", "-q:v", "5", "small.mp4"])
        .status()
        .unwrap();
    assert!(status.success());
    ok(
        &dir,
        &[
            "project",
            "new",
            "Small.fosscut",
            "--width",
            "320",
            "--height",
            "180",
        ],
    );
    let asset = ok(&dir, &["media", "import", "small.mp4"])["results"][0]["asset"]
        .as_str()
        .unwrap()
        .to_string();
    ok(&dir, &["clip", "add", "--asset", &asset]);
    ok(&dir, &["text", "add", "Hello", "--duration", "1s"]);

    // One frame, at a smaller size, reported in every time unit.
    let still = ok(
        &dir,
        &["frame", "0.5s", "--out", "still.png", "--width", "160"],
    );
    assert_eq!(still["at"]["frames"], json!(15));
    assert_eq!(still["at"]["timecode"], json!("00:00:00:15"));
    assert_eq!(
        (still["width"].clone(), still["height"].clone()),
        (json!(160), json!(90))
    );
    assert_eq!(png_size(&dir.join("still.png")), (160, 90));

    // Existing files need permission; a dry run writes nothing.
    let guarded = fails(&dir, &["frame", "1s", "--out", "still.png"]);
    assert_eq!(guarded["code"], json!("fosscut.guard.destructive"));
    ok(
        &dir,
        &["frame", "1s", "--out", "still.png", "--allow-destructive"],
    );
    assert_eq!(png_size(&dir.join("still.png")), (320, 180));
    let dry = ok(&dir, &["frame", "1s", "--out", "dry.png", "--dry-run"]);
    assert_eq!(dry["dry_run"], json!(true));
    assert!(!dir.join("dry.png").exists());

    // Times outside the timeline and unknown timelines are typed errors.
    let late = fails(&dir, &["frame", "2s", "--out", "late.png"]);
    assert_eq!(late["code"], json!("fosscut.time.out_of_range"));
    assert!(late["hint"].is_string());
    let unknown = fails(
        &dir,
        &[
            "frame",
            "1s",
            "--out",
            "x.png",
            "--timeline",
            "timeline_0099",
        ],
    );
    assert_eq!(unknown["code"], json!("fosscut.timeline.not_found"));

    // A contact sheet: four 80x45 tiles over the 2 s timeline, two per row.
    let sheet = ok(
        &dir,
        &[
            "contact-sheet",
            "--out",
            "sheet.png",
            "--count",
            "4",
            "--width",
            "80",
        ],
    );
    assert_eq!(sheet["columns"], json!(2));
    let shown: Vec<Value> = sheet["tiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["at"]["frames"].clone())
        .collect();
    assert_eq!(shown, [json!(7), json!(22), json!(37), json!(52)]);
    assert_eq!(png_size(&dir.join("sheet.png")), (164, 94));
    let too_many = fails(
        &dir,
        &["contact-sheet", "--out", "big.png", "--count", "500"],
    );
    assert_eq!(too_many["code"], json!("fosscut.request.invalid_arguments"));

    let human = |args: &[&str]| {
        let output = fosscut(&dir, args);
        assert!(output.status.success(), "{args:?}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let text = human(&["frame", "15f", "--out", "half.png", "--width", "64"]);
    assert_eq!(
        text.trim(),
        "wrote half.png (64x36): frame 15 at 00:00:00:15"
    );
    let text = human(&[
        "contact-sheet",
        "--out",
        "two.png",
        "--count",
        "2",
        "--dry-run",
    ]);
    assert_eq!(
        text.trim(),
        "dry run: would write two.png (644x180): 2 frames in 2 columns, 00:00:00:15 to 00:00:01:15"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn skills_install_into_agent_folders() {
    let dir = workspace("skills");
    let shipped = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/fosscut-basics/SKILL.md"),
    )
    .unwrap();
    let listed = ok(&dir, &["skills", "list"]);
    assert_eq!(listed["skills"][0]["name"], json!("fosscut-basics"));
    let names: Vec<&str> = listed["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "fosscut-basics",
            "talking-head-cleanup",
            "vertical-reframe",
            "viral-captions"
        ]
    );
    assert!(listed["skills"][0]["description"].as_str().unwrap().len() > 40);
    let nowhere = fails(&dir, &["skills", "install"]);
    assert_eq!(nowhere["code"], json!("fosscut.request.invalid_arguments"));
    let unknown = fails(
        &dir,
        &[
            "skills", "install", "--target", "claude", "--skill", "magic",
        ],
    );
    assert!(unknown["hint"].as_str().unwrap().contains("fosscut-basics"));

    // Claude Code, in this folder: installed, then up to date, and guarded once changed.
    let installed = ok(&dir, &["skills", "install", "--target", "claude"]);
    assert_eq!(installed["skills"][0]["status"], json!("installed"));
    let file = dir.join(".claude/skills/fosscut-basics/SKILL.md");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), shipped);
    let again = ok(&dir, &["skills", "install", "--target", "claude"]);
    assert_eq!(again["skills"][0]["status"], json!("unchanged"));
    std::fs::write(&file, "my own notes").unwrap();
    let guarded = fails(&dir, &["skills", "install", "--target", "claude"]);
    assert_eq!(guarded["code"], json!("fosscut.guard.destructive"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "my own notes");
    let updated = ok(
        &dir,
        &[
            "skills",
            "install",
            "--target",
            "claude",
            "--allow-destructive",
        ],
    );
    assert_eq!(updated["skills"][0]["status"], json!("updated"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), shipped);

    // Codex, for the user: into the home folder; a dry run writes nothing.
    let home = dir.join("home");
    let codex = |args: &[&str]| {
        let mut all = vec!["skills", "install", "--target", "codex", "--user", "--json"];
        all.extend_from_slice(args);
        let output = command(&dir)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .args(&all)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let planned = codex(&["--dry-run"]);
    assert_eq!(planned["dry_run"], json!(true));
    assert!(!home.join(".agents").exists());
    codex(&[]);
    assert_eq!(
        std::fs::read_to_string(home.join(".agents/skills/fosscut-basics/SKILL.md")).unwrap(),
        shipped
    );

    // Any folder, and the human output.
    let human = fosscut(&dir, &["skills", "install", "--dir", "custom"]);
    assert!(human.status.success());
    let expected = fosscut_mcp::skills::SKILLS
        .iter()
        .map(|skill| {
            format!(
                "installed {} in {}",
                skill.name,
                Path::new("custom").join(skill.name).display()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(String::from_utf8_lossy(&human.stdout).trim(), expected);
    assert!(dir.join("custom/fosscut-basics/SKILL.md").is_file());
    assert!(dir.join("custom/viral-captions/SKILL.md").is_file());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn capabilities_describe_the_commands_you_name() {
    let dir = workspace("capabilities-named");
    let named = ok(&dir, &["capabilities", "clip.split", "timeline.read"]);
    assert_eq!(named["commands"][0]["name"], json!("clip.split"));
    assert_eq!(named["queries"][0]["name"], json!("timeline.read"));
    assert!(named.get("operations").is_none());
    let unknown = fails(&dir, &["capabilities", "clip.fold"]);
    assert_eq!(unknown["code"], json!("fosscut.request.invalid_arguments"));
    let human = fosscut(&dir, &["capabilities", "clip.split"]);
    let text = String::from_utf8_lossy(&human.stdout);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("clip.split  Split a clip"), "{text}");
    assert!(
        lines[1].starts_with("  at ") && lines[1].contains("required"),
        "{text}"
    );
    assert!(
        lines[2].starts_with("  clip ") && lines[2].contains("required"),
        "{text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn media_analysis_from_the_command_line() {
    if !can_render() {
        return;
    }
    let dir = workspace("analysis");
    // Shots joined at 1.5 s and 3 s, audio with pauses at 1-1.5 s and 3-3.6 s.
    let status = Command::new("ffmpeg")
        .current_dir(&dir)
        .args(["-v", "error", "-filter_complex"])
        .arg("color=c=0x202020:s=320x180:r=30:d=1.5[a];color=c=0xE0E0E0:s=320x180:r=30:d=1.5[b];testsrc2=s=320x180:r=30:d=1[c];[a][b][c]concat=n=3:v=1:a=0[v]")
        .args(["-f", "lavfi", "-i"])
        .arg("aevalsrc=if(between(t\\,1\\,1.5)+between(t\\,3\\,3.6)\\,0\\,0.25*sin(2*PI*440*t)):s=48000:d=4")
        .args(["-map", "[v]", "-map", "0:a", "-c:v", "mpeg4", "-q:v", "3", "-c:a", "aac", "-shortest", "talk.mp4"])
        .status()
        .unwrap();
    assert!(status.success());
    ok(&dir, &["project", "new", "Talk.fosscut"]);
    ok(&dir, &["media", "import", "talk.mp4"]);

    let first = ok(&dir, &["media", "analyze", "asset_0001"]);
    assert_eq!(first["computed"], json!(["loudness", "silences", "scenes"]));
    let ranges: Vec<(Value, Value)> = first["results"]["silences"]["ranges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["start"]["seconds"].clone(), r["end"]["seconds"].clone()))
        .collect();
    assert_eq!(
        ranges,
        [(json!("1"), json!("1.5")), (json!("3"), json!("3.6"))]
    );
    let cuts: Vec<Value> = first["results"]["scenes"]["cuts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["frame"].clone())
        .collect();
    assert_eq!(cuts, [json!(45), json!(90)]);

    // Cached the second time; options measure again.
    let again = ok(&dir, &["media", "analyze", "asset_0001"]);
    assert_eq!(again["cached"], first["computed"]);
    assert_eq!(again["results"], first["results"]);
    let long_pauses = ok(
        &dir,
        &[
            "media",
            "analyze",
            "asset_0001",
            "silences",
            "--min-silence-ms",
            "550",
        ],
    );
    assert_eq!(long_pauses["computed"], json!(["silences"]));
    assert_eq!(
        long_pauses["results"]["silences"]["ranges"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let unknown = fails(&dir, &["media", "analyze", "asset_0001", "faces"]);
    assert_eq!(unknown["code"], json!("fosscut.analysis.unknown_analyzer"));

    let human = fosscut(&dir, &["media", "analyze", "asset_0001"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("from the cache: loudness, silences, scenes"),
        "{text}"
    );
    assert!(
        text.contains("silences  2 ranges, 1.1 s in total: 1-1.5 s, 3-3.6 s"),
        "{text}"
    );
    assert!(
        text.contains("scenes    2 cuts: frame 45 (1.5 s), frame 90 (3 s)"),
        "{text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Copies the speech fixture into `dir` and imports it as `asset_0001` of `Talk.fosscut`.
fn import_speech(dir: &Path) {
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/speech/librispeech-5105-28233-0005/audio.flac"
        ),
        dir.join("speech.flac"),
    )
    .unwrap();
    ok(dir, &["project", "new", "Talk.fosscut"]);
    ok(dir, &["media", "import", "speech.flac"]);
}

#[test]
fn models_from_the_command_line() {
    let dir = workspace("models");
    let listed = ok(&dir, &["models", "list"]);
    let models = listed["models"].as_array().unwrap();
    assert_eq!(models.len(), 4);
    assert!(models.iter().all(|m| m["installed"] == json!(false)));
    assert!(Path::new(listed["dir"].as_str().unwrap()).ends_with("cache/models"));

    // Downloading needs --yes; the error says what it would download.
    let unconfirmed = fails(&dir, &["models", "install", "whisper-tiny.en"]);
    assert_eq!(
        unconfirmed["code"],
        json!("fosscut.model.confirmation_required")
    );
    assert_eq!(unconfirmed["details"]["size_bytes"], json!(77_704_715));
    let human = fosscut(&dir, &["models", "install", "whisper-tiny.en"]);
    let text = String::from_utf8_lossy(&human.stderr);
    assert!(
        text.contains("downloads 77.7 MB (MIT license)") && text.contains("--yes"),
        "{text}"
    );
    let dry = ok(
        &dir,
        &["models", "install", "whisper-tiny.en", "--yes", "--dry-run"],
    );
    assert_eq!(dry["status"], json!("dry_run"));
    let unknown = fails(&dir, &["models", "install", "whisper-huge", "--yes"]);
    assert_eq!(unknown["code"], json!("fosscut.model.unknown"));
    std::fs::write(dir.join("fake.bin"), "not a model").unwrap();
    let wrong = fails(
        &dir,
        &["models", "install", "whisper-tiny.en", "--from", "fake.bin"],
    );
    assert_eq!(wrong["code"], json!("fosscut.model.checksum_mismatch"));

    // A model already in the folder is left alone, and capabilities report it.
    let folder = dir.join("cache/models");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::File::create(folder.join("ggml-tiny.en.bin"))
        .unwrap()
        .set_len(77_704_715)
        .unwrap();
    let human = fosscut(&dir, &["models", "install", "whisper-tiny.en", "--yes"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.starts_with("already installed: whisper-tiny.en in "),
        "{text}"
    );
    let human = fosscut(&dir, &["models", "install", "whisper-base.en", "--dry-run"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.starts_with("dry run: would install whisper-base.en in "),
        "{text}"
    );
    let human = fosscut(&dir, &["models", "list"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("whisper-tiny.en               77.7 MB  MIT  en            installed"),
        "{text}"
    );
    assert!(text.contains("whisper-small") && text.contains("not installed"));
    let capabilities = ok(&dir, &["capabilities"]);
    assert_eq!(capabilities["models"][3]["name"], json!("whisper-tiny.en"));
    assert_eq!(capabilities["models"][3]["installed"], json!(true));
    assert!(
        capabilities["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["name"] == json!("models.install"))
    );

    // Without a model, a transcript fails before decoding, saying how to install one.
    if has_ffmpeg() {
        import_speech(&dir);
        std::fs::remove_file(folder.join("ggml-tiny.en.bin")).unwrap();
        let missing = fails(&dir, &["media", "analyze", "asset_0001", "transcript"]);
        assert_eq!(missing["code"], json!("fosscut.model.not_installed"));
        assert!(missing["hint"].as_str().unwrap().contains("models install"));
    } else {
        assert!(
            !ffmpeg_required(),
            "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn transcripts_from_the_command_line() {
    let models = std::env::var_os("FOSSCUT_MODELS_DIR")
        .map(std::path::PathBuf::from)
        .filter(|dir| {
            std::fs::metadata(dir.join("ggml-tiny.en.bin")).is_ok_and(|m| m.len() == 77_704_715)
        });
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_SPEECH").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        models.is_some() || !required,
        "FOSSCUT_REQUIRE_SPEECH is set but whisper-tiny.en is not in FOSSCUT_MODELS_DIR"
    );
    let Some(models) = models else {
        return;
    };
    if !has_ffmpeg() {
        return;
    }
    let dir = workspace("transcript");
    import_speech(&dir);
    let args = [
        "media",
        "analyze",
        "asset_0001",
        "transcript",
        "--model",
        "whisper-tiny.en",
        "--language",
        "en",
        "--min-pause-ms",
        "250",
    ];
    let output = command(&dir)
        .env("FOSSCUT_MODELS_DIR", &models)
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.stderr.len() < 200,
        "whisper.cpp logs stay quiet: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["computed"], json!(["transcript"]));
    let transcript = &report["results"]["transcript"];
    assert_eq!(transcript["model"], json!("whisper-tiny.en"));
    assert_eq!(transcript["language"], json!("en"));
    assert_eq!(transcript["min_pause_ms"], json!(250));
    assert!(transcript["words"].as_array().unwrap().len() >= 34);

    let human = command(&dir)
        .env("FOSSCUT_MODELS_DIR", &models)
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("from the cache: transcript"), "{text}");
    assert!(
        text.contains("language en, model whisper-tiny.en") && text.contains("fillers   none"),
        "{text}"
    );
    assert!(
        text.contains("pauses    ") && text.contains("over 250 ms"),
        "{text}"
    );
    assert!(text.contains("sandy shore"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

const SUBTITLES: &str = "1\n00:00:00,500 --> 00:00:01,800\n<i>Most people</i> never\nnotice this.\n\n2\n00:00:02,000 --> 00:00:03,250\nBut once you do,\n\n3\n00:00:03,300 --> 00:00:04,500\neverything changes.\n";

#[test]
fn captions_from_subtitle_files() {
    let dir = workspace("captions-srt");
    ok(
        &dir,
        &[
            "project",
            "new",
            "Short.fosscut",
            "--width",
            "1080",
            "--height",
            "1920",
        ],
    );
    std::fs::write(dir.join("subs.srt"), SUBTITLES).unwrap();
    let imported = ok(
        &dir,
        &[
            "-p",
            "Short.fosscut",
            "captions",
            "import",
            "subs.srt",
            "--preset",
            "highlight",
        ],
    );
    assert_eq!(imported["committed"], json!(true));
    let pages = imported["results"][0]["pages"].as_array().unwrap();
    let texts: Vec<&str> = pages.iter().map(|p| p["text"].as_str().unwrap()).collect();
    assert_eq!(
        texts,
        [
            "Most people never\nnotice this.",
            "But once you do,",
            "everything changes."
        ]
    );
    assert_eq!(pages[0]["start"]["frames"], 15);

    // Export, import into a second project and export again: the same file.
    ok(
        &dir,
        &["-p", "Short.fosscut", "captions", "export", "out.srt"],
    );
    let first = std::fs::read_to_string(dir.join("out.srt")).unwrap();
    assert!(
        first.starts_with(
            "1\n00:00:00,500 --> 00:00:01,800\nMost people never\nnotice this.\n\n2\n"
        ),
        "{first}"
    );
    ok(
        &dir,
        &[
            "project",
            "new",
            "Copy.fosscut",
            "--width",
            "1080",
            "--height",
            "1920",
        ],
    );
    ok(
        &dir,
        &["-p", "Copy.fosscut", "captions", "import", "out.srt"],
    );
    ok(
        &dir,
        &["-p", "Copy.fosscut", "captions", "export", "again.srt"],
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("again.srt")).unwrap(),
        first
    );

    // Exports never overwrite unless allowed; a dry run writes nothing.
    let guarded = fails(
        &dir,
        &["-p", "Short.fosscut", "captions", "export", "out.srt"],
    );
    assert_eq!(guarded["code"], json!("fosscut.guard.destructive"));
    let dry = ok(
        &dir,
        &[
            "-p",
            "Short.fosscut",
            "captions",
            "export",
            "dry.srt",
            "--dry-run",
        ],
    );
    assert_eq!(dry["cues"], json!(3));
    assert!(!dir.join("dry.srt").exists());
    ok(
        &dir,
        &[
            "-p",
            "Short.fosscut",
            "captions",
            "export",
            "out.srt",
            "--allow-destructive",
        ],
    );

    // Restyle and reword.
    let styled = ok(
        &dir,
        &[
            "-p",
            "Short.fosscut",
            "captions",
            "style",
            "--preset",
            "boxed",
            "--style",
            r#"{"placement": "top"}"#,
        ],
    );
    assert_eq!(styled["results"][0]["style"]["placement"], json!("top"));
    let clip = pages[1]["clip"].as_str().unwrap();
    let edited = ok(
        &dir,
        &[
            "-p",
            "Short.fosscut",
            "captions",
            "edit",
            clip,
            "--text",
            "Once you see it,\\nyou can't unsee it",
            "--emphasis",
            "4,7",
        ],
    );
    assert_eq!(
        edited["results"][0]["text"],
        json!("Once you see it,\nyou can't unsee it")
    );
    let timeline = ok(
        &dir,
        &[
            "-p",
            "Short.fosscut",
            "timeline",
            "read",
            "--detail",
            "full",
        ],
    );
    let track = &timeline["tracks"][0];
    assert_eq!(track["kind"], json!("caption"));
    assert_eq!(track["caption_style"]["box"]["enabled"], json!(true));
    assert_eq!(track["clips"][1]["lines"][1][0]["emphasis"], json!(true));

    // Readable output and errors.
    let human = fosscut(
        &dir,
        &[
            "-p",
            "Short.fosscut",
            "captions",
            "export",
            "out.srt",
            "--allow-destructive",
        ],
    );
    assert!(String::from_utf8_lossy(&human.stdout).contains("out.srt"));
    std::fs::write(dir.join("bad.srt"), "1\n00:00:01,000 --> soon\nHi\n").unwrap();
    let bad = fails(
        &dir,
        &["-p", "Short.fosscut", "captions", "import", "bad.srt"],
    );
    assert_eq!(bad["code"], json!("fosscut.captions.invalid_srt"));
    assert_eq!(bad["details"]["line"], json!(2));
    let missing = fails(
        &dir,
        &["-p", "Short.fosscut", "captions", "import", "none.srt"],
    );
    assert_eq!(missing["code"], json!("fosscut.request.invalid_arguments"));
    ok(&dir, &["project", "new", "Empty.fosscut"]);
    let empty = fails(
        &dir,
        &["-p", "Empty.fosscut", "captions", "export", "e.srt"],
    );
    assert_eq!(empty["code"], json!("fosscut.captions.none"));
    let nothing = fails(&dir, &["-p", "Empty.fosscut", "captions", "generate"]);
    assert_eq!(nothing["code"], json!("fosscut.captions.no_transcript"));
    let no_track = fails(
        &dir,
        &[
            "-p",
            "Empty.fosscut",
            "captions",
            "generate",
            "--source-track",
            "track_0099",
        ],
    );
    assert_eq!(no_track["code"], json!("fosscut.request.invalid_arguments"));

    // The caption shows up in a frame, inside the bottom of the tall canvas.
    if can_render() {
        ok(
            &dir,
            &[
                "-p",
                "Short.fosscut",
                "captions",
                "style",
                "--style",
                r#"{"placement": "bottom"}"#,
            ],
        );
        ok(
            &dir,
            &[
                "-p",
                "Short.fosscut",
                "frame",
                "1s",
                "--out",
                "caption.png",
                "--width",
                "270",
            ],
        );
        let png = std::fs::read(dir.join("caption.png")).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        let (width, height) = (info.width as usize, info.height as usize);
        let bright_rows: Vec<usize> = (0..height)
            .filter(|&y| {
                (0..width).any(|x| pixels[(y * width + x) * info.color_type.samples() + 1] > 200)
            })
            .collect();
        assert!(!bright_rows.is_empty(), "the caption is drawn");
        let (top, bottom) = (bright_rows[0], *bright_rows.last().unwrap());
        assert!(
            top > height / 2 && bottom < height * 78 / 100,
            "inside the safe area: rows {top} to {bottom} of {height}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn captions_generated_from_a_transcript() {
    let models = std::env::var_os("FOSSCUT_MODELS_DIR")
        .map(std::path::PathBuf::from)
        .filter(|dir| {
            std::fs::metadata(dir.join("ggml-tiny.en.bin")).is_ok_and(|m| m.len() == 77_704_715)
        });
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_SPEECH").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        models.is_some() || !required,
        "FOSSCUT_REQUIRE_SPEECH is set but whisper-tiny.en is not in FOSSCUT_MODELS_DIR"
    );
    let Some(models) = models else {
        return;
    };
    if !has_ffmpeg() {
        return;
    }
    let dir = workspace("captions-generate");
    import_speech(&dir);
    let with_models = |args: &[&str]| {
        let output = command(&dir)
            .env("FOSSCUT_MODELS_DIR", &models)
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    // Show 1 s to 7 s of the recording from 2 s on the timeline.
    ok(
        &dir,
        &[
            "clip",
            "add",
            "--asset",
            "asset_0001",
            "--source-in",
            "1s",
            "--start",
            "2s",
            "--duration",
            "6s",
        ],
    );
    let missing = fails(&dir, &["captions", "generate"]);
    assert_eq!(missing["code"], json!("fosscut.captions.no_transcript"));
    assert_eq!(missing["details"]["assets"], json!(["asset_0001"]));
    let analysis = with_models(&["media", "analyze", "asset_0001", "transcript"]);
    let generated = with_models(&["captions", "generate", "--preset", "bold-pop"]);
    assert_eq!(generated["committed"], json!(true));

    // Every word spoken in the shown part, at its media time moved onto the timeline.
    let seconds = |v: &Value| v["seconds"].as_str().unwrap().parse::<f64>().unwrap();
    let expected: Vec<(String, f64)> = analysis["results"]["transcript"]["words"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| (1.0..7.0).contains(&seconds(&w["start"])))
        .map(|w| {
            (
                w["text"].as_str().unwrap().to_string(),
                seconds(&w["start"]) + 1.0,
            )
        })
        .collect();
    let timeline = ok(&dir, &["timeline", "read", "--detail", "full"]);
    let captions = timeline["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == json!("caption"))
        .unwrap();
    let shown: Vec<(String, f64)> = captions["clips"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["lines"].as_array().unwrap().iter())
        .flat_map(|l| l.as_array().unwrap().iter())
        .map(|w| {
            (
                w["text"].as_str().unwrap().to_string(),
                seconds(&w["start"]),
            )
        })
        .collect();
    assert!(expected.len() > 10, "{expected:?}");
    assert_eq!(shown.len(), expected.len());
    for ((word, at), (want, want_at)) in shown.iter().zip(&expected) {
        assert_eq!(word, want);
        assert!((at - want_at).abs() < 1e-5, "{word}: {at} vs {want_at}");
    }
    assert_eq!(captions["caption_style"]["weight"], json!("black"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cleanup_and_reframe_from_the_command_line() {
    if !has_ffmpeg() {
        assert!(
            !ffmpeg_required(),
            "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
        );
        return;
    }
    let dir = workspace("cleanup-reframe");
    test_pattern(&dir);
    ok(&dir, &["project", "new", "Talk.fosscut"]);
    ok(&dir, &["media", "import", "test-pattern.mp4"]);
    ok(&dir, &["clip", "add", "--asset", "asset_0001"]);

    // Cut two ranges; the primary video needs permission.
    let guarded = fails(&dir, &["timeline", "remove-ranges", "1s..2s", "4s..4.5s"]);
    assert_eq!(guarded["code"], json!("fosscut.guard.destructive"));
    let cut = ok(
        &dir,
        &[
            "timeline",
            "remove-ranges",
            "1s..2s",
            "00:00:04:00..00:00:04:15",
            "--allow-destructive",
        ],
    );
    let result = &cut["results"][0];
    assert_eq!(result["removed"]["seconds"], json!("1.5"));
    assert_eq!(result["duration_after"]["seconds"], json!("6.5"));
    let gaps = ok(
        &dir,
        &[
            "timeline",
            "remove-ranges",
            "5s..6s",
            "--no-ripple",
            "--allow-destructive",
            "--dry-run",
        ],
    );
    assert_eq!(
        gaps["results"][0]["duration_after"]["seconds"],
        json!("6.5")
    );
    let bad = fails(&dir, &["timeline", "remove-ranges", "1s-2s"]);
    assert_eq!(bad["code"], json!("fosscut.request.invalid_arguments"));
    let unreadable = fails(&dir, &["timeline", "remove-ranges", "soon..later"]);
    assert_eq!(
        unreadable["code"],
        json!("fosscut.request.invalid_arguments")
    );

    // A vertical copy that fills the canvas, then one shot moved to show its left side.
    let copy = ok(
        &dir,
        &[
            "timeline",
            "duplicate",
            "--width",
            "1080",
            "--height",
            "1920",
            "--fill",
            "--name",
            "Vertical",
        ],
    );
    let vertical = copy["results"][0]["timeline"].as_str().unwrap().to_string();
    let view = ok(
        &dir,
        &[
            "timeline",
            "read",
            "--timeline",
            &vertical,
            "--detail",
            "full",
        ],
    );
    let shot = view["tracks"][0]["clips"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let moved = ok(
        &dir,
        &[
            "clip",
            "set",
            "--clip",
            &shot,
            "--position",
            "0.4,0",
            "--scale",
            "3.2",
            "--opacity",
            "1",
        ],
    );
    assert_eq!(
        moved["results"][0]["transform"]["position"],
        json!([0.4, 0.0])
    );
    let human = fosscut(
        &dir,
        &["clip", "set", "--clip", &shot, "--position", "-0.1,0.05"],
    );
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let tilted = ok(&dir, &["clip", "set", "--clip", &shot, "--rotation", "-2"]);
    assert_eq!(
        tilted["results"][0]["transform"]["position"],
        json!([-0.1, 0.05])
    );
    let single = fails(&dir, &["clip", "set", "--clip", &shot, "--position", "0.3"]);
    assert_eq!(single["code"], json!("fosscut.request.invalid_arguments"));

    if can_render() {
        ok(
            &dir,
            &[
                "frame",
                "0.5s",
                "--timeline",
                &vertical,
                "--out",
                "vertical.png",
                "--width",
                "270",
            ],
        );
        assert_eq!(png_size(&dir.join("vertical.png")), (270, 480));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Columns of a PNG frame grab where most of a row band is white.
fn white_columns(path: &Path) -> Vec<usize> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let (width, height, samples) = (
        info.width as usize,
        info.height as usize,
        info.color_type.samples(),
    );
    let middle = height / 2;
    (0..width)
        .filter(|&x| {
            let at = (middle * width + x) * samples;
            pixels[at..at + 3].iter().all(|&c| c > 220)
        })
        .collect()
}

#[test]
fn a_reframed_subject_stays_in_the_vertical_frame() {
    if !can_render() {
        return;
    }
    let dir = workspace("reframe-subject");
    // Two shots with a white square at the left, then at the right, cut at frame 45.
    let status = Command::new("ffmpeg")
        .current_dir(&dir)
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg("color=c=0x203050:s=640x360:r=30:d=1.5,drawbox=x=60:y=140:w=80:h=80:color=white:t=fill[a];color=c=0x503040:s=640x360:r=30:d=1.5,drawbox=x=500:y=140:w=80:h=80:color=white:t=fill[b];[a][b]concat=n=2:v=1:a=0")
        .args(["-c:v", "mpeg4", "-q:v", "2", "shots.mp4"])
        .status()
        .unwrap();
    assert!(status.success());
    ok(
        &dir,
        &[
            "project",
            "new",
            "Wide.fosscut",
            "--width",
            "640",
            "--height",
            "360",
        ],
    );
    ok(&dir, &["media", "import", "shots.mp4"]);
    ok(&dir, &["clip", "add", "--asset", "asset_0001"]);
    let copy = ok(
        &dir,
        &[
            "timeline",
            "duplicate",
            "--width",
            "360",
            "--height",
            "640",
            "--fill",
        ],
    );
    let vertical = copy["results"][0]["timeline"].as_str().unwrap().to_string();
    let grab = |time: &str, out: &str| {
        ok(
            &dir,
            &[
                "frame",
                time,
                "--timeline",
                &vertical,
                "--out",
                out,
                "--width",
                "180",
            ],
        );
        white_columns(&dir.join(out))
    };
    // Filled but not yet framed, the centre of the 3.16x picture shows neither square.
    assert!(grab("0.7s", "before.png").is_empty());

    // The skill's formula: x = -(u - 0.5) * s within (s - 1) / 2, for u = 0.156 and 0.844.
    let view = ok(
        &dir,
        &[
            "timeline",
            "read",
            "--timeline",
            &vertical,
            "--detail",
            "full",
        ],
    );
    let clip = view["tracks"][0]["clips"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let scale = view["tracks"][0]["clips"][0]["transform"]["scale"]
        .as_f64()
        .unwrap();
    let split = ok(&dir, &["clip", "split", "--clip", &clip, "--at", "1.5s"]);
    let second = split["results"][0]["right"]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let frame = |u: f64| {
        let x: f64 = -(u - 0.5) * scale;
        x.clamp(-(scale - 1.0) / 2.0, (scale - 1.0) / 2.0)
    };
    for (shot, u) in [(&clip, 100.0 / 640.0), (&second, 540.0 / 640.0)] {
        let position = format!("{},0", frame(u));
        ok(
            &dir,
            &["clip", "set", "--clip", shot, "--position", &position],
        );
    }
    for (time, out) in [("0.7s", "first.png"), ("2.2s", "second.png")] {
        let columns = grab(time, out);
        assert!(!columns.is_empty(), "{out}: the square is in frame");
        let centre = columns.iter().sum::<usize>() / columns.len();
        assert!(
            (30..150).contains(&centre),
            "{out}: square centred at column {centre} of 180"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn keyframes_flips_and_deactivation_show_in_frames() {
    if !can_render() {
        return;
    }
    let dir = workspace("keyframes");
    // A white square on the left of a dark frame.
    let status = Command::new("ffmpeg")
        .current_dir(&dir)
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg("color=c=0x203050:s=640x360:r=30:d=3,drawbox=x=60:y=140:w=80:h=80:color=white:t=fill")
        .args(["-c:v", "mpeg4", "-q:v", "2", "square.mp4"])
        .status()
        .unwrap();
    assert!(status.success());
    let new = [
        "project",
        "new",
        "Keys.fosscut",
        "--width",
        "640",
        "--height",
        "360",
    ];
    ok(&dir, &new);
    ok(&dir, &["media", "import", "square.mp4"]);
    let placed = ok(&dir, &["clip", "add", "--asset", "asset_0001"]);
    let clip = placed["results"][0]["clip"].as_str().unwrap().to_string();
    let grab = |time: &str, out: &str| {
        ok(&dir, &["frame", time, "--out", out, "--width", "320"]);
        white_columns(&dir.join(out))
    };
    assert!(!grab("1s", "plain.png").is_empty());

    // Fade the picture in over the first second.
    for (at, value, easing) in [("0s", "0", "ease_in"), ("1s", "1", "linear")] {
        ok(
            &dir,
            &[
                "keyframe",
                "set",
                "--clip",
                &clip,
                "--property",
                "opacity",
                "--at",
                at,
                "--value",
                value,
                "--easing",
                easing,
            ],
        );
    }
    assert!(grab("0s", "start.png").is_empty(), "invisible at the start");
    assert!(
        !grab("2s", "after.png").is_empty(),
        "visible after the fade"
    );
    let blocked = fails(&dir, &["clip", "set", "--clip", &clip, "--opacity", "1"]);
    assert_eq!(blocked["code"], json!("fosscut.clip.property_animated"));

    // A horizontal flip moves the square to the right side.
    ok(
        &dir,
        &[
            "clip",
            "set",
            "--clip",
            &clip,
            "--set",
            "transform.flip_horizontal=true",
            "--blend",
            "screen",
            "--fade-in",
            "0.5s",
        ],
    );
    let flipped = grab("2s", "flipped.png");
    assert!(flipped.iter().all(|&x| x > 160), "{flipped:?}");
    let view = ok(&dir, &["timeline", "read", "--detail", "full"]);
    let shown = &view["tracks"][0]["clips"][0];
    assert_eq!(shown["blend"], json!("screen"));
    assert_eq!(
        shown["keyframes"]["opacity"][1]["at"]["seconds"],
        json!("1")
    );

    // Deactivated, the clip draws nothing; removing the keyframes keeps the value at 2 s.
    ok(
        &dir,
        &["clip", "set", "--clip", &clip, "--enabled", "false"],
    );
    assert!(grab("2s", "off.png").is_empty());
    ok(&dir, &["clip", "set", "--clip", &clip, "--enabled", "true"]);
    ok(
        &dir,
        &[
            "keyframe",
            "remove",
            "--clip",
            &clip,
            "--property",
            "opacity",
            "--all",
            "--at",
            "2s",
        ],
    );
    assert!(
        !grab("0s", "kept.png").is_empty(),
        "opacity 1 everywhere now"
    );
    let bad = fails(
        &dir,
        &["clip", "set", "--clip", &clip, "--set", "crop.left"],
    );
    assert_eq!(bad["code"], json!("fosscut.request.invalid_arguments"));
}

/// The mean of one channel over a PNG.
fn mean_channel(path: &Path, channel: usize) -> f64 {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let samples = info.color_type.samples();
    let values: Vec<f64> = pixels[..info.buffer_size()]
        .chunks(samples)
        .map(|p| f64::from(p[channel]))
        .collect();
    values.iter().sum::<f64>() / values.len() as f64
}

#[test]
fn effects_luts_adjustments_and_auto_color_from_the_command_line() {
    if !can_render() {
        return;
    }
    let dir = workspace("effects");
    let status = Command::new("ffmpeg")
        .current_dir(&dir)
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg("testsrc2=s=320x180:r=30:d=2,colorchannelmixer=rr=0.55:gg=0.75:bb=1.0")
        .args(["-c:v", "mpeg4", "-q:v", "2", "blue.mp4"])
        .status()
        .unwrap();
    assert!(status.success());
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for b in [0.0, 1.0] {
        for g in [0.0, 1.0] {
            for r in [0.0, 1.0] {
                // Swap red and blue.
                cube.push_str(&format!("{b} {g} {r}\n"));
            }
        }
    }
    std::fs::write(dir.join("swap.cube"), cube).unwrap();
    std::fs::write(dir.join("broken.cube"), "LUT_3D_SIZE 9\n0 0 0\n").unwrap();
    let new = [
        "project",
        "new",
        "Grade.fosscut",
        "--width",
        "320",
        "--height",
        "180",
    ];
    ok(&dir, &new);
    ok(&dir, &["media", "import", "blue.mp4"]);
    ok(&dir, &["media", "import", "swap.cube"]);
    let media = ok(&dir, &["media", "list"]);
    let kinds: Vec<&str> = media["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["video", "lut"]);
    let broken = fails(&dir, &["media", "import", "broken.cube"]);
    assert_eq!(broken["code"], json!("fosscut.media.probe_failed"));
    let placed = ok(&dir, &["clip", "add", "--asset", "asset_0001"]);
    let clip = placed["results"][0]["clip"].as_str().unwrap().to_string();
    let grab = |out: &str| {
        ok(&dir, &["frame", "1s", "--out", out, "--width", "160"]);
        let path = dir.join(out);
        (mean_channel(&path, 0), mean_channel(&path, 2))
    };
    let (plain_red, plain_blue) = grab("plain.png");
    assert!(
        plain_blue > plain_red + 20.0,
        "blue to start with: {plain_red} {plain_blue}"
    );

    // The LUT swaps red and blue.
    let lut = ok(
        &dir,
        &[
            "effect",
            "add",
            "--clip",
            &clip,
            "--type",
            "color.lut",
            "--param",
            "lut=\"asset_0002\"",
        ],
    );
    let effect = lut["results"][0]["effect"].as_str().unwrap().to_string();
    let (red, blue) = grab("lut.png");
    assert!(red > blue + 20.0, "swapped: {red} {blue}");
    ok(
        &dir,
        &[
            "effect",
            "set",
            "--clip",
            &clip,
            "--effect",
            &effect,
            "--enabled",
            "false",
        ],
    );
    let (red, blue) = grab("bypassed.png");
    assert!(blue > red + 20.0, "bypassed: {red} {blue}");
    ok(
        &dir,
        &["effect", "remove", "--clip", &clip, "--effect", &effect],
    );

    // An adjustment layer turns everything below it gray; a look adds its effects.
    let adjust = ok(
        &dir,
        &[
            "adjustment",
            "add",
            "--start",
            "0s",
            "--duration",
            "2s",
            "--type",
            "color.adjust",
        ],
    );
    let layer = adjust["results"][0]["clip"].as_str().unwrap().to_string();
    let grade = adjust["results"][0]["effects"][0]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &dir,
        &[
            "effect",
            "set",
            "--clip",
            &layer,
            "--effect",
            &grade,
            "--param",
            "saturation=-1",
        ],
    );
    let (red, blue) = grab("gray.png");
    assert!(
        (red - blue).abs() < 3.0,
        "gray under the adjustment: {red} {blue}"
    );
    ok(
        &dir,
        &["look", "apply", "vintage", "--clip", &layer, "--replace"],
    );
    let view = ok(&dir, &["timeline", "read", "--detail", "full"]);
    let effects = &view["tracks"][1]["clips"][0]["effects"];
    assert_eq!(
        effects.as_array().unwrap().len(),
        3,
        "replaced by the vintage look's three effects"
    );
    let unknown = fails(&dir, &["look", "apply", "sparkle", "--clip", &layer]);
    assert_eq!(unknown["code"], json!("fosscut.look.unknown"));
    ok(&dir, &["undo"]);

    // Automatic color warms the blue picture.
    ok(
        &dir,
        &["effect", "remove", "--clip", &layer, "--effect", &grade],
    );
    let auto = ok(&dir, &["color", "auto", "--clip", &clip]);
    assert_eq!(
        auto["results"][0]["effects"][0]["type"],
        json!("color.adjust")
    );
    let temperature = auto["results"][0]["effects"][0]["params"]["temperature"]
        .as_f64()
        .unwrap();
    assert!(temperature > 0.0, "warmer: {temperature}");
    let (red_after, blue_after) = grab("auto.png");
    assert!(
        blue_after - red_after < (plain_blue - plain_red) / 2.0,
        "closer to neutral: {plain_red} {plain_blue} became {red_after} {blue_after}"
    );
    let text = fails(
        &dir,
        &["color", "match", "--clip", &clip, "--reference", &clip],
    );
    assert_eq!(text["code"], json!("fosscut.request.invalid_arguments"));
    let copy = ok(&dir, &["effect", "copy", "--from", &clip, "--to", &layer]);
    assert_eq!(
        copy["results"][0]["clips"][0]["effects"][0]["type"],
        json!("color.adjust")
    );
}

#[test]
fn masks_and_chroma_keys_from_the_command_line() {
    if !can_render() {
        return;
    }
    let dir = workspace("masks");
    for (name, source) in [
        ("white.mp4", "color=c=white:s=320x180:r=30:d=2"),
        (
            "screen.mp4",
            "color=c=0x00ff00:s=320x180:r=30:d=2,drawbox=x=140:y=70:w=40:h=40:color=white:t=fill",
        ),
        ("red.mp4", "color=c=0xff0000:s=320x180:r=30:d=2"),
    ] {
        let status = Command::new("ffmpeg")
            .current_dir(&dir)
            .args(["-v", "error", "-f", "lavfi", "-i", source])
            .args(["-c:v", "mpeg4", "-q:v", "2", name])
            .status()
            .unwrap();
        assert!(status.success());
    }
    let new = [
        "project",
        "new",
        "Masks.fosscut",
        "--width",
        "320",
        "--height",
        "180",
    ];
    ok(&dir, &new);
    for file in ["white.mp4", "screen.mp4", "red.mp4"] {
        ok(&dir, &["media", "import", file]);
    }
    let white = ok(&dir, &["clip", "add", "--asset", "asset_0001"]);
    let clip = white["results"][0]["clip"].as_str().unwrap().to_string();
    let grab = |out: &str| {
        ok(&dir, &["frame", "1s", "--out", out, "--width", "160"]);
        white_columns(&dir.join(out))
    };
    assert_eq!(grab("plain.png").len(), 160);
    let added = ok(
        &dir,
        &[
            "mask",
            "add",
            "--clip",
            &clip,
            "--shape",
            "rectangle",
            "--set",
            "center=[0.25,0.5]",
            "--set",
            "size=[0.5,1]",
        ],
    );
    let mask = added["results"][0]["mask"].as_str().unwrap().to_string();
    let left = grab("left.png");
    assert!(
        left.iter().all(|&x| x < 82) && left.len() > 70,
        "the left half: {left:?}"
    );
    ok(
        &dir,
        &[
            "mask",
            "set",
            "--clip",
            &clip,
            "--mask",
            &mask,
            "--set",
            "invert=true",
        ],
    );
    let right = grab("right.png");
    assert!(right.iter().all(|&x| x > 78), "inverted: {right:?}");
    let bad = fails(&dir, &["mask", "add", "--clip", &clip, "--shape", "path"]);
    assert_eq!(
        bad["code"],
        json!("fosscut.model.invalid"),
        "a path needs points"
    );
    ok(&dir, &["mask", "remove", "--clip", &clip, "--mask", &mask]);
    ok(
        &dir,
        &["clip", "set", "--clip", &clip, "--enabled", "false"],
    );

    // The green screen over red: keyed, the red shows around the white square.
    let below = ok(&dir, &["track", "add", "--kind", "video"]);
    let below = below["results"][0]["track"].as_str().unwrap().to_string();
    let above = ok(&dir, &["track", "add", "--kind", "video"]);
    let above = above["results"][0]["track"].as_str().unwrap().to_string();
    let red_clip = [
        "clip",
        "add",
        "--asset",
        "asset_0003",
        "--start",
        "0s",
        "--track",
    ];
    let mut args = red_clip.to_vec();
    args.push(&below);
    ok(&dir, &args);
    let screen_clip = [
        "clip",
        "add",
        "--asset",
        "asset_0002",
        "--start",
        "0s",
        "--track",
    ];
    let mut args = screen_clip.to_vec();
    args.push(&above);
    let screen = ok(&dir, &args);
    let screen = screen["results"][0]["clip"].as_str().unwrap().to_string();
    ok(
        &dir,
        &["effect", "add", "--clip", &screen, "--type", "key.chroma"],
    );
    ok(
        &dir,
        &["frame", "1s", "--out", "keyed.png", "--width", "160"],
    );
    let red = mean_channel(&dir.join("keyed.png"), 0);
    let green = mean_channel(&dir.join("keyed.png"), 1);
    assert!(
        red > 200.0 && green < 60.0,
        "red shows through: {red} {green}"
    );
    assert!(
        !white_columns(&dir.join("keyed.png")).is_empty(),
        "the square stays"
    );
}

#[test]
fn transitions_animations_and_effects_from_the_command_line() {
    if !can_render() {
        return;
    }
    let dir = workspace("transitions");
    for (name, source) in [
        ("white.mp4", "color=c=white:s=320x180:r=30:d=3"),
        ("red.mp4", "color=c=0xff0000:s=320x180:r=30:d=3"),
    ] {
        let status = Command::new("ffmpeg")
            .current_dir(&dir)
            .args(["-v", "error", "-f", "lavfi", "-i", source])
            .args(["-c:v", "mpeg4", "-q:v", "2", name])
            .status()
            .unwrap();
        assert!(status.success());
    }
    let new = [
        "project",
        "new",
        "Cuts.fosscut",
        "--width",
        "320",
        "--height",
        "180",
    ];
    ok(&dir, &new);
    ok(&dir, &["media", "import", "white.mp4"]);
    ok(&dir, &["media", "import", "red.mp4"]);
    let first = ok(
        &dir,
        &["clip", "add", "--asset", "asset_0001", "--duration", "2s"],
    );
    let first = first["results"][0]["clip"].as_str().unwrap().to_string();
    let second = ok(
        &dir,
        &["clip", "add", "--asset", "asset_0002", "--duration", "2s"],
    );
    let second = second["results"][0]["clip"].as_str().unwrap().to_string();
    let channels = |time: &str, out: &str| {
        ok(&dir, &["frame", time, "--out", out, "--width", "160"]);
        let path = dir.join(out);
        (mean_channel(&path, 0), mean_channel(&path, 1))
    };
    let (_, green) = channels("2s", "cut.png");
    assert!(green < 20.0, "a plain cut: red at 2 s");
    ok(
        &dir,
        &[
            "transition",
            "set",
            "--clip",
            &first,
            "--type",
            "dissolve",
            "--duration",
            "1s",
        ],
    );
    let (red, green) = channels("2s", "middle.png");
    assert!(
        red > 240.0 && green > 100.0 && green < 230.0,
        "half way: {red} {green}"
    );
    let lonely = fails(
        &dir,
        &["transition", "set", "--clip", &second, "--type", "wipe"],
    );
    assert_eq!(lonely["code"], json!("fosscut.transition.no_neighbor"));

    ok(
        &dir,
        &["animation", "set", "--clip", &first, "--in", "fade:1s"],
    );
    let (red, _) = channels("0s", "start.png");
    assert!(red < 10.0, "faded in from black: {red}");
    ok(
        &dir,
        &[
            "effect",
            "add",
            "--clip",
            &second,
            "--type",
            "stylize.invert",
        ],
    );
    let (red, green) = channels("3.5s", "inverted.png");
    assert!(
        red < 20.0 && green > 230.0,
        "red inverted to cyan: {red} {green}"
    );
    let bad = fails(
        &dir,
        &["animation", "set", "--clip", &first, "--combo", "fade"],
    );
    assert_eq!(bad["code"], json!("fosscut.request.invalid_arguments"));
    // The export renders through the transition, and the edit keeps its length.
    let exported = ok_with_progress(&dir, &["export", "Cuts.mp4"]);
    assert_eq!(exported["frames"], json!(120), "{exported}");
    ok(&dir, &["transition", "remove", "--clip", &first]);
}

/// ADR-0023: a clip at twice the speed exports half as many frames, a reversed clip shows
/// its last frame first, and a freeze frame holds one picture and pushes the rest later.
#[test]
fn speed_reverse_and_freeze_from_the_command_line() {
    if !can_render() {
        return;
    }
    let dir = workspace("speed");
    // Brightness rises with time, so the frame shown tells the source time.
    let status = Command::new("ffmpeg")
        .current_dir(&dir)
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg("nullsrc=s=160x90:r=30:d=2,geq=lum='16+T*100':cb=128:cr=128,format=yuv420p")
        .args(["-c:v", "mpeg4", "-q:v", "2", "ramp.mp4"])
        .status()
        .unwrap();
    assert!(status.success());
    let new = [
        "project",
        "new",
        "Speed.fosscut",
        "--width",
        "160",
        "--height",
        "90",
    ];
    ok(&dir, &new);
    ok(&dir, &["media", "import", "ramp.mp4"]);
    let added = ok(&dir, &["clip", "add", "--asset", "asset_0001"]);
    let clip = added["results"][0]["clip"].as_str().unwrap().to_string();
    let fast = ok(&dir, &["clip", "speed", "--clip", &clip, "--rate", "2"]);
    assert_eq!(
        fast["results"][0]["placement"]["duration"]["seconds"], "1",
        "{fast}"
    );
    let exported = ok_with_progress(&dir, &["export", "Fast.mp4"]);
    assert_eq!(exported["frames"], json!(30), "{exported}");
    let level = |time: &str, out: &str| {
        ok(&dir, &["frame", time, "--out", out, "--width", "80"]);
        mean_channel(&dir.join(out), 1)
    };
    ok(&dir, &["clip", "speed", "--clip", &clip, "--reset"]);
    ok(&dir, &["clip", "speed", "--clip", &clip, "--reverse"]);
    let (start, end) = (level("0s", "start.png"), level("1.9s", "end.png"));
    assert!(
        start > end + 100.0,
        "bright first, dark last: {start} then {end}"
    );
    ok(&dir, &["clip", "speed", "--clip", &clip, "--forward"]);
    let frozen = ok(
        &dir,
        &[
            "clip",
            "freeze",
            "--clip",
            &clip,
            "--at",
            "1s",
            "--duration",
            "1s",
        ],
    );
    assert_eq!(frozen["results"][0]["moved"].as_array().unwrap().len(), 1);
    let (a, b) = (level("1.1s", "held-a.png"), level("1.9s", "held-b.png"));
    assert!((a - b).abs() < 1.0, "one picture while frozen: {a} and {b}");
    let after = level("2.5s", "after.png");
    assert!(
        after > a + 20.0,
        "the clip goes on after the freeze: {after}"
    );
    let exported = ok_with_progress(&dir, &["export", "Frozen.mp4"]);
    assert_eq!(exported["frames"], json!(90), "three seconds now");
    let curved = ok(
        &dir,
        &[
            "clip", "speed", "--clip", &clip, "--curve", "0:1,1:3", "--pitch", "shift",
        ],
    );
    assert_eq!(curved["results"][0]["speed"]["pitch"], "shift", "{curved}");
    let bad = fails(&dir, &["clip", "speed", "--clip", &clip, "--curve", "half"]);
    assert_eq!(bad["code"], "fosscut.request.invalid_arguments", "{bad}");
}

/// ADR-0024 and the timeline commands: markers, extracting sound into a linked clip that
/// moves with its video, copying and pasting clips, split all, ripple trims and alignment.
#[test]
fn timeline_editing_from_the_command_line() {
    if !can_render() {
        return;
    }
    let dir = workspace("timeline-editing");
    let status = Command::new("ffmpeg")
        .current_dir(&dir)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=160x90:r=30:d=4",
        ])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4"])
        .args(["-c:v", "mpeg4", "-c:a", "aac", "-shortest", "clip.mp4"])
        .status()
        .unwrap();
    assert!(status.success());
    let new = [
        "project",
        "new",
        "Edit.fosscut",
        "--width",
        "160",
        "--height",
        "90",
    ];
    ok(&dir, &new);
    ok(&dir, &["media", "import", "clip.mp4"]);
    let added = ok(&dir, &["clip", "add", "--asset", "asset_0001"]);
    let video = added["results"][0]["clip"].as_str().unwrap().to_string();
    ok(&dir, &["marker", "add", "--at", "1s", "--name", "beat"]);
    let extracted = ok(&dir, &["clip", "extract-audio", "--clip", &video]);
    let audio = extracted["results"][0]["audio"]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let moved = ok(&dir, &["clip", "move", "--clip", &video, "--start", "1s"]);
    assert_eq!(moved["results"][0]["linked"][0]["clip"], audio, "{moved}");
    let read = ok(&dir, &["timeline", "read", "--detail", "full"]);
    assert_eq!(read["markers"][0]["name"], "beat");
    let sound = &read["tracks"][1]["clips"][0];
    assert_eq!(
        sound["start"]["seconds"], "1",
        "the sound moved with the picture"
    );
    assert_eq!(read["tracks"][0]["clips"][0]["audio_detached"], true);
    // Copy both and paste them later, keeping their arrangement.
    let copied = ok(&dir, &["clips", "copy", "--clip", &video, "--clip", &audio]);
    let pasted = ok(
        &dir,
        &[
            "clips",
            "paste",
            "--at",
            "6s",
            "--from",
            &copied.to_string(),
        ],
    );
    assert_eq!(pasted["results"][0]["clips"].as_array().unwrap().len(), 2);
    let split = ok(&dir, &["timeline", "split", "--at", "2s"]);
    assert_eq!(split["results"][0]["splits"].as_array().unwrap().len(), 2);
    // Delete left of 1.5 s with ripple: the linked sound loses the same part.
    let trimmed = ok(
        &dir,
        &[
            "clip", "trim", "--clip", &video, "--start", "1.5s", "--ripple",
        ],
    );
    assert_eq!(
        trimmed["results"][0]["linked"][0]["clip"], audio,
        "{trimmed}"
    );
    let read = ok(&dir, &["timeline", "read", "--detail", "full"]);
    assert_eq!(read["tracks"][0]["clips"][0]["duration"]["seconds"], "0.5");
    assert_eq!(read["tracks"][1]["clips"][0]["duration"]["seconds"], "0.5");
    ok(
        &dir,
        &[
            "clip",
            "set",
            "--clip",
            &video,
            "--set",
            "transform.scale=0.5",
        ],
    );
    let aligned = ok(
        &dir,
        &["clip", "align", "--clip", &video, "--horizontal", "right"],
    );
    assert_eq!(aligned["results"][0]["position"], json!([0.25, 0.0]));
    let restored = ok(&dir, &["clip", "restore-audio", "--clip", &video]);
    assert_eq!(restored["results"][0]["removed"], json!([audio]));
    let bad = fails(&dir, &["clips", "paste", "--at", "1s", "--from", "{}"]);
    assert_eq!(bad["code"], "fosscut.request.invalid_arguments", "{bad}");
}
