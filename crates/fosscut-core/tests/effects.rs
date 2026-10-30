//! Effects on clips (ADR-0011, items 1 and 2): `effect.add`, `effect.set`,
//! `effect.remove`, `effect.move` and `effect.copy`, adjustment layers, looks, keyframed
//! effect parameters, and how splits, cuts and copies keep effect ids unique.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeSet;

use fosscut_core::{ApiError, Session, codes};
use fosscut_model::MemoryStore;
use serde_json::{Value, json};
use support::*;

fn fails(session: &mut Session<MemoryStore>, command: &str, args: Value) -> ApiError {
    session.execute(request(command, args)).expect_err(command)
}

fn full(session: &Session<MemoryStore>) -> Value {
    session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap()
}

fn effect_ids(session: &Session<MemoryStore>) -> Vec<String> {
    let view = full(session);
    let mut ids = Vec::new();
    for track in view["tracks"].as_array().unwrap() {
        for clip in track["clips"].as_array().unwrap() {
            for effect in clip["effects"].as_array().into_iter().flatten() {
                ids.push(effect["id"].as_str().unwrap().to_string());
            }
        }
    }
    ids
}

fn lut_args() -> Value {
    json!({
        "name": "warm.cube",
        "kind": "lut",
        "path": { "absolute": "/looks/warm.cube" },
        "info": { "lut": { "size": 17 } }
    })
}

#[test]
fn effects_are_added_with_checked_sparse_parameters() {
    let (mut session, asset, clip) = session_with_clip();
    let added = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "color.adjust", "params": { "exposure": 0.5, "contrast": 0 } }),
    );
    let result = &added.results[0];
    let effect = result["effect"].as_str().unwrap().to_string();
    assert!(effect.starts_with("effect_"));
    assert_eq!(
        result["effects"][0]["params"],
        json!({ "exposure": 0.5 }),
        "the default contrast is not stored"
    );
    let view = &full(&session)["tracks"][0]["clips"][0]["effects"][0];
    assert_eq!(view["type"], "color.adjust");
    assert_eq!(view["version"], 1);

    for (args, code, case) in [
        (
            json!({ "type": "color.glow" }),
            codes::EFFECT_UNKNOWN_TYPE,
            "unknown type",
        ),
        (
            json!({ "type": "color.adjust", "params": { "glow": 1 } }),
            codes::INVALID_ARGUMENTS,
            "unknown parameter",
        ),
        (
            json!({ "type": "color.adjust", "params": { "exposure": 9 } }),
            codes::INVALID_ARGUMENTS,
            "out of range",
        ),
        (
            json!({ "type": "color.wheels", "params": { "lift": 0.2 } }),
            codes::INVALID_ARGUMENTS,
            "wheels are pairs",
        ),
        (
            json!({ "type": "color.curves", "params": { "red": [[0, 0]] } }),
            codes::INVALID_ARGUMENTS,
            "a curve needs two points",
        ),
        (
            json!({ "type": "color.curves", "params": { "red": [[0.5, 0], [0.5, 1]] } }),
            codes::INVALID_ARGUMENTS,
            "points in order of x",
        ),
        (
            json!({ "type": "color.lut" }),
            codes::INVALID_ARGUMENTS,
            "a LUT effect needs its LUT",
        ),
        (
            json!({ "type": "color.lut", "params": { "lut": asset } }),
            codes::INVALID_ARGUMENTS,
            "a video is not a LUT",
        ),
        (
            json!({ "type": "color.lut", "params": { "lut": "asset_0099" } }),
            codes::ASSET_NOT_FOUND,
            "missing LUT",
        ),
        (
            json!({ "type": "color.adjust", "index": 5 }),
            codes::INVALID_ARGUMENTS,
            "index beyond the effects",
        ),
    ] {
        let mut args = args;
        args["clip"] = json!(clip);
        assert_eq!(fails(&mut session, "effect.add", args).code, code, "{case}");
    }

    let lut = run(&mut session, "media.add", lut_args());
    let lut_asset = lut.results[0]["asset"].clone();
    let graded = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "color.lut", "params": { "lut": lut_asset, "intensity": 0.6 } }),
    );
    assert_eq!(
        graded.results[0]["effects"][1]["params"]["lut"],
        json!({ "asset": lut_asset })
    );
    let placed = fails(&mut session, "clip.add", json!({ "asset": lut_asset }));
    assert_eq!(placed.code, codes::INVALID_ARGUMENTS, "a LUT is not a clip");
    let curve = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "color.curves", "params": { "master": [[0, 0.05], [0.5, 0.55], [1, 1]] } }),
    );
    assert_eq!(curve.results[0]["effects"].as_array().unwrap().len(), 3);

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
        "effect.add",
        json!({ "clip": sound.results[0]["clip"], "type": "color.adjust" }),
    );
    assert!(audio.message.contains("audio clip"), "{}", audio.message);
}

