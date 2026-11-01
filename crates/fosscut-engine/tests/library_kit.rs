//! The motion-graphics library (ADR-0028): device frames and the street map import as
//! drawings and render with their screens open, every UI kit sticker and title renders, and
//! the shop app composition renders inside the phone's screen where the library says it is.
//! The composition needs a browser: skipped without one unless `FOSSCUT_REQUIRE_BROWSER` is
//! set.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use common::{renderer, required};
use fosscut_core::Request;
use fosscut_engine::frame::frame;
use fosscut_engine::library;
use fosscut_engine::project::{Projects, create, snapshot};
use fosscut_model::TimelineFormat;
use fosscut_render::{Image, Renderer};
use fosscut_time::{Flicks, Rate};
use serde_json::{Value, json};

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-kit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn new_project(dir: &Path, width: u32, height: u32) -> (PathBuf, Projects) {
    let bundle = dir.join("Kit.fosscut");
    let format = TimelineFormat {
        width,
        height,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    create(&bundle, "Kit", format).unwrap();
    (bundle, Projects::with_runtime("test", dir.join("runtime")))
}

fn run(projects: &Projects, bundle: &Path, command: &str, args: Value) -> Value {
    let response = projects
        .execute(bundle, Request::single(command, args))
        .unwrap_or_else(|e| panic!("{command}: {e:?}"));
    response.results[0].clone()
}

/// Imports library media item `name` as the front ends do and places it.
fn add_media(
    projects: &Projects,
    bundle: &Path,
    name: &str,
    lines: &[String],
    place: Value,
) -> Value {
    let item = library::find(name, None).unwrap();
    assert!(library::imports_media(&item), "{name}");
    let args = library::media_args(bundle, &item, lines).unwrap();
    let asset = run(projects, bundle, "media.add", args)["asset"].clone();
    let mut place = place;
    place["asset"] = asset;
    run(projects, bundle, "clip.add", place)
}

fn grab(bundle: &Path, seconds: f64, width: u32, renderer: &mut Renderer) -> Image {
    let project = snapshot(bundle).unwrap();
    let timeline = project.manifest.primary_timeline.clone();
    frame(
        &project,
        bundle,
        &timeline,
        Flicks::from_seconds_f64(seconds).unwrap(),
        Some(width),
        renderer,
    )
    .unwrap()
    .image
}

/// The colour at fractions `x`, `y` of the image.
fn pixel(image: &Image, x: f64, y: f64) -> [u8; 3] {
    let (px, py) = (
        ((x * f64::from(image.width)) as u32).min(image.width - 1),
        ((y * f64::from(image.height)) as u32).min(image.height - 1),
    );
    let at = ((py * image.width + px) * 4) as usize;
    [image.rgba[at], image.rgba[at + 1], image.rgba[at + 2]]
}

fn near(color: [u8; 3], expected: [u8; 3], tolerance: u8) -> bool {
    color
        .iter()
        .zip(expected)
        .all(|(a, b)| a.abs_diff(b) <= tolerance)
}

/// How many pixels of `a` and `b` differ.
fn changed(a: &Image, b: &Image) -> usize {
    a.rgba
        .chunks(4)
        .zip(b.rgba.chunks(4))
        .filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p.abs_diff(*q) > 8))
        .count()
}

/// A solid red background over the whole timeline, so open screens show it.
fn red_background(projects: &Projects, bundle: &Path, seconds: u32) {
    run(
        projects,
        bundle,
        "shape.add",
        json!({ "item": "ui-screen-dark", "fill": { "color": "#FF0000" },
                "start": "0s", "duration": format!("{seconds}s") }),
    );
}

