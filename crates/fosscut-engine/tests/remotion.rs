//! Rendering the user's own Remotion project (ADR-0039) with a fake `npx` that records its
//! command line and writes ProRes 4444 with FFmpeg, so no Remotion is needed: the command,
//! confirmation, progress, cancellation, failures, the background job with its import, and
//! decoding the result with its alpha. A real Remotion run is skipped unless
//! `FOSSCUT_REQUIRE_REMOTION=1`, with `FOSSCUT_REMOTION_PROJECT` naming a Remotion project
//! and `FOSSCUT_REMOTION_COMPOSITION` its composition (`Main` when unset).

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::required;
use fosscut_engine::jobs::{JobState, Jobs};
use fosscut_engine::remotion::{self, LICENCE_NOTE, RemotionRequest};
use fosscut_engine::{EngineError, Progress};
use fosscut_media::ffmpeg::FfmpegVideoDecoder;
use fosscut_media::{DecoderOptions, PixelFormat, VideoDecoder};
use fosscut_time::Flicks;

fn ffmpeg() -> bool {
    let found = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        found || !required("FOSSCUT_REQUIRE_FFMPEG"),
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    found
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-remotion-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A Remotion project as `npm install` leaves it, without any real Remotion.
fn project(dir: &Path) -> PathBuf {
    let project = dir.join("my-video");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(project.join("node_modules/@remotion/cli")).unwrap();
    std::fs::write(
        project.join("package.json"),
        r#"{"name":"my-video","dependencies":{"remotion":"4.0.0","@remotion/cli":"4.0.0"}}"#,
    )
    .unwrap();
    std::fs::write(project.join("src/index.ts"), "// registerRoot(Root)").unwrap();
    project
}

/// A fake `npx` that records its arguments and working folder in `args.txt` beside it, then
/// behaves as `mode` says: `render` prints progress and writes a half-transparent red
/// ProRes 4444 clip to the output argument, `fail` prints an error and exits 1, `hang`
/// waits until killed.
fn fake_npx(dir: &Path, mode: &str) -> PathBuf {
    let script = dir.join(format!("npx-{mode}"));
    let log = dir.join("args.txt");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$PWD" "$@" > "{log}"
case "{mode}" in
  render)
    out="$6"
    printf 'Bundling 100%%\n'
    printf 'Rendered 1/3\rRendered 2/3\rRendered 3/3\n'
    printf 'Encoded 3/3\n' >&2
    ffmpeg -v error -y -f lavfi -i "color=c=red@0.5:s=64x36:r=30:d=0.1,format=yuva444p10le" \
      -c:v prores_ks -profile:v 4444 -pix_fmt yuva444p10le -f mov "$out" || exit 3
    ;;
  fail)
    echo "Error: No composition with the ID Nope found." >&2
    exit 1
    ;;
  hang)
    echo "Rendered 0/90"
    sleep 30 &
    echo $! > "{dir}/child.pid"
    wait
    ;;
esac
"#,
            log = log.display(),
            dir = dir.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn request(dir: &Path, npx: PathBuf, confirmed: bool) -> RemotionRequest {
    let bundle = dir.join("Show.fosscut");
    std::fs::create_dir_all(&bundle).unwrap();
    RemotionRequest {
        project: project(dir),
        composition: "Main".into(),
        props: Some(serde_json::json!({ "title": "Sale" })),
        entry: None,
        bundle,
        confirmed,
        npx: Some(npx),
    }
}

fn web_code(error: &EngineError) -> &str {
    error.code()
}

