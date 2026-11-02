//! Web compositions and Remotion renders end to end (ADR-0039): `fosscut web import` and
//! `media import` of HTML, typed errors, frame grabs of a web clip, `fosscut web remotion`
//! with a fake `npx` first on the path, and the same through MCP's `media_import`.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::mcp::{NamedClient, call, json_of};
use common::{can_render, has_ffmpeg, workspace};
use rmcp::ServiceExt;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::{Value, json};

/// The path with `dir/bin` (where a fake `npx` lives) first.
fn path_with_bin(dir: &Path) -> std::ffi::OsString {
    let mut paths = vec![dir.join("bin")];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).unwrap()
}

fn fosscut(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fosscut"))
        .current_dir(dir)
        .env_remove("FOSSCUT_PROJECT")
        .env_remove("FOSSCUT_NPX")
        .env("PATH", path_with_bin(dir))
        .env("FOSSCUT_ACTOR", "agent:test")
        .env("FOSSCUT_CACHE_DIR", dir.join("cache"))
        .env("FOSSCUT_CONFIG_DIR", dir.join("config"))
        .args(args)
        .arg("--json")
        .output()
        .unwrap()
}

fn ok(dir: &Path, args: &[&str]) -> Value {
    let output = fosscut(dir, args);
    assert!(
        output.status.success(),
        "{args:?} failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fails(dir: &Path, args: &[&str]) -> Value {
    let output = fosscut(dir, args);
    assert_eq!(output.status.code(), Some(1), "{args:?} should fail");
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"].clone()
}

/// A composition folder `name` in `dir`: a 320x180 page with a red square that slides right
/// over 2 s on a transparent background, described by its meta tag unless `tagged` is false.
fn composition(dir: &Path, name: &str, tagged: bool) -> PathBuf {
    let folder = dir.join(name);
    std::fs::create_dir_all(folder.join("css")).unwrap();
    let tag = if tagged {
        r#"<meta name="fosscut:composition" content="width=320, height=180, fps=30, duration=2">"#
    } else {
        ""
    };
    std::fs::write(
        folder.join("index.html"),
        format!(
            r#"<!doctype html><html><head>{tag}<link rel="stylesheet" href="css/style.css"></head>
            <body><div id="sq"></div></body></html>"#
        ),
    )
    .unwrap();
    std::fs::write(
        folder.join("css/style.css"),
        "html,body{margin:0} #sq{position:absolute;top:70px;left:0;width:40px;height:40px;background:#f00;animation:go 2s linear forwards} @keyframes go{to{left:160px}}",
    )
    .unwrap();
    folder
}

fn new_project(dir: &Path) {
    ok(
        dir,
        &[
            "project",
            "new",
            "Ad.fosscut",
            "--width",
            "320",
            "--height",
            "180",
        ],
    );
}

#[test]
fn web_compositions_import_from_the_command_line() {
    let dir = workspace("web-import");
    new_project(&dir);
    let p = ["-p", "Ad.fosscut"];
    let run = |args: &[&str]| ok(&dir, &[&p[..], args].concat());
    composition(&dir, "square", true);
    let added = run(&["web", "import", "square"]);
    assert_eq!(added["results"][0]["asset"], "asset_0001");
    let media = run(&["media", "list"]);
    let asset = &media["assets"][0];
    assert_eq!(asset["kind"], "web");
    assert_eq!(asset["video"]["width"], 320);
    let relative = asset["path"]["relative"].as_str().unwrap();
    assert!(
        dir.join("Ad.fosscut").join(relative).is_file(),
        "{relative}"
    );
    assert!(
        dir.join("Ad.fosscut")
            .join(relative)
            .parent()
            .unwrap()
            .join("css/style.css")
            .is_file()
    );
    // `media import` takes compositions too, and the same content is the same asset.
    let again = run(&["media", "import", "square"]);
    assert_eq!(again["results"][0]["existing"], true);

    // Without a meta tag, values come from the options; missing ones are named.
    composition(&dir, "untagged", false);
    let error = fails(
        &dir,
        &[
            "-p",
            "Ad.fosscut",
            "web",
            "import",
            "untagged",
            "--width",
            "100",
        ],
    );
    assert_eq!(error["code"], "fosscut.web.composition_unknown");
    assert_eq!(error["details"]["missing"], json!(["height", "duration"]));
    let sized = run(&[
        "web",
        "import",
        "untagged",
        "--width",
        "160",
        "--height",
        "90",
        "--fps",
        "25",
        "--duration",
        "50f",
        "--name",
        "Lower third",
    ]);
    let id = sized["results"][0]["asset"].as_str().unwrap().to_string();
    let media = run(&["media", "list"]);
    let lower = media["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id.as_str())
        .unwrap();
    assert_eq!(lower["name"], "Lower third");
    assert_eq!(lower["video"]["rate"], "25");
    assert_eq!(lower["duration"]["seconds"], "2");

    std::fs::write(
        dir.join("cdn.html"),
        r#"<script src="https://cdn.example.com/gsap.js"></script>"#,
    )
    .unwrap();
    let error = fails(
        &dir,
        &[
            "-p",
            "Ad.fosscut",
            "web",
            "import",
            "cdn.html",
            "--width",
            "10",
            "--height",
            "10",
            "--duration",
            "1s",
        ],
    );
    assert_eq!(error["code"], "fosscut.web.remote_reference");
    assert!(error["hint"].as_str().unwrap().contains("relative paths"));
    let error = fails(&dir, &["-p", "Ad.fosscut", "web", "import", "nowhere"]);
    assert_eq!(error["code"], "fosscut.web.not_composition");

    let capabilities = ok(&dir, &["capabilities"]);
    let names: Vec<&str> = capabilities["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["name"].as_str())
        .collect();
    assert!(names.contains(&"web.import") && names.contains(&"web.remotion"));
    assert_eq!(capabilities["web"]["meta_tag"], "fosscut:composition");
}

#[test]
fn web_clips_render_in_frame_grabs() {
    if !can_render() || fosscut_engine_browser().is_none() {
        return;
    }
    let dir = workspace("web-frame");
    new_project(&dir);
    composition(&dir, "square", true);
    let p = ["-p", "Ad.fosscut"];
    let run = |args: &[&str]| ok(&dir, &[&p[..], args].concat());
    run(&["web", "import", "square"]);
    run(&["clip", "add", "--asset", "asset_0001"]);
    run(&["frame", "1s", "--out", "one.png"]);
    let file = std::fs::File::open(dir.join("one.png")).unwrap();
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let at = |x: u32, y: u32| {
        let i = ((y * info.width + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    // 1 s in, the square spans x 80 to 120; the transparent page shows black below it.
    assert!(
        at(100, 90)[0] > 240 && at(100, 90)[1] < 20,
        "{:?}",
        at(100, 90)
    );
    assert_eq!(at(20, 90), [0, 0, 0]);
}

/// The browser web compositions would use, or `None` to skip locally (CI with
/// `FOSSCUT_REQUIRE_BROWSER` fails instead).
fn fosscut_engine_browser() -> Option<Value> {
    let dir = workspace("web-browser");
    let found = ok(&dir, &["web", "browser"])["browser"].clone();
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_BROWSER").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        !found.is_null() || !required,
        "FOSSCUT_REQUIRE_BROWSER is set but no browser was found"
    );
    (!found.is_null()).then_some(found)
}

/// A Remotion project as `npm install` leaves it, and a fake `npx` in `dir/bin` that writes
/// a half-transparent ProRes 4444 clip to its output argument.
fn remotion_project(dir: &Path) -> PathBuf {
    let project = dir.join("my-video");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(project.join("node_modules/@remotion/cli")).unwrap();
    std::fs::write(
        project.join("package.json"),
        r#"{"devDependencies":{"@remotion/cli":"4.0.0"},"dependencies":{"remotion":"4.0.0"}}"#,
    )
    .unwrap();
    std::fs::write(project.join("src/index.ts"), "").unwrap();
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let npx = dir.join("bin/npx");
    std::fs::write(
        &npx,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\necho 'Rendered 3/3'\nffmpeg -v error -y -f lavfi -i 'color=c=blue@0.5:s=64x36:r=30:d=0.2,format=yuva444p10le' -c:v prores_ks -profile:v 4444 -pix_fmt yuva444p10le -f mov \"$6\"\n",
            dir.join("npx-args.txt").display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&npx, std::fs::Permissions::from_mode(0o755)).unwrap();
    project
}

#[test]
fn remotion_renders_run_only_when_confirmed_and_are_imported() {
    if !has_ffmpeg() {
        return;
    }
    let dir = workspace("web-remotion");
    new_project(&dir);
    remotion_project(&dir);
    let error = fails(
        &dir,
        &["-p", "Ad.fosscut", "web", "remotion", "my-video", "Promo"],
    );
    assert_eq!(error["code"], "fosscut.web.confirmation_required");
    assert!(error["message"].as_str().unwrap().contains("licence"));
    assert!(!dir.join("npx-args.txt").exists(), "nothing ran");

    let dry = ok(
        &dir,
        &[
            "-p",
            "Ad.fosscut",
            "web",
            "remotion",
            "my-video",
            "Promo",
            "--yes",
            "--dry-run",
        ],
    );
    assert_eq!(dry["dry_run"], true);
    assert!(!dir.join("npx-args.txt").exists(), "a dry run runs nothing");

    let done = ok(
        &dir,
        &[
            "-p",
            "Ad.fosscut",
            "web",
            "remotion",
            "my-video",
            "Promo",
            "--yes",
            "--props",
            r#"{"title":"Sale"}"#,
        ],
    );
    assert_eq!(done["remotion"]["asset"], "asset_0001");
    assert!(
        done["remotion"]["output"]
            .as_str()
            .unwrap()
            .ends_with("renders/Promo.mov")
    );
    let args = std::fs::read_to_string(dir.join("npx-args.txt")).unwrap();
    assert!(
        args.starts_with("--no-install\nremotion\nrender\nsrc/index.ts\nPromo\n"),
        "{args}"
    );
    let media = ok(&dir, &["-p", "Ad.fosscut", "media", "list"]);
    assert_eq!(media["assets"][0]["kind"], "video");
    assert_eq!(media["assets"][0]["path"]["relative"], "renders/Promo.mov");

    let error = fails(
        &dir,
        &[
            "-p",
            "Ad.fosscut",
            "web",
            "remotion",
            "Ad.fosscut",
            "Promo",
            "--yes",
        ],
    );
    assert_eq!(error["code"], "fosscut.web.not_remotion");
}

#[tokio::test(flavor = "multi_thread")]
async fn media_import_takes_compositions_and_remotion_renders_over_mcp() {
    if !has_ffmpeg() {
        return;
    }
    let dir = workspace("web-mcp");
    new_project(&dir);
    composition(&dir, "untagged", false);
    remotion_project(&dir);
    let path = path_with_bin(&dir);
    let command = tokio::process::Command::new(env!("CARGO_BIN_EXE_fosscut")).configure(|cmd| {
        cmd.arg("mcp")
            .arg("--project")
            .arg(dir.join("Ad.fosscut"))
            .current_dir(&dir)
            .env_remove("FOSSCUT_PROJECT")
            .env_remove("FOSSCUT_NPX")
            .env("PATH", &path)
            .env("FOSSCUT_CACHE_DIR", dir.join("cache"))
            .env("FOSSCUT_CONFIG_DIR", dir.join("config"));
    });
    let client = NamedClient("web-test")
        .serve(TokioChildProcess::new(command).unwrap())
        .await
        .unwrap();

    let missing = call(&client, "media_import", json!({ "files": ["untagged"] })).await;
    assert_eq!(missing.is_error, Some(true));
    assert_eq!(
        json_of(&missing)["error"]["code"],
        "fosscut.web.composition_unknown"
    );
    let imported = call(
        &client,
        "media_import",
        json!({
            "files": ["untagged"],
            "composition": { "width": 320, "height": 180, "duration": { "seconds": 2 } },
        }),
    )
    .await;
    assert_eq!(imported.is_error, Some(false), "{:?}", json_of(&imported));
    assert_eq!(json_of(&imported)["results"][0]["asset"], "asset_0001");

    let unconfirmed = call(
        &client,
        "media_import",
        json!({ "files": ["my-video"], "remotion": { "composition": "Promo" } }),
    )
    .await;
    assert_eq!(unconfirmed.is_error, Some(true));
    let error = json_of(&unconfirmed)["error"].clone();
    assert_eq!(error["code"], "fosscut.web.confirmation_required");
    assert!(
        error["details"]["licence"]
            .as_str()
            .unwrap()
            .contains("Remotion")
    );

    let started = call(
        &client,
        "media_import",
        json!({
            "files": ["my-video"],
            "remotion": { "composition": "Promo", "confirm": true },
        }),
    )
    .await;
    let job = json_of(&started)["job"].as_str().unwrap().to_string();
    assert_eq!(json_of(&started)["kind"], "remotion");
    let status = json_of(
        &call(
            &client,
            "job_status",
            json!({ "job": job, "wait_seconds": 60 }),
        )
        .await,
    );
    assert_eq!(status["state"], "succeeded", "{status}");
    assert_eq!(status["remotion"]["asset"], "asset_0002");
    let media = json_of(&call(&client, "media_list", json!({})).await);
    assert_eq!(media["assets"][1]["path"]["relative"], "renders/Promo.mov");
    client.cancel().await.unwrap();
}
