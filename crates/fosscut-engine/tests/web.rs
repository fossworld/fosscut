//! Web compositions in a headless browser (ADR-0039): the time contract (CSS animations,
//! the seek hook, pages driven by animation frames), transparency, fonts, and web clips in
//! frame grabs and exports. Skipped without a browser unless `FOSSCUT_REQUIRE_BROWSER` is
//! set; macOS CI runners have Google Chrome.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{renderer, required};
use fosscut_engine::web::{Composition, CompositionOptions, Found, WebDecoder, find_browser};
use fosscut_media::{PixelFormat, VideoDecoder, VideoFrame};
use fosscut_model::{
    Asset, AssetKind, Clip, ClipContent, Id, IdKind, Project, TimelineFormat, Track, TrackKind,
};
use fosscut_time::{Flicks, Rate};

/// The browser to render with, or `None` to skip.
fn browser() -> Option<Found> {
    let found = find_browser();
    assert!(
        found.is_some() || !required("FOSSCUT_REQUIRE_BROWSER"),
        "FOSSCUT_REQUIRE_BROWSER is set but no browser was found"
    );
    found
}

/// A fresh folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-web-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A page `index.html` in a new folder, with `body` inside a transparent document.
fn page(name: &str, head: &str, body: &str) -> PathBuf {
    let dir = folder(name);
    std::fs::write(
        dir.join("index.html"),
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><style>html,body{{margin:0}}</style>{head}</head><body>{body}</body></html>"
        ),
    )
    .unwrap();
    dir.join("index.html")
}

fn composition(width: u32, height: u32, seconds: i64) -> Composition {
    Composition {
        width,
        height,
        rate: Rate::FPS_30,
        duration: Flicks::from_seconds(seconds).unwrap(),
    }
}

fn pixel(frame: &VideoFrame, x: u32, y: u32) -> [u8; 4] {
    assert_eq!(frame.format, PixelFormat::Rgba);
    let row = frame.planes[0].row(y as usize);
    let i = x as usize * 4;
    [row[i], row[i + 1], row[i + 2], row[i + 3]]
}

fn at(frame: i64) -> Flicks {
    Rate::FPS_30.frames_to_flicks(frame)
}

/// The left edge of the first red pixel in row `y`, if any.
fn red_from(frame: &VideoFrame, y: u32) -> Option<u32> {
    (0..frame.width).find(|&x| {
        let [r, g, b, a] = pixel(frame, x, y);
        r > 200 && g < 60 && b < 60 && a > 200
    })
}

#[test]
fn css_animations_follow_composition_time_on_a_transparent_page() {
    let Some(found) = browser() else {
        return;
    };
    let entry = page(
        "css",
        "<style>
            #box { position: absolute; top: 0; left: 0; width: 20px; height: 40px; background: rgb(255,0,0);
                   animation: slide 2s linear forwards; }
            #veil { position: absolute; top: 40px; left: 0; width: 320px; height: 40px; background: rgba(0,0,255,0.5); }
            @keyframes slide { from { left: 0px } to { left: 200px } }
        </style>",
        "<div id=\"box\"></div><div id=\"veil\"></div>",
    );
    let mut decoder = WebDecoder::open(&entry, composition(320, 80, 3), &found).unwrap();
    assert_eq!(decoder.info().codec, "html");
    assert_eq!(decoder.info().frame_count, Some(90));

    let middle = decoder.frame_at(at(30)).unwrap();
    assert_eq!((middle.width, middle.height), (320, 80));
    assert_eq!(middle.pts, at(30));
    assert_eq!(
        red_from(&middle, 10),
        Some(100),
        "1 s into a 2 s slide of 200 px"
    );
    // Nothing drawn stays transparent; a half-transparent element stays half transparent.
    assert_eq!(pixel(&middle, 300, 10)[3], 0);
    let veil = pixel(&middle, 300, 60);
    assert!(
        (veil[3] as i32 - 128).abs() <= 2 && veil[2] > 250,
        "{veil:?}"
    );

    // Any order of frames gives the same pictures.
    let start = decoder.frame_at(at(0)).unwrap();
    assert_eq!(red_from(&start, 10), Some(0));
    let end = decoder.frame_at(at(75)).unwrap();
    assert_eq!(
        red_from(&end, 10),
        Some(200),
        "held at the end by fill forwards"
    );
    let again = decoder.frame_at(at(30)).unwrap();
    assert!(again.planes[0].data == middle.planes[0].data);

    // Times past the end show the last frame.
    assert_eq!(decoder.frame_at(at(500)).unwrap().pts, at(89));
}

