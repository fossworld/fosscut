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
    assert_eq!(names, ["fosscut-basics", "viral-captions"]);
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
    assert_eq!(resources.len(), 2);
    assert_eq!(resources[0].uri, "fosscut://skills/fosscut-basics");
    assert_eq!(resources[1].uri, "fosscut://skills/viral-captions");
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
    assert_eq!(models.len(), 4);
    assert!(models.iter().all(|m| m["installed"] == json!(false)));
    assert!(models.iter().all(|m| m["license"] == json!("MIT")));

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
