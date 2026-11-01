//! Exports, frame grabs, contact sheets and the preview with the effects that follow a
//! subject (ADR-0033): a missing matte or pose fails before anything renders with
//! `fosscut.analysis.missing`, and analyses written straight into the cache (no model runs)
//! reach the renderer.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::{frames, project, renderer, workspace};
use fosscut_analysis::matte::{MatteFrame, MatteModel, MatteTrack};
use fosscut_analysis::pose::{Landmark, PoseFrame, PoseTrack};
use fosscut_analysis::vision::{DataFile, pack};
use fosscut_analysis::{Analyzer, Options, cache};
use fosscut_engine::frame::{contact_sheet_in, frame_in};
use fosscut_engine::preview::{PreviewOptions, Previewer};
use fosscut_engine::{EngineError, ExportRequest, Preset, export, plan};
use fosscut_media::{DecoderOptions, VideoDecoder};
use fosscut_model::{ContentIdentity, Effect, Id, IdKind, ParamValue, Project};
use fosscut_render::Image;
use serde_json::json;

/// A made-up content identity for the clip's file, recorded in the media table, so the
/// cache is keyed without hashing the file.
fn identity() -> ContentIdentity {
    ContentIdentity {
        size: 4242,
        sample_blake3: "5e".repeat(32),
    }
}

/// The shared project with `effects` on its video clip.
fn with_effects(effects: Vec<Effect>) -> Project {
    let mut project = project("../clip.mp4");
    project.media.assets[0].identity = Some(identity());
    let timeline = project.manifest.primary_timeline.clone();
    let doc = project.timelines.get_mut(&timeline).unwrap();
    doc.tracks[0].clips[0].effects = effects;
    assert!(
        fosscut_model::validate(&project).is_empty(),
        "{:?}",
        fosscut_model::validate(&project)
    );
    project
}

fn effect(n: u64, kind: &str, params: &[(&str, ParamValue)]) -> Effect {
    Effect {
        id: Id::new(IdKind::Effect, 100 + n),
        effect_type: kind.into(),
        version: 1,
        enabled: true,
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
        keyframes: BTreeMap::new(),
    }
}

fn name(v: &str) -> ParamValue {
    ParamValue::Name(v.into())
}

fn request(project: &Project, cache: &Path, output: &str) -> ExportRequest {
    let timeline = project.manifest.primary_timeline.clone();
    let preset = Preset::default_video().unwrap();
    ExportRequest {
        bundle: workspace().join("Show.fosscut"),
        settings: preset.settings(&project.timelines[&timeline].format),
        timeline,
        output: workspace().join(format!("{output}.{}", preset.extension())),
        allow_overwrite: true,
        expected_revision: None,
        request_id: None,
        cache: cache.to_path_buf(),
        options: Default::default(),
    }
}

