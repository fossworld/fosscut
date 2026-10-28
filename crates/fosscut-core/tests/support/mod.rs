//! Shared fixtures for core tests.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use fosscut_core::{Request, Response, Session};
use fosscut_model::{Id, IdKind, MemoryStore, Project, TimelineFormat};
use fosscut_time::Rate;
use serde_json::{Value, json};

/// A new project in memory: 1920x1080, 30 fps.
pub fn new_session() -> Session<MemoryStore> {
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let project = Project::new(Id::new(IdKind::Project, 0x1f2e3d), "Test project", format);
    Session::create(MemoryStore::new(), project).unwrap()
}

/// Media arguments for the round 1 fixture: 8 s, 1280x720, 30 fps, H.264 with AAC audio.
pub fn test_pattern_args() -> Value {
    json!({
        "name": "test-pattern.mp4",
        "kind": "video",
        "path": { "absolute": "/fixtures/test-pattern.mp4" },
        "identity": { "size": 1_234_567, "sample_blake3": "ab".repeat(32) },
        "info": {
            "duration": 8 * 705_600_000i64,
            "container": "mov,mp4,m4a,3gp,3g2,mj2",
            "video": { "width": 1280, "height": 720, "rate": "30", "codec": "h264" },
            "audio": { "sample_rate": 48000, "channels": 1, "codec": "aac" }
        }
    })
}

pub fn request(command: &str, args: Value) -> Request {
    let mut request = Request::single(command, args);
    request.actor = "test".into();
    request
}

pub fn run(session: &mut Session<MemoryStore>, command: &str, args: Value) -> Response {
    session
        .execute(request(command, args))
        .unwrap_or_else(|e| panic!("{command} failed: {e:?}"))
}

/// A session with the test pattern imported and placed on the primary video track.
/// Returns the session, the asset id and the clip id.
pub fn session_with_clip() -> (Session<MemoryStore>, String, String) {
    let mut session = new_session();
    let added = run(&mut session, "media.add", test_pattern_args());
    let asset = added.results[0]["asset"].as_str().unwrap().to_string();
    let placed = run(&mut session, "clip.add", json!({ "asset": asset }));
    let clip = placed.results[0]["clip"].as_str().unwrap().to_string();
    (session, asset, clip)
}

/// Stored documents of a session's project, for before/after comparisons.
pub fn documents(session: &Session<MemoryStore>) -> Vec<(String, Vec<u8>)> {
    let store = session.store();
    store
        .paths()
        .into_iter()
        .filter(|p| !p.starts_with("history/"))
        .map(|p| (p.clone(), store.read(&p).unwrap()))
        .collect()
}

use fosscut_model::ProjectStore;
