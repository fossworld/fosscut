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

use fosscut_model::{
    Align, Animated, Animator, BlendMode, CaptionBox, CaptionContent, CaptionLine, CaptionStyle,
    CaptionWord, Color, FontWeight, PageFade, Placement, TextContent, TextStyle, Transform,
    WordReveal,
};
use fosscut_render::caption::CaptionLayer;
use fosscut_render::color::TransferFunction;
use fosscut_render::effects::PlannedEffect;
use fosscut_render::lut::Lut3d;
use fosscut_render::plan::{Layer, Mask, MaskShape, RenderPlan};
use fosscut_render::{Image, Renderer};
use fosscut_time::Flicks;
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
    check_at(name, renderer, plan, frames, SIZE);
}

fn check_at(
    name: &str,
    renderer: &mut Renderer,
    plan: &RenderPlan,
    frames: &mut Frames,
    size: (u32, u32),
) {
    let image = renderer
        .render_image(plan, frames, size, TransferFunction::Bt709)
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
            ..Transform::default()
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

/// Fourteen tiles of one warm color over the test pattern, one per blend mode in
/// `BlendMode` order, left to right and top to bottom; the last tile is flipped, cropped
/// and stretched to show those placements too.
#[test]
fn blend_modes_and_placements() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::LinearBurn,
        BlendMode::Add,
        BlendMode::Difference,
        BlendMode::Exclusion,
    ];
    let mut layers = vec![Layer::new(source(1))];
    for (index, mode) in modes.into_iter().enumerate() {
        let (column, row) = ((index % 7) as f64, (index / 7) as f64);
        layers.push(Layer {
            blend: mode,
            transform: Transform {
                position: [-0.42 + column * 0.14, -0.2 + row * 0.4],
                scale: 0.12,
                ..Transform::default()
            },
            ..Layer::new(source(2))
        });
    }
    layers.push(Layer {
        transform: Transform {
            position: [0.0, 0.42],
            scale: 0.3,
            stretch: [2.0, 0.5],
            rotation: -6.0,
            flip_horizontal: true,
            ..Transform::default()
        },
        crop: [0.0, 0.0, 0.75, 1.0],
        ..Layer::new(source(1))
    });
    let plan = RenderPlan {
        canvas: (1920, 1080),
        layers,
        skipped: Vec::new(),
    };
    let mut frames = Frames::default()
        .with(1, pattern(320, 180, 0))
        .with(2, solid(160, 90, [150, 70, 190]));
    check("blend-modes", &mut renderer, &plan, &mut frames);
}

fn planned(kind: &str, values: &[(&str, fosscut_model::ParamValue)]) -> PlannedEffect {
    let spec = fosscut_model::effect_type(kind).unwrap();
    let mut all: std::collections::BTreeMap<String, fosscut_model::ParamValue> = spec
        .params
        .iter()
        .map(|p| (p.name.to_string(), p.default.value()))
        .collect();
    for (name, value) in values {
        all.insert(name.to_string(), value.clone());
    }
    PlannedEffect {
        effect_type: kind.into(),
        values: all,
        seed: 3,
    }
}

