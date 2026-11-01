//! Frames with an alpha channel (R'G'B'A, from ProRes 4444, PNG with alpha and web
//! compositions, ADR-0039) composite over the layers below by their alpha, in linear light,
//! with their own transfer function.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use fosscut_media::{
    ColorInfo, Matrix, PixelFormat, Plane, Primaries, Range, Transfer, VideoFrame,
};
use fosscut_render::color::TransferFunction;
use fosscut_render::plan::{Layer, RenderPlan};
use fosscut_time::{Flicks, Rate};
use support::*;

const SIZE: (u32, u32) = (16, 4);

/// A 16x4 R'G'B'A frame: a green that is fully transparent in columns 0 to 3, opaque red in
/// 4 to 7, and white at half alpha from 8 on.
fn rgba(transfer: Transfer) -> VideoFrame {
    let mut data = Vec::new();
    for _ in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            data.extend_from_slice(&match x {
                0..4 => [0, 255, 0, 0],
                4..8 => [255, 0, 0, 255],
                _ => [255, 255, 255, 128],
            });
        }
    }
    VideoFrame {
        pts: Flicks::ZERO,
        duration: Rate::FPS_30.frame_duration(),
        width: SIZE.0,
        height: SIZE.1,
        format: PixelFormat::Rgba,
        planes: vec![Plane {
            stride: SIZE.0 as usize * 4,
            rows: SIZE.1 as usize,
            data: Arc::from(data),
        }],
        color: ColorInfo {
            matrix: Matrix::Bt709,
            range: Range::Full,
            primaries: Primaries::Bt709,
            transfer,
        },
    }
}

fn pixel(image: &fosscut_render::Image, x: u32) -> [u8; 3] {
    let i = ((SIZE.1 / 2 * image.width + x) * 4) as usize;
    [image.rgba[i], image.rgba[i + 1], image.rgba[i + 2]]
}

#[test]
fn rgba_frames_blend_by_their_alpha_over_lower_layers() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    for transfer in [Transfer::Srgb, Transfer::Bt709] {
        // Black below, the R'G'B'A frame above.
        let mut frames = Frames::default()
            .with(1, solid(SIZE.0, SIZE.1, [16, 128, 128]))
            .with(2, rgba(transfer));
        let plan = RenderPlan {
            canvas: SIZE,
            layers: vec![Layer::new(source(1)), Layer::new(source(2))],
            skipped: Vec::new(),
        };
        let image = renderer
            .render_image(&plan, &mut frames, SIZE, TransferFunction::Srgb)
            .unwrap();
        assert_eq!(
            pixel(&image, 1),
            [0, 0, 0],
            "{transfer:?}: transparent shows below"
        );
        assert_eq!(pixel(&image, 5), [255, 0, 0], "{transfer:?}: opaque red");
        // Half of white in linear light over black, encoded as sRGB for the output.
        let decode = match transfer {
            Transfer::Srgb => TransferFunction::Srgb,
            _ => TransferFunction::Bt709,
        };
        let linear = decode.decode(1.0) * 128.0 / 255.0;
        let expected = (TransferFunction::Srgb.encode(linear) * 255.0).round() as i32;
        let half = pixel(&image, 12);
        for channel in half {
            assert!(
                (i32::from(channel) - expected).abs() <= 2,
                "{transfer:?}: {half:?} for {expected}"
            );
        }
    }
}

#[test]
fn rgba_frames_keep_their_code_values_through_a_matching_transfer() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    // An opaque sRGB orange stays the same orange in an sRGB output.
    let mut data = Vec::new();
    for _ in 0..SIZE.0 * SIZE.1 {
        data.extend_from_slice(&[255, 122, 69, 255]);
    }
    let mut frame = rgba(Transfer::Srgb);
    frame.planes[0].data = Arc::from(data);
    let mut frames = Frames::default().with(1, frame);
    let plan = RenderPlan {
        canvas: SIZE,
        layers: vec![Layer::new(source(1))],
        skipped: Vec::new(),
    };
    let image = renderer
        .render_image(&plan, &mut frames, SIZE, TransferFunction::Srgb)
        .unwrap();
    let orange = pixel(&image, 8);
    for (got, want) in orange.iter().zip([255u8, 122, 69]) {
        assert!(got.abs_diff(want) <= 1, "{orange:?}");
    }
}
