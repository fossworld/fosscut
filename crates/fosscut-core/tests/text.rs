//! Styled text (ADR-0027): presets and style patches, spans, reveals, project fonts, saved
//! presets and title templates; and where sound effects may go (ADR-0026) and bilingual
//! caption pages.

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

/// The clip `id` as `timeline.read` with full detail shows it.
fn clip(session: &Session<MemoryStore>, id: &Value) -> Value {
    let timeline = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    timeline["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .find(|c| &c["id"] == id)
        .unwrap()
}

/// A text clip's text, style, spans and reveal as `timeline.read` shows them.
fn content(session: &Session<MemoryStore>, id: &Value) -> Value {
    let found = clip(session, id);
    let mut content = json!({ "text": found["text"], "style": found["style"] });
    for key in ["spans", "reveal"] {
        if let Some(value) = found.get(key) {
            content[key] = value.clone();
        }
    }
    content
}

#[test]
fn titles_take_presets_patches_spans_and_reveals() {
    let mut session = new_session();
    let added = run(
        &mut session,
        "text.add",
        json!({
            "text": "Breaking news",
            "preset": "outline",
            "style": { "case": "upper", "letter_spacing": 0.05, "stroke": { "width": 0.15 } },
            "spans": [{ "start": 9, "end": 13, "color": "#FFCC00", "bold": true }],
            "reveal": { "unit": "letter", "duration": { "seconds": "1" }, "exit": true },
        }),
    );
    let id = added.results[0]["clip"].clone();
    let changed = run(
        &mut session,
        "text.set",
        json!({ "clip": id, "style": { "underline": true } }),
    );
    let text = &changed.results[0]["content"];
    assert_eq!(text["text"], "Breaking news");
    assert_eq!(text["style"]["bold"], true, "from the outline preset");
    assert_eq!(
        text["style"]["stroke"],
        json!({ "enabled": true, "width": 0.15 })
    );
    assert_eq!(text["style"]["case"], "upper");
    assert_eq!(text["style"]["letter_spacing"], 0.05);
    assert_eq!(text["style"]["underline"], true);
    assert_eq!(text["spans"][0]["color"], "#FFCC00");
    assert_eq!(text["reveal"]["unit"], "letter");
    assert_eq!(text["reveal"]["duration"], SECOND);
    assert_eq!(text["reveal"]["exit"], true);

    // New text drops spans past its end; a preset replaces the style; null resets.
    let changed = run(
        &mut session,
        "text.set",
        json!({ "clip": id, "text": "Short", "preset": "neon", "style": { "glow": { "radius": null } }, "reveal": null }),
    );
    let text = &changed.results[0]["content"];
    assert_eq!(text["text"], "Short");
    assert!(text.get("spans").is_none() && text.get("reveal").is_none());
    assert_eq!(text["style"]["glow"]["color"], "#FF2FD6");
    assert!(
        text["style"]["glow"].get("radius").is_none(),
        "back to the default"
    );

    for (args, code) in [
        (
            json!({ "text": "x", "preset": "nope" }),
            codes::TEXT_UNKNOWN_PRESET,
        ),
        (
            json!({ "text": "x", "style": { "font": "Comic Sans" } }),
            codes::TEXT_UNKNOWN_FONT,
        ),
        (
            json!({ "text": "x", "style": { "letter_spacing": 5 } }),
            codes::TEXT_INVALID,
        ),
        (
            json!({ "text": "abc", "spans": [{ "start": 2, "end": 9, "bold": true }] }),
            codes::TEXT_INVALID,
        ),
        (
            json!({ "text": "x", "style": { "shadow": { "enabled": true, "blur": 3 } } }),
            codes::TEXT_INVALID,
        ),
        (
            json!({ "text": "x", "reveal": { "unit": "word", "duration": "0f" } }),
            codes::TEXT_INVALID,
        ),
        (
            json!({ "text": "x", "style": { "weight": "heavy" } }),
            codes::TEXT_INVALID,
        ),
    ] {
        assert_eq!(
            fails(&mut session, "text.add", args.clone()).code,
            code,
            "{args}"
        );
    }
    let (mut other, _, video) = session_with_clip();
    let error = fails(
        &mut other,
        "text.set",
        json!({ "clip": video, "text": "x" }),
    );
    assert_eq!(error.code, codes::TEXT_NOT_TEXT);
    assert_eq!(
        fails(&mut session, "text.set", json!({ "clip": id })).code,
        codes::TEXT_INVALID
    );
}

#[test]
fn project_fonts_can_be_named_once_imported() {
    let mut session = new_session();
    let error = fails(
        &mut session,
        "text.add",
        json!({ "text": "Hi", "style": { "font": "Fixture Sans" } }),
    );
    assert_eq!(error.code, codes::TEXT_UNKNOWN_FONT);
    assert!(error.hint.unwrap().contains("media import"));
    run(
        &mut session,
        "media.add",
        json!({
            "name": "Fixture Sans.ttf",
            "kind": "font",
            "path": { "absolute": "/fonts/Fixture Sans.ttf" },
            "info": { "font": { "families": ["Fixture Sans"], "faces": 1 } },
        }),
    );
    let added = run(
        &mut session,
        "text.add",
        json!({ "text": "Hi", "style": { "font": "Fixture Sans" }, "spans": [{ "start": 0, "end": 1, "font": "Inter" }] }),
    );
    assert!(added.committed);
    let info = session.query("project.info", json!({})).unwrap();
    assert_eq!(info["fonts"], json!(["Fixture Sans", "Inter"]));
    // A font is not media to place.
    let font = session.query("media.list", json!({})).unwrap()["assets"][0]["id"].clone();
    let error = fails(&mut session, "clip.add", json!({ "asset": font }));
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);
    let incomplete = fails(
        &mut session,
        "media.add",
        json!({ "name": "x.ttf", "kind": "font", "path": { "absolute": "/x.ttf" } }),
    );
    assert_eq!(incomplete.code, codes::MEDIA_INCOMPLETE);
}