#[test]
fn the_seek_hook_gets_each_frames_time() {
    let Some(found) = browser() else {
        return;
    };
    let entry = page(
        "hook",
        "<style>#bar { height: 30px; background: rgb(255,0,0); width: 0 }</style>",
        "<div id=\"bar\"></div>
        <script>
            window.fosscut = {
                seek(seconds) {
                    return new Promise((resolve) => {
                        requestAnimationFrame(() => {
                            document.getElementById('bar').style.width = Math.round(seconds * 100) + 'px';
                            resolve();
                        });
                    });
                },
            };
        </script>",
    );
    let mut decoder = WebDecoder::open(&entry, composition(400, 30, 4), &found).unwrap();
    for (frame, width) in [(45, 150), (12, 40), (90, 300), (45, 150)] {
        let shown = decoder.frame_at(at(frame)).unwrap();
        let last_red = (0..400).rev().find(|&x| pixel(&shown, x, 15)[0] > 200);
        assert_eq!(last_red, Some(width - 1), "frame {frame}");
    }
}

#[test]
fn pages_driven_by_animation_frames_step_through_every_frame() {
    let Some(found) = browser() else {
        return;
    };
    // The page keeps state: each animation frame moves the square one pixel, and a timer
    // turns it green after one second.
    let entry = page(
        "frames",
        "",
        "<canvas id=\"c\" width=\"200\" height=\"20\"></canvas>
        <script>
            const context = document.getElementById('c').getContext('2d');
            let x = 0;
            let color = 'rgb(255,0,0)';
            setTimeout(() => { color = 'rgb(0,255,0)'; }, 1000);
            function draw() {
                context.clearRect(0, 0, 200, 20);
                context.fillStyle = color;
                context.fillRect(x, 0, 10, 20);
                x += 1;
                requestAnimationFrame(draw);
            }
            requestAnimationFrame(draw);
        </script>",
    );
    let mut decoder = WebDecoder::open(&entry, composition(200, 20, 2), &found).unwrap();
    let left = |frame: &VideoFrame| (0..200).find(|&x| pixel(frame, x, 10)[3] > 200);
    let twenty = decoder.frame_at(at(20)).unwrap();
    assert_eq!(left(&twenty), Some(20), "one step per frame from frame 0");
    assert!(pixel(&twenty, 25, 10)[0] > 200, "red before the timer");
    // Going back reloads the page and steps again, so the result is the same.
    let five = decoder.frame_at(at(5)).unwrap();
    assert_eq!(left(&five), Some(5));
    let again = decoder.frame_at(at(20)).unwrap();
    assert!(again.planes[0].data == twenty.planes[0].data);
    let later = decoder.frame_at(at(40)).unwrap();
    assert_eq!(left(&later), Some(40));
    assert!(pixel(&later, 45, 10)[1] > 200, "green after the 1 s timer");
}

#[test]
fn fonts_load_before_the_first_frame() {
    let Some(found) = browser() else {
        return;
    };
    let font = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fosscut-render/assets/fonts/inter/Inter-Bold.ttf");
    let entry = page(
        "fonts",
        "<style>
            @font-face { font-family: 'Test Inter'; src: url('Inter-Bold.ttf'); font-display: block; }
            p { font: 48px 'Test Inter'; color: rgb(255,255,255); margin: 0; }
        </style>",
        "<p>Hello</p>",
    );
    std::fs::copy(&font, entry.parent().unwrap().join("Inter-Bold.ttf")).unwrap();
    let mut decoder = WebDecoder::open(&entry, composition(240, 60, 1), &found).unwrap();
    let first = decoder.frame_at(Flicks::ZERO).unwrap();
    let inked = first.planes[0]
        .data
        .chunks(4)
        .filter(|p| p[3] > 128)
        .count();
    assert!(
        inked > 300,
        "the text is drawn in the loaded font: {inked} pixels"
    );
}

#[test]
fn the_same_frame_renders_the_same_in_another_browser() {
    let Some(found) = browser() else {
        return;
    };
    let entry = page(
        "twice",
        "<style>div { width: 60px; height: 60px; background: linear-gradient(90deg, #ff7a45, #2050ff);
               border-radius: 30px; animation: spin 3s linear infinite; }
               @keyframes spin { to { transform: rotate(360deg) } }</style>",
        "<div></div>",
    );
    let shot = |found: &Found| {
        let mut decoder = WebDecoder::open(&entry, composition(80, 80, 3), found).unwrap();
        decoder.frame_at(at(37)).unwrap()
    };
    let (one, two) = (shot(&found), shot(&found));
    assert!(one.planes[0].data == two.planes[0].data);
}

