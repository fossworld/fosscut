//! Staggered animations and the motion-design additions to animations and text reveals
//! (ADR-0011, clip animations; ADR-0027, reveals): `clips.stagger` gives clips one preset
//! with growing delays in a chosen order, `animation.set` takes a delay, and `text.add`
//! takes reveals with new motions, overlap, easing, order and an exit motion.

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

/// A title on a text track of its own, so titles can overlap in time.
fn title(session: &mut Session<MemoryStore>, text: &str, start: &str, duration: &str) -> String {
    let track = run(session, "track.add", json!({ "kind": "text" }));
    let added = run(
        session,
        "text.add",
        json!({ "text": text, "start": start, "duration": duration, "track": track.results[0]["track"] }),
    );
    added.results[0]["clip"].as_str().unwrap().to_string()
}

/// A session with titles three seconds long from the start, placed at the given positions
/// on the canvas; returns their ids in that order.
fn titles(positions: &[[f64; 2]]) -> (Session<MemoryStore>, Vec<String>) {
    let mut session = new_session();
    let mut ids = Vec::new();
    for (index, position) in positions.iter().enumerate() {
        let id = title(&mut session, &format!("Card {index}"), "0f", "3s");
        run(
            &mut session,
            "clip.set",
            json!({ "clip": id, "set": { "transform.position": position } }),
        );
        ids.push(id);
    }
    (session, ids)
}

/// The delay in flicks of each clip's animation at `place`, keyed by clip id.
fn delays(session: &Session<MemoryStore>, place: &str) -> Vec<(String, i64)> {
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    full["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .filter_map(|c| {
            let animation = c["animations"].get(place)?;
            Some((
                c["id"].as_str().unwrap().to_string(),
                animation["delay"].as_i64().unwrap_or(0),
            ))
        })
        .collect()
}