#[test]
fn styles_are_saved_as_project_presets() {
    let mut session = new_session();
    let title = run(
        &mut session,
        "text.add",
        json!({ "text": "Brand", "style": { "color": "#FF3366", "bold": true } }),
    );
    let id = title.results[0]["clip"].clone();
    let saved = run(
        &mut session,
        "text.preset_save",
        json!({ "name": "Brand red", "clip": id }),
    );
    assert_eq!(saved.results[0]["presets"][0]["name"], "Brand red");
    assert!(
        !saved.changes.is_empty(),
        "the change summary shows the preset"
    );
    // A saved preset hides a built-in one of the same name.
    run(
        &mut session,
        "text.preset_save",
        json!({ "name": "outline", "style": { "italic": true } }),
    );
    let used = run(
        &mut session,
        "text.add",
        json!({ "text": "Again", "preset": "Brand red" }),
    );
    let again = content(&session, &used.results[0]["clip"]);
    assert_eq!(again["style"]["color"], "#FF3366");
    let shadowed = run(
        &mut session,
        "text.add",
        json!({ "text": "Mine", "preset": "outline" }),
    );
    let mine = content(&session, &shadowed.results[0]["clip"]);
    assert_eq!(mine["style"]["italic"], true);
    assert!(mine["style"].get("stroke").is_none());
    let info = session.query("project.info", json!({})).unwrap();
    assert_eq!(info["text_presets"], json!(["Brand red", "outline"]));

    let error = fails(
        &mut session,
        "text.preset_save",
        json!({ "name": "Brand red", "style": {} }),
    );
    assert_eq!(error.code, codes::TEXT_PRESET_EXISTS);
    run(
        &mut session,
        "text.preset_save",
        json!({ "name": "Brand red", "style": { "color": "#00FF00" }, "replace": true }),
    );
    for args in [
        json!({ "name": " ", "style": {} }),
        json!({ "name": "both", "clip": id, "style": {} }),
        json!({ "name": "neither" }),
        json!({ "name": "far", "style": { "line_height": 9 } }),
    ] {
        assert_eq!(
            fails(&mut session, "text.preset_save", args.clone()).code,
            codes::TEXT_INVALID,
            "{args}"
        );
    }
    let removed = run(
        &mut session,
        "text.preset_remove",
        json!({ "name": "outline" }),
    );
    assert_eq!(removed.results[0]["presets"].as_array().unwrap().len(), 1);
    assert_eq!(
        fails(
            &mut session,
            "text.preset_remove",
            json!({ "name": "outline" })
        )
        .code,
        codes::TEXT_UNKNOWN_PRESET
    );
    // Clips keep the style they were given.
    assert_eq!(
        content(&session, &shadowed.results[0]["clip"])["style"]["italic"],
        true
    );
}

