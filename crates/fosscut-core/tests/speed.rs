//! Clip speed and freeze frames (ADR-0023): `clip.speed` and `clip.freeze`, and how
//! splits and trims move the source through the speed map.

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

fn clips(session: &Session<MemoryStore>) -> Vec<Value> {
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    full["tracks"][0]["clips"].as_array().unwrap().clone()
}

fn flicks(value: &Value) -> i64 {
    value["flicks"].as_i64().unwrap()
}

#[test]
fn constant_speed_keeps_the_media_and_changes_the_length() {
    let (mut session, _, clip) = session_with_clip();
    let fast = run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "rate": 2 }),
    );
    let result = &fast.results[0];
    assert_eq!(flicks(&result["placement"]["duration"]), 4 * SECOND);
    assert_eq!(flicks(&result["source_used"]), 8 * SECOND);
    assert_eq!(result["speed"]["rate"], 2.0);
    let view = &clips(&session)[0];
    assert_eq!(view["speed"]["rate"], 2.0);
    assert_eq!(flicks(&view["source_used"]), 8 * SECOND);
    let slow = run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "rate": 0.5, "pitch": "shift", "interpolate": "blend" }),
    );
    assert_eq!(
        flicks(&slow.results[0]["placement"]["duration"]),
        16 * SECOND
    );
    let view = &clips(&session)[0];
    assert_eq!(view["speed"]["pitch"], "shift");
    assert_eq!(view["speed"]["interpolate"], "blend");
    let reset = run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "reset": true }),
    );
    assert!(reset.results[0].get("speed").is_none());
    let view = &clips(&session)[0];
    assert!(view.get("speed").is_none() && view.get("source_used").is_none());
    assert_eq!(flicks(&view["duration"]), 8 * SECOND);
}

#[test]
fn growing_into_the_next_clip_needs_ripple() {
    let (mut session, _, clip) = session_with_clip();
    let split = run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "4s" }),
    );
    let second = split.results[0]["right"]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let error = fails(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "rate": 0.5 }),
    );
    assert_eq!(error.code, codes::CLIP_OVERLAP);
    let rippled = run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "rate": 0.5, "ripple": true }),
    );
    assert_eq!(rippled.results[0]["moved"], json!([second]));
    let after = clips(&session);
    assert_eq!(
        flicks(&after[1]["start"]),
        8 * SECOND,
        "moved by four seconds"
    );
    // Faster, with ripple, closes the gap again.
    run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "rate": 1, "ripple": true }),
    );
    assert_eq!(flicks(&clips(&session)[1]["start"]), 4 * SECOND);
}

#[test]
fn curves_presets_and_reverse() {
    let (mut session, _, clip) = session_with_clip();
    let hero = run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "preset": "hero" }),
    );
    let used = flicks(&hero.results[0]["source_used"]);
    assert!((used - 8 * SECOND).abs() < SECOND / 10, "{used}");
    let view = &clips(&session)[0];
    assert_eq!(view["speed"]["start_rate"], 2.0);
    assert_eq!(view["speed"]["curve"].as_array().unwrap().len(), 6);
    let curve = run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "curve": [{ "at": 0, "rate": 1 }, { "at": 1, "rate": 3 }], "reverse": true }),
    );
    // A mean rate of 2 halves the length.
    let length = flicks(&curve.results[0]["placement"]["duration"]);
    assert!((length - 4 * SECOND).abs() <= SECOND / 30, "{length}");
    assert_eq!(clips(&session)[0]["speed"]["reverse"], true);
    let forwards = run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "reverse": false }),
    );
    assert!(forwards.results[0]["speed"].get("reverse").is_none());
    for (args, case) in [
        (json!({ "rate": 0 }), "freeze through clip.freeze"),
        (json!({ "rate": 200 }), "too fast"),
        (json!({ "rate": 2, "preset": "hero" }), "two at once"),
        (json!({ "preset": "warp" }), "unknown preset"),
        (
            json!({ "curve": [{ "at": 0.5, "rate": 1 }, { "at": 0.2, "rate": 2 }] }),
            "out of order",
        ),
        (json!({ "curve": [{ "at": 0, "rate": 1 }] }), "one point"),
        (json!({ "reset": true, "rate": 2 }), "reset alone"),
        (json!({}), "nothing to do"),
    ] {
        let mut args = args;
        args["clip"] = json!(clip);
        assert_eq!(
            fails(&mut session, "clip.speed", args).code,
            codes::INVALID_ARGUMENTS,
            "{case}"
        );
    }
    let title = run(&mut session, "text.add", json!({ "text": "Hi" }));
    let error = fails(
        &mut session,
        "clip.speed",
        json!({ "clip": title.results[0]["clip"], "rate": 2 }),
    );
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);
    let caps = session
        .query("capabilities", json!({ "detail": "full" }))
        .unwrap();
    let presets = caps["speed_presets"].as_array().unwrap();
    assert_eq!(presets.len(), 6);
    assert!(
        presets
            .iter()
            .all(|p| p["curve"].as_array().unwrap().len() >= 3)
    );
}

