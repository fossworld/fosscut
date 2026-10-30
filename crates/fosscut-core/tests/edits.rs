//! Whole-edit commands: clip properties through `clip.set`, vertical copies through
//! `timeline.duplicate`, and cutting time ranges across picture, titles and captions through
//! `timeline.remove_ranges`, with its destructive guard and undo.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{ApiError, Session, codes};
use fosscut_model::MemoryStore;
use serde_json::{Value, json};
use support::*;

fn fails(session: &mut Session<MemoryStore>, command: &str, args: Value) -> ApiError {
    session.execute(request(command, args)).expect_err(command)
}

fn destructive(session: &mut Session<MemoryStore>, command: &str, args: Value) -> Value {
    let mut request = request(command, args);
    request.allow_destructive = true;
    let response = session.execute(request).unwrap();
    response.results[0].clone()
}

fn timeline(session: &Session<MemoryStore>, args: Value) -> Value {
    session.query("timeline.read", args).unwrap()
}

fn seconds(report: &Value) -> &str {
    report["seconds"].as_str().unwrap()
}

#[test]
fn clip_properties_are_set_by_path() {
    let (mut session, _, clip) = session_with_clip();
    let set = run(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": {
            "transform.position": [-0.25, 0.1],
            "transform.scale": 2.5,
            "transform.rotation": 90,
            "opacity": 0.5,
            "volume": 0.2
        } }),
    );
    let result = &set.results[0];
    assert_eq!(result["transform"]["position"], json!([-0.25, 0.1]));
    assert_eq!(result["transform"]["scale"], 2.5);
    assert_eq!(result["transform"]["rotation"], 90.0);
    assert_eq!(
        (result["opacity"].clone(), result["volume"].clone()),
        (json!(0.5), json!(0.2))
    );
    let full = timeline(&session, json!({ "detail": "full" }));
    assert_eq!(full["tracks"][0]["clips"][0]["transform"]["scale"], 2.5);

    let unknown = fails(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "speed": 2 } }),
    );
    assert_eq!(unknown.code, codes::INVALID_ARGUMENTS);
    assert!(unknown.hint.unwrap().contains("transform.position"));
    for (set, case) in [
        (
            json!({ "transform.position": [1] }),
            "position needs two numbers",
        ),
        (
            json!({ "transform.position": "left" }),
            "position is a pair",
        ),
        (json!({ "transform.scale": 0 }), "scale above zero"),
        (json!({ "transform.rotation": "x" }), "rotation is a number"),
        (json!({}), "something to set"),
    ] {
        let error = fails(
            &mut session,
            "clip.set",
            json!({ "clip": clip, "set": set }),
        );
        assert_eq!(error.code, codes::INVALID_ARGUMENTS, "{case}");
    }
    let opaque = fails(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "opacity": 2 } }),
    );
    assert_eq!(opaque.code, codes::MODEL_INVALID);
    let title = run(&mut session, "text.add", json!({ "text": "Quiet" }));
    let silent = fails(
        &mut session,
        "clip.set",
        json!({ "clip": title.results[0]["clip"], "set": { "volume": 1 } }),
    );
    assert_eq!(silent.code, codes::INVALID_ARGUMENTS);
    let missing = fails(
        &mut session,
        "clip.set",
        json!({ "clip": "clip_0999", "set": { "opacity": 1 } }),
    );
    assert_eq!(missing.code, codes::CLIP_NOT_FOUND);
}

#[test]
fn audio_clips_have_no_picture() {
    let (mut session, _, _) = session_with_clip();
    let voice = run(
        &mut session,
        "media.add",
        json!({
            "name": "voice.wav",
            "kind": "audio",
            "path": { "absolute": "/fixtures/voice.wav" },
            "identity": { "size": 4242, "sample_blake3": "cd".repeat(32) },
            "info": { "duration": 4 * 705_600_000i64, "audio": { "sample_rate": 48000, "channels": 1 } }
        }),
    );
    let asset = voice.results[0]["asset"].clone();
    let audio = run(&mut session, "track.add", json!({ "kind": "audio" }));
    let placed = run(
        &mut session,
        "clip.add",
        json!({ "asset": asset, "track": audio.results[0]["track"], "duration": "1s" }),
    );
    let clip = placed.results[0]["clip"].clone();
    let error = fails(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "opacity": 0.5 } }),
    );
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);
    let louder = run(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "volume": 1.5 } }),
    );
    assert_eq!(louder.results[0]["volume"], 1.5);
}

