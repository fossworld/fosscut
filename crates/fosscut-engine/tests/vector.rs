//! SVG drawings (ADR-0036): `.svg` files import as vector assets with their size, view box
//! and paths; image clips of them render through the engine's frame path; and
//! `vector.explode`, given only the asset, reads the paths from the file and places shape
//! clips that draw the same picture.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::PathBuf;

use common::renderer;
use fosscut_core::Request;
use fosscut_engine::frame::frame;
use fosscut_engine::project::{Projects, create, snapshot};
use fosscut_model::{AssetKind, ClipContent, Id, TimelineFormat, TrackKind};
use fosscut_render::Image;
use fosscut_time::{Flicks, Rate};
use serde_json::{Value, json};

/// A badge written for this test, in solid colours only: a filled circle with a stroke, a
/// round-capped tick and a square with a hole, 160 by 90 units shown 320 by 180 pixels.
const BADGE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 160 90">
  <circle cx="45" cy="45" r="30" fill="#2B6CB0" stroke="#F6E05E" stroke-width="6"/>
  <path id="tick" d="M 30 46 L 42 58 L 62 32" fill="none" stroke="#FFFFFF" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"/>
  <path id="frame" fill="#E53E3E" fill-rule="evenodd" d="M 95 20 h 50 v 50 h -50 z M 108 33 h 24 v 24 h -24 z"/>
</svg>"##;

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-vector-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("badge.svg"), BADGE).unwrap();
    dir
}

fn run(projects: &Projects, bundle: &std::path::Path, command: &str, args: Value) -> Value {
    let response = projects
        .execute(bundle, Request::single(command, args))
        .unwrap_or_else(|e| panic!("{command}: {e:?}"));
    response.results[0].clone()
}

/// Mean absolute difference of the colour channels, 0 to 255.
fn mean_difference(a: &Image, b: &Image) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let total: u64 = a
        .rgba
        .iter()
        .zip(&b.rgba)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    total as f64 / a.rgba.len() as f64
}

#[test]
fn svg_files_import_as_vector_drawings() {
    let dir = workspace("import");
    let probed = fosscut_engine::import::probe(&dir.join("badge.svg")).unwrap();
    assert_eq!(probed.kind, AssetKind::Vector);
    let video = probed.info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (320, 180));
    assert_eq!(video.codec.as_deref(), Some("svg"));
    let vector = probed.info.vector.as_ref().unwrap();
    assert_eq!(vector.view_box, [0.0, 0.0, 160.0, 90.0]);
    assert_eq!(vector.paths, 3);
    assert!(probed.info.duration.is_none());
    std::fs::write(dir.join("broken.svg"), "<svg").unwrap();
    let error = fosscut_engine::import::probe(&dir.join("broken.svg"))
        .err()
        .unwrap();
    assert_eq!(error.code, "fosscut.media.probe_failed");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn drawings_render_and_explode_into_the_same_picture() {
    let dir = workspace("explode");
    let bundle = dir.join("Badge.fosscut");
    let format = TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    create(&bundle, "Badge", format).unwrap();
    let projects = Projects::with_runtime("test", dir.join("runtime"));
    let args =
        fosscut_engine::import::media_add_args(&dir.join("badge.svg"), None, &bundle).unwrap();
    assert_eq!(args["kind"], "vector");
    let asset = run(&projects, &bundle, "media.add", args)["asset"].clone();
    // The drawing as an image clip for the first second.
    run(
        &projects,
        &bundle,
        "clip.add",
        json!({ "asset": asset, "start": "0s", "duration": "1s" }),
    );
    // Split into shapes for the next second, given only the asset: the paths come from
    // the file.
    let exploded = run(
        &projects,
        &bundle,
        "vector.explode",
        json!({ "asset": asset, "start": "1s", "duration": "1s" }),
    );
    let parts = exploded["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[1]["name"], "tick");
    let project = snapshot(&bundle).unwrap();
    let nested = Id::parse(exploded["timeline"].as_str().unwrap()).unwrap();
    let doc = &project.timelines[&nested];
    assert_eq!((doc.format.width, doc.format.height), (1920, 1080));
    assert!(doc.tracks.iter().all(|t| t.kind == TrackKind::Video));
    let tick = &doc.tracks[1].clips[0];
    let ClipContent::Shape(shape) = &tick.content else {
        panic!("a shape clip");
    };
    assert!(!shape.closed && shape.fill.is_none());
    assert!((shape.stroke.as_ref().unwrap().width - 7.0 / 90.0).abs() < 1e-6);

    let Some(mut renderer) = renderer() else {
        return;
    };
    let timeline = project.manifest.primary_timeline.clone();
    let at = |seconds: f64, renderer: &mut _| {
        frame(
            &project,
            &bundle,
            &timeline,
            Flicks::from_seconds_f64(seconds).unwrap(),
            Some(640),
            renderer,
        )
        .unwrap()
        .image
    };
    let drawing = at(0.5, &mut renderer);
    let shapes = at(1.5, &mut renderer);
    // The drawing covers the canvas: blue in the circle, red in the frame, the hole and
    // the margins black.
    let pixel = |image: &Image, x: f64, y: f64| {
        let (x, y) = ((x / 160.0 * 640.0) as u32, (y / 90.0 * 360.0) as u32);
        let at = ((y * image.width + x) * 4) as usize;
        [image.rgba[at], image.rgba[at + 1], image.rgba[at + 2]]
    };
    for image in [&drawing, &shapes] {
        let blue = pixel(image, 50.0, 30.0);
        assert!(blue[2] > 150 && blue[0] < 80, "{blue:?}");
        let red = pixel(image, 100.0, 45.0);
        assert!(red[0] > 180 && red[1] < 90, "{red:?}");
        let hole = pixel(image, 120.0, 45.0);
        assert!(hole.iter().all(|c| *c < 20), "{hole:?}");
        let white = pixel(image, 42.0, 56.0);
        assert!(white.iter().all(|c| *c > 200), "the tick: {white:?}");
    }
    // The shapes draw what the drawing draws, up to antialiasing along the edges.
    let difference = mean_difference(&drawing, &shapes);
    assert!(difference < 2.0, "mean difference {difference}");
    std::fs::remove_dir_all(&dir).unwrap();
}
