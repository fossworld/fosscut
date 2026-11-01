//! Vector paths (ADR-0036) from the command line: a route drawn from SVG path data with a
//! stroke and trims, drawn on by an animation, a dot travelling along it, an SVG drawing
//! imported, shown and exploded into shape clips, and frame grabs of the result.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::mcp::{call, json_of, start};
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

const LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">
  <circle cx="32" cy="32" r="24" fill="none" stroke="#FFC845" stroke-width="6"/>
  <path id="arrow" d="M 20 40 L 44 20 M 30 20 L 44 20 L 44 34" fill="none" stroke="#FF5C8A" stroke-width="5" stroke-linecap="round"/>
</svg>"##;

/// The clip `id` as `timeline read` shows it, from any timeline.
fn stored_clip(dir: &Path, id: &str) -> Value {
    let project = ok(dir, &["-p", "Map.fosscut", "query", "project.info"]);
    for timeline in project["timelines"].as_array().unwrap() {
        let read = ok(
            dir,
            &[
                "-p",
                "Map.fosscut",
                "query",
                "timeline.read",
                &format!(
                    r#"{{"timeline": "{}", "detail": "full"}}"#,
                    timeline["id"].as_str().unwrap()
                ),
            ],
        );
        for track in read["tracks"].as_array().unwrap() {
            for clip in track["clips"].as_array().unwrap() {
                if clip["id"] == id {
                    return clip.clone();
                }
            }
        }
    }
    panic!("no clip {id}");
}

#[test]
fn routes_draw_on_with_a_follower_and_drawings_explode() {
    let dir = workspace("vector");
    let p = ["-p", "Map.fosscut"];
    let with =
        |args: &[&str]| -> Vec<String> { p.iter().chain(args).map(|s| s.to_string()).collect() };
    let run = |args: &[&str]| {
        let all = with(args);
        let refs: Vec<&str> = all.iter().map(String::as_str).collect();
        ok(&dir, &refs)
    };
    ok(&dir, &["project", "new", "Map.fosscut"]);

    // A route across the canvas, hidden by its trim, drawn on over its first second.
    let route = run(&[
        "shape",
        "add",
        "--path-data",
        "M 160 860 C 560 160 1060 1000 1760 240",
        "--stroke",
        "#FFC845",
        "--stroke-width",
        "0.012",
        "--cap",
        "round",
        "--dash",
        "0.03,0.015",
        "--duration",
        "4s",
    ]);
    let route = route["results"][0]["clip"].as_str().unwrap().to_string();
    run(&[
        "animation",
        "set",
        "--clip",
        &route,
        "--in",
        "draw_on:1s",
        "--out",
        "draw_off:0.5s",
    ]);
    let stored = stored_clip(&dir, &route);
    let content = &stored["content"];
    assert_eq!(content["shape"], "path");
    assert_eq!(content["closed"], false);
    assert!(content["fill"].is_null(), "{content}");
    assert_eq!(content["stroke"]["cap"], "round");
    assert_eq!(content["stroke"]["dashes"][1], 0.015);
    run(&["shape", "set", "--clip", &route, "--trim", "0..1"]);

    // A dot that travels the route over its four seconds, turned with it.
    let dot = run(&[
        "shape",
        "add",
        "--shape",
        "ellipse",
        "--size",
        "0.02,0.035",
        "--fill",
        "#FF5C8A",
        "--duration",
        "4s",
    ]);
    let dot = dot["results"][0]["clip"].as_str().unwrap().to_string();
    let followed = run(&[
        "clip", "follow", "--clip", &dot, "--path", &route, "--travel", "0..1", "--orient", "true",
    ]);
    assert_eq!(followed["results"][0]["follow"]["clip"], route);
    let stored = stored_clip(&dir, &dot);
    assert_eq!(stored["follow"]["orient"], true);
    let keys = &stored["keyframes"]["follow.progress"];
    assert_eq!(keys.as_array().unwrap().len(), 2, "{stored}");
    assert_eq!(keys[1]["value"], 1.0);
    let error = fails(
        &dir,
        &[
            "-p",
            "Map.fosscut",
            "clip",
            "follow",
            "--clip",
            &route,
            "--path",
            &route,
        ],
    );
    assert_eq!(error["code"], "fosscut.follow.invalid");

    // An SVG drawing imports as a vector asset, shows as an image and explodes into shapes.
    std::fs::write(dir.join("logo.svg"), LOGO).unwrap();
    let imported = run(&["media", "import", "logo.svg"]);
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
        .unwrap()
        .clone();
    assert_eq!(found["kind"], "vector");
    assert_eq!(found["vector"]["paths"], 2);
    run(&[
        "clip",
        "add",
        "--asset",
        &asset,
        "--start",
        "4s",
        "--duration",
        "1s",
    ]);
    let exploded = run(&[
        "vector",
        "explode",
        "--asset",
        &asset,
        "--start",
        "5s",
        "--duration",
        "1s",
    ]);
    let result = &exploded["results"][0];
    assert_eq!(result["parts"].as_array().unwrap().len(), 2);
    assert_eq!(result["parts"][1]["name"], "arrow");
    let error = fails(
        &dir,
        &["-p", "Map.fosscut", "vector", "explode", "--asset", &dot],
    );
    assert_eq!(error["code"], "fosscut.asset.not_found");

    // Frame grabs: the route half drawn with the dot at its end, the drawing, its shapes.
    if can_render() {
        for (time, name) in [
            ("2s", "route.png"),
            ("4.5s", "logo.png"),
            ("5.5s", "shapes.png"),
        ] {
            run(&["frame", time, "--out", name, "--width", "480"]);
            let bytes = std::fs::read(dir.join(name)).unwrap();
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{name}");
        }
        let decode = |name: &str| {
            let decoder = png::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(dir.join(name)).unwrap(),
            ));
            let mut reader = decoder.read_info().unwrap();
            let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut rgba).unwrap();
            rgba
        };
        // The drawing and its shapes look alike.
        let (logo, shapes) = (decode("logo.png"), decode("shapes.png"));
        let difference: u64 = logo
            .iter()
            .zip(&shapes)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        let mean = difference as f64 / logo.len() as f64;
        assert!(mean < 3.0, "mean difference {mean}");
        assert!(logo.iter().any(|v| *v > 200), "the drawing shows");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// An agent through MCP alone: an SVG file through media_import, then edit with