#[test]
fn timelines_are_copied_onto_a_vertical_canvas() {
    let (mut session, _, clip) = session_with_clip();
    run(
        &mut session,
        "clip.set",
        json!({ "clip": clip, "set": { "transform.position": [0.1, 0] } }),
    );
    run(&mut session, "text.add", json!({ "text": "Title" }));
    let copied = run(
        &mut session,
        "timeline.duplicate",
        json!({ "width": 1080, "height": 1920, "fit": "fill", "name": "Vertical" }),
    );
    let result = &copied.results[0];
    assert_eq!(result["name"], "Vertical");
    assert_eq!(result["clips"], 2);
    assert_eq!(result["primary"], false);
    assert_eq!(result["format"]["width"], 1080);
    let id = result["timeline"].clone();
    let vertical = timeline(&session, json!({ "timeline": id, "detail": "full" }));
    let picture = &vertical["tracks"][0]["clips"][0];
    assert_ne!(picture["id"], json!(clip), "copies get their own ids");
    // A 1280x720 picture fitted to a 1080x1920 canvas covers it when scaled by
    // (1920 / 720) / (1080 / 1280).
    let scale = picture["transform"]["scale"].as_f64().unwrap();
    assert!(
        (scale - (1920.0 / 720.0) / (1080.0 / 1280.0)).abs() < 1e-9,
        "{scale}"
    );
    assert_eq!(picture["transform"]["position"], json!([0.1, 0.0]));
    assert!(
        vertical["tracks"][1]["clips"][0]["transform"]
            .get("scale")
            .is_none(),
        "titles keep their size (the default, omitted)"
    );
    // The original is unchanged and still primary.
    let original = timeline(&session, json!({ "detail": "full" }));
    assert_eq!(original["format"]["width"], 1920);
    assert_eq!(original["tracks"][0]["clips"][0]["id"], json!(clip));
    let info = session.query("project.info", json!({})).unwrap();
    assert_eq!(info["timelines"].as_array().unwrap().len(), 2);

    // A copy kept as it is, made primary.
    let kept = run(
        &mut session,
        "timeline.duplicate",
        json!({ "primary": true }),
    );
    assert_eq!(kept.results[0]["primary"], true);
    assert_eq!(kept.results[0]["name"], "Timeline 1 1920x1080");
    let primary = timeline(&session, json!({ "detail": "full" }));
    assert_eq!(primary["timeline"], kept.results[0]["timeline"]);
    assert!(
        primary["tracks"][0]["clips"][0]["transform"]
            .get("scale")
            .is_none()
    );
    let flat = fails(&mut session, "timeline.duplicate", json!({ "height": 0 }));
    assert_eq!(flat.code, codes::INVALID_ARGUMENTS);
    let none = fails(
        &mut session,
        "timeline.duplicate",
        json!({ "timeline": "timeline_0099" }),
    );
    assert_eq!(none.code, codes::TIMELINE_NOT_FOUND);
}

/// The 8 s test pattern on the primary video track, a 3 s title, and captions with words
/// at 0.5-0.9, 1.2-1.6 (inside the first cut), 2.5-2.9 and 4.6-5.0 s.
fn edit() -> (Session<MemoryStore>, String) {
    let (mut session, _, clip) = session_with_clip();
    run(
        &mut session,
        "text.add",
        json!({ "text": "Intro", "start": "0s", "duration": "3s" }),
    );
    let word = |text: &str, start: &str, end: &str| json!({ "text": text, "start": { "seconds": start }, "end": { "seconds": end } });
    run(
        &mut session,
        "captions.add",
        json!({ "words": [
            word("one", "0.5", "0.9"),
            word("um", "1.2", "1.6"),
            word("two", "2.5", "2.9"),
            word("three.", "4.6", "5")
        ], "paging": { "max_words": 4, "max_gap_ms": 5000 } }),
    );
    (session, clip)
}

