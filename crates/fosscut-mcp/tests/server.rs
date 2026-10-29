//! The MCP server in process, through the MCP client of the official SDK: the tool list
//! agents see, a safe editing session with every envelope field, typed recovery paths, and
//! frames and an export job on a text-only timeline (ADR-0002, ADR-0006).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use base64::Engine;
use fosscut_mcp::FosscutServer;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig,
    GetPromptRequestParams, Implementation, ReadResourceRequestParams, ResourceContents,
};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

struct TestClient;

impl ClientHandler for TestClient {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("fosscut-test", "1.0"),
        )
    }
}

type Client = RunningService<RoleClient, TestClient>;

/// A server in this process, connected to a client through an in-memory pipe.
async fn connect(project: Option<PathBuf>) -> Client {
    let (server_io, client_io) = tokio::io::duplex(1 << 22);
    tokio::spawn(async move {
        let server = FosscutServer::new(project).serve(server_io).await.unwrap();
        let _ = server.waiting().await;
    });
    TestClient.serve(client_io).await.unwrap()
}

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn call_raw(client: &Client, tool: &str, args: Value) -> CallToolResult {
    let arguments = args.as_object().cloned().unwrap_or_default();
    client
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(arguments))
        .await
        .unwrap()
}

/// The JSON a tool answered with (its last text block).
fn json_of(result: &CallToolResult) -> Value {
    let text = result
        .content
        .iter()
        .rev()
        .find_map(|c| c.as_text())
        .map(|t| t.text.clone())
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Calls a tool that must succeed and returns its JSON.
async fn ok(client: &Client, tool: &str, args: Value) -> Value {
    let result = call_raw(client, tool, args).await;
    assert_ne!(
        result.is_error,
        Some(true),
        "{tool}: {:?}",
        json_of(&result)
    );
    json_of(&result)
}

/// Calls a tool that must fail and returns the typed error.
async fn fails(client: &Client, tool: &str, args: Value) -> Value {
    let result = call_raw(client, tool, args).await;
    assert_eq!(result.is_error, Some(true), "{tool} should fail");
    json_of(&result)["error"].clone()
}

/// The PNG an image tool returned: width and height from its header.
fn png_size(result: &CallToolResult) -> (u32, u32) {
    let image = result.content[0].as_image().unwrap();
    assert_eq!(image.mime_type, "image/png");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let number = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (number(16), number(20))
}

fn can_render() -> bool {
    let available = fosscut_render::Renderer::headless().is_ok();
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_GPU").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        available || !required,
        "FOSSCUT_REQUIRE_GPU is set but there is no adapter"
    );
    available
}

