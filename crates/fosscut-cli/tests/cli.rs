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
        .env("FOSSCUT_CACHE_DIR", dir.join("cache"))
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
    if !has_ffmpeg() {
        assert!(
            !ffmpeg_required(),
            "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
        );
        return;
    }
    if fosscut_render::Renderer::headless().is_err() {
        let required =
            std::env::var_os("FOSSCUT_REQUIRE_GPU").is_some_and(|v| !v.is_empty() && v != "0");
        assert!(
            !required,
            "FOSSCUT_REQUIRE_GPU is set but there is no adapter"
        );
        return;
    }
    let dir = workspace("baseline");
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
    assert!(status.success());
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

    let report = ok(
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
    let human = fosscut(
        &dir,
        &[
            "export",
            "Baseline.mp4",
            "--preset",
            preset,
            "--allow-destructive",
        ],
    );
    assert!(human.status.success());
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("240 frames") && text.contains("verified against the request"),
        "{text}"
    );
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
