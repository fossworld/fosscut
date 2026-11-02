//! Graphics keep their code values (ADR-0041): hex colours in shapes and text, pictures
//! tagged sRGB and web composition frames come out of exports and frame grabs with the code
//! values they were given, while compositing still happens in linear light. Exports are
//! decoded with FFmpeg. The web composition test skips without a browser unless
//! `FOSSCUT_REQUIRE_BROWSER` is set.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{renderer, required};
use fosscut_engine::web::CompositionOptions;
use fosscut_media::ffmpeg::FfmpegVideoDecoder;
use fosscut_media::{DecoderOptions, Transfer, VideoDecoder};
use fosscut_model::{
    Asset, AssetKind, Clip, ClipContent, Color, Id, IdKind, MediaInfo, PathHints, Project,
    TextContent, TextStyle, TimelineFormat, Track, TrackKind, VideoInfo,
};
use fosscut_time::{Flicks, Rate};

/// A dark brand colour: decoded as sRGB, it used to come out as (30, 43, 72).
const NAVY: [u8; 3] = [0x2E, 0x3B, 0x57];
/// An orange: it used to come out as (255, 109, 53).
const ORANGE: [u8; 3] = [0xFF, 0x7A, 0x45];

/// A fresh folder for one test, holding a `Show.fosscut` bundle.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-colours-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Show.fosscut")).unwrap();
    dir
}

/// A 320x180, 30 fps project of 1 s whose tracks, bottom to top, each hold one clip of
/// `contents`, with `assets` in its media.
fn project(assets: Vec<Asset>, contents: Vec<(TrackKind, ClipContent)>) -> Project {
    let format = TimelineFormat {
        width: 320,
        height: 180,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Show", format);
    project.media.assets = assets;
    project.manifest.next_id = 20;
    let timeline = project.manifest.primary_timeline.clone();
    let doc = project.timelines.get_mut(&timeline).unwrap();
    for (index, (kind, content)) in contents.into_iter().enumerate() {
        doc.tracks.push(Track {
            id: Id::new(IdKind::Track, 2 + 2 * index as u64),
            kind,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![Clip::new(
                Id::new(IdKind::Clip, 3 + 2 * index as u64),
                Flicks::ZERO,
                Rate::FPS_30.frames_to_flicks(30),
                content,
            )],
        });
    }
    assert!(
        fosscut_model::validate(&project).is_empty(),
        "{:?}",
        fosscut_model::validate(&project)
    );
    project
}

/// A shape filling the canvas in `hex`, written as stored so it reads with any defaults.
fn fill(hex: &str) -> (TrackKind, ClipContent) {
    let content = serde_json::from_value(serde_json::json!({
        "kind": "shape",
        "shape": "rectangle",
        "size": [1.0, 1.0],
        "fill": { "color": hex },
    }))
    .unwrap();
    (TrackKind::Video, content)
}

/// Exports `project` as ProRes into `dir` and returns its middle frame as RGB, decoded by
/// FFmpeg with the BT.709 limited-range tags the export carries.
fn exported(project: &Project, dir: &Path, renderer: &mut fosscut_render::Renderer) -> Vec<u8> {
    let timeline = project.manifest.primary_timeline.clone();
    let format = project.timelines[&timeline].format.clone();
    let preset = fosscut_engine::Preset::ALL
        .into_iter()
        .find(|p| p.available() && p.name() == "mov-prores")
        .expect("ProRes is always available");
    let output = dir.join("colours.mov");
    let request = fosscut_engine::ExportRequest {
        bundle: dir.join("Show.fosscut"),
        settings: preset.settings(&format),
        timeline,
        output: output.clone(),
        allow_overwrite: true,
        expected_revision: None,
        request_id: None,
        cache: dir.join("cache"),
        options: Default::default(),
    };
    let report = fosscut_engine::export(project, &request, renderer, &mut |_| true).unwrap();
    assert_eq!(report.frames, 30);
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-ss", "0.5", "-i"])
        .arg(&output)
        .args(["-frames:v", "1", "-vf"])
        .arg(
            "scale=in_color_matrix=bt709:in_range=tv:out_range=pc:\
             flags=accurate_rnd+full_chroma_int,format=rgb24",
        )
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(decoded.status.success(), "{decoded:?}");
    assert_eq!(decoded.stdout.len(), 320 * 180 * 3);
    decoded.stdout
}

fn rgb_at(rgb: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * 320 + x) * 3;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

fn assert_code_values(got: [u8; 3], want: [u8; 3], what: &str) {
    assert!(
        got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1),
        "{what}: {got:?}, want {want:?}"
    );
}

