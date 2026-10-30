//! Masks on clips (ADR-0011, item 4): `mask.add`, `mask.set` and `mask.remove`, mask
//! keyframes, the chroma key's color parameter, and new mask ids for copies.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{ApiError, Session, codes};
use fosscut_model::MemoryStore;
use serde_json::{Value, json};
use support::*;

fn fails(session: &mut Session<MemoryStore>, command: &str, args: Value) -> ApiError {
    session.execute(request(command, args)).expect_err(command)
}

fn masks(session: &Session<MemoryStore>, clip: usize) -> Value {
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    full["tracks"][0]["clips"][clip]["masks"].clone()
}

#[test]
fn masks_are_added_changed_and_removed() {
    let (mut session, _, clip) = session_with_clip();
    let added = run(
        &mut session,
        "mask.add",
        json!({ "clip": clip, "shape": "rectangle", "center": [0.3, 0.4], "size": [0.4, 0.5], "roundness": 0.4, "feather": 0.2 }),
    );
    let result = &added.results[0];
    let mask = result["mask"].as_str().unwrap().to_string();
    assert!(mask.starts_with("mask_"));
    assert_eq!(result["masks"][0]["roundness"], 0.4);
    let star = run(
        &mut session,
        "mask.add",
        json!({ "clip": clip, "shape": "star", "mode": "subtract", "index": 0 }),
    );
    assert_eq!(
        star.results[0]["masks"][0]["shape"], "star",
        "index 0 combines first"
    );
    let path = run(
        &mut session,
        "mask.add",
        json!({ "clip": clip, "shape": "path", "points": [
            { "at": [-1, -1] }, { "at": [1, -1], "out": [0, 0.4] }, { "at": [0, 1], "in": [0.3, 0] }
        ] }),
    );
    assert_eq!(
        path.results[0]["masks"][2]["points"][1]["out"],
        json!([0.0, 0.4])
    );
    let brush = run(
        &mut session,
        "mask.add",
        json!({ "clip": clip, "shape": "brush", "strokes": [{ "points": [[0.2, 0.2], [0.4, 0.3]], "radius": 0.05 }] }),
    );
    assert_eq!(
        brush.results[0]["masks"][3]["size"],
        json!([1.0, 1.0]),
        "a brush covers the picture"
    );
    run(
        &mut session,
        "mask.add",
        json!({ "clip": clip, "shape": "text", "text": "SALE" }),
    );
    let set = run(
        &mut session,
        "mask.set",
        json!({ "clip": clip, "mask": mask, "invert": true, "shape": "ellipse", "enabled": false }),
    );
    let changed = set.results[0]["masks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == mask.as_str())
        .unwrap()
        .clone();
    assert_eq!(
        (
            changed["shape"].clone(),
            changed["invert"].clone(),
            changed["enabled"].clone()
        ),
        (json!("ellipse"), json!(true), json!(false))
    );
    let outline = session
        .query("timeline.read", json!({ "detail": "normal" }))
        .unwrap();
    assert_eq!(
        outline["tracks"][0]["clips"][0]["masks"][0],
        json!({ "id": star.results[0]["mask"], "shape": "star" })
    );
    run(
        &mut session,
        "mask.remove",
        json!({ "clip": clip, "mask": mask }),
    );
    assert_eq!(masks(&session, 0).as_array().unwrap().len(), 4);

    for (args, code, case) in [
        (
            json!({ "shape": "path", "points": [{ "at": [0, 0] }] }),
            codes::MODEL_INVALID,
            "a path needs three points",
        ),
        (
            json!({ "shape": "text" }),
            codes::MODEL_INVALID,
            "a text mask needs text",
        ),
        (
            json!({ "shape": "brush" }),
            codes::MODEL_INVALID,
            "a brush needs strokes",
        ),
        (
            json!({ "shape": "ellipse", "feather": 2 }),
            codes::INVALID_ARGUMENTS,
            "feather within 0 to 1",
        ),
        (
            json!({ "shape": "ellipse", "size": [-1, 1] }),
            codes::INVALID_ARGUMENTS,
            "no negative size",
        ),
        (
            json!({ "shape": "blob" }),
            codes::INVALID_ARGUMENTS,
            "unknown shape",
        ),
        (
            json!({ "shape": "ellipse", "bogus": 1 }),
            codes::INVALID_ARGUMENTS,
            "unknown field",
        ),
        (
            json!({ "shape": "ellipse", "index": 9 }),
            codes::INVALID_ARGUMENTS,
            "index beyond the masks",
        ),
    ] {
        let mut args = args;
        args["clip"] = json!(clip);
        assert_eq!(fails(&mut session, "mask.add", args).code, code, "{case}");
    }
    let missing = fails(
        &mut session,
        "mask.set",
        json!({ "clip": clip, "mask": "mask_0099", "invert": true }),
    );
    assert_eq!(missing.code, codes::MASK_NOT_FOUND);
    let voice = run(
        &mut session,
        "media.add",
        json!({
            "name": "voice.wav", "kind": "audio", "path": { "absolute": "/voice.wav" },
            "info": { "duration": 705_600_000i64, "audio": { "sample_rate": 48000, "channels": 1 } }
        }),
    );
    let sound = run(
        &mut session,
        "clip.add",
        json!({ "asset": voice.results[0]["asset"] }),
    );
    let audio = fails(
        &mut session,
        "mask.add",
        json!({ "clip": sound.results[0]["clip"], "shape": "ellipse" }),
    );
    assert!(audio.message.contains("audio clip"));
}

#[test]
fn masks_animate_and_copies_get_new_ids() {
    let (mut session, _, clip) = session_with_clip();
    let added = run(
        &mut session,
        "mask.add",
        json!({ "clip": clip, "shape": "ellipse" }),
    );
    let mask = added.results[0]["mask"].as_str().unwrap().to_string();
    let center = format!("masks.{mask}.center");
    for (at, value) in [("0s", json!([0.2, 0.5])), ("4s", json!([0.8, 0.5]))] {
        run(
            &mut session,
            "keyframe.set",
            json!({ "clip": clip, "property": center, "at": at, "value": value }),
        );
    }
    let view = masks(&session, 0);
    assert_eq!(view[0]["keyframes"]["center"][1]["at"]["seconds"], "4");
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    assert_eq!(
        full["tracks"][0]["clips"][0]["picture"],
        json!([1280, 720]),
        "the displayed picture size, for placing masks"
    );
    let animated = fails(
        &mut session,
        "mask.set",
        json!({ "clip": clip, "mask": mask, "center": [0.5, 0.5] }),
    );
    assert_eq!(animated.code, codes::CLIP_PROPERTY_ANIMATED);
    for (property, value, code) in [
        (
            format!("masks.{mask}.feather"),
            json!(3),
            codes::INVALID_ARGUMENTS,
        ),
        (
            format!("masks.{mask}.points"),
            json!(1),
            codes::INVALID_ARGUMENTS,
        ),
        (
            format!("masks.{mask}.center"),
            json!(0.5),
            codes::INVALID_ARGUMENTS,
        ),
        (
            "masks.mask_0099.center".to_string(),
            json!([0.5, 0.5]),
            codes::MASK_NOT_FOUND,
        ),
    ] {
        let error = fails(
            &mut session,
            "keyframe.set",
            json!({ "clip": clip, "property": property, "at": "1s", "value": value }),
        );
        assert_eq!(error.code, code, "{property}");
    }
    run(
        &mut session,
        "keyframe.remove",
        json!({ "clip": clip, "property": center, "all": true, "at": "2s" }),
    );
    assert_eq!(
        masks(&session, 0)[0]["center"],
        json!(null),
        "back in the middle: the default, omitted"
    );
    run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "3s" }),
    );
    let left = masks(&session, 0)[0]["id"].clone();
    let right = masks(&session, 1)[0]["id"].clone();
    assert_ne!(left, right, "the right half's mask has its own id");
}

#[test]
fn chroma_keys_take_a_color() {
    let (mut session, _, clip) = session_with_clip();
    let key = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "key.chroma", "params": { "color": "#0000ff", "similarity": 0.4 } }),
    );
    assert_eq!(key.results[0]["effects"][0]["params"]["color"], "#0000FF");
    let bad = fails(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "key.chroma", "params": { "color": "blue" } }),
    );
    assert_eq!(bad.code, codes::INVALID_ARGUMENTS);
}
