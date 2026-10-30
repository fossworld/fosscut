//! Compound clips (ADR-0025): `clip.compound`, `clip.decompose`, placing a compound
//! timeline with `clip.add`, and keeping a timeline that compound clips show.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{ApiError, Session, codes};
use fosscut_model::MemoryStore;
use serde_json::{Value, json};
use support::*;

const SECOND: i64 = 705_600_000;

fn fails(session: &mut Session<MemoryStore>, command: &str, args: Value) -> ApiError {
    session.execute(request(command, args)).expect_err(command)
}

fn read(session: &Session<MemoryStore>, timeline: Option<&str>) -> Value {
    let args = match timeline {
        Some(t) => json!({ "detail": "full", "timeline": t }),
        None => json!({ "detail": "full" }),
    };
    session.query("timeline.read", args).unwrap()
}

#[test]
fn chosen_clips_become_one_compound_clip_and_come_back() {
    let (mut session, _, video) = session_with_clip();
    let title = run(
        &mut session,
        "text.add",
        json!({ "text": "Hello", "start": "2s", "duration": "3s" }),
    );
    let title = title.results[0]["clip"].as_str().unwrap().to_string();
    let made = run(
        &mut session,
        "clip.compound",
        json!({ "clips": [video, title] }),
    );
    let result = &made.results[0];
    let nested = result["timeline"].as_str().unwrap().to_string();
    let compound = result["compound"]["clip"].as_str().unwrap().to_string();
    assert_eq!(result["compound"]["duration"]["seconds"], "8");
    let outer = read(&session, None);
    let clips: Vec<&Value> = outer["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap())
        .collect();
    assert_eq!(clips.len(), 1, "one clip in their place");
    assert_eq!(clips[0]["kind"], "timeline");
    assert_eq!(clips[0]["timeline"], nested);
    assert_eq!(clips[0]["timeline_name"], "Compound 1");
    let inner = read(&session, Some(&nested));
    assert_eq!(
        inner["tracks"].as_array().unwrap().len(),
        2,
        "both tracks inside"
    );
    assert_eq!(inner["tracks"][1]["clips"][0]["start"]["seconds"], "2");
    let info = session.query("project.info", json!({})).unwrap();
    assert!(
        info["timelines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == nested && t["role"] == "compound")
    );
    // Trimmed compound clips read their timeline through the map.
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": compound, "start": "1s" }),
    );
    let past = fails(
        &mut session,
        "clip.trim",
        json!({ "clip": compound, "end": "12s" }),
    );
    assert_eq!(
        past.code,
        codes::CLIP_SOURCE_RANGE,
        "past the timeline's end"
    );
    // A second use of the same compound timeline.
    let again = run(
        &mut session,
        "clip.add",
        json!({ "compound": nested, "start": "10s" }),
    );
    let second = again.results[0]["clip"].as_str().unwrap().to_string();
    let busy = fails(
        &mut session,
        "timeline.remove",
        json!({ "timeline": nested }),
    );
    assert_eq!(busy.code, codes::TIMELINE_IN_USE);
    // Decomposing the trimmed one puts back what it showed, from 1 s on.
    let back = run(&mut session, "clip.decompose", json!({ "clip": compound }));
    let placed = back.results[0]["clips"].as_array().unwrap();
    assert_eq!(placed.len(), 2);
    assert_eq!(placed[0]["start"]["seconds"], "1");
    assert_eq!(placed[0]["duration"]["seconds"], "7");
    assert!(
        back.results[0].get("removed_timeline").is_none(),
        "still shown"
    );
    let last = run(&mut session, "clip.decompose", json!({ "clip": second }));
    assert_eq!(last.results[0]["removed_timeline"], nested);
    assert!(
        session
            .query("timeline.read", json!({ "timeline": nested }))
            .is_err()
    );
}

#[test]
fn compound_clips_check_what_they_show() {
    let (mut session, _, video) = session_with_clip();
    let made = run(&mut session, "clip.compound", json!({ "clips": [video] }));
    let compound = made.results[0]["compound"]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let nested = made.results[0]["timeline"].as_str().unwrap().to_string();
    let primary = session.query("project.info", json!({})).unwrap()["primary_timeline"].clone();
    for (args, code) in [
        (json!({ "compound": primary }), codes::INVALID_ARGUMENTS),
        (
            json!({ "compound": "timeline_0999" }),
            codes::TIMELINE_NOT_FOUND,
        ),
        (
            json!({ "compound": nested, "asset": "asset_0001" }),
            codes::INVALID_ARGUMENTS,
        ),
        (json!({}), codes::INVALID_ARGUMENTS),
        (
            json!({ "compound": nested, "timeline": nested }),
            codes::INVALID_ARGUMENTS,
        ),
    ] {
        assert_eq!(
            fails(&mut session, "clip.add", args.clone()).code,
            code,
            "{args}"
        );
    }
    run(
        &mut session,
        "clip.set",
        json!({ "clip": compound, "set": { "opacity": 0.5 } }),
    );
    let own = fails(&mut session, "clip.decompose", json!({ "clip": compound }));
    assert_eq!(
        own.code,
        codes::INVALID_ARGUMENTS,
        "its opacity would be lost"
    );
    let words = run(&mut session, "text.add", json!({ "text": "Hi" }));
    let plain = fails(
        &mut session,
        "clip.decompose",
        json!({ "clip": words.results[0]["clip"] }),
    );
    assert_eq!(plain.code, codes::INVALID_ARGUMENTS);
    let none = fails(&mut session, "clip.compound", json!({ "clips": [] }));
    assert_eq!(none.code, codes::INVALID_ARGUMENTS);
    let view = read(&session, None);
    assert_eq!(
        view["tracks"][0]["clips"][0]["duration"]["flicks"],
        8 * SECOND
    );
    // Speed and freezes apply to a compound clip as a whole.
    run(
        &mut session,
        "clip.set",
        json!({ "clip": compound, "set": { "opacity": 1 } }),
    );
    let fast = run(
        &mut session,
        "clip.speed",
        json!({ "clip": compound, "rate": 2 }),
    );
    assert_eq!(fast.results[0]["placement"]["duration"]["seconds"], "4");
    let held = run(
        &mut session,
        "clip.freeze",
        json!({ "clip": compound, "at": "1s", "duration": "1s" }),
    );
    let view = read(&session, None);
    let frozen = view["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == held.results[0]["freeze"]["clip"])
        .unwrap()
        .clone();
    assert_eq!(frozen["kind"], "timeline");
    assert_eq!(
        frozen["source_in"]["seconds"], "2",
        "the frame at 1 s, 2 s inside"
    );
}
