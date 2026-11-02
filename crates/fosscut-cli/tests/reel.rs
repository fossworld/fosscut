//! Talking-head reel helpers end to end, from the `fosscut` binary and its MCP server: a
//! music bed filled to a length, a separate recording synced under the camera, music ducked
//! under a tone that stands in for speech, punch-ins, the reel caption look placed by height,
//! and (with the speech model) keywords emphasised in generated captions.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::mcp::{call, json_of, start};
use common::{ffmpeg_required, has_ffmpeg, workspace};
use serde_json::{Value, json};

fn command(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fosscut"));
    command
        .current_dir(dir)
        .env_remove("FOSSCUT_PROJECT")
        .env("FOSSCUT_ACTOR", "agent:test")
        .env("FOSSCUT_CACHE_DIR", dir.join("cache"))
        .env("FOSSCUT_CONFIG_DIR", dir.join("config"))
        .env_remove("FOSSCUT_MODELS_DIR");
    command
}

fn fosscut(dir: &Path, args: &[&str]) -> Output {
    command(dir).args(args).arg("--json").output().unwrap()
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

fn ffmpeg_in(dir: &Path, args: &[&str]) {
    let status = Command::new("ffmpeg")
        .current_dir(dir)
        .args(["-y", "-v", "error"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
}

fn available() -> bool {
    if has_ffmpeg() {
        return true;
    }
    assert!(
        !ffmpeg_required(),
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    false
}

/// A camera with a room's sound, the same sound recorded 1.5 s later on its own, a tone
/// heard from 1 s to 2 s (speech) and an unrelated noise.
fn media(dir: &Path) {
    ffmpeg_in(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=duration=8:color=pink:seed=7:sample_rate=48000",
            "-af",
            "volume='0.15+0.85*gt(sin(2*PI*1.3*t)*sin(2*PI*0.37*t+0.5),0.3)':eval=frame",
            "-c:a",
            "pcm_s16le",
            "room.wav",
        ],
    );
    ffmpeg_in(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=8",
            "-i",
            "room.wav",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-shortest",
            "camera.mp4",
        ],
    );
    ffmpeg_in(
        dir,
        &[
            "-ss",
            "1.5",
            "-i",
            "room.wav",
            "-c:a",
            "pcm_s16le",
            "recorder.wav",
        ],
    );
    ffmpeg_in(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=300:sample_rate=48000:duration=6",
            "-af",
            "volume='between(t,1,2)':eval=frame",
            "-c:a",
            "pcm_s16le",
            "speech.wav",
        ],
    );
    ffmpeg_in(
        dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=duration=8:color=brown:seed=41:sample_rate=48000",
            "-af",
            "volume='0.3+0.7*gt(sin(2*PI*0.21*t+2),0.8)':eval=frame",
            "-c:a",
            "pcm_s16le",
            "other.wav",
        ],
    );
}

fn clips_of(timeline: &Value) -> Vec<Value> {
    timeline["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .collect()
}

fn clip<'a>(clips: &'a [Value], id: &str) -> &'a Value {
    clips.iter().find(|c| c["id"] == json!(id)).unwrap()
}

fn seconds(value: &Value) -> f64 {
    value["seconds"].as_str().unwrap().parse().unwrap()
}