#[test]
fn title_templates_become_one_styled_animated_clip() {
    let mut session = new_session();
    let added = run(
        &mut session,
        "text.template",
        json!({ "template": "lower-third", "lines": ["Sam Lee"], "start": "0f", "duration": "1s" }),
    );
    let id = added.results[0]["clip"].clone();
    let placed = clip(&session, &id);
    let text = content(&session, &id);
    assert_eq!(
        text["text"], "Sam Lee",
        "a line left out is dropped, not shown as a placeholder"
    );
    assert_eq!(text["style"]["align"], "left");
    assert_eq!(
        text["spans"][0],
        json!({ "start": 0, "end": 7, "bold": true })
    );
    let placeholders = run(
        &mut session,
        "text.template",
        json!({ "template": "lower-third", "start": "2s" }),
    );
    let shown = content(&session, &placeholders.results[0]["clip"]);
    assert_eq!(
        shown["text"], "Alex Morgan\nFounder",
        "no lines: the placeholders"
    );
    assert_eq!(shown["spans"][1]["start"], 12);
    assert_eq!(placed["transform"]["position"], json!([-0.25, 0.3]));
    // The reveal never takes more than half the clip.
    assert_eq!(text["reveal"]["duration"]["flicks"], SECOND / 2);

    let headline = run(
        &mut session,
        "text.template",
        json!({ "template": "headline", "lines": ["Launch day", ""], "style": { "size": 0.12 } }),
    );
    let text = content(&session, &headline.results[0]["clip"]);
    assert_eq!(text["text"], "Launch day", "an empty line is dropped");
    assert_eq!(text["style"]["size"], 0.12);
    assert_eq!(text["reveal"]["motion"], "rise");

    for (args, code) in [
        (json!({ "template": "nope" }), codes::TEXT_UNKNOWN_TEMPLATE),
        (
            json!({ "template": "callout", "lines": ["a", "b"] }),
            codes::TEXT_INVALID,
        ),
        (
            json!({ "template": "callout", "lines": [" "] }),
            codes::TEXT_EMPTY,
        ),
    ] {
        assert_eq!(
            fails(&mut session, "text.template", args.clone()).code,
            code,
            "{args}"
        );
    }
    let named = session
        .query(
            "capabilities",
            json!({ "names": ["text.template", "audio.limiter"] }),
        )
        .unwrap();
    assert_eq!(named["commands"][0]["name"], "text.template");
    assert_eq!(named["effects"][0]["type"], "audio.limiter");
    assert_eq!(named["effects"][0]["params"][0]["animatable"], false);
    let capabilities = session.query("capabilities", json!({})).unwrap();
    let templates = capabilities["text"]["templates"].as_array().unwrap();
    assert!(templates.len() >= 6);
    assert!(capabilities["text"]["presets"].as_array().unwrap().len() >= 10);
    assert!(capabilities["text"]["style_schema"].is_object());
}

#[test]
fn sound_effects_go_on_clips_with_sound_and_pages_take_translations() {
    let (mut session, _, video) = session_with_clip();
    // The test pattern has sound: both kinds of effect fit it.
    run(
        &mut session,
        "effect.add",
        json!({ "clip": video, "type": "audio.denoise", "params": { "amount": 0.5 } }),
    );
    run(
        &mut session,
        "effect.add",
        json!({ "clip": video, "type": "color.adjust" }),
    );
    let title = run(&mut session, "text.add", json!({ "text": "Quiet" }));
    let error = fails(
        &mut session,
        "effect.add",
        json!({ "clip": title.results[0]["clip"], "type": "audio.enhance_voice" }),
    );
    assert_eq!(error.code, codes::EFFECT_TARGET);
    let error = fails(
        &mut session,
        "effect.add",
        json!({ "clip": video, "type": "audio.vocals", "params": { "mode": "louder" } }),
    );
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);
    // Copying a picture effect onto a title is fine; a sound one is refused.
    let error = fails(
        &mut session,
        "effect.copy",
        json!({ "from": video, "to": [title.results[0]["clip"]] }),
    );
    assert_eq!(error.code, codes::EFFECT_TARGET);

    let captions = run(
        &mut session,
        "captions.add",
        json!({ "words": [
            { "text": "Hello", "start": { "seconds": "0.5" }, "end": { "seconds": "0.9" } },
            { "text": "world.", "start": { "seconds": "0.9" }, "end": { "seconds": "1.4" } }
        ] }),
    );
    let page = captions.results[0]["pages"][0]["clip"].clone();
    let edited = run(
        &mut session,
        "captions.edit",
        json!({ "clip": page, "translation": " Hola mundo. " }),
    );
    assert_eq!(edited.results[0]["translation"], "Hola mundo.");
    assert_eq!(edited.results[0]["text"], "Hello world.");
    let cleared = run(
        &mut session,
        "captions.edit",
        json!({ "clip": page, "translation": null }),
    );
    assert!(cleared.results[0].get("translation").is_none());
    run(
        &mut session,
        "captions.style",
        json!({ "style": { "translation": { "size": 0.8, "italic": true } } }),
    );
    let error = fails(
        &mut session,
        "captions.style",
        json!({ "style": { "translation": { "size": 2 } } }),
    );
    assert_eq!(error.code, codes::MODEL_INVALID);
}

#[test]
fn capabilities_list_every_reveal_motion_and_only_those() {
    let session = new_session();
    let caps = session.query("capabilities", json!({})).unwrap();
    let reveal = &caps["text"]["reveal"];
    let names = |key: &str| -> Vec<String> {
        reveal[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    // Every listed motion is one the model reads, and every motion the model has is listed:
    // the schema's enum is the full set.
    let schema = serde_json::to_value(schemars::schema_for!(fosscut_model::RevealMotion)).unwrap();
    let mut known: Vec<String> = schema["enum"]
        .as_array()
        .or_else(|| schema["oneOf"].as_array())
        .unwrap()
        .iter()
        .filter_map(|v| {
            v.as_str()
                .map(str::to_string)
                .or_else(|| v["const"].as_str().map(str::to_string))
        })
        .collect();
    let mut listed = names("motions");
    known.sort();
    listed.sort();
    assert_eq!(listed, known);
    for order in names("orders") {
        serde_json::from_value::<fosscut_model::RevealOrder>(json!(order)).unwrap();
    }
}
