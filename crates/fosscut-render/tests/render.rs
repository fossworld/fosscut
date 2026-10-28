//! GPU rendering checked against values computed by hand: color conversion, compositing,
//! masks, LUTs, rotation and filtering (ADR-0008, ADR-0016).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use fosscut_media::{ColorInfo, Range, Transfer, VideoDecoder, VideoFrame};
use fosscut_model::Transform;
use fosscut_render::color::{TransferFunction, reference_yuv_to_linear};
use fosscut_render::lut::Lut3d;
use fosscut_render::plan::{Layer, Mask, MaskShape, RenderPlan};
use fosscut_render::{DecoderFrames, FrameProvider, Image};
use support::*;

const SIZE: (u32, u32) = (64, 36);

fn render(layers: Vec<Layer>, frames: &mut Frames) -> Option<Image> {
    let mut renderer = renderer()?;
    let plan = RenderPlan {
        canvas: SIZE,
        layers,
        skipped: Vec::new(),
    };
    Some(
        renderer
            .render_image(&plan, frames, SIZE, TransferFunction::Bt709)
            .unwrap(),
    )
}

fn code(signal: f32) -> u8 {
    (signal.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// What a flat Y'CbCr color should come out as, computed on the CPU.
fn expected(color: &ColorInfo, yuv: [u8; 3]) -> [u8; 3] {
    reference_yuv_to_linear(color, yuv[0], yuv[1], yuv[2])
        .unwrap()
        .map(|l| code(TransferFunction::Bt709.encode(l)))
}

fn assert_near(actual: [u8; 4], expected: [u8; 3], tolerance: u8, what: &str) {
    for c in 0..3 {
        assert!(
            actual[c].abs_diff(expected[c]) <= tolerance,
            "{what}: got {actual:?}, expected {expected:?}"
        );
    }
    assert_eq!(actual[3], 255, "{what}: output is opaque");
}

#[test]
fn color_bars_convert_with_the_stream_matrix_and_range() {
    let bars = [
        ([51, 109, 212], [191, 0, 0]),
        ([133, 63, 52], [0, 191, 0]),
        ([28, 212, 120], [0, 0, 191]),
        ([168, 44, 136], [191, 191, 0]),
        ([235, 128, 128], [255, 255, 255]),
        ([16, 128, 128], [0, 0, 0]),
    ];
    for (yuv, rgb) in bars {
        let mut frames = Frames::default().with(1, solid(32, 32, yuv));
        let Some(image) = render(vec![Layer::new(source(1))], &mut frames) else {
            return;
        };
        assert_near(image.pixel(32, 18), rgb, 2, &format!("bar {yuv:?}"));
        assert_near(
            image.pixel(32, 18),
            expected(&bt709_limited(), yuv),
            1,
            "CPU reference",
        );
    }
}

#[test]
fn planar_full_range_and_srgb_sources_are_handled() {
    let full = ColorInfo {
        range: Range::Full,
        ..bt709_limited()
    };
    let mut frames = Frames::default()
        .with(1, yuv420p(32, 32, full, |_, _| [255, 128, 128]))
        .with(2, yuv420p(32, 32, bt709_limited(), |_, _| [126, 128, 128]));
    // Square sources are pillarboxed on the 64x36 canvas: sample the middle.
    let Some(image) = render(vec![Layer::new(source(1))], &mut frames) else {
        return;
    };
    assert_near(image.pixel(32, 18), [255, 255, 255], 1, "full-range white");
    let image = render(vec![Layer::new(source(2))], &mut frames).unwrap();
    assert_near(
        image.pixel(32, 18),
        expected(&bt709_limited(), [126, 128, 128]),
        1,
        "planar gray",
    );

    // An sRGB-tagged source is decoded with the sRGB curve, so mid gray comes out darker
    // in the BT.709 output.
    let srgb = ColorInfo {
        range: Range::Full,
        transfer: Transfer::Srgb,
        ..bt709_limited()
    };
    let mut frames = Frames::default().with(3, nv12(32, 32, srgb, |_, _| [128, 128, 128]));
    let image = render(vec![Layer::new(source(3))], &mut frames).unwrap();
    assert_near(
        image.pixel(32, 18),
        expected(&srgb, [128, 128, 128]),
        1,
        "sRGB gray",
    );
    assert!(image.pixel(32, 18)[0] < 128);
}

#[test]
fn untouched_pixels_round_trip_through_linear_light() {
    let ramp = |x: u32, _| [16 + (x * 219 / 63) as u8, 128, 128];
    let mut frames = Frames::default().with(1, nv12(64, 36, bt709_limited(), ramp));
    let Some(image) = render(vec![Layer::new(source(1))], &mut frames) else {
        return;
    };
    for x in [0, 10, 31, 50, 63] {
        let want = expected(&bt709_limited(), ramp(x, 0));
        assert_near(image.pixel(x, 18), want, 1, &format!("ramp at x={x}"));
    }
}

#[test]
fn layers_stack_with_placement_and_opacity_in_linear_light() {
    let red = [51, 109, 212];
    let blue = [28, 212, 120];
    let overlay = Layer {
        transform: Transform {
            position: [-0.25, -0.25],
            scale: 0.5,
            rotation: 0.0,
        },
        opacity: 0.5,
        ..Layer::new(source(2))
    };
    let mut frames = Frames::default()
        .with(1, solid(64, 36, red))
        .with(2, solid(64, 36, blue));
    let Some(image) = render(vec![Layer::new(source(1)), overlay], &mut frames) else {
        return;
    };
    // Outside the overlay (bottom right): the base layer.
    assert_near(image.pixel(50, 30), [191, 0, 0], 2, "base");
    // Inside the overlay (the top-left quarter): a 50% mix computed in linear light.
    let (r, b) = (
        TransferFunction::Bt709.decode(0.75),
        TransferFunction::Bt709.decode(0.75),
    );
    let mix = [
        code(TransferFunction::Bt709.encode(0.5 * r)),
        0,
        code(TransferFunction::Bt709.encode(0.5 * b)),
    ];
    assert_near(image.pixel(16, 9), mix, 2, "overlay at 50%");
    assert!(
        mix[0] > 128,
        "linear-light mixing is brighter than signal averaging"
    );
}

#[test]
fn masks_limit_a_layer_with_feather_and_invert() {
    let white = [235, 128, 128];
    let mask = |invert, feather| Mask {
        shape: MaskShape::Ellipse,
        center: [0.5, 0.5],
        size: [0.5, 0.5],
        feather,
        invert,
    };
    let layer = |mask| Layer {
        mask: Some(mask),
        ..Layer::new(source(1))
    };
    let mut frames = Frames::default().with(1, solid(64, 36, white));
    let Some(image) = render(vec![layer(mask(false, 0.0))], &mut frames) else {
        return;
    };
    assert_near(
        image.pixel(32, 18),
        [255, 255, 255],
        1,
        "inside the ellipse",
    );
    assert_near(image.pixel(2, 2), [0, 0, 0], 1, "outside the ellipse");

    let inverted = render(vec![layer(mask(true, 0.0))], &mut frames).unwrap();
    assert_near(inverted.pixel(32, 18), [0, 0, 0], 1, "inverted inside");
    assert_near(inverted.pixel(2, 2), [255, 255, 255], 1, "inverted outside");

    let rectangle = Layer {
        mask: Some(Mask {
            shape: MaskShape::Rectangle,
            ..mask(false, 0.0)
        }),
        ..Layer::new(source(1))
    };
    let image = render(vec![rectangle], &mut frames).unwrap();
    assert_near(
        image.pixel(17, 10),
        [255, 255, 255],
        1,
        "rectangle corner region",
    );

    // Feathered: full inside, partial near the edge, none far outside.
    let soft = render(vec![layer(mask(false, 1.0))], &mut frames).unwrap();
    let edge = soft.pixel(32 + 12, 18)[0];
    assert!(edge > 10 && edge < 245, "feathered edge is partial: {edge}");
    assert_near(
        soft.pixel(2, 2),
        [0, 0, 0],
        1,
        "far outside a feathered mask",
    );
}

#[test]
fn luts_apply_to_the_display_referred_signal() {
    let invert = Lut3d::parse_cube(
        "LUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n",
    )
    .unwrap();
    let layer = |lut: Lut3d| Layer {
        lut: Some(Arc::new(lut)),
        ..Layer::new(source(1))
    };
    let mut frames = Frames::default().with(1, solid(64, 36, [51, 109, 212]));
    let Some(image) = render(vec![layer(invert)], &mut frames) else {
        return;
    };
    // 75% red inverts to 25% red with full green and blue.
    assert_near(image.pixel(32, 18), [64, 255, 255], 2, "inverted signal");
    let unchanged = render(vec![layer(Lut3d::identity(33))], &mut frames).unwrap();
    assert_near(unchanged.pixel(32, 18), [191, 0, 0], 2, "identity LUT");
}

#[test]
fn source_rotation_turns_the_frame_for_display() {
    // White top-left quadrant in the stored frame; rotated 90 degrees clockwise it shows
    // in the top-right.
    let frame = nv12(36, 36, bt709_limited(), |x, y| {
        if x < 18 && y < 18 {
            [235, 128, 128]
        } else {
            [16, 128, 128]
        }
    });
    let mut frames = Frames::default().with(1, frame);
    let rotated = Layer {
        rotation: 90,
        ..Layer::new(source(1))
    };
    let Some(mut renderer) = renderer() else {
        return;
    };
    let plan = RenderPlan {
        canvas: (36, 36),
        layers: vec![rotated],
        skipped: Vec::new(),
    };
    let image = renderer
        .render_image(&plan, &mut frames, (36, 36), TransferFunction::Bt709)
        .unwrap();
    assert_near(image.pixel(27, 9), [255, 255, 255], 1, "top right");
    assert_near(image.pixel(9, 9), [0, 0, 0], 1, "top left");
}

#[test]
fn downscaling_filters_in_linear_light_without_aliasing() {
    // A one-pixel black and white checkerboard shrunk 8x must become flat 50% linear
    // gray, not moire and not the darker signal average.
    let checker = nv12(512, 288, bt709_limited(), |x, y| {
        if (x + y) % 2 == 0 {
            [235, 128, 128]
        } else {
            [16, 128, 128]
        }
    });
    let mut frames = Frames::default().with(1, checker);
    let Some(image) = render(vec![Layer::new(source(1))], &mut frames) else {
        return;
    };
    let gray = code(TransferFunction::Bt709.encode(0.5));
    let values: Vec<u8> = (4..60)
        .step_by(7)
        .flat_map(|x| (4..32).step_by(5).map(move |y| (x, y)))
        .map(|(x, y)| image.pixel(x, y)[0])
        .collect();
    for value in &values {
        assert!(
            value.abs_diff(gray) <= 3,
            "expected flat {gray}, got {values:?}"
        );
    }
}

#[test]
fn empty_plans_are_black_and_problems_are_typed_errors() {
    let mut frames = Frames::default();
    let Some(image) = render(Vec::new(), &mut frames) else {
        return;
    };
    assert!(image.rgba.chunks(4).all(|p| p == [0, 0, 0, 255]));

    let mut renderer = renderer().unwrap();
    let plan = RenderPlan {
        canvas: SIZE,
        layers: vec![Layer::new(source(9))],
        skipped: Vec::new(),
    };
    let missing = renderer.render_image(&plan, &mut frames, SIZE, TransferFunction::Bt709);
    assert_eq!(missing.unwrap_err().code(), "fosscut.render.missing_frame");

    let hdr = ColorInfo {
        transfer: Transfer::Pq,
        ..bt709_limited()
    };
    let mut frames = Frames::default().with(9, nv12(8, 8, hdr, |_, _| [100, 128, 128]));
    let error = renderer.render_image(&plan, &mut frames, SIZE, TransferFunction::Bt709);
    assert_eq!(
        error.unwrap_err().code(),
        "fosscut.render.unsupported_color"
    );

    let mut empty = solid(8, 8, [16, 128, 128]);
    empty.planes.pop();
    let mut frames = Frames::default().with(9, empty);
    let error = renderer.render_image(&plan, &mut frames, SIZE, TransferFunction::Bt709);
    assert_eq!(error.unwrap_err().code(), "fosscut.render.missing_frame");
}

/// A decoder that serves one solid frame, standing in for FFmpeg.
struct OneFrame(VideoFrame, fosscut_media::VideoStreamInfo);

impl VideoDecoder for OneFrame {
    fn info(&self) -> &fosscut_media::VideoStreamInfo {
        &self.1
    }
    fn acceleration(&self) -> fosscut_media::Acceleration {
        fosscut_media::Acceleration::Software
    }
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, fosscut_media::MediaError> {
        Ok(Some(self.0.clone()))
    }
    fn frame_at(
        &mut self,
        time: fosscut_time::Flicks,
    ) -> Result<VideoFrame, fosscut_media::MediaError> {
        if time.is_negative() {
            return Err(fosscut_media::MediaError::EndOfStream {
                requested_flicks: time.get(),
                end_flicks: 0,
            });
        }
        Ok(self.0.clone())
    }
}

#[test]
fn decoder_frames_route_assets_to_their_decoders() {
    let info = fosscut_media::VideoStreamInfo {
        index: 0,
        codec: "test".into(),
        width: 8,
        height: 8,
        rate: None,
        time_base: (1, 30),
        duration: None,
        frame_count: None,
        rotation: 0,
        color: bt709_limited(),
        source_pixel_format: "nv12".into(),
    };
    let mut frames = DecoderFrames::new();
    frames.insert(
        asset(1),
        Box::new(OneFrame(solid(8, 8, [235, 128, 128]), info)),
    );
    assert_eq!(frames.frame(&source(1)).unwrap().width, 8);
    let missing = frames.frame(&source(2)).unwrap_err();
    assert_eq!(missing.code(), "fosscut.render.missing_frame");
    let mut early = source(1);
    early.time = fosscut_time::Flicks::new(-5);
    let error = frames.frame(&early).unwrap_err();
    assert!(
        error.to_string().contains("fosscut.media.end_of_stream"),
        "{error}"
    );
}

#[test]
fn psnr_compares_images() {
    let a = Image {
        width: 2,
        height: 1,
        rgba: vec![10, 20, 30, 255, 40, 50, 60, 255],
    };
    let mut b = a.clone();
    assert_eq!(a.psnr(&b), Some(f64::INFINITY));
    b.rgba[0] = 11;
    let value = a.psnr(&b).unwrap();
    assert!(value > 50.0 && value.is_finite());
    let other = Image {
        width: 1,
        height: 1,
        rgba: vec![0; 4],
    };
    assert_eq!(a.psnr(&other), None);
}
