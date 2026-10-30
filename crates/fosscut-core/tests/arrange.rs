//! Timeline editing: groups that edit together and extracted sound (ADR-0024), markers,
//! the clipboard, copied attributes, split all, replacing media, ripple trims and
//! alignment.

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

fn read(session: &Session<MemoryStore>) -> Value {
    session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap()
}

/// Every clip of the timeline by id, with its track id added.
fn clip(session: &Session<MemoryStore>, id: &str) -> Value {
    let view = read(session);
    for track in view["tracks"].as_array().unwrap() {
        for clip in track["clips"].as_array().unwrap() {
            if clip["id"] == id {
                let mut clip = clip.clone();
                clip["track"] = track["id"].clone();
                return clip;
            }
        }
    }
    panic!("no clip {id}");
}

fn flicks(value: &Value) -> i64 {
    value["flicks"].as_i64().unwrap()
}

fn title(session: &mut Session<MemoryStore>, start: &str, text: &str) -> String {
    let added = run(
        session,
        "text.add",
        json!({ "text": text, "start": start, "duration": "2s" }),
    );
    added.results[0]["clip"].as_str().unwrap().to_string()
}

#[test]
fn groups_move_split_trim_and_remove_together() {
    let (mut session, _, video) = session_with_clip();
    let caption = title(&mut session, "1s", "Linked");
    let grouped = run(
        &mut session,
        "clip.group",
        json!({ "clips": [video, caption] }),
    );
    let group = grouped.results[0]["group"].as_str().unwrap().to_string();
    assert_eq!(clip(&session, &caption)["group"], group);
    let moved = run(
        &mut session,
        "clip.move",
        json!({ "clip": video, "start": "2s" }),
    );
    assert_eq!(moved.results[0]["linked"][0]["clip"], caption);
    assert_eq!(flicks(&clip(&session, &caption)["start"]), 3 * SECOND);
    // Alone, with linked: false.
    run(
        &mut session,
        "clip.move",
        json!({ "clip": caption, "start": "4s", "linked": false }),
    );
    assert_eq!(flicks(&clip(&session, &video)["start"]), 2 * SECOND);
    // A split cuts the members spanning the time; their right parts form a new group.
    let split = run(
        &mut session,
        "clip.split",
        json!({ "clip": video, "at": "5s" }),
    );
    let right = split.results[0]["right"]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let caption_right = split.results[0]["linked"][0]["right"]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let new_group = clip(&session, &right)["group"].clone();
    assert!(new_group.is_string() && new_group != json!(group));
    assert_eq!(clip(&session, &caption_right)["group"], new_group);
    assert_eq!(
        clip(&session, &caption)["group"],
        group,
        "left parts keep theirs"
    );
    // Trimming a shared edge trims both.
    let trimmed = run(
        &mut session,
        "clip.trim",
        json!({ "clip": right, "start": "5.5s" }),
    );
    assert_eq!(trimmed.results[0]["linked"][0]["clip"], caption_right);
    assert_eq!(
        flicks(&clip(&session, &caption_right)["start"]),
        5 * SECOND + SECOND / 2
    );
    // A member that cannot follow fails the whole command.
    let blocker = title(&mut session, "8s", "In the way");
    let caption_track = clip(&session, &caption)["track"].clone();
    run(
        &mut session,
        "clip.move",
        json!({ "clip": blocker, "track": caption_track, "start": "1.5s", "linked": false }),
    );
    let before = read(&session)["revision"].clone();
    let error = fails(
        &mut session,
        "clip.move",
        json!({ "clip": video, "start": "0s" }),
    );
    assert_eq!(error.code, codes::CLIP_OVERLAP);
    assert_eq!(read(&session)["revision"], before, "nothing moved");
    // Removing one removes the group; the group left with one clip dissolves.
    let mut remove = request("clip.remove", json!({ "clip": right }));
    remove.allow_destructive = true;
    let removed = session.execute(remove).unwrap();
    assert_eq!(removed.results[0]["linked"], json!([caption_right]));
    run(&mut session, "clip.ungroup", json!({ "clips": [caption] }));
    assert!(
        clip(&session, &video).get("group").is_none(),
        "a group of one dissolves"
    );
    for (args, code) in [
        (json!({ "clips": [video] }), codes::INVALID_ARGUMENTS),
        (
            json!({ "clips": [video, "clip_0999"] }),
            codes::CLIP_NOT_FOUND,
        ),
    ] {
        assert_eq!(fails(&mut session, "clip.group", args).code, code);
    }
    let none = fails(&mut session, "clip.ungroup", json!({ "clips": [video] }));
    assert_eq!(none.code, codes::INVALID_ARGUMENTS);
}

