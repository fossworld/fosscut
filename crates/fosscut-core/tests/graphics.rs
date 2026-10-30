//! Shapes, stickers, backgrounds and project templates (ADR-0028): every built-in item is
//! valid, shapes are drawn and changed through commands, and templates build timelines.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{ApiError, Session, codes};
use fosscut_model::{MemoryStore, Project};
use serde_json::{Value, json};
use support::*;

fn fails(session: &mut Session<MemoryStore>, command: &str, args: Value) -> ApiError {
    session.execute(request(command, args)).expect_err(command)
}

fn full(session: &Session<MemoryStore>, timeline: Option<&Value>) -> Value {
    let mut args = json!({ "detail": "full" });
    if let Some(timeline) = timeline {
        args["timeline"] = timeline.clone();
    }
    session.query("timeline.read", args).unwrap()
}

#[test]
fn every_library_item_and_template_is_valid() {
    let project = Project::new(
        fosscut_model::Id::new(fosscut_model::IdKind::Project, 1),
        "Check",
        fosscut_model::TimelineFormat {
            width: 1920,
            height: 1080,
            rate: fosscut_time::Rate::FPS_30,
            sample_rate: 48_000,
        },
    );
    let visuals = fosscut_core::visuals();
    assert!(visuals.len() >= 30, "{}", visuals.len());
    let mut names: Vec<&str> = visuals.iter().map(|v| v.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), visuals.len(), "unique names");
    for kind in [
        fosscut_core::VisualKind::Shape,
        fosscut_core::VisualKind::Sticker,
        fosscut_core::VisualKind::Background,
    ] {
        assert!(
            visuals.iter().filter(|v| v.kind == kind).count() >= 8,
            "{kind:?}"
        );
    }
    for item in &visuals {
        let issues = fosscut_model::shape_issues(&project, &item.content);
        assert!(issues.is_empty(), "{}: {issues:?}", item.name);
        assert!(!item.description.is_empty() && !item.tags.is_empty());
    }
    let templates = fosscut_core::project_templates();
    assert!(templates.len() >= 5);
    let titles: Vec<String> = fosscut_core::text_templates()
        .into_iter()
        .map(|t| t.name)
        .collect();
    for template in &templates {
        for element in &template.elements {
            if let Some(title) = &element.title {
                assert!(titles.contains(title), "{}: {title}", template.name);
            }
            for item in element.background.iter().chain(&element.sticker) {
                assert!(names.contains(&item.as_str()), "{}: {item}", template.name);
            }
        }
        if let Some(kind) = &template.transition {
            assert!(fosscut_model::transition_type(kind).is_some(), "{kind}");
        }
    }
}

#[test]
fn shapes_stickers_and_backgrounds_are_drawn_and_changed() {
    let (mut session, _, _) = session_with_clip();
    // A background goes below the footage, a sticker above it.
    let background = run(
        &mut session,
        "shape.add",
        json!({ "item": "sunset", "start": "0s", "duration": "2s" }),
    );
    let sticker = run(
        &mut session,
        "shape.add",
        json!({ "item": "badge-new", "start": "0s" }),
    );
    let read = full(&session, None);
    let tracks = read["tracks"].as_array().unwrap();
    let position = |clip: &Value| {
        tracks
            .iter()
            .position(|t| {
                t["clips"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| &c["id"] == clip)
            })
            .unwrap()
    };
    let footage = tracks
        .iter()
        .position(|t| {
            t["clips"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["kind"] == "video")
        })
        .unwrap();
    assert!(position(&background.results[0]["clip"]) < footage);
    assert!(position(&sticker.results[0]["clip"]) > footage);
    let badge = tracks
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .find(|c| c["id"] == sticker.results[0]["clip"])
        .unwrap();
    assert_eq!(badge["kind"], "shape");
    assert_eq!(badge["text"], "NEW");
    assert_eq!(badge["content"]["label"]["text"], "NEW");

    // Plain shapes take their fields; a merge patch changes them.
    let star = run(
        &mut session,
        "shape.add",
        json!({ "shape": "star", "size": [0.2, 0.3], "fill": { "color": "#FFCC00" }, "start": "3s" }),
    );
    let changed = run(
        &mut session,
        "shape.set",
        json!({ "clip": star.results[0]["clip"], "set": { "fill": { "gradient": { "to": "#FF0000", "radial": true } }, "outline": { "enabled": true } } }),
    );
    let content = &changed.results[0]["content"];
    assert_eq!(content["fill"]["color"], "#FFCC00");
    assert_eq!(content["fill"]["gradient"]["radial"], true);
    assert_eq!(content["outline"]["enabled"], true);

    for (command, args, code) in [
        (
            "shape.add",
            json!({ "item": "unicorn" }),
            codes::LIBRARY_UNKNOWN_ITEM,
        ),
        (
            "shape.add",
            json!({ "shape": "brush" }),
            codes::SHAPE_INVALID,
        ),
        (
            "shape.add",
            json!({ "size": [0, 0.2] }),
            codes::SHAPE_INVALID,
        ),
        (
            "shape.add",
            json!({ "shape": "path", "points": [] }),
            codes::SHAPE_INVALID,
        ),
        (
            "shape.set",
            json!({ "clip": star.results[0]["clip"], "set": { "roundness": 3 } }),
            codes::SHAPE_INVALID,
        ),
        (
            "shape.set",
            json!({ "clip": star.results[0]["clip"], "set": { "size": "big" } }),
            codes::SHAPE_INVALID,
        ),
    ] {
        assert_eq!(
            fails(&mut session, command, args.clone()).code,
            code,
            "{args}"
        );
    }
    let title = run(&mut session, "text.add", json!({ "text": "Not a shape" }));
    let error = fails(
        &mut session,
        "shape.set",
        json!({ "clip": title.results[0]["clip"], "set": {} }),
    );
    assert_eq!(error.code, codes::SHAPE_INVALID);
    let capabilities = session.query("capabilities", json!({})).unwrap();
    assert!(capabilities["library"]["visuals"].as_array().unwrap().len() >= 30);
    assert!(
        capabilities["library"]["templates"]
            .as_array()
            .unwrap()
            .len()
            >= 5
    );
}