#[test]
fn effects_change_bypass_move_and_go() {
    let (mut session, _, clip) = session_with_clip();
    let first = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "color.vignette", "params": { "amount": 0.5 } }),
    );
    let vignette = first.results[0]["effect"].clone();
    let second = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "blur.gaussian", "index": 0 }),
    );
    let blur = second.results[0]["effect"].clone();
    assert_eq!(
        second.results[0]["effects"][0]["effect"], blur,
        "index 0 applies first"
    );

    let set = run(
        &mut session,
        "effect.set",
        json!({ "clip": clip, "effect": vignette, "params": { "feather": 0.9 }, "reset": ["amount"], "enabled": false }),
    );
    let changed = &set.results[0]["effects"][1];
    assert_eq!(changed["params"], json!({ "feather": 0.9 }));
    assert_eq!(changed["enabled"], false);
    assert_eq!(
        full(&session)["tracks"][0]["clips"][0]["effects"][1]["enabled"],
        false
    );

    let moved = run(
        &mut session,
        "effect.move",
        json!({ "clip": clip, "effect": vignette, "index": 0 }),
    );
    assert_eq!(moved.results[0]["effects"][0]["effect"], vignette);
    for (command, args, code) in [
        (
            "effect.set",
            json!({ "effect": "effect_0099", "enabled": true }),
            codes::EFFECT_NOT_FOUND,
        ),
        (
            "effect.set",
            json!({ "effect": blur }),
            codes::INVALID_ARGUMENTS,
        ),
        (
            "effect.move",
            json!({ "effect": blur, "index": 2 }),
            codes::INVALID_ARGUMENTS,
        ),
        (
            "effect.set",
            json!({ "effect": blur, "reset": ["size"] }),
            codes::INVALID_ARGUMENTS,
        ),
    ] {
        let mut args = args;
        args["clip"] = json!(clip);
        assert_eq!(fails(&mut session, command, args).code, code, "{command}");
    }
    let removed = run(
        &mut session,
        "effect.remove",
        json!({ "clip": clip, "effect": blur }),
    );
    assert_eq!(removed.results[0]["effects"].as_array().unwrap().len(), 1);
    let gone = fails(
        &mut session,
        "effect.remove",
        json!({ "clip": clip, "effect": blur }),
    );
    assert_eq!(gone.code, codes::EFFECT_NOT_FOUND);
}