#[test]
fn extracted_sound_stays_linked_and_restores() {
    let (mut session, _, video) = session_with_clip();
    run(
        &mut session,
        "clip.set",
        json!({ "clip": video, "set": { "volume": 0.5, "audio.fade_in": "1s" } }),
    );
    let extracted = run(&mut session, "clip.extract_audio", json!({ "clip": video }));
    let audio = extracted.results[0]["audio"]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    let view = clip(&session, &audio);
    assert_eq!(view["kind"], "audio");
    assert_eq!(view["volume"], 0.5);
    assert_eq!(view["group"], clip(&session, &video)["group"]);
    assert_eq!(clip(&session, &video)["audio_detached"], true);
    let again = fails(&mut session, "clip.extract_audio", json!({ "clip": video }));
    assert_eq!(again.code, codes::INVALID_ARGUMENTS);
    // The pair moves together.
    run(
        &mut session,
        "clip.move",
        json!({ "clip": video, "start": "1s" }),
    );
    assert_eq!(flicks(&clip(&session, &audio)["start"]), SECOND);
    let restored = run(&mut session, "clip.restore_audio", json!({ "clip": video }));
    assert_eq!(restored.results[0]["removed"], json!([audio]));
    let view = clip(&session, &video);
    assert!(view.get("audio_detached").is_none() && view.get("group").is_none());
    let twice = fails(&mut session, "clip.restore_audio", json!({ "clip": video }));
    assert_eq!(twice.code, codes::INVALID_ARGUMENTS);
    let words = title(&mut session, "0s", "Hi");
    let text = fails(&mut session, "clip.extract_audio", json!({ "clip": words }));
    assert_eq!(text.code, codes::INVALID_ARGUMENTS);
}

#[test]
fn markers_mark_times_in_order() {
    let (mut session, _, _) = session_with_clip();
    let late = run(
        &mut session,
        "marker.add",
        json!({ "at": "3s", "name": "drop", "color": "#00ff00" }),
    );
    let late = late.results[0]["marker"].as_str().unwrap().to_string();
    run(&mut session, "marker.add", json!({ "at": "1s" }));
    let markers = read(&session)["markers"].clone();
    assert_eq!(flicks(&markers[0]["at"]), SECOND, "in time order");
    assert_eq!(markers[1]["name"], "drop");
    assert_eq!(markers[1]["color"], "#00FF00");
    run(
        &mut session,
        "marker.set",
        json!({ "marker": late, "at": "0s", "name": "", "color": "" }),
    );
    let first = &read(&session)["markers"][0];
    assert_eq!(first["id"], late);
    assert!(first.get("name").is_none() && first.get("color").is_none());
    run(&mut session, "marker.remove", json!({ "marker": late }));
    assert_eq!(read(&session)["markers"].as_array().unwrap().len(), 1);
    for (command, args, code) in [
        (
            "marker.remove",
            json!({ "marker": late }),
            codes::MARKER_NOT_FOUND,
        ),
        (
            "marker.set",
            json!({ "marker": late, "at": "1s" }),
            codes::MARKER_NOT_FOUND,
        ),
        (
            "marker.add",
            json!({ "at": "1s", "color": "green" }),
            codes::INVALID_ARGUMENTS,
        ),
        (
            "marker.add",
            json!({ "at": { "frames": -3 } }),
            codes::INVALID_ARGUMENTS,
        ),
    ] {
        assert_eq!(fails(&mut session, command, args).code, code, "{command}");
    }
}

#[test]
fn copied_clips_paste_with_new_ids_and_their_arrangement() {
    let (mut session, _, video) = session_with_clip();
    let words = title(&mut session, "2s", "Copy me");
    run(
        &mut session,
        "clip.group",
        json!({ "clips": [video, words] }),
    );
    run(
        &mut session,
        "effect.add",
        json!({ "clip": video, "type": "color.vignette" }),
    );
    let copied = session
        .query("clips.copy", json!({ "clips": [video, words] }))
        .unwrap();
    let pasted = run(
        &mut session,
        "clips.paste",
        json!({ "clips": copied["clips"], "at": "10s" }),
    );
    let placements = pasted.results[0]["clips"].as_array().unwrap();
    let (new_video, new_words) = (
        placements[0]["clip"].as_str().unwrap(),
        placements[1]["clip"].as_str().unwrap(),
    );
    assert_ne!(new_video, video);
    assert_eq!(
        flicks(&placements[1]["start"]),
        12 * SECOND,
        "the offset kept"
    );
    let (a, b) = (clip(&session, new_video), clip(&session, new_words));
    assert_eq!(a["group"], b["group"]);
    assert_ne!(
        a["group"],
        clip(&session, &video)["group"],
        "a group of their own"
    );
    assert_ne!(
        a["effects"][0]["id"],
        clip(&session, &video)["effects"][0]["id"]
    );
    let error = fails(
        &mut session,
        "clips.paste",
        json!({ "clips": copied["clips"], "at": "11s" }),
    );
    assert_eq!(error.code, codes::CLIP_OVERLAP, "never overwrites");
    let empty = fails(
        &mut session,
        "clips.paste",
        json!({ "clips": [], "at": "0s" }),
    );
    assert_eq!(empty.code, codes::INVALID_ARGUMENTS);
    let none = session
        .query("clips.copy", json!({ "clips": [] }))
        .expect_err("nothing to copy");
    assert_eq!(none.code, codes::INVALID_ARGUMENTS);
}