#[test]
fn hex_colours_keep_their_code_values_in_exports() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let dir = folder("hex");
    let title = TextContent {
        text: "I".into(),
        style: TextStyle {
            color: Color::parse("#FF7A45").unwrap(),
            size: 0.8,
            bold: true,
            ..TextStyle::default()
        },
        ..Default::default()
    };
    let project = project(
        Vec::new(),
        vec![
            fill("#2E3B57"),
            (TrackKind::Text, ClipContent::Text(Box::new(title))),
        ],
    );
    let rgb = exported(&project, &dir, &mut renderer);
    assert_code_values(rgb_at(&rgb, 10, 10), NAVY, "the shape's fill");
    // The text's ink: the colour most pixels away from the background have, which is the
    // inside of the letter's stem.
    let mut counts = std::collections::HashMap::<[u8; 3], usize>::new();
    for pixel in rgb.as_chunks::<3>().0 {
        if pixel.iter().zip(NAVY).any(|(p, n)| p.abs_diff(n) > 8) {
            *counts.entry(*pixel).or_default() += 1;
        }
    }
    let (ink, count) = counts.into_iter().max_by_key(|(_, n)| *n).unwrap();
    assert!(
        count > 500,
        "the letter is large: {count} pixels of {ink:?}"
    );
    assert_code_values(ink, ORANGE, "the text's colour");
}

#[test]
fn pictures_tagged_srgb_keep_their_code_values_in_exports() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let dir = folder("png");
    // A PNG with an sRGB chunk: navy on the left half, orange on the right.
    let picture = dir.join("tagged.png");
    let (width, height) = (320u32, 180u32);
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);
    for _ in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(if x < width / 2 { &NAVY } else { &ORANGE });
        }
    }
    {
        let file = std::io::BufWriter::new(std::fs::File::create(&picture).unwrap());
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
    }
    // FFmpeg reads the chunk as the sRGB transfer, so this is the tagged path.
    let mut decoder = FfmpegVideoDecoder::open(&picture, DecoderOptions::default()).unwrap();
    let frame = decoder.frame_at(Flicks::ZERO).unwrap();
    assert_eq!(frame.color.transfer, Transfer::Srgb);

    let asset = Id::new(IdKind::Asset, 1);
    let project = project(
        vec![Asset {
            folder: None,
            id: asset.clone(),
            kind: AssetKind::Image,
            name: "tagged.png".into(),
            path: PathHints {
                relative: None,
                absolute: Some(picture.display().to_string()),
            },
            identity: None,
            info: MediaInfo {
                video: Some(VideoInfo {
                    width,
                    height,
                    rate: None,
                    codec: Some("png".into()),
                    rotation: 0,
                }),
                ..Default::default()
            },
        }],
        vec![(TrackKind::Video, ClipContent::Image { asset })],
    );
    let rgb = exported(&project, &dir, &mut renderer);
    assert_code_values(rgb_at(&rgb, 40, 90), NAVY, "the picture's navy");
    assert_code_values(rgb_at(&rgb, 280, 90), ORANGE, "the picture's orange");
}

#[test]
fn web_colours_match_the_same_hex_in_a_shape() {
    let found = fosscut_engine::web::find_browser();
    assert!(
        found.is_some() || !required("FOSSCUT_REQUIRE_BROWSER"),
        "FOSSCUT_REQUIRE_BROWSER is set but no browser was found"
    );
    if found.is_none() {
        return;
    }
    let Some(mut renderer) = renderer() else {
        return;
    };
    let dir = folder("web");
    // The page paints its left half navy and an orange band along the top; the rest is
    // transparent, so the navy shape below shows on the right.
    let source = dir.join("page");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("index.html"),
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"fosscut:composition\" content=\"width=320, height=180, fps=30, duration=1\">\
         <style>html, body { margin: 0; background: transparent; }\
         #left { position: absolute; left: 0; top: 0; width: 160px; height: 180px; background: #2E3B57; }\
         #band { position: absolute; left: 0; top: 0; width: 320px; height: 20px; background: #FF7A45; }</style>\
         </head><body><div id=\"left\"></div><div id=\"band\"></div></body></html>",
    )
    .unwrap();
    let bundle = dir.join("Show.fosscut");
    let args =
        fosscut_engine::web::media_add_args(&source, None, &bundle, &CompositionOptions::default())
            .unwrap();
    let asset = Id::new(IdKind::Asset, 1);
    let project = project(
        vec![Asset {
            folder: None,
            id: asset.clone(),
            kind: AssetKind::Web,
            name: args["name"].as_str().unwrap().into(),
            path: serde_json::from_value(args["path"].clone()).unwrap(),
            identity: serde_json::from_value(args["identity"].clone()).unwrap(),
            info: serde_json::from_value(args["info"].clone()).unwrap(),
        }],
        vec![
            fill("#2E3B57"),
            (
                TrackKind::Video,
                ClipContent::Video {
                    asset,
                    source_in: Flicks::ZERO,
                },
            ),
        ],
    );
    let timeline = project.manifest.primary_timeline.clone();
    let still = fosscut_engine::frame::frame_in(
        &project,
        &bundle,
        &timeline,
        Rate::FPS_30.frames_to_flicks(15),
        None,
        &dir.join("cache"),
        &mut renderer,
    )
    .unwrap();
    let rgb = |x: u32, y: u32| {
        let i = ((y * still.image.width + x) * 4) as usize;
        [
            still.image.rgba[i],
            still.image.rgba[i + 1],
            still.image.rgba[i + 2],
        ]
    };
    let web = rgb(80, 100);
    let shape = rgb(240, 100);
    assert_eq!(web, shape, "the page's navy and the shape's navy");
    assert_code_values(web, NAVY, "navy");
    assert_code_values(rgb(240, 10), ORANGE, "the page's orange");
}
