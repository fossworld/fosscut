//! Text layers on the GPU checked against values computed by hand: placement, color in
//! the linear pipeline, opacity, stacking over video, LUTs, and the raster cache
//! (ADR-0011, item 5).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use fosscut_model::{Align, Color, TextContent, TextStyle, Transform};
use fosscut_render::color::TransferFunction;
use fosscut_render::lut::Lut3d;
use fosscut_render::plan::{Layer, RenderPlan};
use fosscut_render::{Image, Renderer};
use support::*;

const SIZE: (u32, u32) = (640, 360);

fn title(text: &str, edit: impl FnOnce(&mut TextStyle)) -> TextContent {
    let mut style = TextStyle {
        size: 0.3,
        bold: true,
        ..TextStyle::default()
    };
    edit(&mut style);
    TextContent {
        text: text.into(),
        style,
    }
}

fn render(renderer: &mut Renderer, layers: Vec<Layer>, frames: &mut Frames) -> Image {
    let plan = RenderPlan {
        canvas: SIZE,
        layers,
        skipped: Vec::new(),
    };
    renderer
        .render_image(&plan, frames, SIZE, TransferFunction::Bt709)
        .unwrap()
}

/// Bounding box of pixels whose red channel exceeds `threshold`: (left, top, right, bottom).
fn ink_box(image: &Image, threshold: u8) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in 0..image.height {
        for x in 0..image.width {
            if image.pixel(x, y)[0] > threshold {
                found = Some(match found {
                    None => (x, y, x, y),
                    Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(x), b.max(y)),
                });
            }
        }
    }
    found
}

fn brightest(image: &Image) -> [u8; 4] {
    let mut best = [0u8; 4];
    for y in 0..image.height {
        for x in 0..image.width {
            let p = image.pixel(x, y);
            if u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])
                > u32::from(best[0]) + u32::from(best[1]) + u32::from(best[2])
            {
                best = p;
            }
        }
    }
    best
}

fn near(pixel: [u8; 4], want: [u8; 3], tolerance: u8) -> bool {
    (0..3).all(|c| pixel[c].abs_diff(want[c]) <= tolerance)
}

fn code(linear: f32) -> u8 {
    (TransferFunction::Bt709.encode(linear).clamp(0.0, 1.0) * 255.0).round() as u8
}

#[test]
fn titles_are_centred_and_follow_their_transform() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default();
    let centred = render(
        &mut renderer,
        vec![Layer::text(title("HI", |_| {}))],
        &mut frames,
    );
    let (l, t, r, b) = ink_box(&centred, 128).expect("the title is drawn");
    let (cx, cy) = ((l + r) as f64 / 2.0, (t + b) as f64 / 2.0);
    assert!((cx - 320.0).abs() < 4.0, "horizontally centred: {cx}");
    // Cap height sits a little above the line box centre; allow for it.
    assert!(
        (cy - 180.0).abs() < 25.0,
        "vertically near the centre: {cy}"
    );
    assert!(b - t > 60, "a 0.3 x 360 px font is tall: {}", b - t);

    let lower = Layer {
        transform: Transform {
            position: [0.25, 0.3],
            scale: 0.5,
            rotation: 0.0,
            ..Transform::default()
        },
        ..Layer::text(title("HI", |_| {}))
    };
    let moved = render(&mut renderer, vec![lower], &mut frames);
    let (l2, t2, r2, b2) = ink_box(&moved, 128).unwrap();
    let (cx2, cy2) = ((l2 + r2) as f64 / 2.0, (t2 + b2) as f64 / 2.0);
    assert!(
        (cx2 - (320.0 + 0.25 * 640.0)).abs() < 4.0,
        "moved right: {cx2}"
    );
    assert!(
        (cy2 - (180.0 + 0.3 * 360.0)).abs() < 15.0,
        "moved down: {cy2}"
    );
    let ratio = f64::from(b2 - t2) / f64::from(b - t);
    assert!((ratio - 0.5).abs() < 0.08, "half size: {ratio}");
}

