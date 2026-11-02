//! Web compositions in the core (ADR-0039): `media.add` takes web assets, video clips show
//! them from a source position like video, and they edit like video (split, trim, replace,
//! speed), while other clip kinds refuse them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{Session, codes};
use fosscut_model::{AssetKind, ClipContent, MemoryStore};
use serde_json::{Value, json};
use support::*;

/// A 6 s, 1080x1920, 30 fps web composition copied into the bundle.
fn web_args() -> Value {
    json!({
        "name": "promo",
        "kind": "web",
        "path": { "relative": "web/0123456789abcdef/index.html" },
        "identity": { "size": 2_048, "sample_blake3": "ef".repeat(32) },
        "info": {
            "duration": 6 * 705_600_000i64,
            "container": "html",
            "video": { "width": 1080, "height": 1920, "rate": "30", "codec": "html" }
        }
    })
}

fn clip_content(session: &Session<MemoryStore>, id: &str) -> ClipContent {
    session
        .project()
        .timelines
        .values()
        .flat_map(|t| t.tracks.iter().flat_map(|tr| &tr.clips))
        .find(|c| c.id.as_str() == id)
        .map(|c| c.content.clone())
        .unwrap()
}

#[test]
fn web_assets_are_added_and_shown_by_video_clips() {
    let mut session = new_session();
    let added = run(&mut session, "media.add", web_args());
    let asset = added.results[0]["asset"].as_str().unwrap().to_string();
    assert_eq!(
        session
            .project()
            .media
            .get(&asset.parse().unwrap())
            .unwrap()
            .kind,
        AssetKind::Web
    );
    // The same content again is the same asset.
    let again = run(&mut session, "media.add", web_args());
    assert_eq!(again.results[0]["existing"], true);

    let placed = run(
        &mut session,
        "clip.add",
        json!({ "asset": asset, "source_in": { "seconds": "1" }, "duration": { "seconds": "3" } }),
    );
    let clip = placed.results[0]["clip"].as_str().unwrap().to_string();
    match clip_content(&session, &clip) {
        ClipContent::Video {
            asset: shown,
            source_in,
        } => {
            assert_eq!(shown.as_str(), asset);
            assert_eq!(source_in, fosscut_time::Flicks::from_seconds(1).unwrap());
        }
        other => panic!("a web clip is a video clip, not {other:?}"),
    }
    // It splits and changes speed like video.
    run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": { "seconds": "2" } }),
    );
    run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "rate": 0.5, "ripple": true }),
    );
    assert!(fosscut_model::validate(session.project()).is_empty());
    let read = run_query(&mut session, "media.list");
    let listed = &read["assets"][0];
    assert_eq!(listed["kind"], "web");
}

fn run_query(session: &mut Session<MemoryStore>, name: &str) -> Value {
    session.query(name, Value::Null).unwrap()
}

#[test]
fn web_assets_need_their_size_rate_and_length() {
    let mut session = new_session();
    let mut no_length = web_args();
    no_length["info"]
        .as_object_mut()
        .unwrap()
        .remove("duration");
    let error = session
        .execute(request("media.add", no_length))
        .unwrap_err();
    assert_eq!(error.code, codes::MEDIA_INCOMPLETE);
    let mut no_size = web_args();
    no_size["info"].as_object_mut().unwrap().remove("video");
    assert_eq!(
        session
            .execute(request("media.add", no_size))
            .unwrap_err()
            .code,
        codes::MEDIA_INCOMPLETE
    );
}

#[test]
fn video_clips_swap_between_video_files_and_web_compositions() {
    let mut session = new_session();
    let web = run(&mut session, "media.add", web_args()).results[0]["asset"]
        .as_str()
        .unwrap()
        .to_string();
    let video = run(&mut session, "media.add", test_pattern_args()).results[0]["asset"]
        .as_str()
        .unwrap()
        .to_string();
    let clip = run(
        &mut session,
        "clip.add",
        json!({ "asset": video, "duration": { "seconds": "2" } }),
    )
    .results[0]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut session,
        "clip.replace",
        json!({ "clip": clip, "asset": web }),
    );
    match clip_content(&session, &clip) {
        ClipContent::Video { asset, .. } => assert_eq!(asset.as_str(), web),
        other => panic!("{other:?}"),
    }
    // A timeline made from the composition takes its canvas and rate.
    let made = run(
        &mut session,
        "timeline.add",
        json!({ "assets": [web], "name": "Vertical" }),
    );
    let timeline = made.results[0]["timeline"].as_str().unwrap();
    let doc = &session.project().timelines[&timeline.parse().unwrap()];
    assert_eq!((doc.format.width, doc.format.height), (1080, 1920));
    assert_eq!(
        doc.duration(),
        fosscut_time::Flicks::from_seconds(6).unwrap()
    );
}