/// Imports the composition at `source` into a new project's bundle, on a video track above
/// a green shape that fills the canvas; returns the project and its bundle.
fn web_project(name: &str, source: &Path) -> (Project, PathBuf) {
    let dir = folder(name);
    let bundle = dir.join("Show.fosscut");
    std::fs::create_dir_all(&bundle).unwrap();
    let args =
        fosscut_engine::web::media_add_args(source, None, &bundle, &CompositionOptions::default())
            .unwrap();
    let format = TimelineFormat {
        width: 320,
        height: 180,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Show", format);
    let asset = Id::new(IdKind::Asset, 1);
    project.media.assets.push(Asset {
        folder: None,
        id: asset.clone(),
        kind: AssetKind::Web,
        name: args["name"].as_str().unwrap().into(),
        path: serde_json::from_value(args["path"].clone()).unwrap(),
        identity: serde_json::from_value(args["identity"].clone()).unwrap(),
        info: serde_json::from_value(args["info"].clone()).unwrap(),
    });
    project.manifest.next_id = 10;
    let timeline = project.manifest.primary_timeline.clone();
    let doc = project.timelines.get_mut(&timeline).unwrap();
    // A shape filling the canvas, written as stored so it reads with any shape defaults.
    let green: ClipContent = serde_json::from_value(serde_json::json!({
        "kind": "shape",
        "shape": "rectangle",
        "size": [1.0, 1.0],
        "fill": { "color": "#00FF00" },
    }))
    .unwrap();
    let duration = Rate::FPS_30.frames_to_flicks(60);
    for (index, content) in [
        green,
        ClipContent::Video {
            asset,
            source_in: Flicks::ZERO,
        },
    ]
    .into_iter()
    .enumerate()
    {
        doc.tracks.push(Track {
            id: Id::new(IdKind::Track, 2 + 2 * index as u64),
            kind: TrackKind::Video,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![Clip::new(
                Id::new(IdKind::Clip, 3 + 2 * index as u64),
                Flicks::ZERO,
                duration,
                content,
            )],
        });
    }
    assert!(
        fosscut_model::validate(&project).is_empty(),
        "{:?}",
        fosscut_model::validate(&project)
    );
    (project, bundle)
}

/// A 320x180 composition: a red square sliding from the left edge to x = 160 over 2 s, on a
/// transparent page.
fn sliding_square(name: &str) -> PathBuf {
    page(
        name,
        "<meta name=\"fosscut:composition\" content=\"width=320, height=180, fps=30, duration=2\">
        <style>#sq { position: absolute; top: 60px; left: 0; width: 40px; height: 40px; background: #ff0000;
                     animation: go 2s linear forwards; }
               @keyframes go { to { left: 160px } }</style>",
        "<div id=\"sq\"></div>",
    )
    .parent()
    .unwrap()
    .to_path_buf()
}

#[test]
fn web_clips_composite_over_lower_tracks_in_frame_grabs() {
    let Some(_found) = browser() else {
        return;
    };
    let Some(mut renderer) = renderer() else {
        return;
    };
    let source = sliding_square("grab-source");
    let (project, bundle) = web_project("grab", &source);
    let timeline = project.manifest.primary_timeline.clone();
    let cache = bundle.parent().unwrap().join("cache");
    let still = fosscut_engine::frame::frame_in(
        &project,
        &bundle,
        &timeline,
        Rate::FPS_30.frames_to_flicks(30),
        None,
        &cache,
        &mut renderer,
    )
    .unwrap();
    let rgba = |x: u32, y: u32| {
        let i = ((y * still.image.width + x) * 4) as usize;
        [
            still.image.rgba[i],
            still.image.rgba[i + 1],
            still.image.rgba[i + 2],
        ]
    };
    // 1 s in, the square spans x 80 to 120; the transparent page shows the green below.
    let square = rgba(100, 80);
    assert!(
        square[0] > 240 && square[1] < 20 && square[2] < 20,
        "{square:?}"
    );
    let around = rgba(20, 80);
    assert!(around[1] > 240 && around[0] < 20, "{around:?}");
    let before = rgba(70, 80);
    assert!(
        before[1] > 240,
        "the square has moved past x 70: {before:?}"
    );
}

#[test]
fn web_clips_export_like_their_frame_grabs() {
    let Some(_found) = browser() else {
        return;
    };
    let Some(mut renderer) = renderer() else {
        return;
    };
    let source = sliding_square("export-source");
    let (project, bundle) = web_project("export", &source);
    let timeline = project.manifest.primary_timeline.clone();
    let dir = bundle.parent().unwrap().to_path_buf();
    let format = project.timelines[&timeline].format.clone();
    let preset = fosscut_engine::Preset::ALL
        .into_iter()
        .find(|p| p.available() && p.name() == "mov-prores")
        .unwrap();
    let request = fosscut_engine::ExportRequest {
        bundle: bundle.clone(),
        settings: preset.settings(&format),
        timeline: timeline.clone(),
        output: dir.join("web.mov"),
        allow_overwrite: false,
        expected_revision: None,
        request_id: None,
        cache: dir.join("cache"),
        options: Default::default(),
    };
    let report = fosscut_engine::export(&project, &request, &mut renderer, &mut |_| true).unwrap();
    assert_eq!(report.frames, 60);
    // The exported frame at 1 s matches its frame grab where the square and the green are.
    let exported = |x: u32, y: u32| -> Vec<u8> {
        let output = Command::new("ffmpeg")
            .args(["-v", "error", "-ss", "1.0166", "-i"])
            .arg(dir.join("web.mov"))
            .args(["-frames:v", "1", "-vf"])
            .arg(format!("format=rgb24,crop=1:1:{x}:{y}"))
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .unwrap();
        output.stdout
    };
    let square = exported(100, 80);
    assert!(square[0] > 230 && square[1] < 30, "{square:?}");
    let green = exported(20, 80);
    assert!(green[1] > 230 && green[0] < 30, "{green:?}");
}
