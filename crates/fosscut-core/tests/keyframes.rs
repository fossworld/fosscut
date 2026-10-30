//! Clip properties of the closed vocabulary through `clip.set`, keyframes through
//! `keyframe.set` and `keyframe.remove`, and how split, trim and range removal carry
//! keyframes and fades.

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

#[test]
fn every_new_property_is_set_by_path() {
    let (mut session, _, clip) = session_with_clip();
    let set = run(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": {
            "transform.stretch": [1.25, 0.8],
            "transform.flip_horizontal": true,
            "crop.left": 0.1,
            "crop.bottom": 0.2,
            "blend": "multiply",
            "audio.pan": -0.5,
            "audio.fade_in": { "seconds": "0.5" },
            "audio.fade_out": { "frames": 30 },
            "audio.channels": "mono"
        } }),
    );
    let result = &set.results[0];
    assert_eq!(result["transform"]["stretch"], json!([1.25, 0.8]));
    assert_eq!(result["transform"]["flip_horizontal"], true);
    assert_eq!(result["crop"], json!({ "left": 0.1, "bottom": 0.2 }));
    assert_eq!(result["blend"], "multiply");
    assert_eq!(
        result["audio"],
        json!({ "fade_in": SECOND / 2, "fade_out": SECOND, "pan": -0.5, "channels": "mono" })
    );
    assert_eq!(result["enabled"], true);
    let view = &clips(&session)[0];
    assert_eq!(view["blend"], "multiply");
    assert!(view.get("enabled").is_none(), "only shown when deactivated");

    run(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "enabled": false } }),
    );
    let outline = session
        .query("timeline.read", json!({ "detail": "normal" }))
        .unwrap();
    assert_eq!(outline["tracks"][0]["clips"][0]["enabled"], false);

    for (set, case) in [
        (json!({ "blend": "glow" }), "unknown blend mode"),
        (json!({ "audio.channels": 2 }), "channels are names"),
        (json!({ "crop.top": 1.5 }), "crop above one"),
        (json!({ "transform.stretch": [1, 0] }), "stretch above zero"),
        (
            json!({ "transform.flip_vertical": "yes" }),
            "flips are booleans",
        ),
        (json!({ "audio.fade_in": 3 }), "fades need a unit"),
        (
            json!({ "audio.fade_in": { "seconds": "-1" } }),
            "fades are not negative",
        ),
        (json!({ "audio.pan": -2 }), "pan within -1 to 1"),
    ] {
        let error = fails(
            &mut session,
            "clip.set",
            json!({ "clip": clip, "set": set }),
        );
        assert_eq!(error.code, codes::INVALID_ARGUMENTS, "{case}");
    }
    let both = fails(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "crop.left": 0.6, "crop.right": 0.6 } }),
    );
    assert_eq!(
        both.code,
        codes::MODEL_INVALID,
        "nothing left of the picture"
    );
    let title = run(&mut session, "text.add", json!({ "text": "Quiet" }));
    let pan = fails(
        &mut session,
        "clip.set",
        json!({ "clip": title.results[0]["clip"], "set": { "audio.pan": 0.5 } }),
    );
    assert!(pan.message.contains("no sound"), "{}", pan.message);
}