/// The test pattern in a 4x2 grid, each tile with one effect: adjust (warm, punchy),
/// curves (an S curve), HSL (reds to orange, blues muted), wheels (teal shadows, warm
/// highlights), vignette, blur, sharpen and grain. An adjustment layer on top lowers the
/// saturation of everything below it a little.
#[test]
fn effects_on_tiles() {
    use fosscut_model::ParamValue::{Number, Pair, Points};
    let Some(mut renderer) = renderer() else {
        return;
    };
    let tiles = vec![
        planned(
            "color.adjust",
            &[
                ("temperature", Number(0.6)),
                ("contrast", Number(0.4)),
                ("saturation", Number(0.3)),
            ],
        ),
        planned(
            "color.curves",
            &[(
                "master",
                Points(vec![[0.0, 0.0], [0.25, 0.15], [0.75, 0.85], [1.0, 1.0]]),
            )],
        ),
        planned(
            "color.hsl",
            &[
                ("red_hue", Number(1.0)),
                ("blue_saturation", Number(-0.8)),
                ("aqua_lightness", Number(0.5)),
            ],
        ),
        planned(
            "color.wheels",
            &[("lift", Pair([-0.4, -0.1])), ("gain", Pair([0.3, 0.17]))],
        ),
        planned(
            "color.vignette",
            &[("amount", Number(0.9)), ("midpoint", Number(0.3))],
        ),
        planned("blur.gaussian", &[("radius", Number(0.04))]),
        planned(
            "detail.sharpen",
            &[("amount", Number(2.0)), ("radius", Number(0.01))],
        ),
        planned(
            "stylize.grain",
            &[("amount", Number(0.7)), ("size", Number(2.0))],
        ),
    ];
    let mut layers = Vec::new();
    for (index, effect) in tiles.into_iter().enumerate() {
        let (column, row) = ((index % 4) as f64, (index / 4) as f64);
        layers.push(Layer {
            transform: Transform {
                position: [-0.375 + column * 0.25, -0.25 + row * 0.5],
                scale: 0.24,
                ..Transform::default()
            },
            effects: vec![effect],
            ..Layer::new(source(1))
        });
    }
    layers.push(Layer {
        source: fosscut_render::plan::LayerSource::Adjustment,
        effects: vec![planned(
            "color.adjust",
            &[("saturation", fosscut_model::ParamValue::Number(-0.3))],
        )],
        ..Layer::new(source(1))
    });
    let plan = RenderPlan {
        canvas: (1920, 1080),
        layers,
        skipped: Vec::new(),
    };
    let mut frames = Frames::default().with(1, pattern(320, 180, 0));
    check_at(
        "effects-tiles",
        &mut renderer,
        &plan,
        &mut frames,
        (640, 360),
    );
}