#[test]
fn colors_and_opacity_are_exact_in_linear_light() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default();
    let colored = |hex: &str| {
        let color = Color::parse(hex).unwrap();
        Layer::text(title("I", |s| s.color = color))
    };
    assert_eq!(
        brightest(&render(
            &mut renderer,
            vec![colored("#FFFFFF")],
            &mut frames
        )),
        [255, 255, 255, 255]
    );
    assert_eq!(
        brightest(&render(
            &mut renderer,
            vec![colored("#FF0000")],
            &mut frames
        )),
        [255, 0, 0, 255]
    );
    // sRGB #808080 is 0.2159 linear, which the BT.709 output encodes as code 123.
    let gray = brightest(&render(
        &mut renderer,
        vec![colored("#808080")],
        &mut frames,
    ));
    let want = code(TransferFunction::Srgb.decode(128.0 / 255.0));
    assert!(
        gray[0].abs_diff(want) <= 1 && gray[0] == gray[2],
        "{gray:?} vs {want}"
    );

    let half = Layer {
        opacity: 0.5,
        ..colored("#FFFFFF")
    };
    let faded = brightest(&render(&mut renderer, vec![half], &mut frames));
    assert!(
        faded[0].abs_diff(code(0.5)) <= 1,
        "{faded:?} vs {}",
        code(0.5)
    );
    let translucent = brightest(&render(
        &mut renderer,
        vec![colored("#FFFFFF80")],
        &mut frames,
    ));
    assert!(
        translucent[0].abs_diff(code(128.0 / 255.0)) <= 1,
        "{translucent:?}"
    );
}

#[test]
fn text_stacks_over_video_and_takes_luts() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default().with(1, solid(64, 36, [51, 109, 212]));
    let over = render(
        &mut renderer,
        vec![Layer::new(source(1)), Layer::text(title("I", |_| {}))],
        &mut frames,
    );
    assert!(
        near(over.pixel(10, 10), [191, 0, 0], 2),
        "video around the text: {:?}",
        over.pixel(10, 10)
    );
    assert_eq!(brightest(&over)[..3], [255, 255, 255], "white text on top");

    let invert = Lut3d::parse_cube(
        "LUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n",
    )
    .unwrap();
    let graded = Layer {
        lut: Some(Arc::new(invert)),
        ..Layer::text(title("I", |_| {}))
    };
    let dark = render(
        &mut renderer,
        vec![Layer::new(source(1)), graded],
        &mut frames,
    );
    let (l, t, r, b) = ink_box(&over, 230).unwrap();
    let centre = dark.pixel((l + r) / 2, (t + b) / 2);
    assert!(
        centre[..3].iter().all(|&c| c < 3),
        "inverted white text is black: {centre:?}"
    );
    assert!(
        near(dark.pixel(10, 10), [191, 0, 0], 2),
        "the LUT only touches the text layer"
    );
}

#[test]
fn empty_and_repeated_text_render_consistently() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let mut frames = Frames::default();
    let blank = render(
        &mut renderer,
        vec![Layer::text(title("   ", |_| {}))],
        &mut frames,
    );
    assert!(
        blank.rgba.chunks(4).all(|p| p == [0, 0, 0, 255]),
        "blank text draws nothing"
    );

    let layers = || {
        vec![
            Layer::text(title("Left\naligned", |s| {
                s.align = Align::Left;
                s.size = 0.1;
                s.italic = true;
            })),
            Layer::text(title("Same", |s| s.size = 0.12)),
        ]
    };
    let first = render(&mut renderer, layers(), &mut frames);
    // Fill and overflow the raster cache, then render the same text again.
    for n in 0..70 {
        render(
            &mut renderer,
            vec![Layer::text(title(&format!("{n}"), |s| s.size = 0.05))],
            &mut frames,
        );
    }
    let again = render(&mut renderer, layers(), &mut frames);
    assert_eq!(
        first, again,
        "rendering is repeatable through cache evictions"
    );
}
