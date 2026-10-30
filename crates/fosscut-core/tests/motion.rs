//! Transitions and clip animations (ADR-0011, items 1 and 4): `transition.set`,
//! `transition.remove` and `animation.set`, and how splits and cuts keep them at the right
//! edges.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{ApiError, Session, codes};
use fosscut_model::MemoryStore;
use serde_json::{Value, json};
use support::*;

fn fails(session: &mut Session<MemoryStore>, command: &str, args: Value) -> ApiError {
    session.execute(request(command, args)).expect_err(command)
}

fn clips(session: &Session<MemoryStore>) -> Vec<Value> {
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    full["tracks"][0]["clips"].as_array().unwrap().clone()
}

#[test]
fn transitions_need_a_touching_next_clip() {
    let (mut session, _, clip) = session_with_clip();
    let alone = fails(
        &mut session,
        "transition.set",
        json!({ "clip": clip, "type": "dissolve" }),
    );
    assert_eq!(alone.code, codes::TRANSITION_NO_NEIGHBOR);
    run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "4s" }),
    );
    let set = run(
        &mut session,
        "transition.set",
        json!({ "clip": clip, "type": "push", "duration": "1s", "params": { "direction": "up" } }),
    );
    let transition = &set.results[0]["transition"];
    assert_eq!(transition["type"], "push");
    assert_eq!(transition["duration"], 705_600_000);
    assert_eq!(transition["params"]["direction"], "up");
    let view = &clips(&session)[0];
    assert_eq!(view["transition"]["duration"]["seconds"], "1");
    let default = run(
        &mut session,
        "transition.set",
        json!({ "clip": clip, "type": "dip", "params": { "color": "#ffffff" } }),
    );
    assert_eq!(
        default.results[0]["transition"]["duration"], 352_800_000,
        "half a second"
    );
    assert_eq!(
        default.results[0]["transition"]["params"]["color"],
        "#FFFFFF"
    );
    for (args, code, case) in [
        (
            json!({ "type": "morph" }),
            codes::TRANSITION_UNKNOWN_TYPE,
            "unknown type",
        ),
        (
            json!({ "type": "wipe", "duration": "5s" }),
            codes::INVALID_ARGUMENTS,
            "longer than a clip",
        ),
        (
            json!({ "type": "wipe", "params": { "direction": "sideways" } }),
            codes::INVALID_ARGUMENTS,
            "bad choice",
        ),
        (
            json!({ "type": "wipe", "params": { "softness": 3 } }),
            codes::INVALID_ARGUMENTS,
            "out of range",
        ),
        (
            json!({ "type": "wipe", "params": { "speed": 1 } }),
            codes::INVALID_ARGUMENTS,
            "unknown parameter",
        ),
    ] {
        let mut args = args;
        args["clip"] = json!(clip);
        assert_eq!(
            fails(&mut session, "transition.set", args).code,
            code,
            "{case}"
        );
    }
    run(&mut session, "transition.remove", json!({ "clip": clip }));
    assert!(clips(&session)[0].get("transition").is_none());
    let again = fails(&mut session, "transition.remove", json!({ "clip": clip }));
    assert_eq!(again.code, codes::INVALID_ARGUMENTS);
}

#[test]
fn animations_take_presets_in_their_places() {
    let (mut session, _, clip) = session_with_clip();
    let set = run(
        &mut session,
        "animation.set",
        json!({ "clip": clip, "in": { "preset": "slide_left" }, "out": { "preset": "fade", "duration": "1s" }, "combo": { "preset": "swing" } }),
    );
    let animations = &set.results[0]["animations"];
    assert_eq!(animations["in"]["preset"], "slide_left");
    assert_eq!(animations["in"]["duration"], 352_800_000);
    assert_eq!(animations["out"]["duration"], 705_600_000);
    assert_eq!(
        animations["combo"]["duration"], 705_600_000,
        "a second per cycle"
    );
    assert_eq!(clips(&session)[0]["animations"]["combo"]["preset"], "swing");
    let cleared = run(
        &mut session,
        "animation.set",
        json!({ "clip": clip, "clear": ["combo", "in"] }),
    );
    let left = &cleared.results[0]["animations"];
    assert!(left.get("in").is_none() && left.get("combo").is_none());
    assert_eq!(left["out"]["preset"], "fade");
    for (args, case) in [
        (json!({ "in": { "preset": "pulse" } }), "pulse is a combo"),
        (
            json!({ "combo": { "preset": "fade" } }),
            "fade is not a combo",
        ),
        (json!({ "in": { "preset": "explode" } }), "unknown preset"),
        (
            json!({ "in": { "preset": "fade", "duration": "0s" } }),
            "no time",
        ),
        (json!({ "clear": ["sideways"] }), "unknown place"),
        (json!({}), "nothing to do"),
    ] {
        let mut args = args;
        args["clip"] = json!(clip);
        assert_eq!(
            fails(&mut session, "animation.set", args).code,
            codes::INVALID_ARGUMENTS,
            "{case}"
        );
    }
}

#[test]
fn edits_keep_transitions_and_animations_at_their_edges() {
    let (mut session, _, clip) = session_with_clip();
    run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "6s" }),
    );
    run(
        &mut session,
        "transition.set",
        json!({ "clip": clip, "type": "dissolve" }),
    );
    run(
        &mut session,
        "animation.set",
        json!({ "clip": clip, "in": { "preset": "fade" }, "out": { "preset": "fade" }, "combo": { "preset": "pulse" } }),
    );
    run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "3s" }),
    );
    let after = clips(&session);
    assert!(
        after[0].get("transition").is_none(),
        "the left part ends at a plain cut"
    );
    assert_eq!(
        after[1]["transition"]["type"], "dissolve",
        "the right part leads into the next clip"
    );
    assert_eq!(after[0]["animations"]["in"]["preset"], "fade");
    assert!(after[0]["animations"].get("out").is_none());
    assert!(after[1]["animations"].get("in").is_none());
    assert_eq!(after[1]["animations"]["out"]["preset"], "fade");
    assert_eq!(
        after[1]["animations"]["combo"]["preset"], "pulse",
        "combos stay on both"
    );
    let mut cut = request(
        "timeline.remove_ranges",
        json!({ "ranges": [{ "start": "4s", "end": "4.5s" }], "ripple": true }),
    );
    cut.allow_destructive = true;
    session.execute(cut).unwrap();
    let after = clips(&session);
    assert!(
        after[1].get("transition").is_none(),
        "the piece before the cut has no transition"
    );
    assert_eq!(after[2]["transition"]["type"], "dissolve");
    let caps = session
        .query("capabilities", json!({ "detail": "outline" }))
        .unwrap();
    assert!(caps["transitions"].as_array().unwrap().len() >= 10);
    assert!(
        caps["animations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["preset"] == "pop")
    );
    let full = session
        .query("capabilities", json!({ "detail": "full" }))
        .unwrap();
    let wipe = full["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["type"] == "wipe")
        .unwrap()
        .clone();
    assert_eq!(wipe["params"][0]["name"], "direction");
}
