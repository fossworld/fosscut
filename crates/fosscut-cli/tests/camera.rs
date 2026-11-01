//! End to end from the `fosscut` binary (ADR-0037): clips placed in depth with `clip set`,
//! a camera clip that pushes in with `camera add` and `camera set`, and a frame grab that
//! shows each change.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::{can_render, workspace};
use serde_json::{Value, json};

fn fosscut(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fosscut"))
        .current_dir(dir)
        .env_remove("FOSSCUT_PROJECT")
        .env("FOSSCUT_ACTOR", "agent:test")
        .env("FOSSCUT_CACHE_DIR", dir.join("cache"))
        .env("FOSSCUT_CONFIG_DIR", dir.join("config"))
        .env_remove("FOSSCUT_MODELS_DIR")
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
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    value["error"].clone()
}

/// How many columns of a PNG hold white pixels: the width of the white square.
fn white_width(path: &Path) -> usize {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let samples = info.color_type.samples();
    let (width, height) = (info.width as usize, info.height as usize);
    (0..width)
        .filter(|x| {
            (0..height).any(|y| {
                let i = (y * width + x) * samples;
                pixels[i..i + 3].iter().all(|c| *c > 200)
            })
        })
        .count()
}

#[test]
fn clips_in_depth_and_a_camera_from_the_command_line() {
    if !can_render() {
        return;
    }
    let dir = workspace("camera");
    // A white square in the middle of a dark frame.
    let status = Command::new("ffmpeg")
        .current_dir(&dir)
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg("color=c=0x203050:s=640x360:r=30:d=3,drawbox=x=280:y=140:w=80:h=80:color=white:t=fill")
        .args(["-c:v", "mpeg4", "-q:v", "2", "square.mp4"])
        .status()
        .unwrap();
    assert!(status.success());
    let new = [
        "project",
        "new",
        "Depth.fosscut",
        "--width",
        "640",
        "--height",
        "360",
    ];
    ok(&dir, &new);
    ok(
        &dir,
        &["-p", "Depth.fosscut", "media", "import", "square.mp4"],
    );
    let placed = ok(
        &dir,
        &[
            "-p",
            "Depth.fosscut",
            "clip",
            "add",
            "--asset",
            "asset_0001",
        ],
    );
    let clip = placed["results"][0]["clip"].as_str().unwrap().to_string();
    let grab = |name: &str| {
        ok(
            &dir,
            &[
                "-p",
                "Depth.fosscut",
                "frame",
                "1s",
                "--out",
                name,
                "--width",
                "320",
            ],
        );
        white_width(&dir.join(name))
    };
    let flat = grab("flat.png");
    assert!(
        (38..=42).contains(&flat),
        "80 of 640 pixels at half size: {flat}"
    );

    // Half a canvas width deeper, it looks smaller: by the framing distance over the new
    // distance, 1 / (1 + 0.5 tan 20°) of its width.
    ok(
        &dir,
        &[
            "-p",
            "Depth.fosscut",
            "clip",
            "set",
            "--clip",
            &clip,
            "--three-d",
            "true",
            "--position-z",
            "0.5",
        ],
    );
    let deep = grab("deep.png");
    let expected = flat as f64 / (1.0 + 0.5 * 2.0 * 20f64.to_radians().tan());
    assert!(
        (deep as f64 - expected).abs() <= 2.0,
        "{deep} vs {expected:.1}"
    );

    // A camera pushed in by as much brings it back to its size.
    let camera = ok(
        &dir,
        &[
            "-p",
            "Depth.fosscut",
            "camera",
            "add",
            "--position-z",
            "0.5",
        ],
    );
    let camera = camera["results"][0]["clip"].as_str().unwrap().to_string();
    let pushed = grab("pushed.png");
    assert!(pushed.abs_diff(flat) <= 1, "{pushed} vs {flat}");
    ok(
        &dir,
        &[
            "-p",
            "Depth.fosscut",
            "camera",
            "set",
            "--clip",
            &camera,
            "--position-z",
            "0.25",
            "--set",
            "depth_of_field=true",
        ],
    );
    let between = grab("between.png");
    assert!(
        deep < between && between < flat,
        "{deep} < {between} < {flat}"
    );
    let view = ok(
        &dir,
        &[
            "-p",
            "Depth.fosscut",
            "timeline",
            "read",
            "--detail",
            "full",
        ],
    );
    let tracks = view["tracks"].as_array().unwrap();
    let shown = tracks.iter().find(|t| t["name"] == "Camera").unwrap();
    assert_eq!(shown["clips"][0]["kind"], "camera");
    assert_eq!(
        shown["clips"][0]["camera"],
        json!({ "position_z": 0.25, "depth_of_field": true })
    );

    // A 2D clip ignores the camera, and the camera takes only camera settings.
    ok(
        &dir,
        &[
            "-p",
            "Depth.fosscut",
            "clip",
            "set",
            "--clip",
            &clip,
            "--three-d",
            "false",
        ],
    );
    assert_eq!(grab("plain.png"), deep, "seen through the default camera");
    let wrong = fails(
        &dir,
        &[
            "-p",
            "Depth.fosscut",
            "camera",
            "set",
            "--clip",
            &clip,
            "--roll",
            "5",
        ],
    );
    assert_eq!(wrong["code"], "fosscut.camera.not_camera");
    let wide = fails(
        &dir,
        &[
            "-p",
            "Depth.fosscut",
            "camera",
            "set",
            "--clip",
            &camera,
            "--fov",
            "200",
        ],
    );
    assert_eq!(wide["code"], "fosscut.camera.invalid");
    // 3D animation presets.
    let animated = ok(
        &dir,
        &[
            "-p",
            "Depth.fosscut",
            "animation",
            "set",
            "--clip",
            &clip,
            "--in",
            "flip_in",
        ],
    );
    assert_eq!(
        animated["results"][0]["animations"]["in"]["preset"],
        "flip_in"
    );
}
