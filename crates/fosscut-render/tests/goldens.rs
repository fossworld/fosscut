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
    CaptionWord, Color, FontWeight, Glow, PageFade, Placement, Shadow, Stroke, TextBox,
    TextContent, TextSpan, TextStyle, Transform, WordReveal,
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
        time: 0.5,
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

/// Every transition 40% of the way from the test pattern to a flat purple, one 128x72
/// render each, stitched left to right and top to bottom in `transition_types()` order.
#[test]
fn transitions_half_way() {
    use fosscut_render::plan::{LayerSource, TransitionLayer};
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default()
        .with(1, pattern(320, 180, 0))
        .with(2, solid(160, 90, [90, 160, 170]));
    let (tile_w, tile_h) = (128u32, 72u32);
    let kinds: Vec<&str> = fosscut_model::transition_types()
        .iter()
        .map(|t| t.name)
        .collect();
    let columns = 5u32;
    let rows = (kinds.len() as u32).div_ceil(columns);
    let mut sheet = Image {
        width: tile_w * columns,
        height: tile_h * rows,
        rgba: vec![0; (tile_w * columns * tile_h * rows * 4) as usize],
    };
    for (index, kind) in kinds.iter().enumerate() {
        let plan = RenderPlan {
            canvas: (1920, 1080),
            layers: vec![Layer {
                source: LayerSource::Transition(Box::new(TransitionLayer {
                    outgoing: Layer::new(source(1)),
                    incoming: Layer::new(source(2)),
                    kind: kind.to_string(),
                    params: Default::default(),
                    progress: 0.4,
                })),
                ..Layer::new(source(1))
            }],
            skipped: Vec::new(),
        };
        let tile = renderer
            .render_image(
                &plan,
                &mut frames,
                (tile_w, tile_h),
                TransferFunction::Bt709,
            )
            .unwrap();
        let (x0, y0) = (
            (index as u32 % columns) * tile_w,
            (index as u32 / columns) * tile_h,
        );
        for y in 0..tile_h {
            let from = (y * tile_w * 4) as usize;
            let to = (((y0 + y) * sheet.width + x0) * 4) as usize;
            sheet.rgba[to..to + (tile_w * 4) as usize]
                .copy_from_slice(&tile.rgba[from..from + (tile_w * 4) as usize]);
        }
    }
    let golden = goldens_dir().join("transitions.png");
    if std::env::var_os("FOSSCUT_UPDATE_GOLDENS").is_some() {
        write_png(&golden, &sheet);
        return;
    }
    let psnr = sheet.psnr(&read_png(&golden)).unwrap();
    assert!(psnr >= MIN_PSNR_DB, "transitions: PSNR {psnr:.1} dB");
}

