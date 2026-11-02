//! Three.js scene templates of the library (ADR-0042) rendered in a browser: every template
//! gives the same frame for the same time alone and after others, keeps a transparent
//! background transparent, and shows measurable bloom, depth of field and motion blur; the
//! phone shows the shop-app composition on its screen; and one reviewed golden frame per
//! template. The templates ask for the GPU renderer, but these tests draw them with
//! SwiftShader, which gives the same pixels on every machine with the same browser; one test
//! checks that the GPU draws the same scene. They skip without a browser unless
//! `FOSSCUT_REQUIRE_BROWSER` is set.
//!
//! `FOSSCUT_UPDATE_GOLDENS=1` rewrites the golden PNGs in `tests/goldens/scenes/` (as
//! `cargo xtask goldens --update` does); review every changed image before committing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use common::required;
use fosscut_engine::library::{self, CompositionInput};
use fosscut_engine::web::{Composition, Found, WebDecoder, WebRenderer, find_browser};
use fosscut_media::{VideoDecoder, VideoFrame};
use fosscut_time::{Flicks, Rate};
use serde_json::{Value, json};

/// Every template with times that show it moving.
const TEMPLATES: [(&str, [f64; 3]); 5] = [
    ("route-map-3d", [1.5, 3.2, 5.6]),
    ("parcel-drop-3d", [0.8, 0.95, 2.5]),
    ("phone-3d", [0.6, 2.0, 4.5]),
    ("logo-extrude-3d", [0.8, 2.0, 3.5]),
    ("product-podium-3d", [0.8, 2.5, 5.0]),
];

/// The test canvas: a quarter of the templates' 1080x1920.
const WIDTH: u32 = 270;
const HEIGHT: u32 = 480;

/// The browser to render with, or `None` to skip.
fn browser() -> Option<Found> {
    let found = find_browser();
    assert!(
        found.is_some() || !required("FOSSCUT_REQUIRE_BROWSER"),
        "FOSSCUT_REQUIRE_BROWSER is set but no browser was found"
    );
    found
}

