//! The library's explainer overlays (ADR-0042, the overlay kit) rendered in a browser with
//! SwiftShader: every template is clear before its entrance, partly built during it,
//! complete at its hold time and fading in its exit; it gives the same frame for the same
//! time alone and after others; its clear margins stay clear (or its full-screen page
//! opaque); and its frame at the hold time matches a reviewed golden PNG. A video in a slot
//! plays at the card's own time. They skip without a browser unless
//! `FOSSCUT_REQUIRE_BROWSER` is set.
//!
//! `FOSSCUT_UPDATE_GOLDENS=1` rewrites the golden PNGs in `tests/goldens/overlays/` (as
//! `cargo xtask goldens --update` does); review every changed image before committing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::required;
use fosscut_engine::library::{self, CompositionInput};
use fosscut_engine::web::{Composition, Found, WebDecoder, WebRenderer, find_browser};
use fosscut_media::{VideoDecoder, VideoFrame};
use fosscut_time::{Flicks, Rate};
use serde_json::{Value, json};

/// The tests render at this share of each template's own canvas.
const SCALE: f64 = 0.4;

/// What a template's frame at its hold time looks like.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Kind {
    /// A card or mark on a clear canvas: its corners stay clear.
    Card,
    /// A full-screen page: opaque everywhere.
    Screen,
}

/// Every template: its canvas, its hold time (its default, with `appear` at 0) and kind.
const TEMPLATES: [(&str, (u32, u32), f64, Kind); 22] = [
    ("video-card", (1080, 860), 3.5, Kind::Card),
    ("app-window", (1080, 780), 3.5, Kind::Card),
    ("terminal", (1080, 700), 5.5, Kind::Card),
    ("review-frame", (1080, 1920), 3.5, Kind::Screen),
    ("social-post", (1080, 1240), 3.5, Kind::Card),
    ("quote-panel", (1080, 760), 3.5, Kind::Card),
    ("before-after", (1080, 760), 3.5, Kind::Card),
    ("reel-fan", (1080, 960), 3.5, Kind::Card),
    ("menu-grid", (1080, 1080), 3.5, Kind::Card),
    ("storyboard-list", (1080, 1180), 4.0, Kind::Card),
    ("cross-out", (1080, 780), 2.5, Kind::Card),
    ("arrow-pointer", (800, 800), 2.5, Kind::Card),
    ("audio-chip", (760, 280), 3.0, Kind::Card),
    ("waveform-sync", (1080, 640), 3.5, Kind::Card),
    ("timeline-cut", (1080, 620), 4.0, Kind::Card),
    ("progress-card", (1080, 460), 3.5, Kind::Card),
    ("stat-counter", (900, 560), 3.0, Kind::Card),
    ("bar-chart", (1080, 820), 3.5, Kind::Card),
    ("clock-card", (760, 900), 3.5, Kind::Card),
    ("lower-third-card", (1080, 360), 3.5, Kind::Card),
    ("comment-cta", (1080, 380), 3.5, Kind::Card),
    ("grid-stage", (1080, 1920), 4.0, Kind::Screen),
];

/// The exit's length (`OVERLAY_EXIT_SECONDS`).
const EXIT: f64 = 0.4;

