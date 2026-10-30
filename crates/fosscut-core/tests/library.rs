//! The media bin and a project's timelines: renaming, removing, folders, relinking and
//! replacing media; adding timelines from media, renaming, choosing and removing them.

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

fn media(session: &Session<MemoryStore>) -> Value {
    session.query("media.list", json!({})).unwrap()
}

fn second_asset(session: &mut Session<MemoryStore>, size: u64, seconds: i64) -> String {
    let mut other = test_pattern_args();
    other["identity"]["size"] = json!(size);
    other["name"] = json!(format!("other-{size}.mp4"));
    other["info"]["duration"] = json!(seconds * SECOND);
    let added = run(session, "media.add", other);
    added.results[0]["asset"].as_str().unwrap().to_string()
}

#[test]
fn assets_rename_move_into_folders_and_are_removed_only_when_unused() {
    let (mut session, asset, clip) = session_with_clip();
    run(
        &mut session,
        "media.rename",
        json!({ "asset": asset, "name": "  Interview  " }),
    );
    assert_eq!(media(&session)["assets"][0]["name"], "Interview");
    run(
        &mut session,
        "media.move",
        json!({ "assets": [asset], "folder": "B-roll/Day 1" }),
    );
    let listed = media(&session);
    assert_eq!(listed["folders"], json!(["B-roll", "B-roll/Day 1"]));
    assert_eq!(listed["assets"][0]["folder"], "B-roll/Day 1");
    // Renaming a folder moves its subfolders and assets.
    run(
        &mut session,
        "media.folder",
        json!({ "action": "rename", "folder": "B-roll", "to": "Footage/B-roll" }),
    );
    let listed = media(&session);
    assert_eq!(
        listed["folders"],
        json!(["Footage", "Footage/B-roll", "Footage/B-roll/Day 1"])
    );
    assert_eq!(listed["assets"][0]["folder"], "Footage/B-roll/Day 1");
    // Removing a folder moves what is in it up.
    run(
        &mut session,
        "media.folder",
        json!({ "action": "remove", "folder": "Footage/B-roll" }),
    );
    let listed = media(&session);
    assert_eq!(listed["folders"], json!(["Footage", "Footage/Day 1"]));
    assert_eq!(listed["assets"][0]["folder"], "Footage/Day 1");
    run(&mut session, "media.move", json!({ "assets": [asset] }));
    assert!(
        media(&session)["assets"][0]
            .get("folder")
            .is_none_or(Value::is_null)
    );
    for (command, args) in [
        ("media.rename", json!({ "asset": asset, "name": " " })),
        ("media.move", json!({ "assets": [asset], "folder": "a//b" })),
        (
            "media.folder",
            json!({ "action": "rename", "folder": "Nowhere", "to": "X" }),
        ),
        (
            "media.folder",
            json!({ "action": "rename", "folder": "Footage", "to": "Footage/In" }),
        ),
        ("media.move", json!({ "assets": [] })),
    ] {
        assert_eq!(
            fails(&mut session, command, args.clone()).code,
            codes::INVALID_ARGUMENTS,
            "{command} {args}"
        );
    }
    // In use: refused, then removed with its clips when asked.
    let busy = fails(&mut session, "media.remove", json!({ "asset": asset }));
    assert_eq!(busy.code, codes::ASSET_IN_USE);
    assert_eq!(busy.details["clips"], json!([clip]));
    let guarded = fails(
        &mut session,
        "media.remove",
        json!({ "asset": asset, "remove_clips": true }),
    );
    assert_eq!(guarded.code, codes::GUARD_DESTRUCTIVE);
    let mut remove = request(
        "media.remove",
        json!({ "asset": asset, "remove_clips": true }),
    );
    remove.allow_destructive = true;
    let removed = session.execute(remove).unwrap();
    assert_eq!(removed.results[0]["clips"], json!([clip]));
    assert!(media(&session)["assets"].as_array().unwrap().is_empty());
    let gone = fails(
        &mut session,
        "media.rename",
        json!({ "asset": asset, "name": "X" }),
    );
    assert_eq!(gone.code, codes::ASSET_NOT_FOUND);
}