/// An empty cache folder of its own.
fn fresh_cache(name: &str) -> PathBuf {
    let dir = workspace().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn assert_missing(error: EngineError, analyzer: &str, options: serde_json::Value, effect: &str) {
    assert_eq!(error.code(), "fosscut.analysis.missing", "{error}");
    assert_eq!(
        error.details(),
        json!({ "asset": "asset_0001", "analyzer": analyzer, "options": options, "effect": effect })
    );
    assert!(
        error
            .hint()
            .unwrap()
            .contains(&format!("fosscut media analyze asset_0001 {analyzer}"))
    );
}

#[test]
fn missing_analyses_fail_before_anything_renders() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let cache = fresh_cache("cache-missing-subject");
    let timeline_of = |p: &Project| p.manifest.primary_timeline.clone();
    let bundle = workspace().join("Show.fosscut");

    let keyed = with_effects(vec![effect(1, "key.subject", &[])]);
    let person = json!({ "matte": { "model": "person" } });
    let job = request(&keyed, &cache, "missing-matte");
    let _ = std::fs::remove_file(&job.output);
    assert_missing(
        export(&keyed, &job, &mut renderer, &mut |_| true).unwrap_err(),
        "matte",
        person.clone(),
        "key.subject",
    );
    assert!(!job.output.exists(), "nothing was written");
    assert_missing(
        plan(&keyed, &job).unwrap_err(),
        "matte",
        person.clone(),
        "key.subject",
    );
    let grab = frame_in(
        &keyed,
        &bundle,
        &timeline_of(&keyed),
        frames(20),
        Some(160),
        &cache,
        &mut renderer,
    );
    assert_missing(grab.unwrap_err(), "matte", person.clone(), "key.subject");
    let sheet = contact_sheet_in(
        &keyed,
        &bundle,
        &timeline_of(&keyed),
        4,
        80,
        &cache,
        &mut renderer,
    );
    assert_missing(sheet.unwrap_err(), "matte", person, "key.subject");

    // The subject matte is another analysis, and the body effects need the pose.
    let subject = with_effects(vec![effect(
        1,
        "key.subject",
        &[("source", name("subject"))],
    )]);
    assert_missing(
        plan(&subject, &request(&subject, &cache, "missing-subject")).unwrap_err(),
        "matte",
        json!({ "matte": { "model": "subject" } }),
        "key.subject",
    );
    let glowing = with_effects(vec![effect(1, "body.glow", &[])]);
    assert_missing(
        plan(&glowing, &request(&glowing, &cache, "missing-pose")).unwrap_err(),
        "pose",
        json!({}),
        "body.glow",
    );

    // A bypassed effect needs nothing.
    let mut bypassed = with_effects(vec![effect(1, "body.glow", &[])]);
    let timeline = timeline_of(&bypassed);
    bypassed.timelines.get_mut(&timeline).unwrap().tracks[0].clips[0].effects[0].enabled = false;
    assert!(plan(&bypassed, &request(&bypassed, &cache, "bypassed")).is_ok());
    frame_in(
        &bypassed,
        &bundle,
        &timeline,
        frames(20),
        Some(160),
        &cache,
        &mut renderer,
    )
    .unwrap();
    // The preview shows the clip without its effect while the analysis is missing.
    let mut previewer = Previewer::with_options(PreviewOptions {
        cache: Some(cache.clone()),
        ..PreviewOptions::default()
    });
    let shown = previewer
        .grab(
            &mut renderer,
            &glowing,
            &bundle,
            &timeline,
            frames(20),
            (160, 90),
        )
        .unwrap();
    let plain = frame_in(
        &bypassed,
        &bundle,
        &timeline,
        frames(20),
        Some(160),
        &cache,
        &mut renderer,
    )
    .unwrap();
    assert!(shown.image.psnr(&plain.image).unwrap() > 40.0, "unchanged");
}

/// Writes `data` with `result` into `cache` as `analyzer`'s entry with `options`.
fn cache_analysis(
    cache: &Path,
    analyzer: Analyzer,
    options: &Options,
    result: serde_json::Value,
    data: &[u8],
) {
    let key = options.for_analyzer(analyzer);
    let path = cache::entry_path(cache, &identity(), analyzer, &key);
    cache::write_data(&path, data).unwrap();
    cache::write(&path, analyzer, &key, &result).unwrap();
}

/// A matte of the clip's frames: the left half of the picture is the subject.
fn left_half_matte() -> (MatteTrack, Vec<u8>) {
    let (w, h) = (64u32, 36u32);
    let alpha: Vec<u8> = (0..w * h)
        .map(|i| if i % w < w / 2 { 255 } else { 0 })
        .collect();
    let track = MatteTrack {
        size: (w, h),
        frames: (0..60)
            .map(|i| MatteFrame {
                time: frames(i),
                key: true,
                alpha: pack(&alpha),
            })
            .collect(),
    };
    let data = track.encode();
    (track, data)
}

/// A pose of the clip's frames: a person whose mask is a box in the middle of the picture.
fn box_pose() -> Vec<u8> {
    let (w, h) = (64u32, 36u32);
    let mask: Vec<u8> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            if (24..40).contains(&x) && (8..28).contains(&y) {
                255
            } else {
                0
            }
        })
        .collect();
    let landmark = Landmark {
        x: 0.5,
        y: 0.5,
        visibility: 0.9,
        presence: 0.9,
    };
    PoseTrack {
        mask_size: (w, h),
        frames: (0..60)
            .map(|i| PoseFrame {
                time: frames(i),
                presence: 0.95,
                landmarks: [landmark; 33],
                mask: pack(&mask),
            })
            .collect(),
    }
    .encode()
}

fn rgb(image: &Image, x: u32, y: u32) -> [u8; 3] {
    let p = image.pixel(x, y);
    [p[0], p[1], p[2]]
}