#[test]
fn effect_parameters_take_keyframes() {
    let (mut session, _, clip) = session_with_clip();
    let added = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "color.adjust" }),
    );
    let effect = added.results[0]["effect"].as_str().unwrap().to_string();
    let path = format!("effects.{effect}.exposure");
    for (at, value) in [("0s", -1.0), ("2s", 1.0)] {
        run(
            &mut session,
            "keyframe.set",
            json!({ "clip": clip, "property": path, "at": at, "value": value, "easing": "ease_in_out" }),
        );
    }
    let view = &full(&session)["tracks"][0]["clips"][0]["effects"][0];
    assert_eq!(view["keyframes"]["exposure"][1]["at"]["seconds"], "2");
    let animated = fails(
        &mut session,
        "effect.set",
        json!({ "clip": clip, "effect": effect, "params": { "exposure": 0.2 } }),
    );
    assert_eq!(animated.code, codes::CLIP_PROPERTY_ANIMATED);
    let wheel = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "color.wheels" }),
    );
    let wheels = wheel.results[0]["effect"].as_str().unwrap().to_string();
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": format!("effects.{wheels}.gain"), "at": "1s", "value": [0.2, 0.1] }),
    );
    for (property, value, code) in [
        (
            format!("effects.{effect}.exposure"),
            json!(7),
            codes::INVALID_ARGUMENTS,
        ),
        (
            format!("effects.{effect}.glow"),
            json!(1),
            codes::INVALID_ARGUMENTS,
        ),
        (
            "effects.effect_0099.exposure".to_string(),
            json!(1),
            codes::EFFECT_NOT_FOUND,
        ),
        (
            format!("effects.{wheels}.gain"),
            json!(0.3),
            codes::INVALID_ARGUMENTS,
        ),
    ] {
        let error = fails(
            &mut session,
            "keyframe.set",
            json!({ "clip": clip, "property": property, "at": "1s", "value": value }),
        );
        assert_eq!(error.code, code, "{property}");
    }
    let kept = run(
        &mut session,
        "keyframe.remove",
        json!({ "clip": clip, "property": path, "all": true, "at": "1s" }),
    );
    assert!(kept.results[0]["keyframes"].as_object().unwrap().is_empty());
    let view = &full(&session)["tracks"][0]["clips"][0]["effects"][0];
    assert!(
        view["params"].get("exposure").is_none(),
        "the value at 1 s, the middle of the ease, is 0: the default, so not stored"
    );
    assert!(view.get("keyframes").is_none());
}

#[test]
fn copies_adjustments_and_looks() {
    let (mut session, _, clip) = session_with_clip();
    run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "color.adjust", "params": { "saturation": -1 } }),
    );
    let title = run(&mut session, "text.add", json!({ "text": "Graded" }));
    let title = title.results[0]["clip"].clone();
    run(
        &mut session,
        "effect.add",
        json!({ "clip": title, "type": "blur.gaussian" }),
    );
    let copied = run(
        &mut session,
        "effect.copy",
        json!({ "from": clip, "to": [title], "append": true }),
    );
    let effects = &copied.results[0]["clips"][0]["effects"];
    assert_eq!(effects.as_array().unwrap().len(), 2);
    assert_eq!(effects[1]["params"]["saturation"], -1.0);
    run(
        &mut session,
        "effect.copy",
        json!({ "from": clip, "to": [title] }),
    );
    let ids = effect_ids(&session);
    assert_eq!(ids.len(), 2, "replaced: one effect on each clip");
    assert_eq!(
        ids.iter().collect::<BTreeSet<_>>().len(),
        2,
        "copies get new ids"
    );
    let itself = fails(
        &mut session,
        "effect.copy",
        json!({ "from": clip, "to": [clip] }),
    );
    assert_eq!(itself.code, codes::INVALID_ARGUMENTS);

    let adjust = run(
        &mut session,
        "adjustment.add",
        json!({ "start": "1s", "duration": "2s", "effects": [{ "type": "color.adjust", "params": { "contrast": 0.3 } }] }),
    );
    let result = &adjust.results[0];
    assert_eq!(result["effects"].as_array().unwrap().len(), 1);
    let view = full(&session);
    let kinds: Vec<&str> = view["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["video", "video", "text"],
        "above the video, below the titles"
    );
    assert_eq!(view["tracks"][1]["name"], "Adjustments");
    assert_eq!(view["tracks"][1]["clips"][0]["kind"], "adjustment");
    let again = run(
        &mut session,
        "adjustment.add",
        json!({ "start": "4s", "duration": "1s" }),
    );
    assert_eq!(
        again.results[0]["track"], result["track"],
        "the free adjustment track is reused"
    );
    let overlapping = run(
        &mut session,
        "adjustment.add",
        json!({ "start": "1.5s", "duration": "1s" }),
    );
    assert_ne!(
        overlapping.results[0]["track"], result["track"],
        "a new track when it is taken"
    );
    let bad = fails(
        &mut session,
        "adjustment.add",
        json!({ "start": "0s", "duration": "1s", "effects": [{ "type": "color.nope" }] }),
    );
    assert_eq!(bad.code, codes::EFFECT_UNKNOWN_TYPE);

    let look = run(
        &mut session,
        "look.apply",
        json!({ "clip": result["clip"], "look": "vintage" }),
    );
    let types: Vec<&str> = look.results[0]["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "color.adjust",
            "color.adjust",
            "color.vignette",
            "stylize.grain"
        ]
    );
    let replaced = run(
        &mut session,
        "look.apply",
        json!({ "clip": result["clip"], "look": "noir", "replace": true }),
    );
    assert_eq!(
        replaced.results[0]["effects"][0]["params"]["saturation"],
        -1.0
    );
    let unknown = fails(
        &mut session,
        "look.apply",
        json!({ "clip": clip, "look": "sparkle" }),
    );
    assert_eq!(unknown.code, codes::LOOK_UNKNOWN);
    assert!(unknown.hint.unwrap().contains("teal_orange"));
}

