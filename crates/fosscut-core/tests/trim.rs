//! `clip.trim`: moving a clip's edges keeps the picture under the rest of the clip in place,
//! stays within the media, never overwrites, and keeps caption words in sync.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{ApiError, Session, codes};
use fosscut_model::MemoryStore;
use serde_json::{Value, json};
use support::*;

const SECOND: i64 = 705_600_000;

fn fails(session: &mut Session<MemoryStore>, command: &str, args: Value) -> ApiError {
    session
        .execute(request(command, args))
        .expect_err("the command should fail")
}

/// The clip as `timeline.read` reports it.
fn clip(session: &Session<MemoryStore>, id: &str) -> Value {
    let timeline = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    timeline["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not on the timeline"))
}

fn edges(session: &Session<MemoryStore>, id: &str) -> (i64, i64, i64) {
    let c = clip(session, id);
    (
        c["start"]["flicks"].as_i64().unwrap(),
        c["end"]["flicks"].as_i64().unwrap(),
        c["source_in"]["flicks"].as_i64().unwrap_or(0),
    )
}

#[test]
fn trimming_the_start_keeps_the_picture_in_place() {
    let (mut session, _, id) = session_with_clip();
    let placed = run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "start": "1s" }),
    );
    assert_eq!(placed.results[0]["start"]["seconds"], "1");
    assert_eq!(placed.results[0]["duration"]["seconds"], "7");
    assert_eq!(
        edges(&session, &id),
        (SECOND, 8 * SECOND, SECOND),
        "the media moved with the edge"
    );

    run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "end": "00:00:05:15" }),
    );
    assert_eq!(
        edges(&session, &id),
        (SECOND, 5 * SECOND + SECOND / 2, SECOND)
    );
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "start": "2s", "end": "4s" }),
    );
    assert_eq!(edges(&session, &id), (2 * SECOND, 4 * SECOND, 2 * SECOND));
}

#[test]
fn edges_extend_into_more_media_but_never_past_it() {
    let (mut session, _, id) = session_with_clip();
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "start": "2s", "end": "6s" }),
    );
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "start": "0s", "end": "8s" }),
    );
    assert_eq!(
        edges(&session, &id),
        (0, 8 * SECOND, 0),
        "both edges back to the whole media"
    );

    let past_end = fails(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "end": "9s" }),
    );
    assert_eq!(past_end.code, codes::CLIP_SOURCE_RANGE);
    assert_eq!(past_end.details["asset_duration"]["seconds"], "8");
    assert!(past_end.hint.is_some());
    run(
        &mut session,
        "clip.move",
        json!({ "clip": id, "start": "3s" }),
    );
    let before_media = fails(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "start": "2s" }),
    );
    assert_eq!(
        before_media.code,
        codes::CLIP_SOURCE_RANGE,
        "the media starts at the clip's start"
    );
    let negative = fails(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "start": { "seconds": "-1" } }),
    );
    assert_eq!(negative.code, codes::CLIP_INVALID_RANGE);
    assert_eq!(
        edges(&session, &id),
        (3 * SECOND, 11 * SECOND, 0),
        "failed trims change nothing"
    );
}

#[test]
fn trims_never_overwrite_the_next_clip() {
    let (mut session, asset, id) = session_with_clip();
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "end": "4s" }),
    );
    run(
        &mut session,
        "clip.add",
        json!({ "asset": asset, "start": "5s", "duration": "2s" }),
    );
    let overlap = fails(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "end": "6s" }),
    );
    assert_eq!(overlap.code, codes::CLIP_OVERLAP);
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "end": "5s" }),
    );
    assert_eq!(
        edges(&session, &id).1,
        5 * SECOND,
        "up to the next clip is fine"
    );
}

#[test]
fn titles_have_no_media_limit() {
    let mut session = new_session();
    let added = run(
        &mut session,
        "text.add",
        json!({ "text": "Hi", "start": "1s", "duration": "2s" }),
    );
    let id = added.results[0]["clip"].as_str().unwrap().to_string();
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "start": "0s", "end": "30s" }),
    );
    let title = clip(&session, &id);
    assert_eq!(
        (
            title["start"]["seconds"].clone(),
            title["end"]["seconds"].clone()
        ),
        (json!("0"), json!("30"))
    );
    assert_eq!(title["text"], "Hi");
}

#[test]
fn caption_words_follow_the_start_and_leave_with_trimmed_time() {
    let mut session = new_session();
    let added = run(
        &mut session,
        "captions.add",
        json!({ "words": [
            { "text": "One", "start": { "seconds": "1" }, "end": { "seconds": "1.4" } },
            { "text": "two", "start": { "seconds": "1.5" }, "end": { "seconds": "1.9" } },
            { "text": "three.", "start": { "seconds": "2" }, "end": { "seconds": "2.5" } },
        ], "paging": { "max_words": 8 } }),
    );
    let id = added.results[0]["pages"][0]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let words = |session: &Session<MemoryStore>| -> Vec<(String, i64)> {
        let page = session
            .project()
            .timelines
            .values()
            .flat_map(|t| &t.tracks)
            .flat_map(|t| &t.clips)
            .find(|c| c.id.to_string() == id)
            .cloned()
            .unwrap();
        let fosscut_model::ClipContent::Caption(caption) = page.content else {
            panic!("not a caption page")
        };
        caption
            .lines
            .iter()
            .flat_map(|l| &l.words)
            .map(|w| (w.text.clone(), w.start.get()))
            .collect()
    };
    let start = edges(&session, &id).0;
    assert_eq!(start, SECOND);

    // Trimming past "One" drops it; the others keep their place on the timeline.
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "start": "00:00:01:15" }),
    );
    assert_eq!(
        words(&session),
        [("two".into(), 0), ("three.".into(), SECOND / 2)]
    );
    // Extending the start back shifts the words, so they still play at the same moments.
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "start": "1s" }),
    );
    assert_eq!(
        words(&session),
        [("two".into(), SECOND / 2), ("three.".into(), SECOND)]
    );
    // Trimming the end drops the last word.
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "end": "00:00:01:27" }),
    );
    assert_eq!(words(&session), [("two".into(), SECOND / 2)]);

    let empty = fails(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "end": "00:00:01:06" }),
    );
    assert_eq!(empty.code, codes::CAPTIONS_INVALID_WORDS);
}

#[test]
fn bad_trims_are_refused() {
    let (mut session, _, id) = session_with_clip();
    assert_eq!(
        fails(&mut session, "clip.trim", json!({ "clip": id })).code,
        codes::INVALID_ARGUMENTS
    );
    let reversed = fails(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "start": "5s", "end": "4s" }),
    );
    assert_eq!(reversed.code, codes::CLIP_INVALID_RANGE);
    let empty = fails(
        &mut session,
        "clip.trim",
        json!({ "clip": id, "end": "0s" }),
    );
    assert_eq!(empty.code, codes::CLIP_INVALID_RANGE);
    let missing = fails(
        &mut session,
        "clip.trim",
        json!({ "clip": "clip_9999", "end": "1s" }),
    );
    assert_eq!(missing.code, codes::CLIP_NOT_FOUND);
    let bare = fails(&mut session, "clip.trim", json!({ "clip": id, "end": 30 }));
    assert_eq!(bare.code, codes::INVALID_ARGUMENTS, "times need a unit");
}