#[test]
fn cached_analyses_reach_exports_frames_and_the_preview() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let cache = fresh_cache("cache-subject");
    let bundle = workspace().join("Show.fosscut");
    let (track, data) = left_half_matte();
    let summary = track.summary(MatteModel::Person, DataFile::of(&data));
    cache_analysis(
        &cache,
        Analyzer::Matte,
        &Options::default(),
        serde_json::to_value(&summary).unwrap(),
        &data,
    );

    // The right half of the picture becomes red.
    let keyed = with_effects(vec![effect(
        1,
        "key.subject",
        &[("background", name("color")), ("color", name("#FF0000"))],
    )]);
    let timeline = keyed.manifest.primary_timeline.clone();
    let still = frame_in(
        &keyed,
        &bundle,
        &timeline,
        frames(20),
        Some(320),
        &cache,
        &mut renderer,
    )
    .unwrap();
    let red = |p: [u8; 3]| p[0] > 230 && p[1] < 25 && p[2] < 25;
    for y in [20, 90, 160] {
        assert!(
            red(rgb(&still.image, 250, y)),
            "the background is red: {:?}",
            rgb(&still.image, 250, y)
        );
        assert!(
            !red(rgb(&still.image, 60, y)),
            "the subject stays: {:?}",
            rgb(&still.image, 60, y)
        );
    }
    let sheet =
        contact_sheet_in(&keyed, &bundle, &timeline, 2, 160, &cache, &mut renderer).unwrap();
    // The second tile shows the clip (the first half has the title over it too).
    let tile = &sheet.tiles[1];
    assert!(red(rgb(&sheet.image, tile.x + 130, tile.y + 45)));

    // The same in an export, decoded back.
    let job = request(&keyed, &cache, "keyed");
    let report = export(&keyed, &job, &mut renderer, &mut |_| true).unwrap();
    assert_eq!(report.frames, 30);
    let mut decoder =
        fosscut_media::ffmpeg::FfmpegVideoDecoder::open(&job.output, DecoderOptions::default())
            .unwrap();
    let frame = decoder.frame_at(frames(20)).unwrap();
    // Red is high Cr; the test pattern on the left is not that red everywhere.
    let chroma = |x: usize, y: usize| match frame.format {
        fosscut_media::PixelFormat::Nv12 => frame.planes[1].row(y / 2)[(x / 2) * 2 + 1],
        fosscut_media::PixelFormat::Yuv420p => frame.planes[2].row(y / 2)[x / 2],
        fosscut_media::PixelFormat::Rgba => panic!("an opaque export decodes as Y'CbCr"),
    };
    assert!(
        chroma(260, 90) > 220,
        "red on the right: Cr {}",
        chroma(260, 90)
    );
    let left: Vec<u8> = (0..8).map(|i| chroma(10 + i * 18, 90)).collect();
    assert!(
        left.iter().any(|&cr| cr < 200),
        "the picture on the left: {left:?}"
    );

    // The preview reads the same cache and keys its frames by the analysis.
    let mut previewer = Previewer::with_options(PreviewOptions {
        cache: Some(cache.clone()),
        render_cache_bytes: Some(64 << 20),
        ..PreviewOptions::default()
    });
    let shown = previewer
        .grab(
            &mut renderer,
            &keyed,
            &bundle,
            &timeline,
            frames(20),
            (320, 180),
        )
        .unwrap();
    assert!(red(rgb(&shown.image, 250, 90)));
    assert!(
        shown.image.psnr(&still.image).unwrap() > 40.0,
        "preview and frame grab agree"
    );

    // The body effects read the pose: a white outline around the box.
    let pose = box_pose();
    let summary = PoseTrack::decode(&pose)
        .unwrap()
        .summary(DataFile::of(&pose));
    cache_analysis(
        &cache,
        Analyzer::Pose,
        &Options::default(),
        serde_json::to_value(&summary).unwrap(),
        &pose,
    );
    let outlined = with_effects(vec![effect(
        2,
        "body.outline",
        &[
            ("color", name("#FFFFFF")),
            ("width", ParamValue::Number(0.03)),
            ("softness", ParamValue::Number(0.0)),
        ],
    )]);
    let still = frame_in(
        &outlined,
        &bundle,
        &timeline,
        frames(20),
        Some(320),
        &cache,
        &mut renderer,
    )
    .unwrap();
    let plain = frame_in(
        &with_effects(Vec::new()),
        &bundle,
        &timeline,
        frames(20),
        Some(320),
        &cache,
        &mut renderer,
    )
    .unwrap();
    // The box spans x 120 to 200 of 320; the line lies just outside it.
    assert!(
        rgb(&still.image, 115, 90).iter().all(|&c| c >= 250),
        "{:?}",
        rgb(&still.image, 115, 90)
    );
    assert_eq!(
        rgb(&still.image, 160, 90),
        rgb(&plain.image, 160, 90),
        "inside"
    );
    assert_eq!(
        rgb(&still.image, 40, 90),
        rgb(&plain.image, 40, 90),
        "far away"
    );

    // A data file that does not match its entry is never used.
    let key = Options::default().for_analyzer(Analyzer::Pose);
    let entry = cache::entry_path(&cache, &identity(), Analyzer::Pose, &key);
    std::fs::write(cache::data_path(&entry), b"damaged").unwrap();
    let error = frame_in(
        &outlined,
        &bundle,
        &timeline,
        frames(20),
        Some(320),
        &cache,
        &mut renderer,
    )
    .unwrap_err();
    assert_eq!(error.code(), "fosscut.analysis.missing");
}

