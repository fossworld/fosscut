//! Golden-frame tests (ADR-0016, item 4; the `golden-frame-tests` skill): scenes rendered
//! at 320x180 and compared with reviewed PNGs in `tests/goldens/` by PSNR.
//!
//! `cargo xtask goldens --update` rewrites the PNGs (it sets `FOSSCUT_UPDATE_GOLDENS=1`);
//! review every changed image before committing. On a mismatch the actual image is written
//! to `target/golden-diffs/` next to the expected one.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fosscut_model::Transform;
use fosscut_render::color::TransferFunction;
use fosscut_render::lut::Lut3d;
use fosscut_render::plan::{Layer, Mask, MaskShape, RenderPlan};
use fosscut_render::{Image, Renderer};
use support::*;

const SIZE: (u32, u32) = (320, 180);

/// Below 40 dB differences start to be visible. Software rasterizers (lavapipe, WARP) and
/// GPUs differ slightly in filtering and edge coverage, mostly along rotated and masked
/// edges, so 36 dB leaves room for that while still failing on any real change in color,
/// placement, masking or blending.
const MIN_PSNR_DB: f64 = 36.0;

fn goldens_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
}

fn write_png(path: &Path, image: &Image) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&image.rgba)
        .unwrap();
}

fn read_png(path: &Path) -> Image {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgba).unwrap();
    rgba.truncate(info.buffer_size());
    Image {
        width: info.width,
        height: info.height,
        rgba,
    }
}

fn check(name: &str, renderer: &mut Renderer, plan: &RenderPlan, frames: &mut Frames) {
    let image = renderer
        .render_image(plan, frames, SIZE, TransferFunction::Bt709)
        .unwrap();
    let golden = goldens_dir().join(format!("{name}.png"));
    if std::env::var_os("FOSSCUT_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(goldens_dir()).unwrap();
        write_png(&golden, &image);
        eprintln!("updated {}", golden.display());
        return;
    }
    assert!(
        golden.exists(),
        "{} is missing; run `cargo xtask goldens --update` and review it",
        golden.display()
    );
    let expected = read_png(&golden);
    let psnr = image.psnr(&expected).expect("golden has a different size");
    if psnr < MIN_PSNR_DB {
        let diffs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden-diffs");
        std::fs::create_dir_all(&diffs).unwrap();
        write_png(&diffs.join(format!("{name}.actual.png")), &image);
        write_png(&diffs.join(format!("{name}.expected.png")), &expected);
        panic!(
            "{name}: PSNR {psnr:.1} dB is below {MIN_PSNR_DB} dB on {:?} ({:?}); images in {}",
            renderer.adapter().name,
            renderer.adapter().backend,
            diffs.display()
        );
    }
}

#[test]
fn composite_scene() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let warm = Lut3d::parse_cube(include_str!("goldens/warm.cube")).unwrap();
    let overlay = Layer {
        transform: Transform {
            position: [0.18, -0.12],
            scale: 0.55,
            rotation: 12.0,
        },
        opacity: 0.85,
        mask: Some(Mask {
            shape: MaskShape::Ellipse,
            center: [0.5, 0.5],
            size: [0.9, 0.9],
            feather: 0.3,
            invert: false,
        }),
        lut: Some(Arc::new(warm)),
        ..Layer::new(source(2))
    };
    let plan = RenderPlan {
        canvas: (1920, 1080),
        layers: vec![Layer::new(source(1)), overlay],
        skipped: Vec::new(),
    };
    let mut frames = Frames::default()
        .with(1, pattern(640, 360, 0))
        .with(2, pattern(400, 400, 100));
    check("composite-scene", &mut renderer, &plan, &mut frames);
}

#[test]
fn portrait_source_on_a_vertical_canvas() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let plan = RenderPlan {
        canvas: (1080, 1920),
        layers: vec![Layer {
            rotation: 90,
            ..Layer::new(source(1))
        }],
        skipped: Vec::new(),
    };
    let mut frames = Frames::default().with(1, pattern(320, 180, 40));
    let image = renderer
        .render_image(&plan, &mut frames, (180, 320), TransferFunction::Bt709)
        .unwrap();
    // Rendered at the canvas aspect; compare against a golden at that size.
    let golden = goldens_dir().join("portrait-rotated.png");
    if std::env::var_os("FOSSCUT_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(goldens_dir()).unwrap();
        write_png(&golden, &image);
        return;
    }
    let psnr = image.psnr(&read_png(&golden)).unwrap();
    assert!(psnr >= MIN_PSNR_DB, "portrait-rotated: PSNR {psnr:.1} dB");
}
