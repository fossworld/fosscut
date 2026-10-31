//! Multicam clips (ADR-0029) and bulk keyframes: angles lined up by offsets in a compound
//! clip, switching angles by splitting it, and replacing a property's keyframes at once.

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

fn second_angle(session: &mut Session<MemoryStore>) -> String {
    let mut other = test_pattern_args();
    other["identity"]["size"] = json!(77);
    other["name"] = json!("angle-2.mp4");
    let added = run(session, "media.add", other);
    added.results[0]["asset"].as_str().unwrap().to_string()
}

fn read(session: &Session<MemoryStore>, timeline: &Value) -> Value {
    session
        .query(
            "timeline.read",
            json!({ "timeline": timeline, "detail": "full" }),
        )
        .unwrap()
}

#[test]
fn angles_line_up_in_a_compound_clip_and_switch_by_splitting() {
    let mut session = new_session();
    let first = run(&mut session, "media.add", test_pattern_args()).results[0]["asset"]
        .as_str()
        .unwrap()
        .to_string();
    let second = second_angle(&mut session);
    let created = run(
        &mut session,
        "multicam.create",
        json!({ "angles": [first, second], "offsets": ["0s", "1s"], "sound": 2, "name": "Interview" }),
    );
    let made = &created.results[0];
    let inner = read(&session, &made["timeline"]);
    assert_eq!(inner["name"], "Interview");
    let tracks = inner["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 2);
    // The first angle is on top; the second starts a second later; only its sound plays.
    assert_eq!(tracks[1]["id"], made["angles"][0]);
    assert_eq!(tracks[0]["clips"][0]["start"]["flicks"], SECOND);
    assert!(
        tracks[0].get("muted").is_none_or(|m| m == false),
        "angle 2 is heard"
    );
    assert_eq!(tracks[1]["muted"], true, "angle 1 is not");
    let info = session.query("project.info", json!({})).unwrap();
    assert!(
        info["timelines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["role"] == "compound")
    );
    let clip = made["clip"]["clip"].clone();
    let parent = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    let placed = parent["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .find(|c| c["id"] == clip)
        .unwrap();
    assert_eq!(placed["kind"], "timeline");
    // The read shows the angle and every angle, numbered as multicam.switch takes them.
    assert_eq!(placed["angle"], made["angles"][0]);
    assert_eq!(placed["angle_number"], 1);
    let angles = placed["angles"].as_array().unwrap();
    assert_eq!(angles.len(), 2);
    assert_eq!(angles[1]["number"], 2);
    assert_eq!(angles[1]["id"], made["angles"][1]);

    // A compound clip sounds like its timeline, so its volume and fades can be set.
    let set = run(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "volume": 0.5, "audio.fade_in": "0.5s" } }),
    );
    assert!(set.committed, "{set:?}");

    // Switch to angle 2 from 2 s: the clip splits and the right part shows it.
    let switched = run(
        &mut session,
        "multicam.switch",
        json!({ "clip": clip, "at": "2s", "angle": 2 }),
    );
    let right = &switched.results[0];
    assert_eq!(right["angle"], made["angles"][1]);
    assert_eq!(right["clip"]["start"]["flicks"], 2 * SECOND);
    assert_ne!(right["clip"]["clip"], clip);
    // At a part's start, the whole part switches.
    let back = run(
        &mut session,
        "multicam.switch",
        json!({ "clip": right["clip"]["clip"], "at": "2s", "angle": made["angles"][0] }),
    );
    assert_eq!(back.results[0]["clip"]["clip"], right["clip"]["clip"]);

    for (command, args) in [
        ("multicam.create", json!({ "angles": [first] })),
        (
            "multicam.create",
            json!({ "angles": [first, second], "offsets": ["0s"] }),
        ),
        (
            "multicam.create",
            json!({ "angles": [first, second], "sound": 3 }),
        ),
        (
            "multicam.create",
            json!({ "angles": [first, second], "sound": 0 }),
        ),
        (
            "multicam.switch",
            json!({ "clip": clip, "at": "1s", "angle": 5 }),
        ),
        (
            "multicam.switch",
            json!({ "clip": clip, "at": "30s", "angle": 1 }),
        ),
    ] {
        assert_eq!(
            fails(&mut session, command, args.clone()).code,
            codes::MULTICAM_INVALID,
            "{args}"
        );
    }
    let title = run(&mut session, "text.add", json!({ "text": "Not multicam" }));
    let error = fails(
        &mut session,
        "multicam.switch",
        json!({ "clip": title.results[0]["clip"], "at": "0s", "angle": 1 }),
    );
    assert_eq!(error.code, codes::MULTICAM_INVALID);
}

#[test]
fn angles_between_frames_line_up_to_the_sample() {
    let mut session = new_session();
    let first = run(&mut session, "media.add", test_pattern_args()).results[0]["asset"]
        .as_str()
        .unwrap()
        .to_string();
    let second = second_angle(&mut session);
    // 1.75 s is 52.5 frames at 30 fps: the angle starts on frame 53 and skips the half frame
    // at the start of its media.
    let made = run(
        &mut session,
        "multicam.create",
        json!({ "angles": [first, second], "offsets": ["0s", { "seconds": "1.75" }] }),
    );
    let inner = read(&session, &made.results[0]["timeline"]);
    let later = &inner["tracks"][0]["clips"][0];
    assert_eq!(later["start"]["frames"], 53, "{later}");
    assert_eq!(
        later["source_in"]["flicks"],
        53 * SECOND / 30 - 7 * SECOND / 4,
        "{later}"
    );
    assert_eq!(inner["tracks"][1]["clips"][0]["source_in"]["flicks"], 0);
}

#[test]
fn every_keyframe_of_a_property_is_replaced_at_once() {
    let (mut session, _, clip) = session_with_clip();
    let set = run(
        &mut session,
        "keyframe.set_all",
        json!({ "clip": clip, "property": "transform.position", "keys": [
            { "offset": "1s", "value": [0.1, 0.0] },
            { "offset": "0s", "value": [0.0, 0.0], "easing": "hold" },
            { "offset": "2s", "value": [0.2, -0.1] }
        ] }),
    );
    let keys = &set.results[0]["keyframes"]["transform.position"];
    assert_eq!(keys.as_array().unwrap().len(), 3, "{set:?}");
    assert_eq!(keys[0]["value"], json!([0.0, 0.0]), "sorted by time");
    assert_eq!(keys[0]["interpolation"], "hold");
    let cleared = run(
        &mut session,
        "keyframe.set_all",
        json!({ "clip": clip, "property": "transform.position", "keys": [] }),
    );
    assert!(
        cleared.results[0]["keyframes"]
            .get("transform.position")
            .is_none()
    );
    for args in [
        json!({ "clip": clip, "property": "transform.position", "keys": [
            { "offset": "1s", "value": [0, 0] }, { "offset": "1s", "value": [1, 1] }
        ] }),
        json!({ "clip": clip, "property": "blend", "keys": [] }),
        json!({ "clip": clip, "property": "opacity", "keys": [{ "offset": "0s", "value": 5 }] }),
    ] {
        assert_eq!(
            fails(&mut session, "keyframe.set_all", args.clone()).code,
            codes::INVALID_ARGUMENTS,
            "{args}"
        );
    }
}
