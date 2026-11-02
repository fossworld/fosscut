//! The UI kit for motion-graphics ads (ADR-0028): app parts as stickers on a grid of 1/120
//! of the canvas height, title templates for their text, and the drawings and compositions
//! listed by `capabilities`. Every part adds, keeps its size in grid units on any canvas,
//! and the parts whose stroke draws a mark draw only that mark.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use fosscut_core::{Session, VisualKind};
use fosscut_model::{
    ClipContent, Id, IdKind, MaskShape, MemoryStore, Project, TimelineFormat, path_length,
    trim_path,
};
use fosscut_time::{Flicks, Rate};
use serde_json::{Value, json};
use support::*;

/// Every part of the kit, as drawn in `ui-kit.json`.
const KIT: [&str; 21] = [
    "ui-search-bar",
    "ui-caret",
    "ui-button-primary",
    "ui-button-secondary",
    "ui-card",
    "ui-panel",
    "ui-image-slot",
    "ui-price-tag",
    "ui-toast",
    "ui-check-badge",
    "ui-notification",
    "ui-app-icon",
    "ui-tracker",
    "ui-tracker-progress",
    "ui-rating",
    "ui-map-pin",
    "ui-location-dot",
    "swoosh-underline",
    "swoosh-circle",
    "ui-screen-light",
    "ui-screen-dark",
];

