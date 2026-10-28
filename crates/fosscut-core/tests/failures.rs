//! Failure paths of the session: refused requests, history that cannot apply, and recovery
//! from saves interrupted at different points (ADR-0006).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{Request, Session, codes};
use fosscut_model::{
    Id, IdKind, MANIFEST_PATH, MEDIA_PATH, MemoryStore, OPS_LOG_PATH, Project, ProjectStore,
    TimelineFormat,
};
use fosscut_time::Rate;
use serde_json::{Value, json};
use support::*;

fn format(width: u32) -> TimelineFormat {
    TimelineFormat {
        width,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    }
}

#[test]
fn create_refuses_existing_and_invalid_projects() {
    let session = new_session();
    let project = Project::new(Id::new(IdKind::Project, 1), "Again", format(1920));
    let Err(error) = Session::create(session.store().clone(), project) else {
        panic!("creating over an existing project must fail");
    };
    assert_eq!(error.code, codes::REQUEST_INVALID);
    assert!(error.hint.is_some());

    let invalid = Project::new(Id::new(IdKind::Project, 1), "No canvas", format(0));
    let Err(error) = Session::create(MemoryStore::new(), invalid) else {
        panic!("an invalid project must be refused");
    };
    assert_eq!(error.code, codes::MODEL_INVALID);
    assert_eq!(error.details[0]["code"], json!("fosscut.model.canvas"));
}

#[test]
fn malformed_requests_are_refused() {
    let mut session = new_session();
    let mut empty = request("text.add", json!({}));
    empty.commands.clear();
    let error = session.execute(empty).unwrap_err();
    assert_eq!(error.code, codes::REQUEST_INVALID);

    let mixed: Request = serde_json::from_value(json!({
        "commands": [
            { "command": "history.undo", "args": {} },
            { "command": "text.add", "args": { "text": "Hi" } }
        ]
    }))
    .unwrap();
    let error = session.execute(mixed).unwrap_err();
    assert_eq!(error.code, codes::REQUEST_INVALID);
    assert_eq!(session.revision(), 0, "nothing was committed");
}

#[test]
fn redo_needs_an_undone_entry_and_history_dry_runs_change_nothing() {
    let (mut session, _, clip) = session_with_clip();
    let error = session
        .execute(request("history.redo", json!({})))
        .unwrap_err();
    assert_eq!(error.code, codes::NOTHING_TO_REDO);

    let before = documents(&session);
    let mut undo = request("history.undo", json!({}));
    undo.dry_run = true;
    let preview = session.execute(undo).unwrap();
    assert!(preview.dry_run && !preview.committed);
    // The first clip also created the primary video track, so undo removes both.
    assert!(
        preview.changes.removed.contains(&clip),
        "{:?}",
        preview.changes
    );
    assert!(
        preview
            .changes
            .removed
            .iter()
            .any(|id| id.starts_with("track_"))
    );
    assert_eq!(documents(&session), before);
}

#[test]
fn edits_outside_the_log_stop_undo() {
    let (session, _, _) = session_with_clip();
    let mut store = session.store().clone();
    let path = format!(
        "timelines/{}.json",
        session.project().manifest.primary_timeline
    );
    let mut timeline: Value = serde_json::from_slice(&store.read(&path).unwrap()).unwrap();
    timeline["name"] = json!("Renamed by hand");
    store
        .write_atomic(&path, timeline.to_string().as_bytes())
        .unwrap();

    let mut reopened = Session::open(store).unwrap();
    let error = reopened
        .execute(request("history.undo", json!({})))
        .unwrap_err();
    assert_eq!(error.code, codes::HISTORY_DIVERGED);
}

#[test]
fn requests_that_change_nothing_are_not_committed() {
    let (mut session, _, clip) = session_with_clip();
    let revision = session.revision();
    let log_len = session.log().len();
    let response = run(
        &mut session,
        "clip.move",
        json!({ "clip": clip, "start": "0f" }),
    );
    assert!(!response.committed && !response.dry_run);
    assert_eq!(response.revision, revision);
    assert_eq!(session.log().len(), log_len);
}

/// A crash after the log entry was written but while documents were half written: the
/// torn timeline document is rebuilt from the log entry.
#[test]
fn torn_documents_are_rebuilt_from_the_log() {
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
    let path = format!("timelines/{}.json", expected.manifest.primary_timeline);
    crashed.write_atomic(&path, b"{\"id\": \"timel").unwrap();

    let recovered = Session::open(crashed).unwrap();
    assert_eq!(recovered.project(), &expected);
}

/// The first import creates `media.json`; a crash before it exists is recovered too.
#[test]
fn a_missing_media_document_is_rebuilt_from_the_log() {
    let mut session = new_session();
    let before = session.store().clone();
    run(&mut session, "media.add", test_pattern_args());
    let expected = session.project().clone();

    let mut crashed = MemoryStore::new();
    for path in before.paths() {
        if path != MEDIA_PATH {
            crashed
                .write_atomic(&path, &before.read(&path).unwrap())
                .unwrap();
        }
    }
    let log = session.store().read(OPS_LOG_PATH).unwrap();
    crashed.write_atomic(OPS_LOG_PATH, &log).unwrap();

    let recovered = Session::open(crashed).unwrap();
    assert_eq!(recovered.project(), &expected);
}

#[test]
fn a_malformed_manifest_is_reported() {
    let (session, _, _) = session_with_clip();
    let mut store = session.store().clone();
    store.write_atomic(MANIFEST_PATH, b"not json").unwrap();
    let Err(error) = Session::open(store) else {
        panic!("a malformed manifest must not open");
    };
    assert_eq!(error.code, "fosscut.project.malformed");
}

#[test]
fn unknown_queries_suggest_close_names() {
    let session = new_session();
    let error = session.query("timeline.reed", Value::Null).unwrap_err();
    assert_eq!(error.code, codes::UNKNOWN_QUERY);
    assert!(
        error
            .hint
            .as_deref()
            .unwrap_or("")
            .contains("timeline.read"),
        "{error:?}"
    );
}