/// The animation at `place` of `clip`, as stored.
fn animation(session: &Session<MemoryStore>, place: &str, clip: &str) -> Value {
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    full["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .find(|c| c["id"] == clip)
        .unwrap()["animations"][place]
        .clone()
}

fn delay_of(session: &Session<MemoryStore>, place: &str, clip: &str) -> i64 {
    delays(session, place)
        .into_iter()
        .find(|(id, _)| id == clip)
        .map(|(_, d)| d)
        .unwrap()
}

#[test]
fn staggers_give_growing_delays_in_the_listed_order() {
    let (mut session, ids) = titles(&[[0.0, 0.0]; 4]);
    let staggered = run(
        &mut session,
        "clips.stagger",
        json!({ "clips": ids, "preset": "pop_in" }),
    );
    let result = &staggered.results[0];
    assert_eq!(result["preset"], "pop_in");
    assert_eq!(result["place"], "in");
    let reported: Vec<&str> = result["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["clip"].as_str().unwrap())
        .collect();
    assert_eq!(reported, ids, "listed order");
    // A tenth of a second apart by default, at the timeline's frames.
    assert_eq!(result["clips"][2]["delay"]["frames"], 6);
    assert_eq!(result["clips"][3]["delay"]["seconds"], "0.3");
    for (turn, id) in ids.iter().enumerate() {
        assert_eq!(delay_of(&session, "in", id), turn as i64 * SECOND / 10);
    }
    assert_eq!(result["clips"][1]["animations"]["in"]["preset"], "pop_in");
    assert_eq!(
        result["clips"][1]["animations"]["in"]["duration"],
        SECOND / 2,
        "half a second"
    );
    // The first clip waits for nothing, so its delay stays out of the document.
    let first = animation(&session, "in", &ids[0]);
    assert_eq!(first["preset"], "pop_in");
    assert!(first.get("delay").is_none(), "{first}");
}

#[test]
fn orders_follow_time_place_and_a_seeded_shuffle() {
    // Placed right to left and top to bottom: x falls while y rises.
    let positions = [[0.3, -0.3], [0.1, -0.1], [-0.1, 0.1], [-0.3, 0.3]];
    let (mut session, ids) = titles(&positions);
    let order_of = |session: &mut Session<MemoryStore>, args: Value| -> Vec<String> {
        let mut args = args;
        args["clips"] = json!(ids);
        args["preset"] = json!("rise_in");
        let out = run(session, "clips.stagger", args);
        out.results[0]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["clip"].as_str().unwrap().to_string())
            .collect()
    };
    let reversed: Vec<String> = ids.iter().rev().cloned().collect();
    assert_eq!(
        order_of(&mut session, json!({ "order": "reverse" })),
        reversed
    );
    assert_eq!(
        order_of(&mut session, json!({ "order": "left_to_right" })),
        reversed
    );
    assert_eq!(
        order_of(&mut session, json!({ "order": "right_to_left" })),
        ids
    );
    assert_eq!(
        order_of(&mut session, json!({ "order": "top_to_bottom" })),
        ids
    );
    assert_eq!(
        order_of(&mut session, json!({ "order": "bottom_to_top" })),
        reversed
    );
    let centre = order_of(&mut session, json!({ "order": "center_out" }));
    assert_eq!(
        (&centre[..2], &centre[2..]),
        (&ids[1..3], &[ids[0].clone(), ids[3].clone()][..]),
        "the inner two first, ties as listed"
    );
    // The leftmost moves first, so it has no delay and the rightmost the longest.
    order_of(
        &mut session,
        json!({ "order": "left_to_right", "step": "0.2s" }),
    );
    assert_eq!(delay_of(&session, "in", &ids[3]), 0);
    assert_eq!(delay_of(&session, "in", &ids[0]), 3 * SECOND / 5);
    // A shuffle is a permutation, the same for the same seed.
    let shuffled = order_of(&mut session, json!({ "order": "random", "seed": 7 }));
    let again = order_of(&mut session, json!({ "order": "random", "seed": 7 }));
    assert_eq!(shuffled, again);
    let mut sorted = shuffled.clone();
    sorted.sort();
    let mut expected = ids.clone();
    expected.sort();
    assert_eq!(sorted, expected);
    let others: Vec<Vec<String>> = (0..6)
        .map(|seed| order_of(&mut session, json!({ "order": "random", "seed": seed })))
        .collect();
    assert!(
        others.iter().any(|o| *o != shuffled),
        "seeds shuffle differently"
    );
}

#[test]
fn staggers_by_timeline_start_exits_and_combos() {
    let mut session = new_session();
    let ids: Vec<String> = ["2s", "0s", "1s"]
        .iter()
        .map(|start| title(&mut session, "Word", start, "4s"))
        .collect();
    let out = run(
        &mut session,
        "clips.stagger",
        json!({ "clips": ids, "preset": "blur_in", "order": "timeline", "start": "0.5s", "step": "0.3s" }),
    );
    let order: Vec<&str> = out.results[0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["clip"].as_str().unwrap())
        .collect();
    assert_eq!(order, [&ids[1], &ids[2], &ids[0]]);
    assert_eq!(delay_of(&session, "in", &ids[1]), SECOND / 2);
    assert_eq!(delay_of(&session, "in", &ids[0]), 11 * SECOND / 10);
    // Exits: the first in order leaves first, so it waits longest before the end.
    run(
        &mut session,
        "clips.stagger",
        json!({ "clips": ids, "preset": "glow_out", "place": "out", "step": "0.2s" }),
    );
    assert_eq!(delay_of(&session, "out", &ids[0]), 2 * SECOND / 5);
    assert_eq!(delay_of(&session, "out", &ids[2]), 0);
    // Combos start their cycles one step apart: a wave.
    run(
        &mut session,
        "clips.stagger",
        json!({ "clips": ids, "preset": "pulse", "place": "combo", "duration": "1s" }),
    );
    assert_eq!(delay_of(&session, "combo", &ids[2]), SECOND / 5);
    // Each stagger is one undoable step.
    run(&mut session, "history.undo", json!({}));
    assert!(delays(&session, "combo").is_empty());
    assert_eq!(delays(&session, "out").len(), 3);
}

#[test]
fn bad_staggers_change_nothing() {
    let (mut session, ids) = titles(&[[0.0, 0.0]; 3]);
    let before = documents(&session);
    let cases = [
        (
            json!({ "clips": [], "preset": "pop_in" }),
            codes::INVALID_ARGUMENTS,
            "no clips",
        ),
        (
            json!({ "clips": [ids[0], ids[0]], "preset": "pop_in" }),
            codes::INVALID_ARGUMENTS,
            "listed twice",
        ),
        (
            json!({ "clips": ids, "preset": "pop_out" }),
            codes::INVALID_ARGUMENTS,
            "an exit as an entrance",
        ),
        (
            json!({ "clips": ids, "preset": "pulse", "place": "in" }),
            codes::INVALID_ARGUMENTS,
            "a combo as an entrance",
        ),
        (
            json!({ "clips": ids, "preset": "explode" }),
            codes::INVALID_ARGUMENTS,
            "unknown preset",
        ),
        (
            json!({ "clips": ids, "preset": "pop_in", "step": "-0.1s" }),
            codes::INVALID_ARGUMENTS,
            "negative step",
        ),
        (
            json!({ "clips": ids, "preset": "pop_in", "step": "1.5s" }),
            codes::INVALID_ARGUMENTS,
            "the last clip ends before its turn is over",
        ),
        (
            json!({ "clips": ids, "preset": "pop_in", "order": "sideways" }),
            codes::INVALID_ARGUMENTS,
            "unknown order",
        ),
        (
            json!({ "clips": [ids[0], "clip_0999"], "preset": "pop_in" }),
            codes::CLIP_NOT_FOUND,
            "unknown clip",
        ),
    ];
    for (args, code, case) in cases {
        assert_eq!(
            fails(&mut session, "clips.stagger", args).code,
            code,
            "{case}"
        );
        assert_eq!(documents(&session), before, "{case}: nothing changed");
    }
    let too_short = fails(
        &mut session,
        "clips.stagger",
        json!({ "clips": ids, "preset": "pop_in", "step": "1.5s" }),
    );
    assert!(too_short.hint.is_some());
}

#[test]
fn animations_take_a_delay() {
    let (mut session, _, clip) = session_with_clip();
    let set = run(
        &mut session,
        "animation.set",
        json!({ "clip": clip, "in": { "preset": "swoop_in", "delay": "0.5s" }, "out": { "preset": "blur_out", "delay": { "frames": 3 } } }),
    );
    let animations = &set.results[0]["animations"];
    assert_eq!(animations["in"]["delay"], SECOND / 2);
    assert_eq!(animations["out"]["delay"], SECOND / 10);
    assert_eq!(animations["out"]["preset"], "blur_out");
    let negative = fails(
        &mut session,
        "animation.set",
        json!({ "clip": clip, "in": { "preset": "fade", "delay": "-1s" } }),
    );
    assert_eq!(negative.code, codes::INVALID_ARGUMENTS);
    let caps = session.query("capabilities", json!({})).unwrap();
    let presets: Vec<&str> = caps["animations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["preset"].as_str().unwrap())
        .collect();
    for name in [
        "blur_in", "blur_out", "pop_in", "rise_in", "swoop_in", "glow_in",
    ] {
        assert!(presets.contains(&name), "{name} is listed");
    }
    let commands: Vec<&str> = caps["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(commands.contains(&"clips.stagger"));
    let light: Vec<&str> = caps["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["category"] == "light")
        .map(|e| e["type"].as_str().unwrap())
        .collect();
    assert_eq!(light, ["light.glow", "light.sweep", "light.streaks"]);
}

#[test]
fn light_effects_are_added_and_animated_like_any_other() {
    let (mut session, _, clip) = session_with_clip();
    let added = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "light.glow", "params": { "radius": 0.08, "tint": 1, "color": "#FF7A00" } }),
    );
    let effect = added.results[0]["effect"].as_str().unwrap().to_string();
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": format!("effects.{effect}.intensity"), "offset": "0s", "value": 0 }),
    );
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": format!("effects.{effect}.intensity"), "offset": "1s", "value": 3 }),
    );
    let sweep = run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "light.sweep" }),
    );
    let sweep = sweep.results[0]["effect"].as_str().unwrap().to_string();
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": clip, "property": format!("effects.{sweep}.position"), "offset": "2s", "value": 1 }),
    );
    run(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "light.streaks", "params": { "angle": 30 } }),
    );
    let out_of_range = fails(
        &mut session,
        "effect.add",
        json!({ "clip": clip, "type": "light.glow", "params": { "intensity": 9 } }),
    );
    assert_eq!(out_of_range.code, codes::INVALID_ARGUMENTS);
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    let effects = full["tracks"][0]["clips"][0]["effects"].as_array().unwrap();
    assert_eq!(effects.len(), 3);
    assert_eq!(effects[0]["params"]["color"], "#FF7A00");
}

