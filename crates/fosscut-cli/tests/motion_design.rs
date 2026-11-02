//! Motion design from the command line (ADR-0011, ADR-0027): light effects added and
//! keyframed, text reveals with the new motions and their timing, clip animations with a
//! delay and staggered over several titles, all read back and rendered the same in every
//! process.

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

const SECOND: i64 = 705_600_000;

#[test]
fn light_reveals_and_staggers_from_the_command_line() {
    let dir = workspace("motion-design");
    ok(
        &dir,
        &[
            "project",
            "new",
            "Ad.fosscut",
            "--width",
            "640",
            "--height",
            "360",
        ],
    );
    let p = ["-p", "Ad.fosscut"];
    let run = |args: &[&str]| ok(&dir, &[&p[..], args].concat());

    // A title whose words come into focus from the middle, leaving with a drop.
    let title = run(&[
        "text",
        "add",
        "Everything you love.",
        "--start",
        "0s",
        "--duration",
        "3s",
        "--size",
        "0.12",
        "--reveal",
        "word",
        "--reveal-motion",
        "blur",
        "--reveal-duration",
        "1s",
        "--reveal-overlap",
        "0.7",
        "--reveal-easing",
        "back",
        "--reveal-order",
        "center",
        "--reveal-exit",
        "--reveal-exit-motion",
        "drop",
    ]);
    let title = title["results"][0]["clip"].as_str().unwrap().to_string();

    // Its letters glow orange, flaring up over the first second.
    let glow = run(&[
        "effect",
        "add",
        "--clip",
        &title,
        "--type",
        "light.glow",
        "--param",
        "radius=0.04",
        "--param",
        "tint=1",
        "--param",
        "color=#FF7A00",
    ]);
    let glow = glow["results"][0]["effect"].as_str().unwrap().to_string();
    let intensity = format!("effects.{glow}.intensity");
    for (at, value) in [("0s", "0"), ("1s", "2")] {
        run(&[
            "keyframe",
            "set",
            "--clip",
            &title,
            "--property",
            &intensity,
            "--offset",
            at,
            "--value",
            value,
        ]);
    }
    run(&[
        "effect",
        "add",
        "--clip",
        &title,
        "--type",
        "light.sweep",
        "--param",
        "position=0.3",
    ]);

    // Three cards one after another, popping in a fifth of a second apart, last first.
    let cards: Vec<String> = (0..3)
        .map(|i| {
            let added = run(&[
                "text",
                "add",
                &format!("Card {i}"),
                "--start",
                &format!("{}s", 3 + i * 2),
                "--duration",
                "2s",
            ]);
            added["results"][0]["clip"].as_str().unwrap().to_string()
        })
        .collect();
    let staggered = run(&[
        "clips", "stagger", "--clip", &cards[0], "--clip", &cards[1], "--clip", &cards[2],
        "--preset", "pop_in", "--step", "0.2s", "--order", "reverse",
    ]);
    let order: Vec<&str> = staggered["results"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["clip"].as_str().unwrap())
        .collect();
    assert_eq!(order, [&cards[2], &cards[1], &cards[0]]);
    assert_eq!(
        staggered["results"][0]["clips"][2]["delay"]["seconds"],
        json!("0.4")
    );
    // And an animation with a delay.
    run(&[
        "animation",
        "set",
        "--clip",
        &title,
        "--in",
        "glow_in:0.6s",
        "--in-delay",
        "0.2s",
    ]);

    let doc = run(&["timeline", "read", "--detail", "full"]);
    let clips: Vec<Value> = doc["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .collect();
    let find = |id: &str| clips.iter().find(|c| c["id"] == id).unwrap().clone();
    let shown = find(&title);
    assert_eq!(shown["reveal"]["motion"], "blur");
    assert_eq!(shown["reveal"]["overlap"], json!(0.7));
    assert_eq!(shown["reveal"]["easing"], "back");
    assert_eq!(shown["reveal"]["order"], "center");
    assert_eq!(shown["reveal"]["exit_motion"], "drop");
    assert_eq!(shown["animations"]["in"]["preset"], "glow_in");
    assert_eq!(shown["animations"]["in"]["delay"], json!(SECOND / 5));
    let effects = shown["effects"].as_array().unwrap();
    assert_eq!(effects[0]["type"], "light.glow");
    assert_eq!(effects[1]["type"], "light.sweep");
    assert_eq!(
        find(&cards[0])["animations"]["in"]["delay"],
        json!(2 * SECOND / 5)
    );
    assert!(find(&cards[2])["animations"]["in"].get("delay").is_none());

    // Mistakes are typed errors.
    let wrong_place = fails(
        &dir,
        &[
            &p[..],
            &[
                "clips", "stagger", "--clip", &cards[0], "--preset", "pop_out",
            ],
        ]
        .concat(),
    );
    assert_eq!(wrong_place["code"], "fosscut.request.invalid_arguments");
    let overlap = fails(
        &dir,
        &[
            &p[..],
            &[
                "text",
                "set",
                &title,
                "--reveal",
                "word",
                "--reveal-overlap",
                "2",
            ],
        ]
        .concat(),
    );
    assert_eq!(overlap["code"], "fosscut.text.invalid");
    let capabilities = run(&["capabilities"]);
    let text = capabilities.to_string();
    for name in [
        "light.glow",
        "light.sweep",
        "light.streaks",
        "swoop_in",
        "clips.stagger",
    ] {
        assert!(text.contains(name), "capabilities list {name}");
    }

    if !can_render() {
        return;
    }
    // Frame grabs in separate processes agree to the byte; the glow and the reveal change
    // the picture over time.
    run(&["frame", "0.5s", "--out", "a.png", "--width", "320"]);
    run(&["frame", "1.5s", "--out", "b.png", "--width", "320"]);
    run(&["frame", "0.5s", "--out", "c.png", "--width", "320"]);
    let read = |name: &str| std::fs::read(dir.join(name)).unwrap();
    assert_eq!(read("a.png"), read("c.png"));
    assert_ne!(read("a.png"), read("b.png"));
}
