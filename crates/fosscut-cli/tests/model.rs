//! 3D objects (ADR-0040) from the command line: presets listed, a glTF model imported with
//! `media import` and placed with `model add`, extruded text, changes and keyframes, the
//! library, typed errors, frame grabs that agree whichever process renders them, and an
//! export.

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
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"].clone()
}

#[test]
fn objects_from_the_command_line() {
    let dir = workspace("model");
    std::fs::write(
        dir.join("house.glb"),
        fosscut_render::objects::fixture_glb(None),
    )
    .unwrap();
    ok(
        &dir,
        &[
            "project",
            "new",
            "Objects.fosscut",
            "--width",
            "640",
            "--height",
            "360",
        ],
    );
    let p = ["-p", "Objects.fosscut"];
    let run = |args: &[&str]| ok(&dir, &[&p[..], args].concat());

    let presets = run(&["model", "presets"]);
    let names: Vec<&str> = presets["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    for wanted in ["shipping-box", "city-blocks", "podium", "extruded-title"] {
        assert!(names.contains(&wanted), "{names:?}");
    }

    // A glTF model imports as a model asset, copied into the bundle.
    let imported = run(&["media", "import", "house.glb"]);
    let asset = imported["results"][0]["asset"]
        .as_str()
        .unwrap()
        .to_string();
    let listed = run(&["media", "list"]);
    let found = listed["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == asset.as_str())
        .unwrap();
    assert_eq!(found["kind"], "model");
    let copies = std::fs::read_dir(dir.join("Objects.fosscut").join("models"))
        .unwrap()
        .count();
    assert_eq!(copies, 1);

    let house = run(&[
        "model",
        "add",
        "--asset",
        &asset,
        "--duration",
        "1s",
        "--set",
        r##"{"object": {"size": 0.35}, "floor": {"catcher": false, "color": "#3A3F4A"}}"##,
        "--transform",
        r#"{"rotation_y": 30, "position_z": 0.1}"#,
    ]);
    let clip = house["results"][0]["clip"].as_str().unwrap().to_string();
    let title = run(&[
        "model",
        "add",
        "extruded-title",
        "--text",
        "SALE",
        "--at",
        "0s",
        "--duration",
        "1s",
    ]);
    assert_ne!(title["results"][0]["track"], house["results"][0]["track"]);
    let changed = run(&[
        "model",
        "set",
        "--clip",
        &clip,
        "--set",
        r#"{"light": {"azimuth": 40}}"#,
    ]);
    let content = &changed["results"][0]["content"];
    assert_eq!(content["light"]["azimuth"], json!(40.0));
    assert_eq!(content["object"]["kind"], json!("asset"));
    run(&[
        "keyframe",
        "set",
        "--clip",
        &clip,
        "--property",
        "model.light.intensity",
        "--offset",
        "0.5s",
        "--value",
        "2",
    ]);

    let nothing = fails(&dir, &[&p[..], &["model", "set", "--clip", &clip]].concat());
    assert_eq!(nothing["code"], json!("fosscut.request.invalid_arguments"));
    let bad = fails(
        &dir,
        &[
            &p[..],
            &[
                "model",
                "set",
                "--clip",
                &clip,
                "--set",
                r#"{"light": {"softness": 4}}"#,
            ],
        ]
        .concat(),
    );
    assert_eq!(bad["code"], json!("fosscut.object.invalid"));
    let clip_add = fails(
        &dir,
        &[&p[..], &["clip", "add", "--asset", &asset]].concat(),
    );
    assert_eq!(clip_add["code"], json!("fosscut.request.invalid_arguments"));

    // Through the library: kind model, added with model.add.
    let listed = run(&["library", "list", "--kind", "model"]);
    assert!(listed["items"].as_array().unwrap().len() >= 4);
    run(&["library", "add", "podium", "--at", "1s", "--duration", "1s"]);
    let doc = run(&["timeline", "read", "--detail", "full"]);
    let objects: Vec<&Value> = doc["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap())
        .filter(|c| c["kind"] == "model")
        .collect();
    assert_eq!(objects.len(), 3);
    assert!(
        objects
            .iter()
            .any(|o| o["object"] == "text" && o["text"] == "SALE")
    );

    if !can_render() {
        return;
    }
    // Frame grabs in separate processes agree to the byte.
    run(&["frame", "0.5s", "--out", "a.png", "--width", "320"]);
    run(&["frame", "1.5s", "--out", "b.png", "--width", "320"]);
    run(&["frame", "0.5s", "--out", "c.png", "--width", "320"]);
    let (a, c) = (
        std::fs::read(dir.join("a.png")).unwrap(),
        std::fs::read(dir.join("c.png")).unwrap(),
    );
    assert_eq!(a, c);
    assert_ne!(a, std::fs::read(dir.join("b.png")).unwrap());
    // The export loads the model from the bundle too.
    let exported = run(&["export", "Objects.mp4"]);
    assert!(dir.join("Objects.mp4").is_file(), "{exported}");
}
