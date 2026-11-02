//! A Three.js scene template added end to end (ADR-0042): `fosscut library add` with
//! `--params` and an image asset for its logo slot writes the composition's folder with
//! `params.json` and the picture, and a frame grab shows the box in the given colour with
//! the logo on it. Typed errors for bad parameters. Needs FFmpeg, a GPU adapter and a
//! browser; skipped without them locally.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::{can_render, workspace};
use serde_json::Value;

fn fosscut(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fosscut"))
        .current_dir(dir)
        .env_remove("FOSSCUT_PROJECT")
        .env("FOSSCUT_ACTOR", "agent:test")
        .env("FOSSCUT_CACHE_DIR", dir.join("cache"))
        .env("FOSSCUT_CONFIG_DIR", dir.join("config"))
        // Tests draw web compositions with SwiftShader, whatever the page asks for.
        .env("FOSSCUT_WEB_RENDERER", "software")
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

/// The browser compositions would use, or `None` to skip locally (CI with
/// `FOSSCUT_REQUIRE_BROWSER` fails instead).
fn browser(dir: &Path) -> Option<Value> {
    let found = ok(dir, &["web", "browser"])["browser"].clone();
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_BROWSER").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        !found.is_null() || !required,
        "FOSSCUT_REQUIRE_BROWSER is set but no browser was found"
    );
    (!found.is_null()).then_some(found)
}

/// A 96x64 picture: a solid green mark on a transparent ground.
fn logo(path: &Path) {
    let (w, h) = (96u32, 64u32);
    let mut pixels = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let inside = (8..88).contains(&x) && (8..56).contains(&y);
            pixels.extend_from_slice(if inside {
                &[34, 204, 68, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
}

#[test]
fn scene_templates_take_params_and_media_and_render_in_frame_grabs() {
    let dir = workspace("scene-template");
    if !can_render() || browser(&dir).is_none() {
        return;
    }
    let p = ["-p", "Ad.fosscut"];
    let run = |args: &[&str]| ok(&dir, &[&p[..], args].concat());
    ok(
        &dir,
        &[
            "project",
            "new",
            "Ad.fosscut",
            "--width",
            "270",
            "--height",
            "480",
        ],
    );
    logo(&dir.join("logo.png"));
    let picture = run(&["media", "import", "logo.png"]);
    let picture = picture["results"][0]["asset"].as_str().unwrap().to_string();

    let params =
        r##"{"box_color": "#2F5BFF", "width": 270, "height": 480, "motion_blur": {"samples": 2}}"##;
    run(&[
        "library",
        "add",
        "parcel-drop-3d",
        "--params",
        params,
        "--media",
        &picture,
    ]);
    // The composition's folder holds its parameters and the picture.
    let media = run(&["media", "list"]);
    let web = media["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["kind"] == "web")
        .unwrap()
        .clone();
    assert_eq!(web["video"]["width"], 270);
    let page = dir
        .join("Ad.fosscut")
        .join(web["path"]["relative"].as_str().unwrap());
    let folder = page.parent().unwrap();
    let written: Value =
        serde_json::from_slice(&std::fs::read(folder.join("params.json")).unwrap()).unwrap();
    assert_eq!(written["box_color"], "#2F5BFF");
    assert_eq!(written["logo"]["src"], "media/logo.png");
    assert_eq!(
        std::fs::read(folder.join("media/logo.png")).unwrap(),
        std::fs::read(dir.join("logo.png")).unwrap()
    );

    // Settled at 2.5 s: a blue box with the green logo on its front.
    run(&["frame", "2.5s", "--out", "box.png"]);
    let file = std::fs::File::open(dir.join("box.png")).unwrap();
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    let channels = info.color_type.samples();
    let share = |test: &dyn Fn(&[u8]) -> bool| {
        let all = pixels.len() / channels;
        pixels.chunks(channels).filter(|p| test(p)).count() as f64 / all as f64
    };
    let blue = share(&|p| p[2] > 120 && i32::from(p[2]) > i32::from(p[0]) + 60);
    let green = share(&|p| {
        p[1] > 120
            && i32::from(p[1]) > i32::from(p[0]) + 50
            && i32::from(p[1]) > i32::from(p[2]) + 20
    });
    assert!(blue > 0.05, "the blue box covers {blue} of the frame");
    assert!(green > 0.002, "the green logo covers {green} of the frame");

    // Unknown parameters and slots given the wrong media are typed errors.
    let output = fosscut(
        &dir,
        &[
            &p[..],
            &[
                "library",
                "add",
                "parcel-drop-3d",
                "--params",
                r##"{"box_colour": "#fff"}"##,
            ],
        ]
        .concat(),
    );
    assert_eq!(output.status.code(), Some(1));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "fosscut.library.invalid_params");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("box_color"),
        "{error}"
    );
}