fn caption_words(view: &Value) -> Vec<(String, String)> {
    view["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["kind"] == "caption")
        .flat_map(|t| t["clips"].as_array().unwrap().iter())
        .flat_map(|c| c["lines"].as_array().unwrap().iter())
        .flat_map(|l| l.as_array().unwrap().iter())
        .map(|w| {
            (
                w["text"].as_str().unwrap().to_string(),
                seconds(&w["start"]).to_string(),
            )
        })
        .collect()
}

#[test]
fn ranges_are_cut_from_every_track_and_closed() {
    let (mut session, clip) = edit();
    let ranges = json!([
        { "start": { "seconds": "1" }, "end": { "seconds": "2" } },
        { "start": { "seconds": "4" }, "end": { "seconds": "4.5" } },
        { "start": { "seconds": "1.5" }, "end": { "seconds": "1.8" } }
    ]);

    // Cutting the primary video needs permission; the details say what would go.
    let guarded = fails(
        &mut session,
        "timeline.remove_ranges",
        json!({ "ranges": ranges }),
    );
    assert_eq!(guarded.code, codes::GUARD_DESTRUCTIVE);
    assert_eq!(guarded.details["ranges"].as_array().unwrap().len(), 2);

    let dry = {
        let mut request = request("timeline.remove_ranges", json!({ "ranges": ranges }));
        request.allow_destructive = true;
        request.dry_run = true;
        session.execute(request).unwrap()
    };
    assert!(!dry.committed);

    let result = destructive(
        &mut session,
        "timeline.remove_ranges",
        json!({ "ranges": ranges }),
    );
    assert_eq!(
        result["ranges"].as_array().unwrap().len(),
        2,
        "overlaps merge"
    );
    assert_eq!(seconds(&result["removed"]), "1.5");
    assert_eq!(seconds(&result["duration_before"]), "8");
    assert_eq!(seconds(&result["duration_after"]), "6.5");

    let view = timeline(&session, json!({ "detail": "full" }));
    let video = view["tracks"][0]["clips"].as_array().unwrap();
    let spans: Vec<(&str, &str, &str)> = video
        .iter()
        .map(|c| {
            (
                seconds(&c["start"]),
                seconds(&c["end"]),
                seconds(&c["source_in"]),
            )
        })
        .collect();
    assert_eq!(
        spans,
        [("0", "1", "0"), ("1", "3", "2"), ("3", "6.5", "4.5")]
    );
    assert_eq!(video[0]["id"], json!(clip), "the first part keeps the id");
    assert_eq!(
        result["clips_added"].as_array().unwrap().len(),
        5,
        "two video parts, one title part and two caption pages"
    );
    // The title is cut too: 0-1 s and what was 2-3 s, now 1-2 s.
    let titles: Vec<(&str, &str)> = view["tracks"][1]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (seconds(&c["start"]), seconds(&c["end"])))
        .collect();
    assert_eq!(titles, [("0", "1"), ("1", "2")]);
    // Captions lose the word in the cut and keep the others with their media.
    assert_eq!(
        caption_words(&view),
        [
            ("one".to_string(), "0.5".to_string()),
            ("two".to_string(), "1.5".to_string()),
            ("three.".to_string(), "3.1".to_string())
        ]
    );

    // One undo brings everything back.
    run(&mut session, "history.undo", json!({}));
    let restored = timeline(&session, json!({ "detail": "full" }));
    assert_eq!(restored["tracks"][0]["clips"].as_array().unwrap().len(), 1);
    assert_eq!(caption_words(&restored).len(), 4);
}

#[test]
fn cuts_can_leave_gaps_or_spare_tracks() {
    let (mut session, _) = edit();
    let ranges = json!([{ "start": "1s", "end": "2s" }]);
    // Without ripple the gap stays and nothing moves.
    let gap = destructive(
        &mut session,
        "timeline.remove_ranges",
        json!({ "ranges": ranges, "ripple": false }),
    );
    assert_eq!(seconds(&gap["duration_after"]), "8");
    let view = timeline(&session, json!({ "detail": "full" }));
    let starts: Vec<&str> = view["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| seconds(&c["start"]))
        .collect();
    assert_eq!(starts, ["0", "2"]);
    run(&mut session, "history.undo", json!({}));

    // Only the caption track: no permission needed, the picture keeps its length.
    let doc = timeline(&session, json!({ "detail": "outline" }));
    let captions = doc["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "caption")
        .unwrap()["id"]
        .clone();
    let spared = run(
        &mut session,
        "timeline.remove_ranges",
        json!({ "ranges": ranges, "tracks": [captions] }),
    );
    assert_eq!(seconds(&spared.results[0]["duration_after"]), "8");
    assert_eq!(
        caption_words(&timeline(&session, json!({ "detail": "full" }))).len(),
        3
    );

    // A whole caption page inside a range disappears.
    let all = destructive(
        &mut session,
        "timeline.remove_ranges",
        json!({ "ranges": [{ "start": "0s", "end": "6s" }] }),
    );
    assert!(!all["clips_removed"].as_array().unwrap().is_empty());
    assert!(caption_words(&timeline(&session, json!({ "detail": "full" }))).is_empty());

    for (ranges, case) in [
        (json!([]), "no ranges"),
        (json!([{ "start": "2s", "end": "1s" }]), "backwards"),
        (json!([{ "start": "1s", "end": "1s" }]), "empty"),
    ] {
        let error = fails(
            &mut session,
            "timeline.remove_ranges",
            json!({ "ranges": ranges }),
        );
        assert_eq!(error.code, codes::INVALID_ARGUMENTS, "{case}");
    }
    let wrong = fails(
        &mut session,
        "timeline.remove_ranges",
        json!({ "ranges": [{ "start": "1s", "end": "2s" }], "tracks": ["track_0999"] }),
    );
    assert_eq!(wrong.code, codes::TRACK_NOT_FOUND);
}