fn vertical() -> Session<MemoryStore> {
    let format = TimelineFormat {
        width: 1080,
        height: 1920,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let project = Project::new(Id::new(IdKind::Project, 7), "Ad", format);
    Session::create(MemoryStore::new(), project).unwrap()
}

/// A clip as `timeline.read` shows it.
fn clip_of(session: &Session<MemoryStore>, clip: &Value) -> Value {
    let read = session
        .query("timeline.read", json!({ "detail": "full" }))
        .unwrap();
    read["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .find(|c| &c["id"] == clip)
        .unwrap()
}

/// A shape clip's shape.
fn clip_content(session: &Session<MemoryStore>, clip: &Value) -> Value {
    clip_of(session, clip)["content"].clone()
}

#[test]
fn every_part_is_listed_valid_and_on_the_grid() {
    let visuals = fosscut_core::visuals();
    for name in KIT {
        let item = visuals
            .iter()
            .find(|v| v.name == name)
            .unwrap_or_else(|| panic!("{name} is missing"));
        assert!(item.description.ends_with('.'), "{name}");
        assert!(!item.tags.is_empty(), "{name}");
        if item.kind == VisualKind::Background {
            assert_eq!(item.content.size, [1.0, 1.0], "{name}");
            continue;
        }
        assert_eq!(item.kind, VisualKind::Sticker, "{name}");
        // Sizes are whole or half grid units: height 1/120 of the canvas, width on a
        // 16:9 design canvas.
        let [w, h] = item.content.size;
        let (wu, hu) = (w * 120.0 * 16.0 / 9.0, h * 120.0);
        for units in [wu, hu] {
            assert!(
                ((units * 2.0).round() - units * 2.0).abs() < 1e-3,
                "{name}: {units} grid units"
            );
        }
    }
    // The UI kit comes after the original visuals, and no name repeats.
    let names: Vec<&str> = visuals.iter().map(|v| v.name.as_str()).collect();
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len());
    assert_eq!(names[names.len() - KIT.len()..], KIT);
}

#[test]
fn parts_add_at_their_grid_size_on_a_vertical_canvas() {
    let mut session = vertical();
    for (index, name) in KIT.iter().enumerate() {
        let placed = run(
            &mut session,
            "shape.add",
            json!({ "item": name, "start": format!("{index}s"), "duration": "1s" }),
        );
        let content = clip_content(&session, &placed.results[0]["clip"]);
        let item = fosscut_core::visual(name).unwrap();
        let size = content["size"].as_array().unwrap();
        let height = size[1].as_f64().unwrap();
        assert!((height - item.content.size[1]).abs() < 1e-9, "{name}");
        if item.kind == VisualKind::Sticker {
            // The same number of pixels per grid unit across as down: 1920 / 120 = 16.
            let width_px = size[0].as_f64().unwrap() * 1080.0;
            let expected = item.content.size[0] * 120.0 * 16.0 / 9.0 * 16.0;
            assert!((width_px - expected).abs() < 1e-6, "{name}: {width_px}");
        }
    }
}

/// The length of a part's outline that its stroke shows, as a share of the whole.
fn shown(name: &str) -> f64 {
    let item = fosscut_core::visual(name).unwrap();
    let box_px = [item.content.size[0] * 1920.0, item.content.size[1] * 1080.0];
    let path = item.content.outline(box_px);
    let trimmed = trim_path(
        &path,
        item.content.trim_start,
        item.content.trim_end,
        item.content.trim_offset,
    );
    path_length(&trimmed) / path_length(&path)
}

#[test]
fn marks_drawn_by_the_stroke_are_the_last_part_of_the_outline() {
    // The toast's check, the badge's check, the app icon's bag and the search bar's
    // magnifier: a closed shape first (filled, not stroked) and the mark after it.
    for name in ["ui-toast", "ui-check-badge", "ui-app-icon", "ui-search-bar"] {
        let item = fosscut_core::visual(name).unwrap();
        assert_eq!(item.content.shape, MaskShape::Path, "{name}");
        assert!(item.content.closed, "{name}: the filled shape comes first");
        assert!(item.content.stroke.is_some(), "{name}");
        let start = item.content.trim_start;
        assert!(start > 0.5 && start < 1.0, "{name}: {start}");
        // The description names where the mark starts.
        let named: f64 = item
            .description
            .split("from trim ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|number| number.parse().ok())
            .unwrap_or_else(|| panic!("{name}: no trim in {}", item.description));
        assert!((named - start).abs() < 0.001, "{name}: {named} and {start}");
        let share = shown(name);
        assert!(share > 0.03 && share < 0.4, "{name}: {share}");
    }
    // The tracker's progress stops at its third step.
    let progress = fosscut_core::visual("ui-tracker-progress").unwrap();
    assert!((shown("ui-tracker-progress") - progress.content.trim_end).abs() < 0.01);
    assert!(progress.description.contains("0.69"));
}

#[test]
fn draw_on_draws_only_the_mark() {
    let mut session = vertical();
    let toast = run(
        &mut session,
        "shape.add",
        json!({ "item": "ui-toast", "duration": "2s" }),
    );
    let clip = toast.results[0]["clip"].clone();
    run(
        &mut session,
        "animation.set",
        json!({ "clip": clip, "in": { "preset": "draw_on", "duration": "1s" } }),
    );
    let project = session.project().clone();
    let id = Id::parse(clip.as_str().unwrap()).unwrap();
    let found = project
        .timelines
        .values()
        .find_map(|t| t.find_clip(&id).map(|(_, c)| c.clone()))
        .unwrap();
    let item = fosscut_core::visual("ui-toast").unwrap();
    let start = item.content.trim_start;
    let at = |seconds: f64| {
        found
            .shape_at(Flicks::from_seconds_f64(seconds).unwrap())
            .unwrap()
    };
    // Before the draw the stroke shows nothing of the pill; halfway it has drawn part of
    // the check; afterwards all of it, and never anything before the check.
    let first = at(0.0);
    assert!((first.trim_start - start).abs() < 1e-9 && (first.trim_end - start).abs() < 1e-9);
    let middle = at(0.5);
    assert!(middle.trim_end > start && middle.trim_end < 1.0);
    assert!((middle.trim_start - start).abs() < 1e-9);
    assert!((at(1.5).trim_end - 1.0).abs() < 1e-9);
    assert!(matches!(found.content, ClipContent::Shape(_)));
}

#[test]
fn title_templates_for_app_text_and_kinetic_type_add() {
    let mut session = vertical();
    for (name, lines) in [
        ("ui-search-text", json!(["desk lamps"])),
        ("ui-status-text", json!(["Arriving today", "2 stops away"])),
        ("kinetic-type", json!(["Order today", "Get it today"])),
    ] {
        let placed = run(
            &mut session,
            "text.template",
            json!({ "template": name, "lines": lines, "duration": "3s" }),
        );
        // A text clip's fields are the clip's own in timeline.read.
        let clip = clip_of(&session, &placed.results[0]["clip"]);
        let text = clip["text"].as_str().unwrap();
        for line in lines.as_array().unwrap() {
            assert!(text.contains(line.as_str().unwrap()), "{name}: {text}");
        }
        assert!(clip["reveal"].is_object(), "{name}");
    }
}

#[test]
fn capabilities_list_the_drawings_and_compositions() {
    let session = new_session();
    let brief = session.query("capabilities", json!({})).unwrap();
    let drawings = brief["library"]["drawings"].as_array().unwrap();
    let phone = drawings.iter().find(|d| d["name"] == "phone").unwrap();
    assert_eq!(phone["kind"], "device");
    assert_eq!(phone["screen"]["canvas"], json!([720, 1560]));
    assert!(
        drawings
            .iter()
            .any(|d| d["name"] == "street-map" && d["kind"] == "drawing")
    );
    let compositions = brief["library"]["compositions"].as_array().unwrap();
    assert_eq!(compositions[0]["name"], "shop-app");
    assert_eq!(compositions[0]["lines"][0], "running shoes");
    let full = session
        .query("capabilities", json!({ "detail": "full" }))
        .unwrap();
    let laptop = full["library"]["drawings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "laptop")
        .unwrap()
        .clone();
    assert_eq!(laptop["size"], json!([1600, 864]));
    assert!(laptop["description"].as_str().unwrap().contains("16:9"));
    let visuals = full["library"]["visuals"].as_array().unwrap();
    assert!(visuals.iter().any(|v| v["name"] == "ui-toast"));
}