/// The test pattern in a 5x2 grid, each tile with one mask: a rounded rectangle, a
/// feathered ellipse, a rotated linear mask, a mirror band, a star, a heart, a curved pen
/// path, text, brush strokes with an erased dot, and an ellipse with a star subtracted.
#[test]
fn mask_shapes() {
    use fosscut_model::{BrushStroke, Id, IdKind, Mask, MaskMode, MaskShape, PathPoint};
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mask = |shape| Mask {
        size: [0.8, 0.8],
        ..Mask::new(Id::new(IdKind::Mask, 1), shape)
    };
    let point = |x: f64, y: f64| PathPoint {
        at: [x, y],
        in_handle: None,
        out_handle: None,
    };
    let tiles: Vec<Vec<Mask>> = vec![
        vec![Mask {
            roundness: 0.6,
            ..mask(MaskShape::Rectangle)
        }],
        vec![Mask {
            feather: 0.8,
            ..mask(MaskShape::Ellipse)
        }],
        vec![Mask {
            rotation: 30.0,
            feather: 0.3,
            ..mask(MaskShape::Linear)
        }],
        vec![Mask {
            size: [1.0, 0.35],
            ..mask(MaskShape::Mirror)
        }],
        vec![mask(MaskShape::Star)],
        vec![mask(MaskShape::Heart)],
        vec![Mask {
            points: vec![
                point(-0.9, -0.6),
                PathPoint {
                    out_handle: Some([0.6, 0.8]),
                    ..point(0.9, -0.6)
                },
                PathPoint {
                    in_handle: Some([0.4, 0.0]),
                    ..point(0.0, 0.9)
                },
            ],
            ..mask(MaskShape::Path)
        }],
        vec![Mask {
            text: Some("FOSS".into()),
            size: [0.95, 0.6],
            ..mask(MaskShape::Text)
        }],
        vec![Mask {
            size: [1.0, 1.0],
            strokes: vec![
                BrushStroke {
                    points: vec![[0.1, 0.3], [0.5, 0.7], [0.9, 0.3]],
                    radius: 0.12,
                    erase: false,
                },
                BrushStroke {
                    points: vec![[0.5, 0.7]],
                    radius: 0.08,
                    erase: true,
                },
            ],
            ..mask(MaskShape::Brush)
        }],
        vec![
            mask(MaskShape::Ellipse),
            Mask {
                size: [0.5, 0.5],
                mode: MaskMode::Subtract,
                ..mask(MaskShape::Star)
            },
        ],
    ];
    let mut layers = Vec::new();
    for (index, masks) in tiles.into_iter().enumerate() {
        let (column, row) = ((index % 5) as f64, (index / 5) as f64);
        layers.push(Layer {
            transform: Transform {
                position: [-0.4 + column * 0.2, -0.25 + row * 0.5],
                scale: 0.19,
                ..Transform::default()
            },
            masks,
            ..Layer::new(source(1))
        });
    }
    let plan = RenderPlan {
        canvas: (1920, 1080),
        layers,
        skipped: Vec::new(),
    };
    let mut frames = Frames::default().with(1, pattern(320, 180, 0));
    check_at("mask-shapes", &mut renderer, &plan, &mut frames, (640, 360));
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

/// Text goldens render at 640x360: at 320x180 a caption is only a few pixels tall.
const TEXT_SIZE: (u32, u32) = (640, 360);

fn text(text: &str, edit: impl FnOnce(&mut TextStyle)) -> TextContent {
    let mut style = TextStyle::default();
    edit(&mut style);
    TextContent {
        text: text.into(),
        style,
    }
}

#[test]
fn title_over_video() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let title = Layer {
        transform: Transform {
            position: [0.0, 0.3],
            ..Transform::default()
        },
        ..Layer::text(text("FossCut audit", |s| {
            s.size = 0.08;
            s.bold = true;
        }))
    };
    let plan = RenderPlan {
        canvas: (1920, 1080),
        layers: vec![Layer::new(source(1)), title],
        skipped: Vec::new(),
    };
    let mut frames = Frames::default().with(1, pattern(640, 360, 0));
    check_at(
        "title-over-video",
        &mut renderer,
        &plan,
        &mut frames,
        TEXT_SIZE,
    );
}

#[test]
fn text_styles() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let quote = Layer {
        transform: Transform {
            position: [-0.18, -0.22],
            ..Transform::default()
        },
        ..Layer::text(text("Left aligned,\nitalic and red", |s| {
            s.size = 0.07;
            s.italic = true;
            s.align = Align::Left;
            s.color = Color::parse("#E5484D").unwrap();
        }))
    };
    let tag = Layer {
        transform: Transform {
            position: [0.2, 0.24],
            scale: 1.0,
            rotation: -5.0,
            ..Transform::default()
        },
        opacity: 0.8,
        ..Layer::text(text("Bold, right,\nturned, 80%", |s| {
            s.size = 0.08;
            s.bold = true;
            s.align = Align::Right;
            s.color = Color::parse("#3AA0DC").unwrap();
        }))
    };
    let plan = RenderPlan {
        canvas: (1920, 1080),
        layers: vec![Layer::new(source(1)), quote, tag],
        skipped: Vec::new(),
    };
    let mut frames = Frames::default().with(1, solid(64, 36, [40, 128, 128]));
    check_at("text-styles", &mut renderer, &plan, &mut frames, TEXT_SIZE);
}