#[test]
fn keyframes_animate_a_property_and_block_plain_values() {
    let (mut session, _, clip) = session_with_clip();
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "opacity", "at": "0s", "value": 0, "easing": "ease_out" }),
    );
    let set = run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "opacity", "at": "2s", "value": 1 }),
    );
    let keys = &set.results[0]["keyframes"]["opacity"];
    assert_eq!(set.results[0]["property"], "opacity");
    assert_eq!(keys.as_array().unwrap().len(), 2);
    assert_eq!(keys[1]["at"]["seconds"], "2");
    assert_eq!(
        keys[0]["interpolation"]["bezier"],
        json!([0.0, 0.0, 0.58, 1.0])
    );
    assert_eq!(keys[1]["interpolation"], "linear");

    // Without a value, a new keyframe takes the animated value, so nothing changes yet.
    let middle = run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "opacity", "offset": { "frames": 30 } }),
    );
    let value = middle.results[0]["keyframes"]["opacity"][1]["value"]
        .as_f64()
        .unwrap();
    assert!(value > 0.5, "ease out passes half way early: {value}");
    // Setting at an existing time replaces the value and keeps the easing.
    let replaced = run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "opacity", "at": "0s", "value": 0.25 }),
    );
    let first = &replaced.results[0]["keyframes"]["opacity"][0];
    assert_eq!(first["value"], 0.25);
    assert_eq!(first["interpolation"]["bezier"][2], 0.58);

    let animated = fails(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "opacity": 1 } }),
    );
    assert_eq!(animated.code, codes::CLIP_PROPERTY_ANIMATED);
    assert!(animated.hint.unwrap().contains("keyframe.set"));

    let position = run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": "transform.position", "at": "1s", "value": [0.25, -0.1], "easing": { "bezier": [0.3, 1.4, 0.6, 1] } }),
    );
    assert_eq!(
        position.results[0]["keyframes"]["transform.position"][0]["value"],
        json!([0.25, -0.1])
    );
    let view = &clips(&session)[0];
    assert_eq!(view["keyframes"]["opacity"].as_array().unwrap().len(), 3);

    for (args, code, case) in [
        (
            json!({ "property": "opacity", "at": "9s", "value": 1 }),
            codes::KEYFRAME_OUTSIDE,
            "after the clip",
        ),
        (
            json!({ "property": "blend", "at": "1s", "value": 1 }),
            codes::INVALID_ARGUMENTS,
            "not animatable",
        ),
        (
            json!({ "property": "opacity", "value": 1 }),
            codes::INVALID_ARGUMENTS,
            "no time",
        ),
        (
            json!({ "property": "opacity", "at": "1s", "offset": "1s" }),
            codes::INVALID_ARGUMENTS,
            "two times",
        ),
        (
            json!({ "property": "transform.position", "at": "1s", "value": 3 }),
            codes::INVALID_ARGUMENTS,
            "position is a pair",
        ),
        (
            json!({ "property": "opacity", "at": "1s", "easing": { "bezier": [2, 0, 0, 1] } }),
            codes::INVALID_ARGUMENTS,
            "x control points within 0 to 1",
        ),
        (
            json!({ "property": "size", "at": "1s" }),
            codes::INVALID_ARGUMENTS,
            "no such property",
        ),
    ] {
        let mut args = args;
        args["clip"] = json!(clip);
        let error = fails(&mut session, "keyframe.set", args);
        assert_eq!(error.code, code, "{case}");
    }
}