#[test]
fn attributes_copy_without_media() {
    let (mut session, _, video) = session_with_clip();
    let words = title(&mut session, "0s", "Styled");
    run(
        &mut session,
        "clip.set",
        json!({ "clip": video, "set": { "opacity": 0.4, "blend": "screen", "transform.scale": 0.5 } }),
    );
    run(
        &mut session,
        "keyframe.set",
        json!({ "clip": video, "property": "transform.rotation", "at": "1s", "value": 10 }),
    );
    run(
        &mut session,
        "effect.add",
        json!({ "clip": video, "type": "color.vignette" }),
    );
    let copied = run(
        &mut session,
        "clip.copy_attributes",
        json!({ "from": video, "to": [words], "attributes": ["opacity", "transform", "effects"] }),
    );
    assert_eq!(copied.results[0]["clips"], json!([words]));
    let view = clip(&session, &words);
    assert_eq!(view["opacity"], 0.4);
    assert_eq!(view["transform"]["scale"], 0.5);
    assert_eq!(view["blend"], "normal", "blend was not asked for");
    assert_eq!(view["kind"], "text", "the media stays");
    assert!(view["keyframes"].get("transform.rotation").is_some());
    assert_eq!(view["effects"][0]["type"], "color.vignette");
    let unknown = fails(
        &mut session,
        "clip.copy_attributes",
        json!({ "from": video, "to": [words], "attributes": ["sparkle"] }),
    );
    assert_eq!(unknown.code, codes::INVALID_ARGUMENTS);
}

#[test]
fn split_all_replace_ripple_trims_and_alignment() {
    let (mut session, asset, video) = session_with_clip();
    let words = title(&mut session, "3s", "Over");
    let split = run(&mut session, "timeline.split", json!({ "at": "4s" }));
    assert_eq!(split.results[0]["splits"].as_array().unwrap().len(), 2);
    let none = fails(&mut session, "timeline.split", json!({ "at": "20s" }));
    assert_eq!(none.code, codes::CLIP_SPLIT_OUTSIDE);
    // Delete left with ripple: the clip keeps its start, what follows comes earlier.
    let right = split.results[0]["splits"][0]["right"]["clip"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut session,
        "clip.trim",
        json!({ "clip": video, "start": "1s", "ripple": true }),
    );
    let view = clip(&session, &video);
    assert_eq!(flicks(&view["start"]), 0);
    assert_eq!(flicks(&view["duration"]), 3 * SECOND);
    assert_eq!(flicks(&view["source_in"]), SECOND);
    assert_eq!(
        flicks(&clip(&session, &right)["start"]),
        3 * SECOND,
        "pulled earlier"
    );
    // Replacing media keeps the place.
    let mut other = test_pattern_args();
    other["identity"]["size"] = json!(9);
    other["name"] = json!("other.mp4");
    let other = run(&mut session, "media.add", other);
    let other = other.results[0]["asset"].as_str().unwrap().to_string();
    run(
        &mut session,
        "clip.replace",
        json!({ "clip": right, "asset": other }),
    );
    assert_eq!(clip(&session, &right)["asset"], other);
    let long = fails(
        &mut session,
        "clip.replace",
        json!({ "clip": right, "asset": asset, "source_in": "6s" }),
    );
    assert_eq!(long.code, codes::CLIP_SOURCE_RANGE);
    let wrong = fails(
        &mut session,
        "clip.replace",
        json!({ "clip": words, "asset": asset }),
    );
    assert_eq!(wrong.code, codes::INVALID_ARGUMENTS);
    // Alignment puts the visible picture against an edge.
    run(
        &mut session,
        "clip.set",
        json!({ "clip": video, "set": { "transform.scale": 0.5, "crop.left": 0.5 } }),
    );
    let aligned = run(
        &mut session,
        "clip.align",
        json!({ "clip": video, "horizontal": "left", "vertical": "bottom" }),
    );
    // Half width, the left half cropped away: the visible quarter starts at the centre
    // of the picture, so the centre sits at the canvas's left edge.
    assert_eq!(aligned.results[0]["position"], json!([-0.5, 0.25]));
    let centred = run(
        &mut session,
        "clip.align",
        json!({ "clip": words, "horizontal": "center" }),
    );
    assert_eq!(centred.results[0]["position"], json!([0.0, 0.0]));
    let edge = fails(
        &mut session,
        "clip.align",
        json!({ "clip": words, "vertical": "top" }),
    );
    assert_eq!(
        edge.code,
        codes::INVALID_ARGUMENTS,
        "text has no known size"
    );
    let nothing = fails(&mut session, "clip.align", json!({ "clip": video }));
    assert_eq!(nothing.code, codes::INVALID_ARGUMENTS);
}
