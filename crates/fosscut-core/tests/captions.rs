//! Caption commands (ADR-0021): paging timed words into caption clips on a vertical canvas,
//! the replace guard, editing a page, styling a track with presets and patches, splitting
//! caption clips, and the typed errors.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{Session, codes};
use fosscut_model::{Id, IdKind, MemoryStore, Project, ProjectStore, TimelineFormat};
use fosscut_time::Rate;
use serde_json::{Value, json};
use support::*;

/// A 1080x1920 project at 30 fps.
fn vertical() -> Session<MemoryStore> {
    let format = TimelineFormat {
        width: 1080,
        height: 1920,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let project = Project::new(Id::new(IdKind::Project, 0x2b), "Short", format);
    Session::create(MemoryStore::new(), project).unwrap()
}

/// Words as `captions.add` takes them, from (text, start ms, end ms).
fn words(list: &[(&str, u32, u32)]) -> Value {
    list.iter()
        .map(|(text, start, end)| {
            json!({
                "text": text,
                "start": { "seconds": format!("{}", f64::from(*start) / 1000.0) },
                "end": { "seconds": format!("{}", f64::from(*end) / 1000.0) },
            })
        })
        .collect()
}

fn fails(session: &mut Session<MemoryStore>, command: &str, args: Value) -> fosscut_core::ApiError {
    session.execute(request(command, args)).expect_err(command)
}

fn timeline_doc(session: &Session<MemoryStore>) -> Value {
    let store = session.store();
    let path = store
        .paths()
        .into_iter()
        .find(|p| p.starts_with("timelines/"))
        .unwrap();
    serde_json::from_slice(&store.read(&path).unwrap()).unwrap()
}

const SPEECH: &[(&str, u32, u32)] = &[
    ("Most", 400, 620),
    ("people", 620, 900),
    ("never", 900, 1150),
    ("notice", 1150, 1500),
    ("this.", 1500, 1900),
    ("But", 2600, 2750),
    ("once", 2750, 3000),
    ("you", 3000, 3100),
    ("do,", 3100, 3400),
    ("everything", 3450, 3900),
    ("changes.", 3900, 4500),
];

#[test]
fn words_become_pages_on_a_caption_track() {
    let mut session = vertical();
    let added = run(
        &mut session,
        "captions.add",
        json!({ "words": words(SPEECH), "preset": "bold-pop" }),
    );
    let result = &added.results[0];
    assert_eq!(result["words"], 11);
    assert_eq!(result["removed"], 0);
    let texts: Vec<&str> = result["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["text"].as_str().unwrap())
        .collect();
    // Four words a page at most, breaking after sentences and at the 0.7 s pause; lines
    // wrap at the 12 characters of bold-pop's black capitals that fit a tall canvas.
    assert_eq!(
        texts,
        [
            "Most people\nnever notice",
            "this.",
            "But once you\ndo,",
            "everything\nchanges."
        ]
    );
    let first = &result["pages"][0];
    assert_eq!(first["start"]["frames"], 12, "the frame of the first word");

    // The track is a caption track with the preset's look stored sparsely.
    let doc = timeline_doc(&session);
    let track = doc["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "caption")
        .unwrap();
    assert_eq!(track["name"], "Captions");
    assert_eq!(track["caption_style"]["weight"], "black");
    assert!(track["caption_style"].get("size").is_some());
    assert!(
        track["caption_style"].get("placement").is_none(),
        "defaults are omitted"
    );
    let clip = &track["clips"][0];
    assert_eq!(clip["kind"], "caption");
    assert_eq!(clip["lines"][0]["words"][0]["text"], "Most");

    // The timeline projection shows text, and at full detail word times on the timeline.
    let normal = session
        .query("timeline.read", json!({ "detail": "normal" }))
        .unwrap();
    let caption = &normal["tracks"][0]["clips"][0];
    assert_eq!(caption["text"], "Most people\nnever notice");
    assert_eq!(caption["words"], 4);
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    let word = &full["tracks"][0]["clips"][3]["lines"][0][0];
    assert_eq!(word["text"], "everything");
    assert_eq!(word["start"]["seconds"], "3.45");
    assert_eq!(full["tracks"][0]["caption_style"]["weight"], "black");

    // Capabilities describe the presets and animators.
    let capabilities = session
        .query("capabilities", json!({ "detail": "outline" }))
        .unwrap();
    assert_eq!(capabilities["captions"]["presets"][0]["name"], "bold-pop");
    assert_eq!(
        capabilities["captions"]["animators"][3]["type"],
        "word.highlight"
    );
    assert!(
        capabilities["captions"]["animators"][3]
            .get("params_schema")
            .is_none()
    );
    let full = session
        .query("capabilities", json!({ "detail": "full" }))
        .unwrap();
    assert!(full["captions"]["animators"][3]["params_schema"].is_object());
    assert!(full["captions"]["presets"][0]["style"].is_object());
}

#[test]
fn captions_never_overwrite_unless_replaced_destructively() {
    let mut session = vertical();
    run(
        &mut session,
        "captions.add",
        json!({ "words": words(SPEECH) }),
    );
    let overlap = fails(
        &mut session,
        "captions.add",
        json!({ "words": words(&[("again", 500, 900)]) }),
    );
    assert_eq!(overlap.code, codes::CLIP_OVERLAP);
    let guarded = fails(
        &mut session,
        "captions.add",
        json!({ "words": words(&[("again", 500, 900)]), "replace": true }),
    );
    assert_eq!(guarded.code, codes::GUARD_DESTRUCTIVE);
    let mut replace = request(
        "captions.add",
        json!({ "words": words(&[("again", 500, 900)]), "replace": true }),
    );
    replace.allow_destructive = true;
    let replaced = session.execute(replace).unwrap();
    assert_eq!(replaced.results[0]["removed"], 4);
    assert_eq!(replaced.results[0]["pages"].as_array().unwrap().len(), 1);
}

#[test]
fn pages_are_edited_word_by_word_or_reworded() {
    let mut session = vertical();
    let added = run(
        &mut session,
        "captions.add",
        json!({ "words": words(SPEECH) }),
    );
    let page = added.results[0]["pages"][3]["clip"].clone();

    // New wording shares the page's spoken time by length.
    let reworded = run(
        &mut session,
        "captions.edit",
        json!({ "clip": page, "text": "it all changes", "emphasis": [1] }),
    );
    assert_eq!(reworded.results[0]["text"], "it all changes");
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    let line = &full["tracks"][0]["clips"][3]["lines"][0];
    assert_eq!(line[0]["start"]["seconds"], "3.45");
    assert_eq!(line[2]["end"]["seconds"], "4.5");
    assert_eq!(line[1]["emphasis"], true);
    assert!(line[0].get("emphasis").is_none());

    // Timed words must stay inside the page.
    let edited = run(
        &mut session,
        "captions.edit",
        json!({ "clip": page, "words": words(&[("All", 3500, 3800), ("change.", 3800, 4400)]) }),
    );
    assert_eq!(edited.results[0]["text"], "All change.");
    let outside = fails(
        &mut session,
        "captions.edit",
        json!({ "clip": page, "words": words(&[("late", 9000, 9500)]) }),
    );
    assert_eq!(outside.code, codes::CAPTIONS_INVALID_WORDS);
    let index = fails(
        &mut session,
        "captions.edit",
        json!({ "clip": page, "emphasis": [5] }),
    );
    assert_eq!(index.code, codes::CAPTIONS_INVALID_WORDS);
    let both = fails(
        &mut session,
        "captions.edit",
        json!({ "clip": page, "text": "x", "words": words(&[("y", 3500, 3600)]) }),
    );
    assert_eq!(both.code, codes::INVALID_ARGUMENTS);
    let nothing = fails(&mut session, "captions.edit", json!({ "clip": page }));
    assert_eq!(nothing.code, codes::INVALID_ARGUMENTS);
    let empty = fails(
        &mut session,
        "captions.edit",
        json!({ "clip": page, "text": "  \n " }),
    );
    assert_eq!(empty.code, codes::CAPTIONS_INVALID_WORDS);
    let title = run(&mut session, "text.add", json!({ "text": "Title" }));
    let not_caption = fails(
        &mut session,
        "captions.edit",
        json!({ "clip": title.results[0]["clip"], "text": "x" }),
    );
    assert_eq!(not_caption.code, codes::TRACK_KIND_MISMATCH);

    // Two lines given explicitly stay two lines.
    let two = run(
        &mut session,
        "captions.edit",
        json!({ "clip": page, "text": "first line\nsecond" }),
    );
    assert_eq!(two.results[0]["text"], "first line\nsecond");
}

#[test]
fn tracks_are_styled_by_presets_and_patches() {
    let mut session = vertical();
    let missing = fails(&mut session, "captions.style", json!({ "preset": "boxed" }));
    assert_eq!(missing.code, codes::TRACK_NOT_FOUND);
    run(
        &mut session,
        "captions.add",
        json!({ "words": words(SPEECH), "preset": "karaoke" }),
    );
    let styled = run(
        &mut session,
        "captions.style",
        json!({ "preset": "boxed", "style": { "size": 0.07, "placement": "middle", "stroke": { "width": 0.2 } } }),
    );
    let style = &styled.results[0]["style"];
    assert_eq!(style["box"]["enabled"], true);
    assert_eq!(style["size"], 0.07);
    assert_eq!(style["placement"], "middle");
    assert_eq!(
        style["stroke"]["enabled"], false,
        "the preset's stroke stays off"
    );
    assert_eq!(style["animators"][1]["type"], "word.highlight");

    // A patch alone changes the current look; animators are replaced as a whole.
    let patched = run(
        &mut session,
        "captions.style",
        json!({ "style": { "animators": [{ "type": "word.reveal" }] } }),
    );
    let animators = patched.results[0]["style"]["animators"].as_array().unwrap();
    assert_eq!(animators.len(), 1);
    assert_eq!(animators[0]["version"], 1);
    assert_eq!(patched.results[0]["style"]["size"], 0.07);

    let unknown = fails(&mut session, "captions.style", json!({ "preset": "neon" }));
    assert_eq!(unknown.code, codes::CAPTIONS_UNKNOWN_PRESET);
    let bad = fails(
        &mut session,
        "captions.style",
        json!({ "style": { "weight": "heavy" } }),
    );
    assert_eq!(bad.code, codes::INVALID_ARGUMENTS);
    let invalid = fails(
        &mut session,
        "captions.style",
        json!({ "style": { "size": -1 } }),
    );
    assert_eq!(invalid.code, codes::MODEL_INVALID, "{invalid:?}");
    let neither = fails(&mut session, "captions.style", json!({}));
    assert_eq!(neither.code, codes::INVALID_ARGUMENTS);

    // Back to every default stores no style at all.
    let defaults = serde_json::to_value(fosscut_model::CaptionStyle::default()).unwrap();
    run(
        &mut session,
        "captions.style",
        json!({ "style": { "box": defaults["box"], "stroke": defaults["stroke"], "size": defaults["size"], "placement": "bottom", "animators": [], "weight": "bold" } }),
    );
    let doc = timeline_doc(&session);
    assert!(doc["tracks"][0].get("caption_style").is_none(), "{doc}");

    let video = run(&mut session, "track.add", json!({ "kind": "video" }));
    let wrong = fails(
        &mut session,
        "captions.style",
        json!({ "track": video.results[0]["track"], "preset": "boxed" }),
    );
    assert_eq!(wrong.code, codes::TRACK_KIND_MISMATCH);
    let wrong_add = fails(
        &mut session,
        "captions.add",
        json!({ "track": video.results[0]["track"], "words": words(&[("x", 9000, 9100)]) }),
    );
    assert_eq!(wrong_add.code, codes::TRACK_KIND_MISMATCH);
}

#[test]
fn caption_clips_split_between_words() {
    let mut session = vertical();
    let added = run(
        &mut session,
        "captions.add",
        json!({ "words": words(SPEECH) }),
    );
    let page = added.results[0]["pages"][0]["clip"].clone();
    // "Most people / never notice": the cut at 0.9 s falls between "people" and "never".
    let split = run(
        &mut session,
        "clip.split",
        json!({ "clip": page, "at": { "seconds": "0.9" } }),
    );
    let right = split.results[0]["right"]["clip"].clone();
    let normal = session
        .query("timeline.read", json!({ "detail": "normal" }))
        .unwrap();
    let clips = normal["tracks"][0]["clips"].as_array().unwrap();
    assert_eq!(clips[0]["text"], "Most people");
    assert_eq!(clips[1]["id"], right);
    assert_eq!(clips[1]["text"], "never notice");
    let full = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    assert_eq!(
        full["tracks"][0]["clips"][1]["lines"][0][0]["start"]["seconds"], "0.9",
        "words keep their timeline times"
    );
    // A cut that would leave a side without words is refused: "this." (1.5 to 1.9 s) on
    // a page that lingers until 2.2 s.
    let lonely = added.results[0]["pages"][1]["clip"].clone();
    let refused = fails(
        &mut session,
        "clip.split",
        json!({ "clip": lonely, "at": { "seconds": "2" } }),
    );
    assert_eq!(refused.code, codes::CAPTIONS_INVALID_WORDS);

    // Caption clips only go on caption tracks.
    let text_track = run(&mut session, "track.add", json!({ "kind": "text" }));
    let moved = fails(
        &mut session,
        "clip.move",
        json!({ "clip": page, "track": text_track.results[0]["track"] }),
    );
    assert_eq!(moved.code, codes::TRACK_KIND_MISMATCH);
}

#[test]
fn bad_words_are_typed_errors() {
    let mut session = vertical();
    for (case, list) in [
        ("empty", json!([])),
        ("blank", words(&[(" ", 0, 100)])),
        ("backwards", words(&[("a", 500, 400)])),
        ("overlap", words(&[("a", 0, 500), ("b", 400, 600)])),
        (
            "negative",
            json!([{ "text": "a", "start": { "seconds": "-1" }, "end": { "seconds": "0" } }]),
        ),
    ] {
        let error = fails(&mut session, "captions.add", json!({ "words": list }));
        assert_eq!(error.code, codes::CAPTIONS_INVALID_WORDS, "{case}");
    }
    let wide = fails(
        &mut session,
        "captions.add",
        json!({ "words": words(SPEECH), "preset": "neon" }),
    );
    assert_eq!(wide.code, codes::CAPTIONS_UNKNOWN_PRESET);
    // Paging options apply: one word a page.
    let single = run(
        &mut session,
        "captions.add",
        json!({ "words": words(&SPEECH[..3]), "paging": { "max_words": 1, "linger_ms": 0 } }),
    );
    assert_eq!(single.results[0]["pages"].as_array().unwrap().len(), 3);
}

#[test]
fn captions_never_lengthen_the_timeline() {
    // The 8 s test pattern, with a last word that ends 0.1 s before the video does: its
    // page would linger 0.3 s past the end, adding frames of black to an export.
    let (mut session, _, _) = session_with_clip();
    let before = session
        .query("timeline.read", json!({ "detail": "outline" }))
        .unwrap()["duration"]
        .clone();
    let added = run(
        &mut session,
        "captions.add",
        json!({ "words": words(&[("Almost", 7000, 7500), ("done.", 7500, 7900)]) }),
    );
    assert_eq!(added.results[0]["pages"][0]["end"]["seconds"], "8");
    let after = session
        .query("timeline.read", json!({ "detail": "outline" }))
        .unwrap()["duration"]
        .clone();
    assert_eq!(after, before);
    // Words spoken after the rest of the edit still get their own time.
    let late = run(
        &mut session,
        "captions.add",
        json!({ "words": words(&[("Bonus", 9000, 9400)]) }),
    );
    assert_eq!(late.results[0]["pages"][0]["end"]["seconds"], "9.4");
}