#[test]
fn removing_keyframes_ends_the_animation_where_asked() {
    let (mut session, _, clip) = session_with_clip();
    for (at, value) in [("0s", 1.0), ("4s", 3.0)] {
        run(
            &mut session,
            "keyframe.set",
            json!({ "clip": clip, "property": "transform.scale", "at": at, "value": value }),
        );
    }
    let missing = fails(
        &mut session,
        "keyframe.remove",
        json!({ "clip": clip, "property": "transform.scale", "at": "1s" }),
    );
    assert_eq!(missing.code, codes::KEYFRAME_NOT_FOUND);
    let one = run(
        &mut session,
        "keyframe.remove",
        json!({ "clip": clip, "property": "transform.scale", "at": "4s" }),
    );
    assert_eq!(
        one.results[0]["keyframes"]["transform.scale"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let last = run(
        &mut session,
        "keyframe.remove",
        json!({ "clip": clip, "property": "transform.scale", "at": "0s" }),
    );
    assert!(
        last.results[0]["keyframes"]
            .get("transform.scale")
            .is_none()
    );
    assert!(
        last.results[0]["transform"].get("scale").is_none(),
        "keeps the last value, 1, which is omitted"
    );

    for (at, value) in [("0s", 1.0), ("4s", 3.0)] {
        run(
            &mut session,
            "keyframe.set",
            json!({ "clip": clip, "property": "transform.scale", "at": at, "value": value }),
        );
    }
    let all = run(
        &mut session,
        "keyframe.remove",
        json!({ "clip": clip, "property": "transform.scale", "all": true, "at": "2s" }),
    );
    assert_eq!(
        all.results[0]["transform"]["scale"], 2.0,
        "the value at 2 s stays"
    );
    let none = fails(
        &mut session,
        "keyframe.remove",
        json!({ "clip": clip, "property": "transform.scale", "all": true }),
    );
    assert_eq!(none.code, codes::KEYFRAME_NOT_FOUND);
    let vague = fails(
        &mut session,
        "keyframe.remove",
        json!({ "clip": clip, "property": "opacity" }),
    );
    assert_eq!(vague.code, codes::INVALID_ARGUMENTS);
}

#[test]
fn edits_keep_keyframes_on_the_content_and_fades_at_the_edges() {
    let (mut session, _, clip) = session_with_clip();
    run(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "audio.fade_in": "0.5s", "audio.fade_out": "0.5s" } }),
    );
    for (at, value) in [("2s", 0.0), ("6s", 1.0)] {
        run(
            &mut session,
            "keyframe.set",
            json!({ "clip": clip, "property": "opacity", "at": at, "value": value }),
        );
    }
    let split = run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "4s" }),
    );
    let right = split.results[0]["right"]["clip"].clone();
    let both = clips(&session);
    let at = |view: &Value| -> Vec<String> {
        view["keyframes"]["opacity"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["at"]["seconds"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(at(&both[0]), ["2", "6"], "same timeline times on the left");
    assert_eq!(at(&both[1]), ["2", "6"], "and on the right");
    assert_eq!(
        both[1]["keyframes"]["opacity"][0]["offset"]["seconds"],
        "-2"
    );
    assert!(
        both[0]["audio"].get("fade_out").is_none(),
        "no fade inside the cut"
    );
    assert_eq!(both[0]["audio"]["fade_in"], SECOND / 2);
    assert!(both[1]["audio"].get("fade_in").is_none());
    assert_eq!(both[1]["audio"]["fade_out"], SECOND / 2);

    run(
        &mut session,
        "clip.trim",
        json!({ "clip": right, "start": "5s" }),
    );
    let trimmed = &clips(&session)[1];
    assert_eq!(at(trimmed), ["2", "6"], "a trim leaves keyframes in place");

    let mut cut = request(
        "timeline.remove_ranges",
        json!({ "ranges": [{ "start": "1s", "end": "2s" }], "ripple": true }),
    );
    cut.allow_destructive = true;
    session.execute(cut).unwrap();
    let after = clips(&session);
    // The piece after the cut moved one second earlier, and its keyframes with it: the
    // content at 2 s now plays at 1 s.
    assert_eq!(after[1]["start"]["seconds"], "1");
    assert_eq!(
        after[1]["keyframes"]["opacity"][0]["at"]["seconds"], "1",
        "rippled with the content"
    );
    assert_eq!(
        after[0]["audio"]["fade_in"],
        SECOND / 2,
        "the first piece keeps the fade in"
    );
    assert!(
        after[1]["audio"].get("fade_in").is_none(),
        "a cut edge gets none"
    );
}

#[test]
fn a_vertical_copy_scales_scale_keyframes_with_the_fill() {
    let (mut session, _, clip) = session_with_clip();
    for (at, value) in [("0s", 1.0), ("4s", 2.0)] {
        run(
            &mut session,
            "keyframe.set",
            json!({ "clip": clip, "property": "transform.scale", "at": at, "value": value }),
        );
    }
    let copy = run(
        &mut session,
        "timeline.duplicate",
        json!({ "width": 1080, "height": 1920, "fit": "fill" }),
    );
    let timeline = copy.results[0]["timeline"].clone();
    let full = session
        .query(
            "timeline.read",
            json!({ "timeline": timeline, "detail": "full" }),
        )
        .unwrap();
    let keys = &full["tracks"][0]["clips"][0]["keyframes"]["transform.scale"];
    let first = keys[0]["value"].as_f64().unwrap();
    let factor = full["tracks"][0]["clips"][0]["transform"]["scale"]
        .as_f64()
        .unwrap();
    assert!((first - factor).abs() < 1e-9, "{first} vs {factor}");
    assert!((keys[1]["value"].as_f64().unwrap() - 2.0 * factor).abs() < 1e-9);
}

#[test]
fn capabilities_publish_the_property_vocabulary() {
    let session = new_session();
    let outline = session.query("capabilities", json!({})).unwrap();
    let paths: Vec<&str> = outline["properties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"transform.stretch") && paths.contains(&"audio.channels"));
    let full = session
        .query("capabilities", json!({ "detail": "full" }))
        .unwrap();
    let blend = full["properties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["path"] == "blend")
        .unwrap();
    assert_eq!(blend["choices"].as_array().unwrap().len(), 14);
    assert_eq!(blend["animatable"], false);
}
