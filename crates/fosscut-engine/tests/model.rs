//! 3D model assets (ADR-0040): `.glb` files, and `.gltf` files with their buffers and
//! images, import as model assets copied into the bundle's `models/` folder by content;
//! files that reach outside their folder are refused; and 3D object clips of them render
//! through the engine's frame path from the copy.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use common::renderer;
use fosscut_core::Request;
use fosscut_engine::frame::frame;
use fosscut_engine::project::{Projects, create, snapshot};
use fosscut_model::{AssetKind, TimelineFormat};
use fosscut_render::Image;
use fosscut_render::objects::{fixture_glb, fixture_gltf};
use fosscut_time::{Flicks, Rate};
use serde_json::{Value, json};

/// A 16 by 16 PNG of blue and white squares.
fn checker_png() -> Vec<u8> {
    let mut rgba = Vec::new();
    for y in 0..16u32 {
        for x in 0..16u32 {
            let on = (x / 4 + y / 4) % 2 == 0;
            rgba.extend_from_slice(if on {
                &[40, 90, 200, 255]
            } else {
                &[240, 240, 240, 255]
            });
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgba)
            .unwrap();
    }
    out
}

/// A folder with `house.gltf` naming `parts/house data.bin` and `walls.png` beside it,
/// `house.glb` with everything inside, and `escape.gltf` naming a file outside its folder.
fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-model-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let source = dir.join("source");
    std::fs::create_dir_all(source.join("parts")).unwrap();
    let png = checker_png();
    let (json, bin) = fixture_gltf(
        Some(&png),
        Some("parts/house%20data.bin"),
        Some("walls.png"),
    );
    std::fs::write(source.join("house.gltf"), json).unwrap();
    std::fs::write(source.join("parts").join("house data.bin"), bin).unwrap();
    std::fs::write(source.join("walls.png"), &png).unwrap();
    std::fs::write(source.join("house.glb"), fixture_glb(Some(&png))).unwrap();
    let (escape, bin) = fixture_gltf(None, Some("../outside.bin"), None);
    std::fs::write(source.join("escape.gltf"), escape).unwrap();
    std::fs::write(dir.join("outside.bin"), bin).unwrap();
    dir
}

fn run(projects: &Projects, bundle: &Path, command: &str, args: Value) -> Value {
    let response = projects
        .execute(bundle, Request::single(command, args))
        .unwrap_or_else(|e| panic!("{command}: {e:?}"));
    response.results[0].clone()
}

fn format() -> TimelineFormat {
    TimelineFormat {
        width: 1920,
        height: 1080,
        rate: Rate::FPS_30,
        sample_rate: 48_000,
    }
}

#[test]
fn models_import_into_the_bundle_with_their_files() {
    let dir = workspace("import");
    let source = dir.join("source");
    let bundle = dir.join("House.fosscut");
    create(&bundle, "House", format()).unwrap();

    let probed = fosscut_engine::import::probe(&source.join("house.gltf")).unwrap();
    assert_eq!(probed.kind, AssetKind::Model);
    let info = probed.info.model.as_ref().unwrap();
    assert_eq!(
        (info.meshes, info.triangles, info.materials, info.textures),
        (2, 18, 2, 1)
    );
    assert!((info.size[1] - 1.6).abs() < 1e-5);
    assert_eq!(info.copyright.as_deref(), Some("The FossCut Contributors"));

    let args =
        fosscut_engine::import::media_add_args(&source.join("house.gltf"), None, &bundle).unwrap();
    assert_eq!(args["kind"], "model");
    assert_eq!(args["name"], "house.gltf");
    let relative = args["path"]["relative"].as_str().unwrap().to_string();
    assert!(
        relative.starts_with("models/") && relative.ends_with("/house.gltf"),
        "{relative}"
    );
    let copied = bundle.join(&relative);
    let folder = copied.parent().unwrap();
    for file in ["house.gltf", "parts/house data.bin", "walls.png"] {
        assert!(folder.join(file).is_file(), "{file} was copied");
    }
    // The copy is what loads: the original folder can go.
    let again =
        fosscut_engine::import::media_add_args(&source.join("house.gltf"), None, &bundle).unwrap();
    assert_eq!(
        again["path"]["relative"], args["path"]["relative"],
        "copied once"
    );
    assert_eq!(again["identity"], args["identity"]);

    let glb =
        fosscut_engine::import::media_add_args(&source.join("house.glb"), Some("Hut"), &bundle)
            .unwrap();
    assert_eq!(glb["name"], "Hut");
    assert_ne!(glb["path"]["relative"], args["path"]["relative"]);

    // A file outside the model's folder is never copied or read.
    let error = fosscut_engine::import::media_add_args(&source.join("escape.gltf"), None, &bundle)
        .err()
        .unwrap();
    assert_eq!(error.code, "fosscut.media.probe_failed");
    std::fs::write(source.join("broken.glb"), b"glTF but not really").unwrap();
    let error = fosscut_engine::import::probe(&source.join("broken.glb"))
        .err()
        .unwrap();
    assert_eq!(error.code, "fosscut.media.probe_failed");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn objects_of_imported_models_render_from_the_bundle() {
    let dir = workspace("render");
    let source = dir.join("source");
    let bundle = dir.join("House.fosscut");
    create(&bundle, "House", format()).unwrap();
    let projects = Projects::with_runtime("test", dir.join("runtime"));
    let args =
        fosscut_engine::import::media_add_args(&source.join("house.gltf"), None, &bundle).unwrap();
    let asset = run(&projects, &bundle, "media.add", args)["asset"].clone();
    run(
        &projects,
        &bundle,
        "model.add",
        json!({
            "asset": asset,
            "set": { "object": { "size": 0.35 }, "floor": { "catcher": false, "color": "#506070" } },
            "transform": { "rotation_y": 30 },
            "duration": "1s"
        }),
    );
    // The original files can go: the copy in the bundle is what renders.
    std::fs::remove_dir_all(&source).unwrap();
    let project = snapshot(&bundle).unwrap();
    let Some(mut renderer) = renderer() else {
        std::fs::remove_dir_all(&dir).unwrap();
        return;
    };
    let timeline = project.manifest.primary_timeline.clone();
    let still = frame(
        &project,
        &bundle,
        &timeline,
        Flicks::from_seconds_f64(0.5).unwrap(),
        Some(640),
        &mut renderer,
    )
    .unwrap();
    let image: &Image = &still.image;
    let count = |test: &dyn Fn([i32; 3]) -> bool| {
        image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| test([i32::from(p[0]), i32::from(p[1]), i32::from(p[2])]))
            .count()
    };
    let roof = count(&|[r, g, b]| r > 2 * g && r > 2 * b && r > 60);
    let walls = count(&|[r, g, b]| b > r + 50 && b > g + 20);
    assert!(roof > 300, "the red roof: {roof}");
    assert!(walls > 100, "the checkered walls: {walls}");
    std::fs::remove_dir_all(&dir).unwrap();
}