#[test]
fn templates_build_editable_timelines_from_media_and_words() {
    let (mut session, asset, _) = session_with_clip();
    let applied = run(
        &mut session,
        "template.apply",
        json!({ "template": "vertical-promo", "media": [asset, asset], "lines": ["Launch day"], "primary": true }),
    );
    let made = &applied.results[0];
    let timeline = made["timeline"].clone();
    assert_eq!(made["name"], "vertical-promo");
    let info = session.query("project.info", json!({})).unwrap();
    assert_eq!(info["primary_timeline"], timeline);
    let read = full(&session, Some(&timeline));
    assert_eq!(read["format"]["width"], 1080);
    assert_eq!(read["format"]["height"], 1920);
    let clips: Vec<Value> = read["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .collect();
    let videos: Vec<&Value> = clips.iter().filter(|c| c["kind"] == "video").collect();
    assert_eq!(videos.len(), 2, "two slots filled, the third left out");
    assert_eq!(videos[0]["transition"]["type"], "dissolve");
    let titles: Vec<&str> = clips
        .iter()
        .filter(|c| c["kind"] == "text")
        .map(|c| c["text"].as_str().unwrap())
        .collect();
    assert_eq!(titles[0], "Launch day", "{titles:?}");
    assert!(titles.iter().any(|t| t.starts_with("Thanks for watching")));
    // The end card and its background follow the slots.
    let background = clips.iter().find(|c| c["kind"] == "shape").unwrap();
    assert_eq!(background["start"]["seconds"], "5");
    assert_eq!(made["clips"].as_array().unwrap().len(), clips.len());

    let quote = run(
        &mut session,
        "template.apply",
        json!({ "template": "quote-card", "name": "Quote" }),
    );
    assert_eq!(quote.results[0]["name"], "Quote");
    for (args, code) in [
        (json!({ "template": "nope" }), codes::TEMPLATE_UNKNOWN),
        (
            json!({ "template": "talking-head-intro", "media": [asset, asset] }),
            codes::INVALID_ARGUMENTS,
        ),
        (
            json!({ "template": "quote-card", "media": ["asset_0999"] }),
            codes::ASSET_NOT_FOUND,
        ),
    ] {
        assert_eq!(
            fails(&mut session, "template.apply", args.clone()).code,
            code,
            "{args}"
        );
    }
}

#[test]
fn clips_can_go_on_a_free_track() {
    let (mut session, asset, clip) = session_with_clip();
    // The clip's track is taken at 1 s: a free track is found or made instead.
    let placed = run(
        &mut session,
        "clip.add",
        json!({ "asset": asset, "start": "1s", "duration": "1s", "free_track": true }),
    );
    let read = full(&session, None);
    let track_of = |id: &Value| {
        read["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| {
                t["clips"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| &c["id"] == id)
            })
            .unwrap()["id"]
            .clone()
    };
    assert_ne!(track_of(&placed.results[0]["clip"]), track_of(&json!(clip)));
    let again = run(
        &mut session,
        "clip.add",
        json!({ "asset": asset, "start": "3s", "duration": "1s", "free_track": true }),
    );
    let read = full(&session, None);
    let tracks_now = |read: &Value| read["tracks"].as_array().unwrap().len();
    let track_again = read["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| {
            t["clips"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == again.results[0]["clip"])
        })
        .unwrap()["id"]
        .clone();
    assert_eq!(
        track_again,
        track_of(&placed.results[0]["clip"]),
        "the new track is free at 3 s and is reused"
    );
    let tracks = tracks_now(&read);
    let error = fails(
        &mut session,
        "clip.add",
        json!({ "asset": asset, "free_track": true }),
    );
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);
    assert_eq!(tracks, 2, "one track added, not two");
}