/// A caption page of timed words, from (text, start ms, end ms, emphasis) per line.
fn page(lines: &[&[(&str, i64, i64, bool)]]) -> CaptionContent {
    CaptionContent {
        lines: lines
            .iter()
            .map(|words| CaptionLine {
                words: words
                    .iter()
                    .map(|&(text, start, end, emphasis)| CaptionWord {
                        text: text.into(),
                        start: Flicks::from_millis(start),
                        end: Flicks::from_millis(end),
                        emphasis,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn preset(name: &str) -> CaptionStyle {
    fosscut_core::caption_preset(name).unwrap()
}

/// A caption page over the test pattern on `canvas`, at `time_ms` into the page.
fn caption_scene(
    canvas: (u32, u32),
    style: CaptionStyle,
    content: CaptionContent,
    time_ms: i64,
) -> RenderPlan {
    RenderPlan {
        canvas,
        layers: vec![
            Layer::new(source(1)),
            Layer::caption(CaptionLayer {
                style,
                content,
                time: Flicks::from_millis(time_ms),
                duration: Flicks::from_millis(2000),
            }),
        ],
        skipped: Vec::new(),
    }
}

/// Vertical goldens render at 360x640, the text goldens' size turned on its side.
const TALL_SIZE: (u32, u32) = (360, 640);

#[test]
fn caption_presets_and_placements() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default().with(1, pattern(640, 360, 0));
    // Tall canvases get a tall picture, so captions sit over video, not letterboxing.
    let mut tall = Frames::default().with(1, pattern(360, 640, 0));
    let speech: &[&[(&str, i64, i64, bool)]] = &[
        &[("This", 0, 250, false), ("changes", 250, 700, true)],
        &[("everything", 750, 1300, false)],
    ];

    // Highlight at the bottom of a tall canvas: "changes" is spoken and emphasised.
    let plan = caption_scene((1080, 1920), preset("highlight"), page(speech), 450);
    check_at(
        "caption-highlight-tall",
        &mut renderer,
        &plan,
        &mut tall,
        TALL_SIZE,
    );

    // Karaoke on a wide canvas: "everything" half filled.
    let plan = caption_scene((1920, 1080), preset("karaoke"), page(speech), 1025);
    check_at(
        "caption-karaoke-wide",
        &mut renderer,
        &plan,
        &mut frames,
        TEXT_SIZE,
    );

    // Boxed at the top of a tall canvas, with the second line spoken.
    let mut boxed = preset("boxed");
    boxed.placement = Placement::Top;
    let plan = caption_scene((1080, 1920), boxed, page(speech), 900);
    check_at(
        "caption-boxed-top",
        &mut renderer,
        &plan,
        &mut tall,
        TALL_SIZE,
    );

    // Bold pop in the middle of a wide canvas, 60 ms in: the page and "This" are popping.
    let mut pop = preset("bold-pop");
    pop.placement = Placement::Middle;
    let plan = caption_scene((1920, 1080), pop, page(speech), 60);
    check_at(
        "caption-bold-pop-middle",
        &mut renderer,
        &plan,
        &mut frames,
        TEXT_SIZE,
    );

    // Words revealed one by one while the page fades in, black weight with a box per line
    // and a hand-set colour.
    let reveal = CaptionStyle {
        weight: FontWeight::Black,
        color: Color::parse("#7CFC9A").unwrap(),
        align: Align::Left,
        background: CaptionBox {
            enabled: true,
            ..CaptionBox::default()
        },
        animators: vec![
            Animator::PageFade(Animated {
                params: PageFade {
                    in_ms: 400,
                    out_ms: 0,
                },
                ..Animated::default()
            }),
            Animator::WordReveal(Animated {
                params: WordReveal { fade_ms: 200 },
                ..Animated::default()
            }),
        ],
        ..CaptionStyle::default()
    };
    let plan = caption_scene((1080, 1920), reveal, page(speech), 300);
    check_at(
        "caption-reveal-fade",
        &mut renderer,
        &plan,
        &mut tall,
        TALL_SIZE,
    );

    // A caption moved and turned by its clip's transform keeps its look.
    let mut plan = caption_scene((1920, 1080), preset("highlight"), page(speech), 450);
    plan.layers[1].transform = Transform {
        position: [0.0, -0.3],
        scale: 1.2,
        rotation: -4.0,
        ..Transform::default()
    };
    check_at(
        "caption-transformed",
        &mut renderer,
        &plan,
        &mut frames,
        TEXT_SIZE,
    );
}