#[test]
fn devices_render_with_open_screens_and_the_map_with_its_streets() {
    let dir = workspace("devices");
    let (bundle, projects) = new_project(&dir, 1080, 1920);
    red_background(&projects, &bundle, 4);
    let names = ["phone", "tablet", "laptop", "browser"];
    for (index, name) in names.iter().enumerate() {
        add_media(
            &projects,
            &bundle,
            name,
            &[],
            json!({ "start": format!("{index}s"), "duration": "1s", "free_track": true }),
        );
    }
    assert!(bundle.join("library/phone.svg").is_file());
    let project = snapshot(&bundle).unwrap();
    let Some(mut renderer) = renderer() else {
        std::fs::remove_dir_all(&dir).unwrap();
        return;
    };
    for (index, name) in names.iter().enumerate() {
        let image = grab(&bundle, index as f64 + 0.5, 270, &mut renderer);
        let item = library::find(name, None).unwrap();
        let report = library::screen_report(&item, &project, None).unwrap();
        let position = report["position"].as_array().unwrap();
        let size = report["size"].as_array().unwrap();
        let (cx, cy) = (
            0.5 + position[0].as_f64().unwrap(),
            0.5 + position[1].as_f64().unwrap(),
        );
        // The middle of the screen shows the red background through it.
        assert!(near(pixel(&image, cx, cy), [255, 0, 0], 30), "{name}");
        // Just above the screen is the device: a bezel, a lid or a toolbar.
        let edge = cy - size[1].as_f64().unwrap() / 2.0 - 0.007;
        let frame_color = pixel(&image, cx, edge);
        assert!(
            !near(frame_color, [255, 0, 0], 60),
            "{name}: the frame at {edge}: {frame_color:?}"
        );
    }

    // The street map: roads on the 240-pixel grid, blocks between them.
    let (map_bundle, map_projects) = new_project(&dir.join("map"), 1920, 1080);
    add_media(
        &map_projects,
        &map_bundle,
        "street-map",
        &[],
        json!({ "start": "0s", "duration": "1s" }),
    );
    let map = grab(&map_bundle, 0.5, 960, &mut renderer);
    // Dark tones come out of a frame grab through the video transfer curve, so compare
    // them with each other: the avenue is lighter and bluer than the block beside it.
    let road = pixel(&map, 1080.0 / 1920.0, 120.0 / 1080.0);
    let block = pixel(&map, 1200.0 / 1920.0, 200.0 / 1080.0);
    let sum = |c: [u8; 3]| c.iter().map(|v| u32::from(*v)).sum::<u32>();
    assert!(
        sum(road) > sum(block) + 30,
        "avenue {road:?}, block {block:?}"
    );
    assert!(road[2] > road[0] + 20, "a blue-grey avenue {road:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_ui_kit_sticker_and_title_renders() {
    let dir = workspace("stickers");
    let (bundle, projects) = new_project(&dir, 1080, 1920);
    let stickers: Vec<String> = library::catalog()
        .into_iter()
        .filter(|i| {
            (i.name.starts_with("ui-") || i.name.starts_with("swoosh-"))
                && matches!(i.kind, "sticker" | "background")
        })
        .map(|i| i.name)
        .collect();
    assert_eq!(stickers.len(), 21, "{stickers:?}");
    let titles = ["ui-search-text", "ui-status-text", "kinetic-type"];
    for (index, name) in stickers.iter().enumerate() {
        let item = library::find(name, None).unwrap();
        let call = library::command_for(
            &item,
            &library::Placement {
                start: Some(json!(format!("{}s", index + 1))),
                duration: Some(json!("1s")),
                ..library::Placement::default()
            },
        )
        .unwrap();
        assert_eq!(call["command"], "shape.add", "{name}");
        run(&projects, &bundle, "shape.add", call["args"].clone());
    }
    for (index, name) in titles.iter().enumerate() {
        let item = library::find(name, None).unwrap();
        let call = library::command_for(
            &item,
            &library::Placement {
                start: Some(json!(format!("{}s", stickers.len() + index + 1))),
                duration: Some(json!("1s")),
                ..library::Placement::default()
            },
        )
        .unwrap();
        run(&projects, &bundle, "text.template", call["args"].clone());
    }
    let Some(mut renderer) = renderer() else {
        std::fs::remove_dir_all(&dir).unwrap();
        return;
    };
    let empty = grab(&bundle, 0.5, 180, &mut renderer);
    let all: Vec<String> = stickers
        .iter()
        .cloned()
        .chain(titles.iter().map(|t| t.to_string()))
        .collect();
    for (index, name) in all.iter().enumerate() {
        // Titles reveal over their first moments: grab late in each second.
        let image = grab(&bundle, index as f64 + 1.9, 180, &mut renderer);
        let differs = changed(&empty, &image);
        assert!(differs > 4, "{name} draws nothing ({differs} pixels)");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_shop_app_fills_the_phone_screen_where_the_library_says() {
    let found = fosscut_engine::web::find_browser();
    assert!(
        found.is_some() || !required("FOSSCUT_REQUIRE_BROWSER"),
        "FOSSCUT_REQUIRE_BROWSER is set but no browser was found"
    );
    let dir = workspace("shop");
    let (bundle, projects) = new_project(&dir, 1080, 1920);
    red_background(&projects, &bundle, 2);
    // The page below, the phone above it, placed with the screen's numbers.
    let page = add_media(
        &projects,
        &bundle,
        "shop-app",
        &["desk lamps".into()],
        json!({ "start": "0s", "duration": "2s", "free_track": true }),
    );
    let phone = add_media(
        &projects,
        &bundle,
        "phone",
        &[],
        json!({ "start": "0s", "duration": "2s", "free_track": true }),
    );
    let project = snapshot(&bundle).unwrap();
    let tracks = &project.timelines[&project.manifest.primary_timeline].tracks;
    let index = |clip: &Value| {
        tracks
            .iter()
            .position(|t| {
                t.clips
                    .iter()
                    .any(|c| c.id.as_str() == clip.as_str().unwrap())
            })
            .unwrap()
    };
    assert!(
        index(&phone["clip"]) > index(&page["clip"]),
        "the phone is above its screen"
    );
    let item = library::find("phone", None).unwrap();
    let screen = library::screen_report(&item, &project, None).unwrap();
    run(
        &projects,
        &bundle,
        "clip.set",
        json!({ "clip": page["clip"], "set": {
            "transform.position": screen["position"], "transform.scale": screen["scale"] } }),
    );
    let page_html = std::fs::read_to_string(bundle.join("library/shop-app/index.html")).unwrap();
    assert!(page_html.contains("\"desk lamps\""));
    let (Some(_), Some(mut renderer)) = (found, renderer()) else {
        std::fs::remove_dir_all(&dir).unwrap();
        return;
    };
    let image = grab(&bundle, 0.2, 360, &mut renderer);
    // Inside the screen near its bottom: the app's pale page, not the red background.
    let size = screen["size"].as_array().unwrap();
    let (w, h) = (size[0].as_f64().unwrap(), size[1].as_f64().unwrap());
    let inside = pixel(&image, 0.5, 0.5 + h / 2.0 - 0.02);
    assert!(near(inside, [0xF4, 0xF6, 0xFA], 12), "the page: {inside:?}");
    // The screen's rounded corner is filled by the page too, and nothing red shows
    // anywhere inside the screen's rectangle away from its corners.
    for (x, y) in [(0.5 - w / 2.0 + 0.03, 0.5), (0.5 + w / 2.0 - 0.03, 0.5)] {
        let side = pixel(&image, x, y);
        assert!(!near(side, [255, 0, 0], 60), "{x}, {y}: {side:?}");
    }
    // Outside the phone, the red background.
    assert!(near(pixel(&image, 0.02, 0.5), [255, 0, 0], 30));
    std::fs::remove_dir_all(&dir).unwrap();
}