#[tokio::test]
async fn agents_see_a_few_well_described_tools() {
    let client = connect(None).await;
    let info = client.peer_info().unwrap();
    assert_eq!(info.server_info.as_ref().unwrap().name, "fosscut");
    let instructions = info.instructions.clone().unwrap();
    assert!(instructions.contains("expected_revision") && instructions.contains("job_status"));

    let mut tools = client.list_all_tools().await.unwrap();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(
        (15..=25).contains(&tools.len()),
        "ADR-0002 keeps the MCP surface coarse: {} tools",
        tools.len()
    );
    let read_only = [
        "capabilities",
        "contact_sheet",
        "frame_grab",
        "history",
        "job_status",
        "media_list",
        "project_info",
        "project_open",
        "query",
        "timeline_read",
    ];
    let listing: Vec<Value> = tools
        .iter()
        .map(|tool| {
            let description = tool.description.as_deref().unwrap_or_default();
            assert!(
                description.len() > 60,
                "{} needs a fuller description",
                tool.name
            );
            assert_eq!(tool.input_schema.get("type"), Some(&json!("object")));
            let hint = tool.annotations.as_ref().and_then(|a| a.read_only_hint);
            assert_eq!(
                hint == Some(true),
                read_only.contains(&tool.name.as_ref()),
                "{} read-only hint",
                tool.name
            );
            json!({
                "name": tool.name,
                "read_only": hint == Some(true),
                "description": description,
                "input": tool.input_schema.as_ref(),
            })
        })
        .collect();
    insta::assert_json_snapshot!("tools", listing);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_session_edits_safely_and_recovers_from_errors() {
    let dir = workspace("session");
    let client = connect(None).await;

    // Nothing is open yet: the error says what to do.
    let none = fails(&client, "project_info", json!({})).await;
    assert_eq!(none["code"], "fosscut.project.not_found");
    assert!(none["hint"].as_str().unwrap().contains("project_open"));
    let missing = fails(
        &client,
        "project_open",
        json!({ "path": dir.join("No.fosscut") }),
    )
    .await;
    assert_eq!(missing["code"], "fosscut.project.not_found");

    let bundle = dir.join("Show.fosscut");
    let info = ok(
        &client,
        "project_new",
        json!({ "path": bundle, "width": 320, "height": 180, "rate": "25" }),
    )
    .await;
    assert_eq!(info["revision"], 0);
    let reused = fails(&client, "project_new", json!({ "path": bundle })).await;
    assert_eq!(reused["code"], "fosscut.request.invalid");

    // Capabilities: names in outline, schemas and every error code in full.
    let outline = ok(&client, "capabilities", json!({})).await;
    assert!(outline["commands"].as_array().unwrap().len() > 3);
    let full = ok(&client, "capabilities", json!({ "detail": "full" })).await;
    let codes: Vec<&str> = full["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["code"].as_str())
        .collect();
    assert!(
        codes.contains(&"fosscut.revision.conflict") && codes.contains(&"fosscut.job.not_found")
    );
    // An agent asks for just the commands it will use: small, with their schemas.
    let named = ok(&client, "capabilities", json!({ "names": ["clip.split"] })).await;
    assert_eq!(named["commands"][0]["name"], "clip.split");
    assert!(named["commands"][0]["input_schema"]["properties"]["at"].is_object());
    assert!(named.to_string().len() * 10 < full.to_string().len());
    let unknown_name = fails(&client, "capabilities", json!({ "names": ["clip.fold"] })).await;
    assert_eq!(unknown_name["details"]["unknown"], json!(["clip.fold"]));

    // A dry run changes nothing; a write needs the current revision and replays on retry.
    let title = json!([{ "command": "text.add", "args": { "text": "Hello", "duration": { "seconds": "2" } } }]);
    let planned = ok(
        &client,
        "edit",
        json!({ "commands": title, "dry_run": true }),
    )
    .await;
    assert_eq!(planned["dry_run"], true);
    assert_eq!(ok(&client, "project_info", json!({})).await["revision"], 0);
    let edit = json!({ "commands": title, "expected_revision": 0, "request_id": "title-1", "reason": "add the title" });
    let applied = ok(&client, "edit", edit.clone()).await;
    assert_eq!(applied["revision"], 1);
    let retried = ok(&client, "edit", edit.clone()).await;
    assert_eq!(retried["replayed"], true);
    assert_eq!(retried["revision"], 1);
    let stale = fails(
        &client,
        "edit",
        json!({ "commands": title, "expected_revision": 0 }),
    )
    .await;
    assert_eq!(stale["code"], "fosscut.revision.conflict");
    assert_eq!(stale["details"]["current_revision"], 1);
    let unknown = fails(
        &client,
        "edit",
        json!({ "commands": [{ "command": "clip.explode" }] }),
    )
    .await;
    assert_eq!(unknown["code"], "fosscut.command.unknown");

    // The timeline, history (with this client as the actor), undo and redo.
    let timeline = ok(&client, "timeline_read", json!({})).await;
    assert_eq!(timeline["duration"]["frames"], 50);
    let history = ok(&client, "history", json!({ "limit": 5 })).await;
    let latest = &history["entries"][0];
    assert_eq!(latest["actor"], "agent:fosscut-test");
    assert_eq!(latest["reason"], "add the title");
    assert_eq!(
        ok(&client, "undo", json!({ "expected_revision": 1 })).await["revision"],
        2
    );
    let empty = ok(&client, "timeline_read", json!({ "detail": "outline" })).await;
    assert_eq!(empty["duration"]["frames"], 0);
    assert_eq!(ok(&client, "redo", json!({})).await["revision"], 3);
    let nothing = fails(&client, "redo", json!({})).await;
    assert_eq!(nothing["code"], "fosscut.history.nothing_to_redo");

    // Media, generic queries and explicit project paths.
    let media = ok(&client, "media_list", json!({})).await;
    assert_eq!(media["assets"], json!([]));
    let queried = ok(
        &client,
        "query",
        json!({ "name": "project.info", "project": bundle }),
    )
    .await;
    assert_eq!(queried["revision"], 3);
    let bad_query = fails(&client, "query", json!({ "name": "project.secrets" })).await;
    assert_eq!(bad_query["code"], "fosscut.query.unknown");
    let bad_import = fails(
        &client,
        "media_import",
        json!({ "files": [dir.join("none.mp4")] }),
    )
    .await;
    assert!(
        bad_import["code"]
            .as_str()
            .unwrap()
            .starts_with("fosscut.media.")
    );
    let no_job = fails(&client, "job_status", json!({ "job": "job_0042" })).await;
    assert_eq!(no_job["code"], "fosscut.job.not_found");

    // A server started on a project uses it until another is opened.
    let preset = connect(Some(bundle.clone())).await;
    assert_eq!(ok(&preset, "project_info", json!({})).await["revision"], 3);
    preset.cancel().await.unwrap();
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn frames_and_export_jobs_on_a_title() {
    if !can_render() {
        return;
    }
    let dir = workspace("render");
    let client = connect(None).await;
    let bundle = dir.join("Card.fosscut");
    ok(
        &client,
        "project_new",
        json!({ "path": bundle, "width": 320, "height": 180 }),
    )
    .await;
    ok(
        &client,
        "edit",
        json!({ "commands": [{ "command": "text.add", "args": { "text": "Card", "duration": { "seconds": "1" } } }] }),
    )
    .await;

    // Agent eyes: a frame and a contact sheet, as PNG images with their times.
    let frame = call_raw(
        &client,
        "frame_grab",
        json!({ "time": { "frames": 12 }, "width": 160 }),
    )
    .await;
    assert_eq!(png_size(&frame), (160, 90));
    assert_eq!(json_of(&frame)["at"]["timecode"], "00:00:00:12");
    let late = fails(&client, "frame_grab", json!({ "time": { "seconds": "5" } })).await;
    assert_eq!(late["code"], "fosscut.time.out_of_range");
    let sheet = call_raw(&client, "contact_sheet", json!({ "count": 4, "width": 80 })).await;
    assert_eq!(png_size(&sheet), (164, 94));
    assert_eq!(json_of(&sheet)["tiles"].as_array().unwrap().len(), 4);

    // Export: a dry run, then a background job followed to its verified report.
    let output = dir.join("Card.m4a");
    let planned = ok(
        &client,
        "export",
        json!({ "output": output, "dry_run": true }),
    )
    .await;
    assert_eq!(planned["preset"], "m4a-aac");
    assert!(!output.exists());
    let started = ok(
        &client,
        "export",
        json!({ "output": output, "request_id": "card" }),
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
        status = ok(
            &client,
            "job_status",
            json!({ "job": job, "wait_seconds": 60 }),
        )
        .await;
    }
    assert_eq!(status["state"], "succeeded", "{status}");
    assert_eq!(status["report"]["verified"]["audio"]["codec"], "aac");
    assert!(output.is_file());
    let cancelled = ok(&client, "job_cancel", json!({ "job": job })).await;
    assert_eq!(
        cancelled["state"], "succeeded",
        "finished jobs are not affected"
    );

    let again = fails(&client, "export", json!({ "output": output })).await;
    assert_eq!(again["code"], "fosscut.guard.destructive");
    let preset = fails(
        &client,
        "export",
        json!({ "output": dir.join("x.mp4"), "preset": "mp4-gif" }),
    )
    .await;
    assert_eq!(preset["code"], "fosscut.request.invalid_arguments");
    assert!(preset["hint"].as_str().unwrap().contains("mp4-h264"));
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn skills_are_prompts_and_resources_for_any_client() {
    let client = connect(None).await;
    let capabilities = &client.peer_info().unwrap().capabilities;
    assert!(capabilities.prompts.is_some() && capabilities.resources.is_some());
    let skill = fosscut_mcp::skills::skill("fosscut-basics").unwrap();

    let prompts = client.list_all_prompts().await.unwrap();
    let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["fosscut-basics"]);
    assert_eq!(prompts[0].description.as_deref(), Some(skill.description()));
    let prompt = client
        .get_prompt(GetPromptRequestParams::new("fosscut-basics"))
        .await
        .unwrap();
    let message = serde_json::to_value(&prompt.messages[0]).unwrap();
    assert_eq!(message["role"], "user");
    assert_eq!(message["content"]["text"], skill.text());
    assert!(
        client
            .get_prompt(GetPromptRequestParams::new("fosscut-magic"))
            .await
            .is_err()
    );

    let resources = client.list_all_resources().await.unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].uri, "fosscut://skills/fosscut-basics");
    assert_eq!(resources[0].mime_type.as_deref(), Some("text/markdown"));
    let read = client
        .read_resource(ReadResourceRequestParams::new(
            "fosscut://skills/fosscut-basics",
        ))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &read.contents[0] else {
        panic!("the skill is text");
    };
    assert_eq!(text, skill.text());
    assert!(
        client
            .read_resource(ReadResourceRequestParams::new("fosscut://skills/none"))
            .await
            .is_err()
    );
    client.cancel().await.unwrap();
}