#[test]
fn nothing_runs_until_the_user_confirms_remotions_licence() {
    let dir = folder("confirm");
    let request = request(&dir, fake_npx(&dir, "render"), false);
    let error = remotion::plan(&request).unwrap_err();
    assert_eq!(web_code(&error), "fosscut.web.confirmation_required");
    assert!(
        error.to_string().contains("paid company licence"),
        "{error}"
    );
    let details = error.details();
    assert_eq!(details["licence"], LICENCE_NOTE);
    let command: Vec<&str> = details["command"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(
        command[1..5],
        ["--no-install", "remotion", "render", "src/index.ts"]
    );
    assert!(command.contains(&"--prores-profile=4444"));
    assert!(command.contains(&"--pixel-format=yuva444p10le"));
    assert!(!dir.join("args.txt").exists(), "npx did not run");
}

#[test]
fn projects_without_remotion_are_refused_before_anything_runs() {
    let dir = folder("refused");
    let npx = fake_npx(&dir, "render");
    let mut plain = request(&dir, npx.clone(), true);
    std::fs::write(
        plain.project.join("package.json"),
        r#"{"dependencies":{"react":"19"}}"#,
    )
    .unwrap();
    assert_eq!(
        web_code(&remotion::plan(&plain).unwrap_err()),
        "fosscut.web.not_remotion"
    );
    plain.project = dir.join("nowhere");
    assert_eq!(
        web_code(&remotion::plan(&plain).unwrap_err()),
        "fosscut.web.not_remotion"
    );

    let mut uninstalled = request(&dir, npx.clone(), true);
    std::fs::remove_dir_all(uninstalled.project.join("node_modules")).unwrap();
    assert_eq!(
        web_code(&remotion::plan(&uninstalled).unwrap_err()),
        "fosscut.web.remotion_unavailable"
    );
    uninstalled.npx = Some(dir.join("no-npx"));
    std::fs::create_dir_all(uninstalled.project.join("node_modules/@remotion/cli")).unwrap();
    assert_eq!(
        web_code(&remotion::plan(&uninstalled).unwrap_err()),
        "fosscut.web.remotion_unavailable"
    );

    let mut entry = request(&dir, npx, true);
    entry.entry = Some("src/missing.tsx".into());
    assert_eq!(
        web_code(&remotion::plan(&entry).unwrap_err()),
        "fosscut.web.not_remotion"
    );
    entry.entry = None;
    entry.composition = "--help".into();
    assert_eq!(
        web_code(&remotion::plan(&entry).unwrap_err()),
        "fosscut.request.invalid_arguments"
    );
}

#[test]
fn a_render_runs_the_projects_own_remotion_and_keeps_its_alpha() {
    if !ffmpeg() {
        return;
    }
    let dir = folder("render");
    let request = request(&dir, fake_npx(&dir, "render"), true);
    let plan = remotion::plan(&request).unwrap();
    assert_eq!(plan.output, request.bundle.join("renders").join("Main.mov"));
    let mut seen = Vec::new();
    let report = remotion::render(&plan, &AtomicBool::new(false), &mut |p| {
        seen.push(p);
        true
    })
    .unwrap();
    assert_eq!(report.output, plan.output.display().to_string());
    assert!(plan.output.is_file());
    assert_eq!(seen.last(), Some(&Progress { done: 3, total: 3 }));

    // The command ran in the project with the hidden output name and the props file.
    let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    assert_eq!(
        Path::new(args[0]).canonicalize().unwrap(),
        request.project.canonicalize().unwrap()
    );
    assert_eq!(
        args[1..6],
        ["--no-install", "remotion", "render", "src/index.ts", "Main"]
    );
    assert!(args[6].ends_with(".Main.mov.partial.mov"), "{}", args[6]);
    assert!(args.contains(&"--codec=prores"));
    assert!(args.iter().any(|a| a.starts_with("--props=")));
    let renders = std::fs::read_dir(request.bundle.join("renders"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        renders,
        vec!["Main.mov"],
        "no props file or partial output is left"
    );

    // FFmpeg decodes ProRes 4444 with its alpha as R'G'B'A.
    let mut decoder = FfmpegVideoDecoder::open(&plan.output, DecoderOptions::default()).unwrap();
    let frame = decoder.frame_at(Flicks::ZERO).unwrap();
    assert_eq!(frame.format, PixelFormat::Rgba);
    let pixel = &frame.planes[0].data[..4];
    assert!(
        pixel[0] > 240 && pixel[1] < 16 && pixel[2] < 16,
        "{pixel:?}"
    );
    assert!(
        (i32::from(pixel[3]) - 128).abs() <= 2,
        "half transparent: {pixel:?}"
    );

    // The next render of the same composition gets a file of its own.
    assert_eq!(
        remotion::plan(&request).unwrap().output,
        request.bundle.join("renders").join("Main-2.mov")
    );
}

#[test]
fn failures_report_what_remotion_printed() {
    let dir = folder("fail");
    let request = request(&dir, fake_npx(&dir, "fail"), true);
    let plan = remotion::plan(&request).unwrap();
    let error = remotion::render(&plan, &AtomicBool::new(false), &mut |_| true).unwrap_err();
    assert_eq!(error.code(), "fosscut.web.remotion_failed");
    assert!(
        error.details()["log"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().contains("No composition"))
    );
    assert!(!plan.output.exists());
}

#[test]
fn cancelling_stops_remotion_and_everything_it_started() {
    let dir = folder("cancel");
    let request = request(&dir, fake_npx(&dir, "hang"), true);
    let plan = remotion::plan(&request).unwrap();
    let started = std::time::Instant::now();
    let pid_file = dir.join("child.pid");
    // Cancel once the fake has started its own child.
    let error = remotion::render(&plan, &AtomicBool::new(false), &mut |_| {
        !pid_file.exists() && started.elapsed() < Duration::from_secs(20)
    })
    .unwrap_err();
    assert_eq!(error.code(), "fosscut.job.cancelled");
    assert!(started.elapsed() < Duration::from_secs(25));
    assert!(!plan.output.exists());
    // The fake's own child (a stand-in for Node's browser and FFmpeg) is gone too.
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    let alive = || {
        Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap()
            .success()
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while alive() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(), "the process group was stopped");
}

#[test]
fn a_job_renders_then_imports() {
    if !ffmpeg() {
        return;
    }
    let dir = folder("job");
    let request = request(&dir, fake_npx(&dir, "render"), true);
    let plan = remotion::plan(&request).unwrap();
    let jobs = Jobs::new();
    let imported = std::sync::Arc::new(std::sync::Mutex::new(None));
    let seen = std::sync::Arc::clone(&imported);
    let status = jobs
        .start_remotion(
            plan,
            Some(Box::new(move |file: &Path| {
                *seen.lock().unwrap() = Some(file.to_path_buf());
                Ok("asset_0007".to_string())
            })),
        )
        .unwrap();
    assert_eq!(status.kind, "remotion");
    let done = jobs.status(&status.job, Duration::from_secs(60)).unwrap();
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    let report = done.remotion.unwrap();
    assert_eq!(report.asset.as_deref(), Some("asset_0007"));
    assert_eq!(
        imported.lock().unwrap().as_deref(),
        Some(request.bundle.join("renders").join("Main.mov").as_path())
    );
}

#[test]
fn a_real_remotion_project_renders_when_asked() {
    if !required("FOSSCUT_REQUIRE_REMOTION") {
        return;
    }
    let project = std::env::var_os("FOSSCUT_REMOTION_PROJECT")
        .expect("FOSSCUT_REQUIRE_REMOTION needs FOSSCUT_REMOTION_PROJECT, a Remotion project");
    let composition =
        std::env::var("FOSSCUT_REMOTION_COMPOSITION").unwrap_or_else(|_| "Main".into());
    let dir = folder("real");
    let bundle = dir.join("Show.fosscut");
    std::fs::create_dir_all(&bundle).unwrap();
    let request = RemotionRequest {
        project: PathBuf::from(project),
        composition,
        props: None,
        entry: None,
        bundle,
        confirmed: true,
        npx: None,
    };
    let plan = remotion::plan(&request).unwrap();
    remotion::render(&plan, &AtomicBool::new(false), &mut |_| true).unwrap();
    let mut decoder = FfmpegVideoDecoder::open(&plan.output, DecoderOptions::default()).unwrap();
    assert_eq!(
        decoder.frame_at(Flicks::ZERO).unwrap().format,
        PixelFormat::Rgba
    );
}
