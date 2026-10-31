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

/// A server in this process, connected to a client through an in-memory pipe. It caches
/// in a folder of this test run, never in the user's cache, and has no models.
async fn connect(project: Option<PathBuf>) -> Client {
    let cache = std::env::temp_dir().join(format!("fosscut-mcp-cache-{}", std::process::id()));
    connect_with_models(project, cache.join("models")).await
}

/// Like [`connect`], with models in `models`.
async fn connect_with_models(project: Option<PathBuf>, models: PathBuf) -> Client {
    let (server_io, client_io) = tokio::io::duplex(1 << 22);
    let cache = std::env::temp_dir().join(format!("fosscut-mcp-cache-{}", std::process::id()));
    tokio::spawn(async move {
        let server = FosscutServer::with_dirs(project, cache, models)
            .serve(server_io)
            .await
            .unwrap();
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
        "media_analyze",
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
    let built_in: Vec<&str> = fosscut_mcp::skills::SKILLS.iter().map(|s| s.name).collect();
    assert_eq!(names, built_in);
    assert!(built_in.contains(&"fosscut-basics") && built_in.contains(&"viral-captions"));
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

    let captions = client
        .get_prompt(GetPromptRequestParams::new("viral-captions"))
        .await
        .unwrap();
    let message = serde_json::to_value(&captions.messages[0]).unwrap();
    assert_eq!(
        message["content"]["text"],
        fosscut_mcp::skills::skill("viral-captions").unwrap().text()
    );

    let resources = client.list_all_resources().await.unwrap();
    let uris: Vec<String> = resources.iter().map(|r| r.uri.to_string()).collect();
    let expected: Vec<String> = built_in
        .iter()
        .map(|name| format!("fosscut://skills/{name}"))
        .collect();
    assert_eq!(uris, expected);
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

fn has_ffmpeg() -> bool {
    let present = ["ffmpeg", "ffprobe"].iter().all(|tool| {
        std::process::Command::new(tool)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    });
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        present || !required,
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    present
}

#[tokio::test]
async fn media_is_analyzed_then_answered_from_the_cache() {
    if !has_ffmpeg() {
        return;
    }
    let dir = workspace("analyze");
    let status = std::process::Command::new("ffmpeg")
        .current_dir(&dir)
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg("aevalsrc=if(between(t\\,1\\,1.5)\\,0\\,0.25*sin(2*PI*440*t)):s=48000:d=3")
        .args(["-c:a", "pcm_s16le", "pause.wav"])
        .status()
        .unwrap();
    assert!(status.success());
    let client = connect(None).await;
    ok(
        &client,
        "project_new",
        json!({ "path": dir.join("Talk.fosscut") }),
    )
    .await;
    ok(
        &client,
        "media_import",
        json!({ "files": [dir.join("pause.wav")] }),
    )
    .await;

    let first = ok(
        &client,
        "media_analyze",
        json!({ "asset": "asset_0001", "wait_seconds": 60 }),
    )
    .await;
    assert_eq!(first["state"], "succeeded", "{first}");
    assert_eq!(first["kind"], "analysis");
    let analysis = &first["analysis"];
    assert_eq!(
        analysis["computed"],
        json!(["loudness", "silences"]),
        "audio only: no scenes"
    );
    let pause = &analysis["results"]["silences"]["ranges"][0];
    assert_eq!(
        (
            pause["start"]["seconds"].clone(),
            pause["end"]["seconds"].clone()
        ),
        (json!("1"), json!("1.5"))
    );
    assert!(analysis["results"]["loudness"]["integrated_lufs"].is_number());

    // The second time comes straight from the cache, without a job.
    let again = ok(&client, "media_analyze", json!({ "asset": "asset_0001" })).await;
    assert!(again.get("job").is_none());
    assert_eq!(again["analysis"]["cached"], json!(["loudness", "silences"]));
    assert_eq!(again["analysis"]["results"], analysis["results"]);
    // Other options measure again.
    let stricter = ok(
        &client,
        "media_analyze",
        json!({ "asset": "asset_0001", "analyzers": ["silences"], "options": { "silences": { "min_duration_ms": 600 } } }),
    )
    .await;
    assert_eq!(
        stricter["analysis"]["results"]["silences"]["ranges"],
        json!([])
    );

    let unknown = fails(
        &client,
        "media_analyze",
        json!({ "asset": "asset_0001", "analyzers": ["faces"] }),
    )
    .await;
    assert_eq!(unknown["code"], "fosscut.analysis.unknown_analyzer");
    let no_video = fails(
        &client,
        "media_analyze",
        json!({ "asset": "asset_0001", "analyzers": ["scenes"] }),
    )
    .await;
    assert_eq!(no_video["code"], "fosscut.analysis.no_video");
    let missing = fails(&client, "media_analyze", json!({ "asset": "asset_0042" })).await;
    assert_eq!(missing["code"], "fosscut.asset.not_found");
    let malformed = fails(&client, "media_analyze", json!({ "asset": "pause.wav" })).await;
    assert_eq!(malformed["code"], "fosscut.request.invalid_arguments");
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn models_install_only_with_confirmation() {
    let dir = workspace("models");
    let folder = dir.join("models");
    let client = connect_with_models(None, folder.clone()).await;
    let listed = ok(&client, "models", json!({})).await;
    let models = listed["models"].as_array().unwrap();
    assert_eq!(models.len(), 10);
    assert!(models.iter().all(|m| m["installed"] == json!(false)));
    assert!(
        models
            .iter()
            .filter(|m| m["task"] == json!("speech"))
            .all(|m| m["license"] == json!("MIT"))
    );

    // Installing asks the agent to confirm with the user first.
    let unconfirmed = fails(&client, "models", json!({ "install": "whisper-tiny.en" })).await;
    assert_eq!(unconfirmed["code"], "fosscut.model.confirmation_required");
    assert_eq!(unconfirmed["details"]["size_bytes"], 77_704_715);
    assert!(
        unconfirmed["hint"]
            .as_str()
            .unwrap()
            .contains("confirm: true")
    );
    let unknown = fails(
        &client,
        "models",
        json!({ "install": "whisper-huge", "confirm": true }),
    )
    .await;
    assert_eq!(unknown["code"], "fosscut.model.unknown");
    std::fs::write(dir.join("fake.bin"), "not a model").unwrap();
    let wrong = fails(
        &client,
        "models",
        json!({ "install": "whisper-tiny.en", "from": dir.join("fake.bin") }),
    )
    .await;
    assert_eq!(wrong["code"], "fosscut.model.checksum_mismatch");

    // A model already there is reported, in capabilities too.
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::File::create(folder.join("ggml-tiny.en.bin"))
        .unwrap()
        .set_len(77_704_715)
        .unwrap();
    let unchanged = ok(&client, "models", json!({ "install": "whisper-tiny.en" })).await;
    assert_eq!(unchanged["status"], "unchanged");
    assert_eq!(unchanged["model"]["installed"], true);
    let capabilities = ok(&client, "capabilities", json!({})).await;
    assert_eq!(capabilities["models"][3]["installed"], true);

    // That file has the right size but is not a model, so the recognition job fails with a
    // typed error; with no model at all, the analysis says what to install at once.
    if has_ffmpeg() {
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/speech/librispeech-5105-28233-0005/audio.flac"
            ),
            dir.join("speech.flac"),
        )
        .unwrap();
        ok(
            &client,
            "project_new",
            json!({ "path": dir.join("Talk.fosscut") }),
        )
        .await;
        ok(
            &client,
            "media_import",
            json!({ "files": [dir.join("speech.flac")] }),
        )
        .await;
        let broken = ok(
            &client,
            "media_analyze",
            json!({ "asset": "asset_0001", "analyzers": ["transcript"], "wait_seconds": 60 }),
        )
        .await;
        assert_eq!(broken["state"], "failed", "{broken}");
        assert_eq!(broken["error"]["code"], "fosscut.analysis.speech_failed");
        std::fs::remove_file(folder.join("ggml-tiny.en.bin")).unwrap();
        let missing = fails(
            &client,
            "media_analyze",
            json!({ "asset": "asset_0001", "analyzers": ["transcript"] }),
        )
        .await;
        assert_eq!(missing["code"], "fosscut.model.not_installed");
    }
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn captions_import_export_and_explain_what_is_missing() {
    let dir = workspace("captions");
    let client = connect(None).await;
    ok(
        &client,
        "project_new",
        json!({ "path": dir.join("Short.fosscut"), "width": 1080, "height": 1920 }),
    )
    .await;
    std::fs::write(
        dir.join("subs.srt"),
        "1\n00:00:00,500 --> 00:00:01,800\nMost people never\nnotice this.\n\n2\n00:00:02,000 --> 00:00:03,250\nBut once you do,\n",
    )
    .unwrap();
    let planned = ok(
        &client,
        "captions",
        json!({ "action": "import", "file": dir.join("subs.srt"), "preset": "karaoke", "dry_run": true }),
    )
    .await;
    assert_eq!(planned["committed"], false);
    let imported = ok(
        &client,
        "captions",
        json!({ "action": "import", "file": dir.join("subs.srt"), "preset": "karaoke", "request_id": "subs-1", "reason": "add subtitles" }),
    )
    .await;
    assert_eq!(imported["committed"], true);
    assert_eq!(
        imported["results"][0]["pages"][1]["text"],
        "But once you do,"
    );
    let again = ok(
        &client,
        "captions",
        json!({ "action": "import", "file": dir.join("subs.srt"), "preset": "karaoke", "request_id": "subs-1", "reason": "add subtitles" }),
    )
    .await;
    assert_eq!(
        again["replayed"], true,
        "a retry does not add the captions twice"
    );
    let history = ok(&client, "history", json!({})).await;
    assert!(history.to_string().contains("agent:fosscut-test"));

    let out = dir.join("out.srt");
    let exported = ok(
        &client,
        "captions",
        json!({ "action": "export", "file": out }),
    )
    .await;
    assert_eq!(exported["cues"], 2);
    assert!(
        exported["preview"]
            .as_str()
            .unwrap()
            .starts_with("1\n00:00:00,500 --> 00:00:01,800\nMost people never\nnotice this."),
        "an agent without file access can read what was written: {exported}"
    );
    // Cue edges sit on frames: 3.25 s is between frames 97 and 98 at 30 fps.
    assert!(
        std::fs::read_to_string(&out)
            .unwrap()
            .contains("00:00:02,000 --> 00:00:03,267")
    );
    let guarded = fails(
        &client,
        "captions",
        json!({ "action": "export", "file": out }),
    )
    .await;
    assert_eq!(guarded["code"], "fosscut.guard.destructive");
    let dry = ok(
        &client,
        "captions",
        json!({ "action": "export", "file": dir.join("dry.srt"), "dry_run": true }),
    )
    .await;
    assert_eq!(dry["dry_run"], true);
    assert!(!dir.join("dry.srt").exists());
    ok(
        &client,
        "captions",
        json!({ "action": "export", "file": out, "allow_destructive": true }),
    )
    .await;

    // What is missing comes back as typed errors with hints.
    let no_file = fails(&client, "captions", json!({ "action": "export" })).await;
    assert_eq!(no_file["code"], "fosscut.request.invalid_arguments");
    let no_import = fails(&client, "captions", json!({ "action": "import" })).await;
    assert_eq!(no_import["code"], "fosscut.request.invalid_arguments");
    let unreadable = fails(
        &client,
        "captions",
        json!({ "action": "import", "file": dir.join("missing.srt") }),
    )
    .await;
    assert_eq!(unreadable["code"], "fosscut.request.invalid_arguments");
    std::fs::write(dir.join("bad.srt"), "1\nsoon --> later\nHi\n").unwrap();
    let bad = fails(
        &client,
        "captions",
        json!({ "action": "import", "file": dir.join("bad.srt") }),
    )
    .await;
    assert_eq!(bad["code"], "fosscut.captions.invalid_srt");
    let wrong_timeline = fails(
        &client,
        "captions",
        json!({ "action": "import", "file": dir.join("subs.srt"), "timeline": "timeline_0042" }),
    )
    .await;
    assert_eq!(wrong_timeline["code"], "fosscut.timeline.not_found");
    let malformed = fails(
        &client,
        "captions",
        json!({ "action": "export", "file": out, "track": "nonsense" }),
    )
    .await;
    assert_eq!(malformed["code"], "fosscut.request.invalid_arguments");
    let generate = fails(&client, "captions", json!({ "action": "generate" })).await;
    assert_eq!(generate["code"], "fosscut.captions.no_transcript");
    assert!(generate["hint"].as_str().unwrap().contains("media_analyze"));
    let no_project = fails(
        &client,
        "captions",
        json!({ "action": "export", "file": out, "project": dir.join("None.fosscut") }),
    )
    .await;
    assert_eq!(no_project["code"], "fosscut.project.not_found");
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A short runtime directory for live sessions: Unix socket paths are limited to about
/// 100 bytes.
fn runtime(name: &str) -> PathBuf {
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let dir = base.join(format!("fcm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The milestone 5 exit check without the app (ADR-0002 Validation, ADR-0022): with the
/// project hosted, an agent's round 1 sequence runs in the host's session, each change
/// reaches the host's subscribers as it commits, and undo happens there too.
#[tokio::test]
async fn an_agents_edits_run_live_in_the_host_and_undo_there() {
    use fosscut_engine::live::{Host, HostOptions};

    let dir = workspace("live");
    let runtime = runtime("live");
    let bundle = dir.join("Live.fosscut");
    let format = fosscut_model::TimelineFormat {
        width: 320,
        height: 180,
        rate: fosscut_time::Rate::new(30, 1).unwrap(),
        sample_rate: 48_000,
    };
    fosscut_engine::project::create(&bundle, "Live", format).unwrap();
    let host = Host::start(
        &bundle,
        HostOptions::new("FossCut", "test").runtime(&runtime),
    )
    .unwrap();
    let changes = host.subscribe();

    let (server_io, client_io) = tokio::io::duplex(1 << 22);
    let (cache, models) = (dir.join("cache"), dir.join("models"));
    let (served, hosted) = (bundle.clone(), runtime.clone());
    tokio::spawn(async move {
        let server = FosscutServer::with_runtime(Some(served), cache, models, hosted)
            .serve(server_io)
            .await
            .unwrap();
        let _ = server.waiting().await;
    });
    let client = TestClient.serve(client_io).await.unwrap();

    let info = ok(&client, "project_info", json!({})).await;
    assert_eq!(info["session"]["live"], true);
    assert_eq!(info["session"]["host"]["name"], "FossCut");
    let mut expected = vec![];

    if has_ffmpeg() {
        let status = std::process::Command::new("ffmpeg")
            .current_dir(&dir)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=30:duration=2",
            ])
            .args(["-c:v", "mpeg4", "pattern.mp4"])
            .status()
            .unwrap();
        assert!(status.success());
        ok(
            &client,
            "media_import",
            json!({ "files": [dir.join("pattern.mp4")] }),
        )
        .await;
        expected.push("media.add");
    }
    let title = ok(
        &client,
        "edit",
        json!({ "commands": [{ "command": "text.add", "args": { "text": "Live", "start": "0f", "duration": { "seconds": "2" } } }] }),
    )
    .await;
    let clip = title["results"][0]["clip"].clone();
    ok(
        &client,
        "edit",
        json!({ "commands": [{ "command": "clip.split", "args": { "clip": clip, "at": { "seconds": "1" } } }] }),
    )
    .await;
    expected.extend(["text.add", "clip.split"]);
    let revision = host.revision();
    ok(&client, "undo", json!({ "expected_revision": revision })).await;
    expected.push("history.undo");

    let seen: Vec<_> = std::iter::from_fn(|| changes.try_recv().ok()).collect();
    let commands: Vec<&str> = seen.iter().map(|c| c.commands[0].as_str()).collect();
    assert_eq!(
        commands, expected,
        "each change reached the host as it committed"
    );
    assert!(seen.iter().all(|c| c.actor == "agent:fosscut-test"));
    assert_eq!(seen.last().unwrap().kind, fosscut_core::EntryKind::Undo);
    let clients: Vec<String> = host.clients().into_iter().map(|c| c.name).collect();
    assert_eq!(
        clients,
        ["agent:fosscut-test"],
        "the agent shows as one connected client"
    );
    let read = ok(&client, "timeline_read", json!({})).await;
    assert_eq!(read["revision"], host.revision());

    if has_ffmpeg() && can_render() {
        let grab = call_raw(
            &client,
            "frame_grab",
            json!({ "time": { "seconds": "0.5" }, "width": 160 }),
        )
        .await;
        assert_ne!(grab.is_error, Some(true), "{:?}", json_of(&grab));
        assert_eq!(
            png_size(&grab),
            (160, 90),
            "frames render from the host's snapshot"
        );
    }

    let at = host.revision();
    host.close();
    let local = ok(
        &client,
        "edit",
        json!({ "commands": [{ "command": "text.add", "args": { "text": "After", "start": { "seconds": "3" }, "duration": { "seconds": "1" } } }] }),
    )
    .await;
    assert_eq!(
        local["revision"],
        at + 1,
        "with the host gone, edits continue on disk"
    );
    let info = ok(&client, "project_info", json!({})).await;
    assert_eq!(info["session"], json!({ "live": false }));
}

#[tokio::test]
async fn agents_grade_with_effects_luts_and_automatic_color() {
    if !has_ffmpeg() || !can_render() {
        return;
    }
    let dir = workspace("color");
    let status = std::process::Command::new("ffmpeg")
        .current_dir(&dir)
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg("testsrc2=s=320x180:r=30:d=1,colorchannelmixer=rr=0.6:gg=0.8:bb=1.0")
        .args(["-c:v", "mpeg4", "-q:v", "2", "cool.mp4"])
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::write(
        dir.join("flat.cube"),
        "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
    )
    .unwrap();
    let client = connect(None).await;
    ok(
        &client,
        "project_new",
        json!({ "path": dir.join("Grade.fosscut"), "width": 320, "height": 180 }),
    )
    .await;
    let imported = ok(
        &client,
        "media_import",
        json!({ "files": [dir.join("cool.mp4"), dir.join("flat.cube")] }),
    )
    .await;
    let lut = imported["results"][1]["asset"].clone();
    let placed = ok(
        &client,
        "edit",
        json!({ "commands": [{ "command": "clip.add", "args": { "asset": imported["results"][0]["asset"] } }] }),
    )
    .await;
    let clip = placed["results"][0]["clip"].clone();
    let effects = ok(
        &client,
        "edit",
        json!({ "commands": [
            { "command": "effect.add", "args": { "clip": clip, "type": "color.lut", "params": { "lut": { "asset": lut }, "intensity": 0.5 } } },
            { "command": "look.apply", "args": { "clip": clip, "look": "warm" } }
        ] }),
    )
    .await;
    assert_eq!(
        effects["results"][1]["effects"].as_array().unwrap().len(),
        2,
        "the LUT, then the warm look's adjust"
    );
    let caps = ok(&client, "capabilities", json!({ "detail": "outline" })).await;
    assert!(caps["effects"].as_array().unwrap().len() >= 9);
    assert!(caps["looks"].as_array().unwrap().len() >= 10);

    let corrected = ok(&client, "color", json!({ "action": "auto", "clip": clip })).await;
    let first = &corrected["results"][0]["effects"][0];
    assert_eq!(first["type"], "color.adjust", "auto color goes first");
    assert!(first["params"]["temperature"].as_f64().unwrap() > 0.0);
    assert!(
        corrected["measured"]["mean"][2].as_f64().unwrap()
            > corrected["measured"]["mean"][0].as_f64().unwrap()
    );
    let unmatched = fails(&client, "color", json!({ "action": "match", "clip": clip })).await;
    assert_eq!(unmatched["code"], "fosscut.request.invalid_arguments");
    let frame = call_raw(
        &client,
        "frame_grab",
        json!({ "time": { "frames": 10 }, "width": 160 }),
    )
    .await;
    assert_eq!(png_size(&frame), (160, 90));
    let undone = ok(&client, "undo", json!({})).await;
    assert!(undone["revision"].as_u64().is_some());
}

#[tokio::test]
async fn the_library_lists_without_a_project_and_places_sounds_on_free_tracks() {
    let client = connect(None).await;
    let listed = ok(
        &client,
        "library",
        json!({ "action": "list", "kind": "music" }),
    )
    .await;
    let items = listed["items"].as_array().unwrap();
    assert!(items.len() >= 6 && items.iter().all(|i| i["bpm"].is_number()));
    let missing = fails(
        &client,
        "library",
        json!({ "action": "add", "name": "whoosh" }),
    )
    .await;
    assert_eq!(missing["code"], "fosscut.project.not_found");
    let dir = workspace("library");
    ok(
        &client,
        "project_new",
        json!({ "path": dir.join("Sounds.fosscut").display().to_string() }),
    )
    .await;
    // Two whooshes at the same moment: the second goes on a track of its own, and the
    // revision check covers the import.
    let first = ok(
        &client,
        "library",
        json!({ "action": "add", "name": "whoosh", "start": { "seconds": "1" }, "expected_revision": 0 }),
    )
    .await;
    let second = ok(
        &client,
        "library",
        json!({ "action": "add", "name": "whoosh", "start": { "seconds": "1" }, "expected_revision": first["revision"] }),
    )
    .await;
    assert_ne!(first["results"][0]["track"], second["results"][0]["track"]);
    let stale = fails(
        &client,
        "library",
        json!({ "action": "add", "name": "pop", "start": { "seconds": "2" }, "expected_revision": 0 }),
    )
    .await;
    assert_eq!(stale["code"], "fosscut.revision.conflict");
    let badge = ok(
        &client,
        "library",
        json!({ "action": "add", "name": "badge-new", "start": { "seconds": "0" } }),
    )
    .await;
    assert_eq!(badge["item"]["kind"], "sticker");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Runs FFmpeg in `dir` and checks it succeeded.
fn ffmpeg_in(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("ffmpeg")
        .current_dir(dir)
        .args(["-y", "-v", "error"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
}

#[tokio::test]
async fn agents_steady_clips_and_cut_multicam_angles_synced_by_sound() {
    if !has_ffmpeg() {
        return;
    }
    let dir = workspace("auto-edit");
    ffmpeg_in(
        &dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:size=60x34:rate=30:duration=1,noise=alls=100:allf=0:all_seed=5,scale=480:270:flags=bicubic,eq=contrast=2",
            "-frames:v",
            "1",
            "texture.png",
        ],
    );
    ffmpeg_in(
        &dir,
        &[
            "-loop",
            "1",
            "-framerate",
            "30",
            "-i",
            "texture.png",
            "-vf",
            "crop=w=320:h=180:x='80+20*sin(2*PI*2*t)':y=45:exact=1,format=yuv420p",
            "-t",
            "1",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "shaky.mp4",
        ],
    );
    ffmpeg_in(
        &dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "anoisesrc=duration=5:color=pink:seed=11:sample_rate=48000",
            "-af",
            "volume='0.15+0.85*gt(sin(2*PI*1.1*t)*sin(2*PI*0.41*t+0.3),0.3)':eval=frame",
            "-c:a",
            "pcm_s16le",
            "room.wav",
        ],
    );
    for (file, picture, skip) in [
        ("wide.mp4", "testsrc2=s=320x180:r=30:d=5", "0"),
        ("close.mp4", "color=c=0x2040C0:s=320x180:r=30:d=4", "0.5"),
    ] {
        ffmpeg_in(
            &dir,
            &[
                "-f",
                "lavfi",
                "-i",
                picture,
                "-ss",
                skip,
                "-i",
                "room.wav",
                "-c:v",
                "mpeg4",
                "-c:a",
                "aac",
                "-shortest",
                file,
            ],
        );
    }
    let client = connect(None).await;
    ok(
        &client,
        "project_new",
        json!({ "path": dir.join("Cams.fosscut"), "width": 320, "height": 180 }),
    )
    .await;
    ok(
        &client,
        "media_import",
        json!({ "files": [dir.join("shaky.mp4"), dir.join("wide.mp4"), dir.join("close.mp4")] }),
    )
    .await;
    let placed = ok(
        &client,
        "edit",
        json!({ "commands": [{ "command": "clip.add", "args": { "asset": "asset_0001" } }] }),
    )
    .await;
    let clip = placed["results"][0]["clip"].as_str().unwrap().to_string();
    let revision = placed["revision"].as_u64().unwrap();

    let planned = ok(
        &client,
        "auto_edit",
        json!({ "action": "stabilize", "clip": clip, "strength": 1, "dry_run": true }),
    )
    .await;
    assert_eq!(planned["dry_run"], true, "{planned}");
    assert!(planned["motion"]["zoom"].as_f64().unwrap() > 1.0);
    let steadied = ok(
        &client,
        "auto_edit",
        json!({ "action": "stabilize", "clip": clip, "strength": 1, "expected_revision": revision }),
    )
    .await;
    assert_eq!(steadied["revision"], revision + 1, "{steadied}");
    let read = ok(&client, "timeline_read", json!({ "detail": "full" })).await;
    let keyed = &read["tracks"][0]["clips"][0];
    assert_eq!(
        keyed["keyframes"]["transform.position"]
            .as_array()
            .map(Vec::len),
        Some(30),
        "{keyed}"
    );
    let stale = fails(
        &client,
        "auto_edit",
        json!({ "action": "stabilize", "clip": clip, "expected_revision": revision }),
    )
    .await;
    assert_eq!(stale["code"], "fosscut.revision.conflict");
    for (args, code) in [
        (
            json!({ "action": "stabilize" }),
            "fosscut.request.invalid_arguments",
        ),
        (
            json!({ "action": "track", "clip": clip, "follow": clip }),
            "fosscut.request.invalid_arguments",
        ),
        (
            json!({ "action": "multicam", "angles": ["asset_0002"] }),
            "fosscut.request.invalid_arguments",
        ),
    ] {
        assert_eq!(fails(&client, "auto_edit", args).await["code"], code);
    }

    let synced = ok(
        &client,
        "auto_edit",
        json!({ "action": "multicam_sync", "angles": ["asset_0002", "asset_0003"] }),
    )
    .await;
    let offset = synced["sync"]["offsets"][1].as_i64().unwrap() as f64 / 705_600_000.0;
    assert!((offset - 0.5).abs() < 0.005, "{synced}");
    let made = ok(
        &client,
        "auto_edit",
        json!({ "action": "multicam", "angles": ["asset_0002", "asset_0003"], "sound": 2, "start": "2s" }),
    )
    .await;
    assert!(made["committed"].as_bool().unwrap(), "{made}");
    assert!(made["sync"]["confidence"][1].as_f64().unwrap() > 0.3);
    let created = &made["results"][0];
    let multicam = created["clip"]["clip"].as_str().unwrap();
    let switched = ok(
        &client,
        "edit",
        json!({ "commands": [{ "command": "multicam.switch", "args": { "clip": multicam, "at": "3s", "angle": 2 } }] }),
    )
    .await;
    assert_eq!(switched["results"][0]["angle"], created["angles"][1]);

    // Given offsets are used as they are, without measuring.
    let given = ok(
        &client,
        "auto_edit",
        json!({ "action": "multicam", "angles": ["asset_0002", "asset_0003"], "offsets": ["0s", "0.5s"], "name": "Given", "start": "8s" }),
    )
    .await;
    assert!(given.get("sync").is_none(), "{given}");
    let inner = ok(
        &client,
        "timeline_read",
        json!({ "timeline": given["results"][0]["timeline"], "detail": "full" }),
    )
    .await;
    assert_eq!(
        inner["tracks"][0]["clips"][0]["start"]["flicks"],
        352_800_000
    );

    // The multicam clip's sound is levelled.
    let leveled = ok(
        &client,
        "audio_normalize",
        json!({ "clips": [multicam], "target_lufs": -16 }),
    )
    .await;
    assert!(
        leveled["normalization"]["clips"][0]["measured_lufs"].is_number(),
        "{leveled}"
    );

    // A title follows the middle of the shaky picture, then the shots split at their cut.
    let title = ok(
        &client,
        "edit",
        json!({ "commands": [{ "command": "text.add", "args": { "text": "Here", "start": "0s", "duration": "1s" } }] }),
    )
    .await["results"][0]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let followed = ok(
        &client,
        "auto_edit",
        json!({ "action": "track", "clip": clip, "region": { "x": 0.5, "y": 0.5, "width": 0.25, "height": 0.25 }, "at": "0s", "follow": title }),
    )
    .await;
    assert_eq!(followed["motion"]["clip"], title.as_str(), "{followed}");
    assert!(followed["committed"].as_bool().unwrap());
    ffmpeg_in(
        &dir,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x180:r=30:d=1",
            "-f",
            "lavfi",
            "-i",
            "color=c=0xC02020:s=320x180:r=30:d=1",
            "-filter_complex",
            "[0][1]concat=n=2:v=1:a=0,format=yuv420p",
            "-c:v",
            "mpeg4",
            "shots.mp4",
        ],
    );
    let asset = ok(
        &client,
        "media_import",
        json!({ "files": [dir.join("shots.mp4")] }),
    )
    .await["results"][0]["asset"]
        .clone();
    let shots = ok(
        &client,
        "edit",
        json!({ "commands": [{ "command": "clip.add", "args": { "asset": asset, "start": "20s" } }] }),
    )
    .await["results"][0]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let split = ok(
        &client,
        "auto_edit",
        json!({ "action": "split_scenes", "clip": shots }),
    )
    .await;
    assert_eq!(
        split["scenes"]["cuts"],
        json!([21 * 705_600_000_i64]),
        "{split}"
    );
    assert!(split["committed"].as_bool().unwrap());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn recordings_start_only_when_confirmed_and_stop_keeping_what_was_recorded() {
    if !has_ffmpeg() {
        return;
    }
    let dir = workspace("record");
    let client = connect(None).await;
    let listed = ok(&client, "record", json!({ "action": "devices" })).await;
    assert!(listed["devices"].is_array(), "{listed}");
    // Agents read the user's preferences, without a project.
    let preferences = ok(&client, "query", json!({ "name": "preferences.read" })).await;
    assert!(
        preferences["preferences"]["editing"]["loudness_target"].is_number(),
        "{preferences}"
    );
    assert!(
        preferences["path"]
            .as_str()
            .unwrap()
            .ends_with("preferences.json")
    );
    ok(
        &client,
        "project_new",
        json!({ "path": dir.join("Voice.fosscut") }),
    )
    .await;
    let voice = json!({
        "action": "start",
        "what": "microphone",
        "input": "lavfi:sine=frequency=440:sample_rate=48000",
        "duration": "1s",
    });
    let refused = fails(&client, "record", voice.clone()).await;
    assert_eq!(refused["code"], "fosscut.record.confirmation_required");
    let mut confirmed = voice.clone();
    confirmed["confirm"] = json!(true);
    let started = ok(&client, "record", confirmed).await;
    assert_eq!(started["kind"], "recording", "{started}");
    let done = ok(
        &client,
        "job_status",
        json!({ "job": started["job"], "wait_seconds": 30 }),
    )
    .await;
    assert_eq!(done["state"], "succeeded", "{done}");
    let output = done["recording"]["output"].as_str().unwrap().to_string();
    assert!(output.contains("recordings"), "{output}");
    assert_eq!(done["recording"]["duration"], 705_600_000);
    let imported = ok(&client, "media_import", json!({ "files": [output] })).await;
    assert_eq!(imported["results"][0]["asset"], "asset_0001", "{imported}");

    // An open-ended recording stops on job_cancel and keeps what it recorded.
    let open = ok(
        &client,
        "record",
        json!({ "action": "start", "what": "microphone", "input": "lavfi:sine=frequency=220:sample_rate=48000", "confirm": true }),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let stopped = ok(
        &client,
        "job_cancel",
        json!({ "job": open["job"], "wait_seconds": 30 }),
    )
    .await;
    assert_eq!(stopped["state"], "succeeded", "{stopped}");
    assert_eq!(stopped["recording"]["stopped"], true);
    assert!(std::path::Path::new(stopped["recording"]["output"].as_str().unwrap()).is_file());
    let wrong = fails(
        &client,
        "record",
        json!({ "action": "start", "what": "radio", "confirm": true }),
    )
    .await;
    assert_eq!(wrong["code"], "fosscut.request.invalid_arguments");
    client.cancel().await.unwrap();
}

/// The folder of the installed local models, or `None` to skip when they are missing
/// locally; CI sets `FOSSCUT_REQUIRE_MODELS=1`, which fails instead.
fn local_models() -> Option<PathBuf> {
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_MODELS").is_some_and(|v| !v.is_empty() && v != "0");
    let dir = std::env::var_os("FOSSCUT_MODELS_DIR").map(PathBuf::from);
    let installed = dir.as_ref().is_some_and(|dir| {
        ["onnxruntime", "kokoro-82m", "opus-mt-en-hu"]
            .iter()
            .all(|model| dir.join(model).is_dir())
    });
    assert!(
        installed || !required,
        "FOSSCUT_REQUIRE_MODELS is set but the local models are not in FOSSCUT_MODELS_DIR"
    );
    installed.then(|| dir.unwrap())
}

#[tokio::test]
async fn agents_speak_text_and_translate_captions_with_local_models() {
    let Some(models) = local_models() else {
        return;
    };
    if !has_ffmpeg() {
        return;
    }
    let dir = workspace("voice");
    let client = connect_with_models(None, models).await;
    ok(
        &client,
        "project_new",
        json!({ "path": dir.join("Voice.fosscut") }),
    )
    .await;
    let voices = ok(&client, "record", json!({ "action": "voices" })).await;
    assert!(
        voices["voices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["name"] == "af_heart"),
        "{voices}"
    );
    let spoken = ok(
        &client,
        "record",
        json!({ "action": "speak", "text": "Welcome to the show.", "voice": "af_heart", "at": "0s" }),
    )
    .await;
    assert!(spoken["committed"].as_bool().unwrap(), "{spoken}");
    assert!(spoken["speech"]["asset"].is_string());
    let unknown = fails(
        &client,
        "record",
        json!({ "action": "speak", "text": "Hi.", "voice": "nobody" }),
    )
    .await;
    assert_eq!(unknown["code"], "fosscut.voice.unknown_voice", "{unknown}");

    std::fs::write(
        dir.join("show.srt"),
        "1\n00:00:01,000 --> 00:00:03,000\nThe dog is sleeping.\n",
    )
    .unwrap();
    ok(
        &client,
        "captions",
        json!({ "action": "import", "file": dir.join("show.srt") }),
    )
    .await;
    let translated = ok(
        &client,
        "captions",
        json!({ "action": "translate", "to": "hu", "mode": "replace" }),
    )
    .await;
    assert!(translated["committed"].as_bool().unwrap(), "{translated}");
    let read = ok(&client, "timeline_read", json!({ "detail": "full" })).await;
    let page = read["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .find(|c| c["kind"] == "caption")
        .unwrap();
    assert!(
        page["text"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("kutya"),
        "{page}"
    );
    let wrong = fails(
        &client,
        "captions",
        json!({ "action": "translate", "to": "fr" }),
    )
    .await;
    assert_eq!(wrong["code"], "fosscut.request.invalid_arguments");
    client.cancel().await.unwrap();
}