#[test]
fn every_look_applies_cleanly() {
    let session = new_session();
    let caps = session
        .query("capabilities", json!({ "detail": "full" }))
        .unwrap();
    let looks = caps["looks"].as_array().unwrap();
    assert!(looks.len() >= 10);
    for look in looks {
        let (mut session, _, clip) = session_with_clip();
        let applied = run(
            &mut session,
            "look.apply",
            json!({ "clip": clip, "look": look["name"] }),
        );
        assert_eq!(
            applied.results[0]["effects"].as_array().unwrap().len(),
            look["effects"].as_array().unwrap().len(),
            "{}",
            look["name"]
        );
    }
    let types: Vec<&str> = caps["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"color.hsl") && types.contains(&"detail.sharpen"));
    let outline = session
        .query("capabilities", json!({ "detail": "outline" }))
        .unwrap();
    assert!(
        outline["effects"][0].get("params").is_none(),
        "outline is brief"
    );
}

#[test]
fn splits_cuts_and_copies_give_effects_new_ids() {
    let (mut session, _, clip) = session_with_clip();
    let added = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "color.adjust", "params": { "exposure": 1 } }),
    );
    let effect = added.results[0]["effect"].as_str().unwrap().to_string();
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": format!("effects.{effect}.contrast"), "at": "5s", "value": 0.5 }),
    );
    run(
        &mut session,
        "clip.split",
        json!({ "clip": clip, "at": "4s" }),
    );
    let view = full(&session);
    let right = &view["tracks"][0]["clips"][1]["effects"][0];
    assert_ne!(right["id"], json!(effect));
    assert_eq!(right["params"]["exposure"], 1.0);
    assert_eq!(
        right["keyframes"]["contrast"][0]["at"]["seconds"], "5",
        "effect keyframes keep their timeline time"
    );
    let mut cut = request(
        "timeline.remove_ranges",
        json!({ "ranges": [{ "start": "1s", "end": "2s" }] }),
    );
    cut.allow_destructive = true;
    session.execute(cut).unwrap();
    let ids: BTreeSet<String> = effect_ids(&session).into_iter().collect();
    assert_eq!(ids.len(), 3, "three pieces, each with its own effect id");
    let copy = run(
        &mut session,
        "timeline.duplicate",
        json!({ "width": 1080, "height": 1920 }),
    );
    let copied = session
        .query(
            "timeline.read",
            json!({ "timeline": copy.results[0]["timeline"], "detail": "full" }),
        )
        .unwrap();
    let copies: BTreeSet<String> = copied["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["effects"][0]["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(copies.len(), 3);
    assert!(copies.is_disjoint(&ids), "the copy's effects have new ids");
}