/// Writes template `name` with `params` (at the test size) into a fresh bundle `tag`, and
/// gives its page and canvas.
fn write(name: &str, params: Value, tag: &str) -> (PathBuf, Composition) {
    let dir = std::env::temp_dir().join(format!("fosscut-scenes-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut params = params;
    params["width"] = json!(WIDTH);
    params["height"] = json!(HEIGHT);
    // Three sub-frames keep SwiftShader quick; tests of motion blur set their own.
    if params.get("motion_blur").is_none() {
        params["motion_blur"] = json!({ "samples": 3 });
    }
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
    assert_eq!((composition.width, composition.height), (WIDTH, HEIGHT));
    (entry, composition)
}

/// Opens template `name` with `params` in a browser of its own, drawn by SwiftShader.
fn open(found: &Found, name: &str, params: Value, tag: &str) -> WebDecoder {
    let (entry, composition) = write(name, params, tag);
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

fn luma(p: &[u8]) -> f64 {
    0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
}

/// The mean absolute luminance step between neighbours in rows `rows` (fractions of the
/// height): how much fine detail the frame has there.
fn detail(frame: &VideoFrame, rows: (f64, f64)) -> f64 {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let pixels = rgba(frame);
    let lum = |x: usize, y: usize| luma(&pixels[(y * w + x) * 4..]);
    let (top, bottom) = (
        (rows.0 * h as f64) as usize,
        ((rows.1 * h as f64) as usize).min(h - 1),
    );
    let mut sum = 0.0;
    let mut count = 0.0;
    for y in top..bottom {
        for x in 0..w - 1 {
            sum += (lum(x + 1, y) - lum(x, y)).abs() + (lum(x, y + 1) - lum(x, y)).abs();
            count += 1.0;
        }
    }
    sum / count
}

/// The mean absolute luminance step down the frame in rows `rows`: detail across vertical
/// motion.
fn vertical_detail(frame: &VideoFrame, rows: (f64, f64)) -> f64 {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let pixels = rgba(frame);
    let lum = |x: usize, y: usize| luma(&pixels[(y * w + x) * 4..]);
    let (top, bottom) = (
        (rows.0 * h as f64) as usize,
        ((rows.1 * h as f64) as usize).min(h - 1),
    );
    let mut sum = 0.0;
    for y in top..bottom {
        for x in 0..w {
            sum += (lum(x, y + 1) - lum(x, y)).abs();
        }
    }
    sum / ((bottom - top).max(1) * w) as f64
}

/// The share of pixels for which `test` holds.
fn share(frame: &VideoFrame, test: impl Fn(&[u8]) -> bool) -> f64 {
    let pixels = rgba(frame);
    let all = pixels.len() / 4;
    pixels.chunks(4).filter(|p| test(p)).count() as f64 / all as f64
}

/// Template `name` gives the same frame for the same time alone and after others, with an
/// opaque background, and its frame at `times[1]` matches its golden PNG.
fn deterministic_and_golden(name: &str, times: [f64; 3]) {
    let Some(found) = browser() else {
        return;
    };
    let mut sequence = open(&found, name, json!({}), &format!("seq-{name}"));
    let frames: Vec<VideoFrame> = times.iter().map(|&t| at(&mut sequence, t)).collect();
    for (t, frame) in times.iter().zip(&frames) {
        assert_eq!((frame.width, frame.height), (WIDTH, HEIGHT));
        // Something is drawn, with light and shade.
        let lit = share(frame, |p| luma(p) > 40.0);
        assert!(lit > 0.02, "{name} at {t} s is nearly black ({lit})");
        assert!(detail(frame, (0.0, 1.0)) > 0.3, "{name} at {t} s is flat");
    }
    assert!(
        rgba(&frames[0]) != rgba(&frames[1]),
        "{name} moves between frames"
    );
    // Going back gives the first frame again, byte for byte.
    let back = at(&mut sequence, times[0]);
    assert!(rgba(&back) == rgba(&frames[0]), "{name}: going back");
    // A fresh browser asked for the middle frame alone draws the same pixels.
    let mut alone = open(&found, name, json!({}), &format!("alone-{name}"));
    let single = at(&mut alone, times[1]);
    assert!(
        rgba(&single) == rgba(&frames[1]),
        "{name}: the frame at {} s alone differs from the one after others",
        times[1]
    );
    // With its background the frame is opaque everywhere.
    assert_eq!(
        share(&single, |p| p[3] == 255),
        1.0,
        "{name} with its background"
    );
    if let Some(failure) = check_golden(name, &single) {
        panic!("{failure} is below {MIN_PSNR_DB} dB (images in target/golden-diffs)");
    }
}

#[test]
fn route_map_is_deterministic_and_matches_its_golden() {
    deterministic_and_golden(TEMPLATES[0].0, TEMPLATES[0].1);
}

#[test]
fn parcel_drop_is_deterministic_and_matches_its_golden() {
    deterministic_and_golden(TEMPLATES[1].0, TEMPLATES[1].1);
}

#[test]
fn phone_is_deterministic_and_matches_its_golden() {
    deterministic_and_golden(TEMPLATES[2].0, TEMPLATES[2].1);
}

#[test]
fn logo_extrude_is_deterministic_and_matches_its_golden() {
    deterministic_and_golden(TEMPLATES[3].0, TEMPLATES[3].1);
}

#[test]
fn product_podium_is_deterministic_and_matches_its_golden() {
    deterministic_and_golden(TEMPLATES[4].0, TEMPLATES[4].1);
}

#[test]
fn transparent_templates_leave_their_background_clear() {
    let Some(found) = browser() else {
        return;
    };
    for (name, times) in TEMPLATES {
        let mut clear = open(
            &found,
            name,
            json!({ "transparent": true }),
            &format!("clear-{name}"),
        );
        let frame = at(&mut clear, times[2]);
        // Clear, or nearly so where the ground keeps only shadows.
        let empty = share(&frame, |p| p[3] < 16);
        let solid = share(&frame, |p| p[3] == 255);
        assert!(
            empty > 0.08,
            "{name}: only {empty} of the frame is transparent"
        );
        assert!(solid > 0.02, "{name}: only {solid} of the frame is opaque");
    }
}

#[test]
fn bloom_depth_of_field_and_motion_blur_are_visible() {
    let Some(found) = browser() else {
        return;
    };
    // Bloom: the glowing route lights the dark city around it.
    let still = json!({ "motion_blur": { "samples": 1 }, "depth_of_field": { "aperture": 0 } });
    let mut plain = still.clone();
    plain["bloom"] = json!({ "strength": 0 });
    let off = at(&mut open(&found, "route-map-3d", plain, "bloom-off"), 3.2);
    let on = at(
        &mut open(&found, "route-map-3d", still.clone(), "bloom-on"),
        3.2,
    );
    let (mut lifted, mut dark) = (0.0, 0.0);
    for (a, b) in rgba(&off).chunks(4).zip(rgba(&on).chunks(4)) {
        if luma(a) < 40.0 {
            lifted += luma(b) - luma(a);
            dark += 1.0;
        }
    }
    let lift = lifted / dark;
    assert!(
        lift > 1.5,
        "bloom lifts dark pixels by {lift:.2} on average"
    );

    // Depth of field: the focus follows the route's head at the top of the frame; the
    // blocks nearest the camera, at the bottom, lose fine detail while the head stays sharp.
    let mut focused = still.clone();
    focused["depth_of_field"] = json!({ "aperture": 0.8 });
    let blurred = at(&mut open(&found, "route-map-3d", focused, "focus-on"), 3.2);
    let sharp = detail(&on, (0.7, 1.0));
    let soft = detail(&blurred, (0.7, 1.0));
    assert!(
        soft < sharp * 0.85,
        "depth of field: detail {sharp:.2} became {soft:.2}"
    );
    let head = (detail(&on, (0.0, 0.2)), detail(&blurred, (0.0, 0.2)));
    assert!(head.1 > head.0 * 0.9, "the focus stays sharp: {head:?}");

    // Motion blur: the falling parcel smears along its fall.
    let parcel = |samples: u32, tag: &str| {
        let params = json!({
            "motion_blur": { "samples": samples, "shutter": 1 },
            "depth_of_field": { "aperture": 0 },
            "sparks": 0,
        });
        at(&mut open(&found, "parcel-drop-3d", params, tag), 0.72)
    };
    let frozen = parcel(1, "blur-off");
    let moving = parcel(16, "blur-on");
    // The rows the sharp box covers, and a margin round them.
    let w = frozen.width as usize;
    let orange = |p: &[u8]| p[0] > 120 && p[0] > p[2].saturating_add(60);
    let rows = |frame: &VideoFrame| -> Vec<usize> {
        rgba(frame)
            .chunks(w * 4)
            .enumerate()
            .filter(|(_, row)| row.chunks(4).any(orange))
            .map(|(y, _)| y)
            .collect()
    };
    let (sharp_rows, blurred_rows) = (rows(&frozen), rows(&moving));
    let h = f64::from(frozen.height);
    let span = (
        (*sharp_rows.first().unwrap() as f64 / h - 0.05).max(0.0),
        (*sharp_rows.last().unwrap() as f64 / h + 0.05).min(1.0),
    );
    let crisp = vertical_detail(&frozen, span);
    let smeared = vertical_detail(&moving, span);
    assert!(
        smeared < crisp * 0.75,
        "motion blur: detail along the fall {crisp:.2} became {smeared:.2}"
    );
    // Orange reaches over more rows, more faintly.
    assert!(
        blurred_rows.len() > sharp_rows.len(),
        "the blurred box spans {} rows, the sharp one {}",
        blurred_rows.len(),
        sharp_rows.len()
    );
}

#[test]
fn the_phone_shows_the_shop_app_on_its_screen() {
    let Some(found) = browser() else {
        return;
    };
    let middle = |frame: &VideoFrame| {
        let (w, h) = (frame.width as usize, frame.height as usize);
        let pixels = rgba(frame);
        let mut light = 0;
        let mut all = 0;
        for y in h * 3 / 10..h * 7 / 10 {
            for x in w * 35 / 100..w * 65 / 100 {
                let p = &pixels[(y * w + x) * 4..];
                all += 1;
                if p[0] > 200 && p[1] > 200 && p[2] > 200 {
                    light += 1;
                }
            }
        }
        f64::from(light) / f64::from(all)
    };
    let mut shown = open(&found, "phone-3d", json!({}), "screen-on");
    let frame = at(&mut shown, 4.5);
    let lit = middle(&frame);
    assert!(
        lit > 0.35,
        "the shop app's light page fills the screen ({lit})"
    );
    // Its accent blue shows: the product cards' prices and the selected chip.
    let blue = share(&frame, |p| p[2] > 180 && p[0] < 110 && p[1] < 150);
    assert!(blue > 0.001, "the shop app's blue ({blue})");
    // The screen plays on its own clock: later, its product page has a large blue button.
    let later = at(&mut shown, 5.9);
    assert!(rgba(&later) != rgba(&frame));
    let mut dark = open(&found, "phone-3d", json!({ "screen": null }), "screen-off");
    let off = middle(&at(&mut dark, 4.5));
    assert!(off < 0.05, "an empty screen stays dark ({off})");
}

#[test]
fn the_gpu_draws_the_same_scenes() {
    let Some(found) = browser() else {
        return;
    };
    // The templates ask for the GPU; it is offered on macOS only.
    let (entry, _) = write("parcel-drop-3d", json!({}), "gpu-ask");
    if WebRenderer::choose(WebRenderer::Gpu, None, cfg!(target_os = "macos")) != WebRenderer::Gpu {
        assert_eq!(WebRenderer::for_page(&entry), WebRenderer::Software);
        return;
    }
    assert_eq!(
        WebRenderer::requested(&std::fs::read_to_string(&entry).unwrap()),
        WebRenderer::Gpu
    );
    for (name, times) in [TEMPLATES[1], TEMPLATES[2]] {
        let (entry, composition) = write(name, json!({}), &format!("gpu-{name}"));
        let mut gpu = WebDecoder::open_with(&entry, composition, &found, WebRenderer::Gpu).unwrap();
        let mut software = open(&found, name, json!({}), &format!("cpu-{name}"));
        let a = at(&mut gpu, times[2]);
        let b = at(&mut software, times[2]);
        let quality = psnr(rgba(&a), rgba(&b));
        assert!(
            quality > 26.0,
            "{name}: the GPU and SwiftShader frames differ ({quality:.1} dB)"
        );
        // The GPU draws the same frame again after another.
        at(&mut gpu, times[0]);
        assert!(
            rgba(&at(&mut gpu, times[2])) == rgba(&a),
            "{name} on the GPU"
        );
    }
}

fn goldens_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join("scenes")
}

fn write_png(path: &Path, frame: &VideoFrame) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
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

/// PSNR of the colour channels of `a` and `b` in dB.
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let (mut sum, mut count) = (0.0, 0.0);
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        for c in 0..3 {
            let d = f64::from(p[c]) - f64::from(q[c]);
            sum += d * d;
            count += 1.0;
        }
    }
    let mse = sum / count;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    }
}

/// SwiftShader draws the same pixels on any machine with the same browser, but Chrome
/// updates change its rasterization and shader compilation slightly along edges, in bloom
/// and in blur; 32 dB allows that while failing on any real change of layout, colour,
/// motion or lighting (a box moved by a few pixels, a colour off by a few percent, or a
/// missing glow drops far below it).
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
    write_png(&diffs.join(format!("{name}.actual.png")), frame);
    std::fs::copy(&golden, diffs.join(format!("{name}.expected.png"))).unwrap();
    Some(format!("{name}: PSNR {quality:.1} dB"))
}
