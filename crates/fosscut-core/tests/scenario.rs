//! Milestone 1 exit scenario at the core level: the round 1 audit edit as commands,
//! with revisions, undo/redo, persistence and crash recovery.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{Session, codes};
use fosscut_model::{MemoryStore, OPS_LOG_PATH, ProjectStore};
use serde_json::json;
use support::*;

#[test]
fn round_one_edit_sequence() {
    let mut session = new_session();
    let asset = run(&mut session, "media.add", test_pattern_args()).results[0]["asset"].clone();
    let placed = run(&mut session, "clip.add", json!({ "asset": asset }));
    assert_eq!(placed.results[0]["end"]["frames"], json!(240));
    let title = run(
        &mut session,
        "text.add",
        json!({ "text": "FossCut audit", "start": { "frames": 0 }, "duration": { "seconds": "3" } }),
    );
    assert_eq!(title.results[0]["end"]["timecode"], json!("00:00:03:00"));
    let split = run(
        &mut session,
        "clip.split",
        json!({ "clip": placed.results[0]["clip"], "at": { "timecode": "00:00:03:28" } }),
    );
    assert_eq!(split.results[0]["left"]["end"]["frames"], json!(118));
    assert_eq!(split.results[0]["right"]["start"]["frames"], json!(118));
    assert_eq!(split.results[0]["right"]["end"]["frames"], json!(240));
    assert_eq!(session.revision(), 4);

    let view = session.query("timeline.read", json!({})).unwrap();
    insta::assert_json_snapshot!("round_one_timeline", view);

    let undo = run(&mut session, "history.undo", json!({}));
    assert_eq!(undo.revision, 5);
    let view = session
        .query("timeline.read", json!({ "detail": "outline" }))
        .unwrap();
    assert_eq!(view["tracks"][0]["clip_count"], json!(1));
    run(&mut session, "history.redo", json!({}));
    let view = session
        .query("timeline.read", json!({ "detail": "outline" }))
        .unwrap();
    assert_eq!(view["tracks"][0]["clip_count"], json!(2));

    let history = session.query("history.list", json!({})).unwrap();
    let kinds: Vec<&str> = history["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["redo", "undo", "commit", "commit", "commit", "commit"]
    );
}

#[test]
fn edits_snap_to_frames_and_never_overwrite() {
    let (mut session, asset, _) = session_with_clip();
    let error = session
        .execute(request(
            "clip.add",
            json!({ "asset": asset, "start": "2s", "duration": "1s" }),
        ))
        .unwrap_err();
    assert_eq!(error.code, codes::CLIP_OVERLAP);
    assert_eq!(error.details["free_from"]["frames"], json!(240));
    let placed = run(
        &mut session,
        "clip.add",
        json!({ "asset": asset, "duration": { "seconds": "1.01" } }),
    );
    assert_eq!(
        placed.results[0]["duration"]["frames"],
        json!(30),
        "1.01 s snaps to 30 frames"
    );
    let error = session
        .execute(request(
            "clip.add",
            json!({ "asset": asset, "source_in": "7s", "duration": "2s" }),
        ))
        .unwrap_err();
    assert_eq!(error.code, codes::CLIP_SOURCE_RANGE);
}

#[test]
fn split_outside_the_clip_is_rejected() {
    let (mut session, _, clip) = session_with_clip();
    for at in ["0f", "240f", "300f"] {
        let error = session
            .execute(request("clip.split", json!({ "clip": clip, "at": at })))
            .unwrap_err();
        assert_eq!(error.code, codes::CLIP_SPLIT_OUTSIDE, "{at}");
    }
}

#[test]
fn history_and_retries_survive_reopening() {
    let (mut session, _, clip) = session_with_clip();
    let mut split = request("clip.split", json!({ "clip": clip, "at": "100f" }));
    split.request_id = Some("split-1".into());
    let first = session.execute(split.clone()).unwrap();
    let store: MemoryStore = session.store().clone();

    let mut reopened = Session::open(store).unwrap();
    assert_eq!(reopened.project(), session.project());
    let replay = reopened.execute(split).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.results, first.results);
    run(&mut reopened, "history.undo", json!({}));
    let view = reopened
        .query("timeline.read", json!({ "detail": "outline" }))
        .unwrap();
    assert_eq!(view["tracks"][0]["clip_count"], json!(1));
}

#[test]
fn interrupted_saves_are_completed_on_open() {
    let (mut session, _, clip) = session_with_clip();
    let before = session.store().clone();
    run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "100f" }),
    );
    let expected = session.project().clone();

    let mut crashed = before;
    let log = session.store().read(OPS_LOG_PATH).unwrap();
    crashed.write_atomic(OPS_LOG_PATH, &log).unwrap();
    let recovered = Session::open(crashed).unwrap();
    assert_eq!(recovered.project(), &expected);
}

#[test]
fn torn_log_lines_are_ignored() {
    let (session, _, _) = session_with_clip();
    let mut store = session.store().clone();
    store
        .append(OPS_LOG_PATH, b"{\"seq\": 99, \"kind\": \"comm")
        .unwrap();
    let reopened = Session::open(store).unwrap();
    assert_eq!(reopened.log().len(), session.log().len());
}

#[test]
fn nothing_to_undo_is_explained() {
    let mut session = new_session();
    let error = session
        .execute(request("history.undo", json!({})))
        .unwrap_err();
    assert_eq!(error.code, codes::NOTHING_TO_UNDO);
}