/// The models folder when ONNX Runtime and the pose model are installed; with
/// `FOSSCUT_REQUIRE_MODELS` set, a missing one fails the test.
fn pose_models() -> Option<PathBuf> {
    let dir = std::env::var_os("FOSSCUT_MODELS_DIR").map(PathBuf::from);
    let ready = dir.as_ref().is_some_and(|d| {
        fosscut_analysis::onnx::runtime_path(d).is_some()
            && fosscut_analysis::models::is_installed(
                d,
                fosscut_analysis::models::manifest("mediapipe-pose-full").unwrap(),
            )
    });
    assert!(
        ready || !common::required("FOSSCUT_REQUIRE_MODELS"),
        "FOSSCUT_REQUIRE_MODELS is set but ONNX Runtime or mediapipe-pose-full is not installed in FOSSCUT_MODELS_DIR"
    );
    ready.then(|| dir.unwrap())
}

#[test]
fn analyses_from_the_models_are_cached_with_their_data_and_render() {
    use fosscut_engine::analysis::{AnalysisRequest, analyze};
    let Some(mut renderer) = renderer() else {
        return;
    };
    let cache = fresh_cache("cache-subject-models");
    let bundle = workspace().join("Show.fosscut");
    let keyed = with_effects(vec![
        effect(1, "key.subject", &[]),
        effect(2, "body.glow", &[]),
    ]);
    let mut job = AnalysisRequest {
        bundle: bundle.clone(),
        asset: keyed.media.assets[0].id.clone(),
        analyzers: vec![Analyzer::Pose, Analyzer::Matte],
        options: Options::default(),
        refresh: false,
        cache: cache.clone(),
        models: workspace().join("no-models"),
    };
    // Without the models nothing is decoded, and the error names what to install.
    let missing = analyze(&keyed, &job, &mut |_| true).unwrap_err();
    assert_eq!(missing.code(), "fosscut.model.not_installed");
    assert!(
        missing.details()["what"]
            .as_str()
            .unwrap()
            .starts_with("mediapipe-pose-full"),
        "{missing}"
    );
    let Some(models) = pose_models() else {
        return;
    };
    job.models = models;
    let report = analyze(&keyed, &job, &mut |_| true).unwrap();
    assert_eq!(report.computed, [Analyzer::Pose, Analyzer::Matte]);
    // The test pattern has nobody in it: 60 frames, no person, an empty matte.
    let pose = report.results.pose.as_ref().unwrap();
    assert_eq!((pose.frames, pose.frames_with_person), (60, 0));
    let matte = report.results.matte.as_ref().unwrap();
    assert_eq!((matte.frames, matte.kind), (60, MatteModel::Person));
    assert!(matte.coverage < 0.01, "{}", matte.coverage);
    for analyzer in [Analyzer::Pose, Analyzer::Matte] {
        let key = job.options.for_analyzer(analyzer);
        let entry = cache::entry_path(&cache, &identity(), analyzer, &key);
        assert!(
            entry.is_file() && cache::data_path(&entry).is_file(),
            "{analyzer:?}"
        );
    }
    let again = analyze(&keyed, &job, &mut |_| true).unwrap();
    assert_eq!(again.cached, [Analyzer::Pose, Analyzer::Matte]);
    assert_eq!(again.results.pose, report.results.pose, "the same results");
    // A lost data file is measured again.
    let key = job.options.for_analyzer(Analyzer::Pose);
    std::fs::remove_file(cache::data_path(&cache::entry_path(
        &cache,
        &identity(),
        Analyzer::Pose,
        &key,
    )))
    .unwrap();
    let redone = analyze(&keyed, &job, &mut |_| true).unwrap();
    assert_eq!(
        (redone.computed, redone.cached),
        (vec![Analyzer::Pose], vec![Analyzer::Matte])
    );

    // Now the clip exports: nobody to keep, so the key leaves black where the clip was.
    let export_job = request(&keyed, &cache, "keyed-by-model");
    let exported = export(&keyed, &export_job, &mut renderer, &mut |_| true).unwrap();
    assert_eq!(exported.frames, 30);
    let still = frame_in(
        &keyed,
        &bundle,
        &keyed.manifest.primary_timeline,
        frames(20),
        Some(160),
        &cache,
        &mut renderer,
    )
    .unwrap();
    assert!(
        still
            .image
            .rgba
            .chunks(4)
            .all(|p| p[0] < 8 && p[1] < 8 && p[2] < 8),
        "all removed"
    );
}