/// One of a few browser slots shared by every test process (nextest runs each test in a
/// process of its own), held while a test uses its browsers, so tests running side by side
/// never start more browsers than the machine starts in time.
struct Slot(#[allow(dead_code)] std::fs::File);

fn slot() -> Slot {
    let dir = std::env::temp_dir().join("fosscut-overlay-browsers");
    std::fs::create_dir_all(&dir).unwrap();
    loop {
        for i in 0..4 {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(dir.join(format!("slot-{i}")))
                .unwrap();
            if file.try_lock().is_ok() {
                return Slot(file);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// The browser to render with and a slot to use it in, or `None` to skip.
fn browser() -> Option<(Found, Slot)> {
    let found = find_browser();
    assert!(
        found.is_some() || !required("FOSSCUT_REQUIRE_BROWSER"),
        "FOSSCUT_REQUIRE_BROWSER is set but no browser was found"
    );
    found.map(|found| (found, slot()))
}

/// The test canvas of a template whose own canvas is `size`.
fn test_size(size: (u32, u32)) -> (u32, u32) {
    let scale = |v: u32| (f64::from(v) * SCALE).round() as u32;
    (scale(size.0), scale(size.1))
}

/// Writes template `name` with `params` at the test size into a fresh bundle `tag`, and
/// gives its page, its folder and its canvas.
fn write(name: &str, size: (u32, u32), params: Value, tag: &str) -> (PathBuf, Composition) {
    let dir = std::env::temp_dir().join(format!("fosscut-overlays-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (width, height) = test_size(size);
    let mut params = params;
    params["width"] = json!(width);
    params["height"] = json!(height);
    let item = library::find(name, Some("composition")).unwrap();
    let input = CompositionInput {
        params: Some(params),
        ..CompositionInput::default()
    };
    let args = library::media_args_with(&dir, &item, &input, None).unwrap();
    let entry = dir.join(args["path"]["relative"].as_str().unwrap());
    let video = &args["info"]["video"];
    let composition = Composition {
        width: video["width"].as_u64().unwrap() as u32,
        height: video["height"].as_u64().unwrap() as u32,
        rate: Rate::FPS_30,
        duration: Flicks::new(args["info"]["duration"].as_i64().unwrap()),
    };
    assert_eq!((composition.width, composition.height), (width, height));
    (entry, composition)
}

/// Opens template `name` with `params` in a browser of its own, drawn by SwiftShader.
fn open(found: &Found, name: &str, size: (u32, u32), params: Value, tag: &str) -> WebDecoder {
    let (entry, composition) = write(name, size, params, tag);
    let decoder = WebDecoder::open_with(&entry, composition, found, WebRenderer::Software).unwrap();
    assert_eq!(decoder.renderer(), WebRenderer::Software);
    decoder
}

fn at(decoder: &mut WebDecoder, seconds: f64) -> VideoFrame {
    decoder
        .frame_at(Flicks::from_seconds_f64(seconds).unwrap())
        .unwrap()
}

fn rgba(frame: &VideoFrame) -> &[u8] {
    &frame.planes[0].data
}

/// The mean opacity of the frame, from 0 (clear) to 1 (opaque).
fn coverage(frame: &VideoFrame) -> f64 {
    let pixels = rgba(frame);
    let sum: u64 = pixels.chunks(4).map(|p| u64::from(p[3])).sum();
    sum as f64 / (pixels.len() / 4) as f64 / 255.0
}

/// The largest opacity in the `size` pixel square at fractions (`x`, `y`) of the frame.
fn alpha_near(frame: &VideoFrame, x: f64, y: f64, size: u32) -> u8 {
    let (w, h) = (frame.width, frame.height);
    let left = ((x * f64::from(w)) as u32).min(w - size);
    let top = ((y * f64::from(h)) as u32).min(h - size);
    let pixels = rgba(frame);
    let mut most = 0;
    for yy in top..top + size {
        for xx in left..left + size {
            most = most.max(pixels[((yy * w + xx) * 4 + 3) as usize]);
        }
    }
    most
}

/// The share of distinct colours among visible pixels: how much the frame shows.
fn detail(frame: &VideoFrame) -> usize {
    let mut seen = std::collections::HashSet::new();
    for p in rgba(frame).chunks(4).filter(|p| p[3] > 200) {
        seen.insert((p[0] >> 3, p[1] >> 3, p[2] >> 3));
    }
    seen.len()
}

/// Template `name` enters, holds and leaves; gives the same frame alone and after others;
/// keeps its margins clear or its page opaque; and matches its golden at its hold time.
fn renders_and_matches_its_golden(index: usize) {
    let (name, size, hold, kind) = TEMPLATES[index];
    let Some((found, _slot)) = browser() else {
        return;
    };
    let mut sequence = open(&found, name, size, json!({}), &format!("seq-{name}"));
    let last = hold + EXIT - 1.0 / 30.0;
    let clear = at(&mut sequence, 0.0);
    let early = at(&mut sequence, 0.12);
    let built = at(&mut sequence, hold - 0.5);
    let held = at(&mut sequence, hold);
    let leaving = at(&mut sequence, hold + 0.2);
    let gone = at(&mut sequence, last);
    let (w, h) = test_size(size);
    assert_eq!((held.width, held.height), (w, h), "{name}");
    // Before and as the entrance starts, nothing is drawn.
    assert!(
        rgba(&clear).chunks(4).all(|p| p[3] == 0),
        "{name} at 0 s is not clear"
    );
    // The entrance builds it up: partly there early, complete by the hold time.
    let (c_early, c_built, c_held) = (coverage(&early), coverage(&built), coverage(&held));
    assert!(
        c_early < c_held * 0.95 || detail(&early) * 2 < detail(&held),
        "{name}: early {c_early:.3} is as full as at the hold time {c_held:.3}"
    );
    assert!(
        c_held >= c_built * 0.97,
        "{name}: still building at the hold time ({c_built:.3} then {c_held:.3})"
    );
    assert!(detail(&held) > 12, "{name} shows little at its hold time");
    // The exit fades it out.
    let (c_leaving, c_gone) = (coverage(&leaving), coverage(&gone));
    assert!(
        c_leaving < c_held && c_gone < c_held * 0.5,
        "{name} does not leave: {c_held:.3}, {c_leaving:.3}, {c_gone:.3}"
    );
    match kind {
        Kind::Card => {
            // Clear around the card: the corners stay transparent.
            for (x, y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                assert_eq!(
                    alpha_near(&held, x, y, 4),
                    0,
                    "{name}: corner ({x}, {y}) is not clear"
                );
            }
            assert!(c_held > 0.02, "{name} covers only {c_held:.3}");
        }
        Kind::Screen => {
            assert!(
                rgba(&held).chunks(4).all(|p| p[3] == 255),
                "{name}'s page is not opaque"
            );
        }
    }
    // Going back gives the hold frame again, byte for byte.
    let again = at(&mut sequence, hold);
    assert!(rgba(&again) == rgba(&held), "{name}: going back");
    // A fresh browser asked for the hold frame alone draws the same pixels.
    let mut alone = open(&found, name, size, json!({}), &format!("alone-{name}"));
    let single = at(&mut alone, hold);
    assert!(
        rgba(&single) == rgba(&held),
        "{name}: the frame at {hold} s alone differs from the one after others"
    );
    if let Some(failure) = check_golden(name, &single) {
        panic!("{failure} is below {MIN_PSNR_DB} dB (images in target/golden-diffs)");
    }
}

macro_rules! template_tests {
    ($($test:ident = $index:expr;)*) => {
        $(
            #[test]
            fn $test() {
                renders_and_matches_its_golden($index);
            }
        )*
    };
}

template_tests! {
    video_card_enters_holds_and_leaves = 0;
    app_window_enters_holds_and_leaves = 1;
    terminal_enters_holds_and_leaves = 2;
    review_frame_enters_holds_and_leaves = 3;
    social_post_enters_holds_and_leaves = 4;
    quote_panel_enters_holds_and_leaves = 5;
    before_after_enters_holds_and_leaves = 6;
    reel_fan_enters_holds_and_leaves = 7;
    menu_grid_enters_holds_and_leaves = 8;
    storyboard_list_enters_holds_and_leaves = 9;
    cross_out_enters_holds_and_leaves = 10;
    arrow_pointer_enters_holds_and_leaves = 11;
    audio_chip_enters_holds_and_leaves = 12;
    waveform_sync_enters_holds_and_leaves = 13;
    timeline_cut_enters_holds_and_leaves = 14;
    progress_card_enters_holds_and_leaves = 15;
    stat_counter_enters_holds_and_leaves = 16;
    bar_chart_enters_holds_and_leaves = 17;
    clock_card_enters_holds_and_leaves = 18;
    lower_third_card_enters_holds_and_leaves = 19;
    comment_cta_enters_holds_and_leaves = 20;
    grid_stage_enters_holds_and_leaves = 21;
}

#[test]
fn every_template_is_tested() {
    let listed: Vec<String> = library::search(Some("composition"), Some("explainer"))
        .into_iter()
        .map(|item| item.name)
        .collect();
    let tested: Vec<&str> = TEMPLATES.iter().map(|t| t.0).collect();
    assert_eq!(listed, tested);
}

#[test]
fn library_names_are_unique_across_kinds() {
    // Adding by name finds an item of any kind, so a name two kinds use needs its kind
    // spelled out. These names were shared before this test; no other may be.
    const SHARED: [(&str, [&str; 2]); 5] = [
        ("boxed", ["text_style", "caption_style"]),
        ("burst", ["shape", "particles"]),
        ("headline", ["title", "text_style"]),
        ("highlight", ["text_style", "caption_style"]),
        ("sparkle", ["particles", "sound"]),
    ];
    let mut kinds: std::collections::BTreeMap<String, Vec<&str>> = Default::default();
    for item in library::catalog() {
        kinds.entry(item.name).or_default().push(item.kind);
    }
    for (name, kinds) in kinds.iter().filter(|(_, k)| k.len() > 1) {
        assert!(
            SHARED
                .iter()
                .any(|(known, pair)| known == name && kinds.as_slice() == pair),
            "{name} is a {}: give the new item a name of its own",
            kinds.join(" and a ")
        );
    }
    // Every overlay is found by its name alone.
    for (name, _, _, _) in TEMPLATES {
        assert_eq!(
            library::find(name, None).unwrap().kind,
            "composition",
            "{name}"
        );
    }
}

#[test]
fn timing_params_move_the_entrance_and_the_exit() {
    let Some((found, _slot)) = browser() else {
        return;
    };
    let (name, size, _, _) = TEMPLATES[16];
    // Appearing at 1 s, holding 2 s and staying: clear before, there after, never leaving.
    let params = json!({ "appear": 1, "hold": 2, "exit": false, "speed": 2 });
    let (_, composition) = write(name, size, params.clone(), "timing-len");
    assert_eq!(
        composition.duration,
        Flicks::from_seconds_f64(3.0).unwrap(),
        "appear plus hold, without an exit"
    );
    let mut late = open(&found, name, size, params, "timing");
    assert_eq!(coverage(&at(&mut late, 0.9)), 0.0, "clear before appear");
    // At speed 2 everything is in 1.2 s after appearing (2.4 s of its own motion), and it
    // stays at the end without leaving.
    let done = at(&mut late, 2.2);
    let end = at(&mut late, 2.95);
    assert!((coverage(&end) - coverage(&done)).abs() < 0.01, "it stays");
    let mut normal = open(&found, name, size, json!({}), "timing-normal");
    let reference = at(&mut normal, 3.0);
    assert!(
        (coverage(&done) - coverage(&reference)).abs() < 0.01,
        "{} against {}",
        coverage(&done),
        coverage(&reference)
    );
    // The dark theme draws a dark card.
    let mut dark = open(
        &found,
        name,
        size,
        json!({ "theme": "dark" }),
        "timing-dark",
    );
    let card = at(&mut dark, 3.0);
    let (w, h) = (card.width as usize, card.height as usize);
    let middle = &rgba(&card)[((h / 2) * w + w / 8) * 4..][..4];
    assert!(
        middle[3] == 255 && middle[0] < 60 && middle[1] < 60,
        "{middle:?}"
    );
}

#[test]
fn an_open_review_frame_shows_the_clip_below() {
    let Some((found, _slot)) = browser() else {
        return;
    };
    let (name, size, hold, _) = TEMPLATES[3];
    let mut open_frame = open(&found, name, size, json!({ "open": true }), "open-frame");
    let frame = at(&mut open_frame, hold);
    // The window's middle is clear, the page round it opaque.
    assert_eq!(alpha_near(&frame, 0.5, 0.45, 8), 0, "the window is clear");
    for (x, y) in [(0.05, 0.45), (0.95, 0.45), (0.5, 0.97)] {
        let (w, h) = (frame.width, frame.height);
        let at = (((y * f64::from(h)) as u32 * w + (x * f64::from(w)) as u32) * 4 + 3) as usize;
        assert_eq!(rgba(&frame)[at], 255, "the page at ({x}, {y})");
    }
}

/// A 2 s 320x180 VP8 clip, which every browser plays, whose picture changes every frame;
/// `None` without FFmpeg.
fn moving_clip(path: &Path) -> Option<()> {
    let ffmpeg = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        ffmpeg || !required("FOSSCUT_REQUIRE_FFMPEG"),
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    if !ffmpeg {
        return None;
    }
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=320x180:rate=30:duration=2")
        .args([
            "-c:v", "libvpx", "-b:v", "2M", "-g", "10", "-pix_fmt", "yuv420p",
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg could not make the clip");
    Some(())
}

#[test]
fn a_video_in_a_slot_plays_at_the_cards_own_time() {
    let Some((found, _slot)) = browser() else {
        return;
    };
    let (name, size, _, _) = TEMPLATES[1];
    let (entry, composition) = write(name, size, json!({ "appear": 0.5 }), "video-slot");
    let folder = entry.parent().unwrap();
    std::fs::create_dir_all(folder.join("media")).unwrap();
    if moving_clip(&folder.join("media/media.webm")).is_none() {
        return;
    }
    // The slot as the engine writes it for a video asset.
    let params_file = folder.join("params.json");
    let mut params: Value =
        serde_json::from_str(&std::fs::read_to_string(&params_file).unwrap()).unwrap();
    params["media"] = json!({ "src": "media/media.webm", "kind": "video", "name": "clip.webm" });
    std::fs::write(&params_file, serde_json::to_vec(&params).unwrap()).unwrap();
    let mut decoder =
        WebDecoder::open_with(&entry, composition, &found, WebRenderer::Software).unwrap();
    let mut plain = open(&found, name, size, json!({ "appear": 0.5 }), "video-plain");
    // The viewer's middle: the right part of the window, above the timeline.
    let viewer = |frame: &VideoFrame| -> Vec<u8> {
        let (w, h) = (frame.width as usize, frame.height as usize);
        let mut out = Vec::new();
        for y in h * 30 / 100..h * 45 / 100 {
            let row = &rgba(frame)[(y * w + w * 45 / 100) * 4..(y * w + w * 75 / 100) * 4];
            out.extend_from_slice(row);
        }
        out
    };
    let first = at(&mut decoder, 2.0);
    let later = at(&mut decoder, 2.4);
    let art = at(&mut plain, 2.0);
    assert!(viewer(&first) != viewer(&art), "the video replaces the art");
    assert!(viewer(&first) != viewer(&later), "the video moves");
    // The same time again, after another, shows the same picture.
    at(&mut decoder, 1.0);
    let again = at(&mut decoder, 2.0);
    assert!(
        viewer(&again) == viewer(&first),
        "the video's frame is exact"
    );
}

fn goldens_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join("overlays")
}

fn write_png(path: &Path, frame: &VideoFrame) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba(frame))
        .unwrap();
}

fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

/// PSNR of `a` and `b` in dB over premultiplied colour and opacity, so the colour of
/// clear pixels does not count and a missing card does.
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let (mut sum, mut count) = (0.0, 0.0);
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        let (pa, qa) = (f64::from(p[3]) / 255.0, f64::from(q[3]) / 255.0);
        for c in 0..3 {
            let d = f64::from(p[c]) * pa - f64::from(q[c]) * qa;
            sum += d * d;
        }
        let d = f64::from(p[3]) - f64::from(q[3]);
        sum += d * d;
        count += 4.0;
    }
    let mse = sum / count;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    }
}

/// SwiftShader and Skia draw the same pixels on any machine with the same browser, but a
/// Chrome update can move text antialiasing and shadow blur slightly; 32 dB allows that
/// while failing on a moved card, a missing part, a changed colour or different text.
const MIN_PSNR_DB: f64 = 32.0;

/// Compares `frame` of template `name` with its golden PNG, or rewrites the PNG when
/// `FOSSCUT_UPDATE_GOLDENS` is set; a failure, with the images written for review.
fn check_golden(name: &str, frame: &VideoFrame) -> Option<String> {
    let golden = goldens_dir().join(format!("{name}.png"));
    if std::env::var_os("FOSSCUT_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(goldens_dir()).unwrap();
        write_png(&golden, frame);
        eprintln!("updated {}", golden.display());
        return None;
    }
    assert!(
        golden.exists(),
        "{} is missing; run with FOSSCUT_UPDATE_GOLDENS=1 and review it",
        golden.display()
    );
    let (w, h, expected) = read_png(&golden);
    assert_eq!((w, h), (frame.width, frame.height), "{name}");
    let quality = psnr(rgba(frame), &expected);
    if quality >= MIN_PSNR_DB {
        return None;
    }
    let diffs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden-diffs");
    std::fs::create_dir_all(&diffs).unwrap();
    write_png(&diffs.join(format!("overlay-{name}.actual.png")), frame);
    std::fs::copy(&golden, diffs.join(format!("overlay-{name}.expected.png"))).unwrap();
    Some(format!("{name}: PSNR {quality:.1} dB"))
}