/// The test pattern in a 4x3 grid, each tile with one stylizing or moving effect: glitch,
/// RGB split, pixelate, VHS, mirror, posterize, invert, shake, zoom blur, flash, rainbow
/// and zoom pulse.
#[test]
fn stylize_and_motion_effects() {
    use fosscut_model::ParamValue::Number;
    let Some(mut renderer) = renderer() else {
        return;
    };
    let tiles = vec![
        planned("stylize.glitch", &[("amount", Number(0.9))]),
        planned("stylize.rgb_split", &[("amount", Number(0.03))]),
        planned("stylize.pixelate", &[("size", Number(0.06))]),
        planned("stylize.vhs", &[("amount", Number(1.0))]),
        planned("stylize.mirror", &[]),
        planned("stylize.posterize", &[("levels", Number(3.0))]),
        planned("stylize.invert", &[]),
        planned("motion.shake", &[("amount", Number(0.08))]),
        planned("motion.zoom_blur", &[("amount", Number(0.8))]),
        PlannedEffect {
            time: 0.0,
            ..planned("motion.flash", &[("amount", Number(0.5))])
        },
        planned("motion.rainbow", &[]),
        planned("motion.zoom_pulse", &[("amount", Number(0.4))]),
    ];
    let mut layers = Vec::new();
    for (index, effect) in tiles.into_iter().enumerate() {
        let (column, row) = ((index % 4) as f64, (index / 4) as f64);
        layers.push(Layer {
            transform: Transform {
                position: [-0.375 + column * 0.25, -0.333 + row * 0.333],
                scale: 0.24,
                ..Transform::default()
            },
            effects: vec![effect],
            ..Layer::new(source(1))
        });
    }
    let plan = RenderPlan {
        canvas: (1920, 1080),
        layers,
        skipped: Vec::new(),
    };
    let mut frames = Frames::default().with(1, pattern(320, 180, 0));
    check_at(
        "stylize-effects",
        &mut renderer,
        &plan,
        &mut frames,
        (640, 360),
    );
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
        ..Default::default()
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

/// Outlines, glows, shadows, boxes, spans and underlines (ADR-0027).
#[test]
fn styled_text() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let at = |position: [f64; 2], content: TextContent| Layer {
        transform: Transform {
            position,
            ..Transform::default()
        },
        ..Layer::text(content)
    };
    let outline = text("Outline", |s| {
        s.size = 0.1;
        s.bold = true;
        s.stroke = Stroke {
            enabled: true,
            width: 0.12,
            ..Stroke::default()
        };
    });
    let neon = text("Neon glow", |s| {
        s.size = 0.09;
        s.bold = true;
        s.color = Color::parse("#FFE6FB").unwrap();
        s.glow = Glow {
            enabled: true,
            color: Color::parse("#FF2FD6").unwrap(),
            radius: 0.4,
        };
    });
    let retro = text("RETRO", |s| {
        s.size = 0.11;
        s.bold = true;
        s.letter_spacing = 0.08;
        s.color = Color::parse("#FFD23F").unwrap();
        s.shadow = Shadow {
            enabled: true,
            color: Color::parse("#E4572E").unwrap(),
            offset: [0.08, 0.08],
            blur: 0.0,
        };
    });
    let mut boxed = text("Boxed and\nunderlined", |s| {
        s.size = 0.07;
        s.align = Align::Left;
        s.background = TextBox {
            enabled: true,
            ..TextBox::default()
        };
        s.shadow = Shadow {
            enabled: true,
            ..Shadow::default()
        };
    });
    boxed.spans = vec![
        TextSpan {
            start: 0,
            end: 5,
            color: Color::parse("#FFE14D").ok(),
            bold: Some(true),
            italic: None,
            underline: None,
            font: None,
            size: None,
        },
        TextSpan {
            start: 10,
            end: 20,
            color: None,
            bold: None,
            italic: Some(true),
            underline: Some(true),
            font: None,
            size: None,
        },
    ];
    let plan = RenderPlan {
        canvas: (1920, 1080),
        layers: vec![
            Layer::new(source(1)),
            at([-0.24, -0.25], outline),
            at([0.24, -0.25], neon),
            at([-0.24, 0.22], retro),
            at([0.24, 0.22], boxed),
        ],
        skipped: Vec::new(),
    };
    let mut frames = Frames::default().with(1, solid(64, 36, [30, 70, 90]));
    check_at("styled-text", &mut renderer, &plan, &mut frames, TEXT_SIZE);
}

/// Stickers from the library on a gradient background (ADR-0028).
#[test]
fn stickers() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let item = |name: &str, position: [f64; 2]| {
        let visual = fosscut_core::visual(name).unwrap();
        Layer {
            transform: Transform {
                position,
                ..Transform::default()
            },
            ..Layer::shape(visual.content)
        }
    };
    let plan = RenderPlan {
        canvas: (1920, 1080),
        layers: vec![
            item("ocean", [0.0, 0.0]),
            item("badge-new", [-0.3, -0.3]),
            item("subscribe", [0.25, -0.3]),
            item("speech-bubble", [-0.28, 0.12]),
            item("gold-star", [0.05, 0.1]),
            item("wow", [0.3, 0.15]),
            item("pointer", [-0.1, 0.35]),
            item("check", [0.15, 0.38]),
        ],
        skipped: Vec::new(),
    };
    let mut frames = Frames::default();
    check_at("stickers", &mut renderer, &plan, &mut frames, TEXT_SIZE);
}

/// A bilingual caption page: the translation under the words.
#[test]
fn bilingual_caption() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut content = page(&[&[
        ("Every", 0, 400, false),
        ("word", 400, 800, true),
        ("counts.", 800, 1300, false),
    ]]);
    content.translation = Some("Cada palabra cuenta.".into());
    let plan = caption_scene((1920, 1080), preset("highlight"), content, 600);
    let mut frames = Frames::default().with(1, pattern(640, 360, 0));
    check_at(
        "bilingual-caption",
        &mut renderer,
        &plan,
        &mut frames,
        TEXT_SIZE,
    );
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
        ..Default::default()
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