#[test]
fn splits_and_trims_follow_the_speed_map() {
    let (mut session, _, clip) = session_with_clip();
    run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "rate": 2 }),
    );
    run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "1s" }),
    );
    let parts = clips(&session);
    assert_eq!(
        flicks(&parts[1]["source_in"]),
        2 * SECOND,
        "two seconds of media in"
    );
    assert_eq!(parts[1]["speed"]["rate"], 2.0);
    assert_eq!(flicks(&parts[0]["source_used"]), 2 * SECOND);
    let right = parts[1]["id"].as_str().unwrap().to_string();
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": right, "start": "1.5s" }),
    );
    assert_eq!(flicks(&clips(&session)[1]["source_in"]), 3 * SECOND);
    let past = fails(
        &mut session,
        "clip.trim",
        json!({ "clip": right, "end": "5s" }),
    );
    assert_eq!(
        past.code,
        codes::CLIP_SOURCE_RANGE,
        "seven seconds of media past the end"
    );
    // Reversed, the left part shows the end of the media.
    run(
        &mut session,
        "clip.speed",
        json!({ "clip": clip, "reverse": true }),
    );
    run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "0.5s" }),
    );
    let parts = clips(&session);
    assert_eq!(flicks(&parts[0]["source_in"]), SECOND);
    assert_eq!(flicks(&parts[1]["source_in"]), 0);
}

#[test]
fn freeze_frames_hold_a_frame_and_push_what_follows() {
    let (mut session, _, clip) = session_with_clip();
    let frozen = run(
        &mut session,
        "clip.freeze",
        json!({ "clip": clip, "at": "3s", "duration": "1s" }),
    );
    let result = &frozen.results[0];
    assert_eq!(flicks(&result["freeze"]["start"]), 3 * SECOND);
    let parts = clips(&session);
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[1]["speed"]["rate"], 0.0);
    assert_eq!(flicks(&parts[1]["source_in"]), 3 * SECOND);
    assert_eq!(flicks(&parts[1]["source_used"]), 0);
    assert_eq!(
        flicks(&parts[2]["start"]),
        4 * SECOND,
        "the rest moved later"
    );
    assert_eq!(
        flicks(&parts[2]["source_in"]),
        3 * SECOND,
        "and goes on where it was"
    );
    assert_eq!(result["after"]["clip"], parts[2]["id"]);
    // At the end, the last frame holds; by default for two seconds.
    let last = parts[2]["id"].as_str().unwrap().to_string();
    let end = run(
        &mut session,
        "clip.freeze",
        json!({ "clip": last, "at": "9s" }),
    );
    assert!(end.results[0].get("after").is_none());
    let parts = clips(&session);
    assert_eq!(flicks(&parts[3]["duration"]), 2 * SECOND);
    assert_eq!(
        flicks(&parts[3]["source_in"]),
        8 * SECOND - SECOND / 30,
        "the last frame"
    );
    let outside = fails(
        &mut session,
        "clip.freeze",
        json!({ "clip": clip, "at": "6s" }),
    );
    assert_eq!(outside.code, codes::CLIP_SPLIT_OUTSIDE);
    let none = fails(
        &mut session,
        "clip.freeze",
        json!({ "clip": clip, "at": "1s", "duration": "0s" }),
    );
    assert_eq!(none.code, codes::INVALID_ARGUMENTS);
    let title = run(&mut session, "text.add", json!({ "text": "Hi" }));
    let text = fails(
        &mut session,
        "clip.freeze",
        json!({ "clip": title.results[0]["clip"], "at": "0s" }),
    );
    assert_eq!(text.code, codes::INVALID_ARGUMENTS);
}