/// vector.explode given only the asset, and a line from SVG path data that a star follows.
#[tokio::test(flavor = "multi_thread")]
async fn agents_explode_drawings_and_follow_paths_through_edit() {
    let dir = workspace("vector-mcp");
    std::fs::write(dir.join("logo.svg"), LOGO).unwrap();
    let client = start(&dir, "vector-agent").await;
    let ok = |result: rmcp::model::CallToolResult| {
        assert_ne!(result.is_error, Some(true), "{}", json_of(&result));
        json_of(&result)
    };
    ok(call(&client, "project_new", json!({ "path": "Logo.fosscut" })).await);
    let imported = ok(call(&client, "media_import", json!({ "files": ["logo.svg"] })).await);
    let asset = imported["results"][0]["asset"].clone();
    let edited = ok(call(
        &client,
        "edit",
        json!({
            "commands": [
                { "command": "vector.explode", "args": { "asset": asset, "duration": "2s" } },
                { "command": "shape.add", "args": {
                    "d": "M 200 900 Q 960 100 1720 900", "duration": "2s",
                    "stroke": { "color": "#FFFFFF", "width": 0.01, "dashes": [0.02, 0.01] }
                } },
            ],
            "expected_revision": imported["revision"],
        }),
    )
    .await);
    let parts = edited["results"][0]["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 2, "{edited}");
    let line = edited["results"][1]["clip"].clone();
    let star = ok(call(
        &client,
        "edit",
        json!({ "commands": [{ "command": "shape.add", "args": {
            "shape": "star", "size": [0.04, 0.07], "start": "0s", "duration": "2s"
        } }] }),
    )
    .await);
    let star = star["results"][0]["clip"].clone();
    let followed = ok(call(
        &client,
        "edit",
        json!({ "commands": [
            { "command": "clip.follow", "args": { "clip": star, "path": line, "progress": 0.5, "orient": true } },
        ] }),
    )
    .await);
    // Halfway along the symmetric curve: the middle of the canvas, at its top, level.
    let position = &followed["results"][0]["position"];
    assert!(position[0].as_f64().unwrap().abs() < 1e-6, "{followed}");
    assert!((followed["results"][0]["rotation"].as_f64().unwrap()).abs() < 0.01);
    let capabilities = ok(call(
        &client,
        "capabilities",
        json!({ "names": ["clip.follow", "vector.explode"] }),
    )
    .await);
    assert!(capabilities.to_string().contains("vector.explode"));
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}