/// Smooth slow motion: a moving test pattern half way between two of its frames, so both
/// positions show at half strength.
#[test]
fn blended_slow_motion() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default();
    frames.moving.insert(asset(1), (320, 180));
    let frame = |n: i64| fosscut_render::plan::SourceRef {
        asset: asset(1),
        time: fosscut_time::Rate::FPS_30.frames_to_flicks(n),
    };
    let plan = RenderPlan {
        canvas: (1920, 1080),
        layers: vec![Layer {
            next_frame: Some((frame(4), 0.5)),
            ..Layer::new(frame(1))
        }],
        skipped: Vec::new(),
    };
    check("blended-slow-motion", &mut renderer, &plan, &mut frames);
}

/// A 16:9 picture on a vertical canvas with a canvas fill, through the planner: its own
/// picture blurred to cover the canvas, then a colour.
#[test]
fn canvas_fills() {
    use fosscut_model::{
        Asset, AssetKind, CanvasFill, Clip, ClipContent, Id, IdKind, MediaInfo, PathHints, Project,
        TimelineFormat, Track, TrackKind, VideoInfo,
    };
    use fosscut_render::plan::{LayerSource, plan_frame};
    let Some(mut renderer) = renderer() else {
        return;
    };
    let format = TimelineFormat {
        width: 1080,
        height: 1920,
        rate: fosscut_time::Rate::FPS_30,
        sample_rate: 48_000,
    };
    let mut project = Project::new(Id::new(IdKind::Project, 1), "Fill", format);
    project.media.assets.push(Asset {
        folder: None,
        id: asset(1),
        kind: AssetKind::Video,
        name: "wide.mp4".into(),
        path: PathHints::default(),
        identity: None,
        info: MediaInfo {
            duration: Some(Flicks::from_seconds(2).unwrap()),
            video: Some(VideoInfo {
                width: 640,
                height: 360,
                rate: Some(fosscut_time::Rate::FPS_30),
                codec: None,
                rotation: 0,
            }),
            ..MediaInfo::default()
        },
    });
    let second = Flicks::from_seconds(1).unwrap();
    let clip = |n: u64, start: Flicks, fill: CanvasFill| Clip {
        canvas: Some(fill),
        ..Clip::new(
            Id::new(IdKind::Clip, n),
            start,
            second,
            ClipContent::Video {
                asset: asset(1),
                source_in: Flicks::ZERO,
            },
        )
    };
    let timeline = project.manifest.primary_timeline.clone();
    project
        .timelines
        .get_mut(&timeline)
        .unwrap()
        .tracks
        .push(Track {
            id: Id::new(IdKind::Track, 2),
            kind: TrackKind::Video,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: vec![
                clip(3, Flicks::ZERO, CanvasFill::Blur { amount: 0.03 }),
                clip(
                    4,
                    second,
                    CanvasFill::Color {
                        color: Color::parse("#2E6B5A").unwrap(),
                    },
                ),
            ],
        });
    let blurred = plan_frame(&project, &timeline, Flicks::ZERO).unwrap();
    assert_eq!(blurred.layers.len(), 2, "the fill, then the clip");
    let fill = &blurred.layers[0];
    assert!(matches!(fill.source, LayerSource::Frame(_)));
    // 640x360 fits a 1080x1920 canvas at 1080x607.5; covering needs 1920 / 607.5.
    assert!(
        (fill.transform.scale - 1920.0 / 607.5).abs() < 1e-9,
        "{}",
        fill.transform.scale
    );
    assert_eq!(fill.effects[0].effect_type, "blur.gaussian");
    assert_eq!(
        blurred.layers[1].transform.scale, 1.0,
        "the clip itself is unchanged"
    );
    let colored = plan_frame(&project, &timeline, second).unwrap();
    assert!(matches!(colored.layers[0].source, LayerSource::Shape(_)));

    let mut frames = Frames::default().with(1, pattern(640, 360, 0));
    check_at(
        "canvas-blur",
        &mut renderer,
        &blurred,
        &mut frames,
        (180, 320),
    );
    check_at(
        "canvas-color",
        &mut renderer,
        &colored,
        &mut frames,
        (180, 320),
    );
}
