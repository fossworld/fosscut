//! Particle emitters (ADR-0038) from the command line: presets listed, added and changed
//! with times in units, animated with keyframes, reached through the library, and frame
//! grabs that agree exactly whichever process renders them.

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
fn particles_from_the_command_line() {
    let dir = workspace("particles");
    ok(
        &dir,
        &[
            "project",
            "new",
            "Sparks.fosscut",
            "--width",
            "640",
            "--height",
            "360",
        ],
    );
    let p = ["-p", "Sparks.fosscut"];
    let run = |args: &[&str]| ok(&dir, &[&p[..], args].concat());

    let presets = run(&["particles", "presets"]);
    let names: Vec<&str> = presets["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    for wanted in ["sparks", "embers", "snow", "confetti", "burst"] {
        assert!(names.contains(&wanted), "{names:?}");
    }

    let added = run(&[
        "particles",
        "add",
        "embers",
        "--at",
        "0s",
        "--duration",
        "2s",
        "--set",
        r#"{"rate": 40, "sprite": {"kind": "star"}}"#,
        "--burst",
        "100@0.5s",
        "--prewarm",
        "1s",
        "--seed",
        "4",
    ]);
    let clip = added["results"][0]["clip"].as_str().unwrap().to_string();
    let changed = run(&[
        "particles",
        "set",
        "--clip",
        &clip,
        "--set",
        r#"{"gravity": 0.3}"#,
    ]);
    let content = &changed["results"][0]["content"];
    assert_eq!(content["gravity"], json!(0.3));
    assert_eq!(content["rate"], json!(40.0));
    assert_eq!(content["seed"], json!(4));
    assert_eq!(content["sprite"]["kind"], json!("star"));
    assert_eq!(content["bursts"][0]["count"], json!(100));
    assert_eq!(
        content["bursts"][0]["at"],
        json!(352_800_000),
        "0.5 s in flicks"
    );

    let nothing = fails(
        &dir,
        &[&p[..], &["particles", "set", "--clip", &clip]].concat(),
    );
    assert_eq!(nothing["code"], json!("fosscut.request.invalid_arguments"));
    let bad = fails(
        &dir,
        &[
            &p[..],
            &[
                "particles",
                "set",
                "--clip",
                &clip,
                "--set",
                r#"{"rate": -1}"#,
            ],
        ]
        .concat(),
    );
    assert_eq!(bad["code"], json!("fosscut.particles.invalid"));
    // A malformed burst is a usage error before anything runs.
    let burst = fosscut(
        &dir,
        &[&p[..], &["particles", "add", "--burst", "lots@1s"]].concat(),
    );
    assert_eq!(burst.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&burst.stderr).contains("whole number"));

    run(&[
        "keyframe",
        "set",
        "--clip",
        &clip,
        "--property",
        "particles.emitter.position",
        "--offset",
        "1s",
        "--value=-0.3,0.2",
    ]);
    run(&["particles", "set", "--clip", &clip, "--no-bursts"]);

    // Through the library: kind particles, added with particles.add.
    let listed = run(&["library", "list", "--kind", "particles"]);
    assert!(listed["items"].as_array().unwrap().len() >= 8);
    let confetti = run(&[
        "library",
        "add",
        "confetti",
        "--at",
        "2s",
        "--duration",
        "1s",
    ]);
    assert_eq!(confetti["results"][0]["start"]["seconds"], json!("2"));
    let doc = run(&["timeline", "read", "--detail", "full"]);
    let particles: Vec<&Value> = doc["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap())
        .filter(|c| c["kind"] == "particles")
        .collect();
    assert_eq!(particles.len(), 2);
    assert!(
        particles[0]["content"]["bursts"].is_null(),
        "no bursts left"
    );

    if !can_render() {
        return;
    }
    // Frame grabs in separate processes agree to the byte.
    run(&["frame", "1s", "--out", "a.png", "--width", "320"]);
    run(&["frame", "0.4s", "--out", "b.png", "--width", "320"]);
    run(&["frame", "1s", "--out", "c.png", "--width", "320"]);
    let (a, c) = (
        std::fs::read(dir.join("a.png")).unwrap(),
        std::fs::read(dir.join("c.png")).unwrap(),
    );
    assert_eq!(a, c);
    assert_ne!(a, std::fs::read(dir.join("b.png")).unwrap());
}