#[test]
fn reel_helpers_from_the_command_line() {
    if !available() {
        return;
    }
    let dir = workspace("reel");
    media(&dir);
    ok(&dir, &["project", "new", "Reel.fosscut"]);
    for file in ["camera.mp4", "recorder.wav", "speech.wav", "other.wav"] {
        ok(&dir, &["media", "import", file]);
    }
    let camera = ok(
        &dir,
        &[
            "clip",
            "add",
            "--asset",
            "asset_0001",
            "--source-in",
            "2s",
            "--duration",
            "4s",
        ],
    );
    let camera_clip = camera["results"][0]["clip"].as_str().unwrap().to_string();
    // The tone that stands in for speech, on the first audio track.
    let speech = ok(
        &dir,
        &[
            "clip",
            "add",
            "--asset",
            "asset_0003",
            "--start",
            "0s",
            "--duration",
            "5s",
        ],
    );
    let speech_clip = speech["results"][0]["clip"].as_str().unwrap().to_string();

    // A music bed of exactly 5 s from a loop, in one call.
    let music = ok(&dir, &["library", "list", "--kind", "music"]);
    let loop_name = music["items"][0]["name"].as_str().unwrap().to_string();
    let bed = ok(
        &dir,
        &[
            "library",
            "add",
            &loop_name,
            "--duration",
            "5s",
            "--at",
            "0s",
        ],
    );
    let music_clip = bed["results"][0]["clip"].as_str().unwrap().to_string();
    assert_eq!(seconds(&bed["results"][0]["duration"]), 5.0);

    // The recorder lined up under the camera clip, from 0.5 s, the camera muted.
    let synced = ok(
        &dir,
        &[
            "audio",
            "sync",
            "--camera",
            "asset_0001",
            "--recording",
            "asset_0002",
        ],
    );
    let offset = synced["sync"]["offset_seconds"].as_f64().unwrap();
    assert!((offset - 1.5).abs() < 0.003, "{synced}");
    assert_eq!(synced["response"]["committed"], json!(true));
    let pair = &synced["sync"]["pairs"][0];
    let timeline = ok(&dir, &["timeline", "read", "--detail", "full"]);
    let clips = clips_of(&timeline);
    let recording = clip(&clips, pair["recording_clip"].as_str().unwrap());
    assert!(
        (seconds(&recording["source_in"]) - 0.5).abs() < 0.02,
        "{recording}"
    );
    assert_eq!(seconds(&recording["duration"]), 4.0);
    assert_eq!(clip(&clips, &camera_clip)["volume"], json!(0.0));
    assert_eq!(recording["group"], clip(&clips, &camera_clip)["group"]);
    // An unrelated recording is refused, naming its measurement.
    let doubtful = fails(
        &dir,
        &[
            "audio",
            "sync",
            "--camera",
            "asset_0001",
            "--recording",
            "asset_0004",
        ],
    );
    assert_eq!(doubtful["code"], json!("fosscut.audio.sync_uncertain"));
    assert!(doubtful["details"]["confidence"].as_f64().unwrap() < 0.3);

    // Music ducked under the tone: a dry run first changes nothing.
    let before = ok(&dir, &["project", "info"]);
    let dry = ok(
        &dir,
        &[
            "audio",
            "duck",
            "--music",
            &music_clip,
            "--speech",
            &speech_clip,
            "--depth",
            "-15",
            "--dry-run",
        ],
    );
    assert_eq!(dry["duck"]["clips"][0]["clip"], json!(music_clip));
    assert_eq!(
        ok(&dir, &["project", "info"])["revision"],
        before["revision"]
    );
    let ducked = ok(
        &dir,
        &[
            "audio",
            "duck",
            "--music",
            &music_clip,
            "--speech",
            &speech_clip,
            "--depth",
            "-15",
            "--from",
            "silences",
        ],
    );
    assert_eq!(ducked["duck"]["speech"].as_array().unwrap().len(), 1);
    let timeline = ok(&dir, &["timeline", "read", "--detail", "full"]);
    let music = clip(&clips_of(&timeline), &music_clip).clone();
    assert_eq!(music["keyframes"]["volume"].as_array().unwrap().len(), 4);

    // Punch-ins on the camera clip: the face falls back to the upper third without a pose.
    let punched = ok(
        &dir,
        &[
            "clip",
            "punch-in",
            "--clip",
            &camera_clip,
            "--moment",
            "1s,2s",
            "--moment",
            "2.5s,3.5s,1.3",
        ],
    );
    assert_eq!(punched["punch_in"]["anchor"], json!("default"));
    let timeline = ok(&dir, &["timeline", "read", "--detail", "full"]);
    let camera = clip(&clips_of(&timeline), &camera_clip).clone();
    assert!(
        camera["keyframes"]["transform.scale"].is_array(),
        "{camera}"
    );
    assert!(camera["keyframes"]["transform.anchor"].is_array());
    let wrong = fails(
        &dir,
        &["clip", "punch-in", "--clip", &camera_clip, "--moment", "1s"],
    );
    assert_eq!(wrong["code"], json!("fosscut.request.invalid_arguments"));

    // The new actions are listed.
    let capabilities = ok(&dir, &["capabilities"]);
    let operations: Vec<&str> = capabilities["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["name"].as_str())
        .collect();
    for name in ["audio.duck", "audio.sync", "clip.punch_in"] {
        assert!(operations.contains(&name), "{name}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn reel_helpers_through_mcp() {
    if !available() {
        return;
    }
    let dir = workspace("reel-mcp");
    media(&dir);
    ok(&dir, &["project", "new", "Reel.fosscut"]);
    for file in ["camera.mp4", "recorder.wav", "speech.wav"] {
        ok(&dir, &["media", "import", file]);
    }
    let camera = ok(
        &dir,
        &["clip", "add", "--asset", "asset_0001", "--duration", "4s"],
    );
    let camera_clip = camera["results"][0]["clip"].as_str().unwrap().to_string();
    let client = start(&dir, "reel-agent").await;
    let project = dir.join("Reel.fosscut").display().to_string();

    let music = json_of(
        &call(
            &client,
            "library",
            json!({ "action": "list", "kind": "music" }),
        )
        .await,
    );
    let loop_name = music["items"][0]["name"].as_str().unwrap().to_string();
    let bed = json_of(
        &call(
            &client,
            "library",
            json!({ "project": project, "action": "add", "name": loop_name, "duration": "3s", "start": "0s" }),
        )
        .await,
    );
    let music_clip = bed["results"][0]["clip"].as_str().unwrap().to_string();
    assert_eq!(seconds(&bed["results"][0]["duration"]), 3.0);

    let synced = json_of(
        &call(
            &client,
            "auto_edit",
            json!({ "project": project, "action": "sync_audio", "camera": "asset_0001", "recording": "asset_0002" }),
        )
        .await,
    );
    assert!(
        (synced["sync"]["offset_seconds"].as_f64().unwrap() - 1.5).abs() < 0.003,
        "{synced}"
    );
    assert_eq!(synced["committed"], json!(true));

    let ducked = json_of(
        &call(
            &client,
            "auto_edit",
            json!({ "project": project, "action": "duck", "music": music_clip,
                    "speech_from": "silences", "attack": 0.1, "release": 0.3 }),
        )
        .await,
    );
    assert_eq!(ducked["committed"], json!(true), "{ducked}");
    assert_eq!(ducked["duck"]["attack_seconds"], json!(0.1));

    let punched = json_of(
        &call(
            &client,
            "auto_edit",
            json!({ "project": project, "action": "punch_in", "clip": camera_clip,
                    "moments": [ { "at": "1s", "duration": "1s", "scale": 1.2 },
                                 { "start": "2.5s", "end": "3.5s" } ],
                    "focus": [0.5, 0.4], "dry_run": true }),
        )
        .await,
    );
    assert_eq!(punched["punch_in"]["anchor"], json!("focus"));
    assert_eq!(punched["committed"], json!(false), "a dry run");
    let missing = json_of(
        &call(
            &client,
            "auto_edit",
            json!({ "project": project, "action": "punch_in", "clip": camera_clip,
                    "moments": [ { "at": "1s" } ] }),
        )
        .await,
    );
    assert_eq!(
        missing["error"]["code"],
        json!("fosscut.request.invalid_arguments"),
        "{missing}"
    );
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn reel_captions_emphasise_keywords_from_the_transcript() {
    let models = std::env::var_os("FOSSCUT_MODELS_DIR")
        .map(std::path::PathBuf::from)
        .filter(|dir| dir.join("ggml-tiny.en.bin").is_file());
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_SPEECH").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        models.is_some() || !required,
        "FOSSCUT_REQUIRE_SPEECH is set but whisper-tiny.en is not in FOSSCUT_MODELS_DIR"
    );
    let Some(models) = models else {
        return;
    };
    if !available() {
        return;
    }
    let dir = workspace("reel-captions");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/speech/librispeech-5105-28233-0005/audio.flac"
        ),
        dir.join("speech.flac"),
    )
    .unwrap();
    ok(&dir, &["project", "new", "Talk.fosscut"]);
    ok(&dir, &["media", "import", "speech.flac"]);
    ok(
        &dir,
        &["clip", "add", "--asset", "asset_0001", "--duration", "6s"],
    );
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
    let analysis = with_models(&["media", "analyze", "asset_0001", "transcript"]);
    let words = analysis["results"]["transcript"]["words"]
        .as_array()
        .unwrap();
    let keyword = words
        .iter()
        .find(|w| {
            w["text"]
                .as_str()
                .unwrap()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .count()
                > 4
                && seconds(&w["start"]) < 5.0
        })
        .unwrap();
    let text = keyword["text"].as_str().unwrap().trim().to_string();
    let wanted = format!("{text}@{}s", keyword["start"]["seconds"].as_str().unwrap());
    let generated = with_models(&[
        "captions",
        "generate",
        "--preset",
        "reel",
        "--emphasis",
        &wanted,
    ]);
    assert_eq!(generated["committed"], json!(true));
    let pages = generated["results"][0]["pages"].as_array().unwrap();
    assert!(
        pages
            .iter()
            .all(|p| !p["text"].as_str().unwrap().contains(' ')),
        "a word a page"
    );
    let timeline = ok(&dir, &["timeline", "read", "--detail", "full"]);
    let captions = timeline["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == json!("caption"))
        .unwrap()
        .clone();
    assert_eq!(captions["caption_style"]["placement"], json!(0.6));
    let emphasised: Vec<Value> = captions["clips"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["lines"].as_array().unwrap().clone())
        .flat_map(|l| l.as_array().unwrap().clone())
        .filter(|w| w["emphasis"] == json!(true))
        .collect();
    assert_eq!(emphasised.len(), 1, "{emphasised:?}");
    assert_eq!(emphasised[0]["text"].as_str().unwrap().trim(), text);
    // A word never said fails and adds nothing.
    let missing = command(&dir)
        .env("FOSSCUT_MODELS_DIR", &models)
        .args(["captions", "generate", "--emphasis", "zebra", "--json"])
        .output()
        .unwrap();
    let error: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(
        error["error"]["code"],
        json!("fosscut.captions.emphasis_not_found")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
