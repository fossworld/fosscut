//! The milestone 4 exit check (ADR-0002, validation): an MCP client reproduces the round 1
//! baseline through `fosscut mcp` alone, over stdio, the way an agent does. It imports the
//! test pattern, adds a 3 s title, splits at 00:00:03:28, checks a frame, exports 1080p30 as
//! a background job and verifies the report.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::mcp::{Client, call, json_of, png_size, start};
use common::{can_render, test_pattern, workspace};
use serde_json::{Value, json};

async fn ok(client: &Client, tool: &str, args: Value) -> Value {
    let result = call(client, tool, args).await;
    assert_ne!(result.is_error, Some(true), "{tool}: {}", json_of(&result));
    json_of(&result)
}

async fn fails(client: &Client, tool: &str, args: Value) -> Value {
    let result = call(client, tool, args).await;
    assert_eq!(result.is_error, Some(true), "{tool} should fail");
    json_of(&result)["error"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_reproduces_the_baseline_through_mcp_alone() {
    if !can_render() {
        return;
    }
    let dir = workspace("mcp-baseline");
    test_pattern(&dir);
    let client = start(&dir, "fosscut-exit-check").await;

    // The round 1 edit, as tool calls with the safety contract.
    let info = ok(
        &client,
        "project_new",
        json!({ "path": "Audit.fosscut", "name": "FossCut audit" }),
    )
    .await;
    assert_eq!(info["revision"], 0);
    let imported = ok(
        &client,
        "media_import",
        json!({ "files": ["test-pattern.mp4"] }),
    )
    .await;
    let asset = imported["results"][0]["asset"]
        .as_str()
        .unwrap()
        .to_string();
    let added = ok(
        &client,
        "edit",
        json!({
            "commands": [{ "command": "clip.add", "args": { "asset": asset } }],
            "expected_revision": imported["revision"],
        }),
    )
    .await;
    let clip = added["results"][0]["clip"].as_str().unwrap().to_string();
    let titled = ok(
        &client,
        "edit",
        json!({
            "commands": [{ "command": "text.add", "args": {
                "text": "FossCut audit", "duration": "3s", "style": { "size": 0.1 }
            } }],
            "expected_revision": added["revision"],
        }),
    )
    .await;
    let split = ok(
        &client,
        "edit",
        json!({
            "commands": [{ "command": "clip.split", "args": { "clip": clip, "at": { "timecode": "00:00:03:28" } } }],
            "expected_revision": titled["revision"],
            "request_id": "split-once",
            "reason": "round 1 split",
        }),
    )
    .await;
    assert_eq!(split["results"][0]["left"]["end"]["frames"], 118);

    let timeline = ok(&client, "timeline_read", json!({})).await;
    assert_eq!(timeline["duration"]["frames"], 240);
    assert_eq!(timeline["revision"], split["revision"]);
    let history = ok(&client, "history", json!({ "limit": 1 })).await;
    assert_eq!(history["entries"][0]["actor"], "agent:fosscut-exit-check");

    // Agent eyes: frames come back as PNG images (the engine's tests check their pixels).
    let frame = call(&client, "frame_grab", json!({ "time": "1s" })).await;
    assert_eq!(png_size(&frame), (640, 360));
    assert_eq!(json_of(&frame)["at"]["frames"], 30);
    let sheet = call(&client, "contact_sheet", json!({ "count": 6 })).await;
    assert_eq!(json_of(&sheet)["tiles"].as_array().unwrap().len(), 6);

    // Export as a background job: H.264 where this machine encodes it, AV1 otherwise.
    let capabilities = ok(&client, "capabilities", json!({})).await;
    let available = |name: &str| {
        capabilities["export_presets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == name && p["available"] == true)
    };
    if !available("mp4-h264") {
        let refused = fails(
            &client,
            "export",
            json!({ "output": "H264.mp4", "preset": "mp4-h264" }),
        )
        .await;
        assert_eq!(refused["code"], "fosscut.export.codec_unavailable");
    }
    let preset = if available("mp4-h264") {
        "mp4-h264"
    } else {
        "mp4-av1"
    };
    let stale = fails(
        &client,
        "export",
        json!({ "output": "Baseline.mp4", "preset": preset, "expected_revision": 0 }),
    )
    .await;
    assert_eq!(stale["code"], "fosscut.revision.conflict");
    let started = ok(
        &client,
        "export",
        json!({
            "output": "Baseline.mp4",
            "preset": preset,
            "expected_revision": split["revision"],
            "request_id": "baseline",
        }),
    )
    .await;
    let job = started["job"].as_str().unwrap().to_string();
    let mut status = ok(
        &client,
        "job_status",
        json!({ "job": job, "wait_seconds": 60 }),
    )
    .await;
    while status["state"] == "running" {
        assert!(status["progress"]["total"] == 240 || status["progress"]["total"] == 0);
        status = ok(
            &client,
            "job_status",
            json!({ "job": job, "wait_seconds": 60 }),
        )
        .await;
    }
    assert_eq!(status["state"], "succeeded", "{status}");

    // The report verifies what was asked for.
    let report = &status["report"];
    let video = &report["verified"]["video"];
    assert_eq!(video["codec"], preset.trim_start_matches("mp4-"));
    assert_eq!(
        (video["width"].clone(), video["height"].clone()),
        (json!(1920), json!(1080))
    );
    assert_eq!(video["rate"], "30");
    assert_eq!(video["frame_count"], 240);
    assert_eq!(report["duration"], 8 * 705_600_000_i64);
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
    assert!(dir.join("Baseline.mp4").is_file());

    client.cancel().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}