#[test]
fn relinking_needs_the_same_file_and_replacing_swaps_media_everywhere() {
    let (mut session, asset, clip) = session_with_clip();
    let identity = test_pattern_args()["identity"].clone();
    run(
        &mut session,
        "media.relink",
        json!({ "asset": asset, "path": { "absolute": "/moved/test-pattern.mp4" }, "identity": identity }),
    );
    assert_eq!(
        media(&session)["assets"][0]["path"]["absolute"],
        "/moved/test-pattern.mp4"
    );
    let other = json!({ "size": 1, "sample_blake3": "cd".repeat(32) });
    let wrong = fails(
        &mut session,
        "media.relink",
        json!({ "asset": asset, "path": { "absolute": "/x.mp4" }, "identity": other }),
    );
    assert_eq!(wrong.code, codes::ASSET_IDENTITY_MISMATCH);
    let nowhere = fails(
        &mut session,
        "media.relink",
        json!({ "asset": asset, "path": {} }),
    );
    assert_eq!(nowhere.code, codes::INVALID_ARGUMENTS);
    // The clip reads all 8 seconds; a 4-second file is too short.
    let short = second_asset(&mut session, 21, 4);
    let too_short = fails(
        &mut session,
        "media.replace",
        json!({ "asset": asset, "with": short }),
    );
    assert_eq!(too_short.code, codes::CLIP_SOURCE_RANGE);
    let long = second_asset(&mut session, 22, 10);
    let replaced = run(
        &mut session,
        "media.replace",
        json!({ "asset": asset, "with": long }),
    );
    assert_eq!(replaced.results[0]["clips"], json!([clip]));
    let view = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    assert_eq!(view["tracks"][0]["clips"][0]["asset"], long);
    let same = fails(
        &mut session,
        "media.replace",
        json!({ "asset": long, "with": long }),
    );
    assert_eq!(same.code, codes::INVALID_ARGUMENTS);
}

#[test]
fn timelines_are_added_from_media_renamed_chosen_and_removed() {
    let (mut session, asset, _) = session_with_clip();
    let image = {
        let mut still = test_pattern_args();
        still["kind"] = json!("image");
        still["name"] = json!("still.png");
        still["identity"]["size"] = json!(31);
        still["info"] = json!({ "video": { "width": 1080, "height": 1920 } });
        run(&mut session, "media.add", still).results[0]["asset"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let added = run(
        &mut session,
        "timeline.add",
        json!({ "assets": [asset, image], "primary": true }),
    );
    let result = &added.results[0];
    let id = result["timeline"].as_str().unwrap().to_string();
    assert_eq!(result["name"], "Timeline 2");
    assert_eq!(result["primary"], true);
    assert_eq!(
        result["duration"]["seconds"], "11",
        "8 s of video and a 3 s still"
    );
    let view = session.query("timeline.read", json!({})).unwrap();
    assert_eq!(view["timeline"], id, "the primary timeline now");
    assert_eq!(view["format"]["width"], 1280, "the first video's canvas");
    assert_eq!(view["tracks"][0]["clip_count"], 2);
    run(
        &mut session,
        "timeline.rename",
        json!({ "timeline": id, "name": "Shorts" }),
    );
    let info = session.query("project.info", json!({})).unwrap();
    assert!(
        info["timelines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "Shorts")
    );
    let first = info["timelines"][0]["id"].as_str().unwrap().to_string();
    run(
        &mut session,
        "timeline.set_primary",
        json!({ "timeline": first }),
    );
    let again = fails(
        &mut session,
        "timeline.set_primary",
        json!({ "timeline": first }),
    );
    assert_eq!(again.code, codes::INVALID_ARGUMENTS);
    let guarded = fails(&mut session, "timeline.remove", json!({ "timeline": id }));
    assert_eq!(guarded.code, codes::GUARD_DESTRUCTIVE, "it has clips");
    let mut remove = request("timeline.remove", json!({ "timeline": first }));
    remove.allow_destructive = true;
    let removed = session.execute(remove).unwrap();
    assert_eq!(
        removed.results[0]["primary"], id,
        "the next one became primary"
    );
    let mut last = request("timeline.remove", json!({ "timeline": id }));
    last.allow_destructive = true;
    assert_eq!(
        session.execute(last).expect_err("the last one").code,
        codes::INVALID_ARGUMENTS
    );
    for args in [
        json!({ "width": 0, "height": 10 }),
        json!({ "rate": "fast" }),
        json!({ "assets": ["asset_0999"] }),
    ] {
        let code = fails(&mut session, "timeline.add", args.clone()).code;
        assert!(
            code == codes::INVALID_ARGUMENTS || code == codes::ASSET_NOT_FOUND,
            "{args}: {code}"
        );
    }
}