#[test]
fn reveals_take_motions_overlap_easing_order_and_an_exit_motion() {
    let mut session = new_session();
    let added = run(
        &mut session,
        "text.add",
        json!({
            "text": "Everything you love.",
            "reveal": {
                "unit": "word", "motion": "blur", "duration": "1s", "overlap": 0.8,
                "easing": "back", "order": "center", "exit": true, "exit_motion": "drop"
            }
        }),
    );
    let clip = added.results[0]["clip"].as_str().unwrap().to_string();
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    let reveal = full["tracks"][0]["clips"][0]["reveal"].clone();
    assert_eq!(reveal["motion"], "blur");
    assert_eq!(reveal["overlap"], 0.8);
    assert_eq!(reveal["easing"], "back");
    assert_eq!(reveal["order"], "center");
    assert_eq!(reveal["exit_motion"], "drop");
    let bezier = run(
        &mut session,
        "text.set",
        json!({ "clip": clip, "reveal": { "unit": "letter", "motion": "collapse", "duration": "1s", "easing": { "bezier": [0.2, 1.4, 0.4, 1.0] } } }),
    );
    assert_eq!(bezier.results[0]["clip"], clip);
    for (reveal, case) in [
        (
            json!({ "unit": "word", "motion": "fade", "duration": "1s", "overlap": 1.5 }),
            "overlap above 1",
        ),
        (
            json!({ "unit": "word", "motion": "fade", "duration": "1s", "easing": { "bezier": [1.5, 0, 0.5, 1] } }),
            "x outside 0 to 1",
        ),
    ] {
        let error = fails(
            &mut session,
            "text.set",
            json!({ "clip": clip, "reveal": reveal }),
        );
        assert_eq!(error.code, codes::TEXT_INVALID, "{case}");
    }
    let unknown = fails(
        &mut session,
        "text.set",
        json!({ "clip": clip, "reveal": { "unit": "word", "motion": "explode", "duration": "1s" } }),
    );
    assert_eq!(unknown.code, codes::INVALID_ARGUMENTS);
}
