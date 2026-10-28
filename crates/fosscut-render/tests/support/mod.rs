//! Shared helpers for GPU tests: a renderer, synthetic Y'CbCr frames whose expected
//! output is known exactly, and a frame provider.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;

use fosscut_media::{
    ColorInfo, Matrix, PixelFormat, Plane, Primaries, Range, Transfer, VideoFrame,
};
use fosscut_model::{Id, IdKind};
use fosscut_render::plan::SourceRef;
use fosscut_render::{FrameProvider, RenderError, Renderer};
use fosscut_time::{Flicks, Rate};

/// A headless renderer, or `None` when no adapter exists. CI sets `FOSSCUT_REQUIRE_GPU=1`,
/// which turns a missing adapter into a failure instead of a skip.
pub fn renderer() -> Option<Renderer> {
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_GPU").is_some_and(|v| !v.is_empty() && v != "0");
    match Renderer::headless() {
        Ok(renderer) => Some(renderer),
        Err(error) if required => panic!("FOSSCUT_REQUIRE_GPU is set but {error}"),
        Err(error) => {
            eprintln!("skipping: {error}");
            None
        }
    }
}

pub fn bt709_limited() -> ColorInfo {
    ColorInfo {
        matrix: Matrix::Bt709,
        range: Range::Limited,
        primaries: Primaries::Bt709,
        transfer: Transfer::Bt709,
    }
}

fn plane(stride: usize, rows: usize, fill: impl Fn(usize, usize) -> u8) -> Plane {
    let mut data = Vec::with_capacity(stride * rows);
    for y in 0..rows {
        for x in 0..stride {
            data.push(fill(x, y));
        }
    }
    Plane {
        stride,
        rows,
        data: Arc::from(data),
    }
}

/// An NV12 frame where `pixel(x, y)` gives the Y'CbCr code values at each luma position
/// (chroma is taken at the top-left luma pixel of each 2x2 block).
pub fn nv12(
    width: u32,
    height: u32,
    color: ColorInfo,
    pixel: impl Fn(u32, u32) -> [u8; 3],
) -> VideoFrame {
    let (w, h) = (width as usize, height as usize);
    let luma = plane(w, h, |x, y| pixel(x as u32, y as u32)[0]);
    let chroma = plane(w.div_ceil(2) * 2, h.div_ceil(2), |x, y| {
        let [_, cb, cr] = pixel((x / 2 * 2) as u32, (y * 2) as u32);
        if x % 2 == 0 { cb } else { cr }
    });
    VideoFrame {
        pts: Flicks::ZERO,
        duration: Rate::FPS_30.frame_duration(),
        width,
        height,
        format: PixelFormat::Nv12,
        planes: vec![luma, chroma],
        color,
    }
}

/// The same frame as three planes (YUV 4:2:0 planar).
pub fn yuv420p(
    width: u32,
    height: u32,
    color: ColorInfo,
    pixel: impl Fn(u32, u32) -> [u8; 3],
) -> VideoFrame {
    let (w, h) = (width as usize, height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let planes = vec![
        plane(w, h, |x, y| pixel(x as u32, y as u32)[0]),
        plane(cw, ch, |x, y| pixel((x * 2) as u32, (y * 2) as u32)[1]),
        plane(cw, ch, |x, y| pixel((x * 2) as u32, (y * 2) as u32)[2]),
    ];
    VideoFrame {
        pts: Flicks::ZERO,
        duration: Rate::FPS_30.frame_duration(),
        width,
        height,
        format: PixelFormat::Yuv420p,
        planes,
        color,
    }
}

pub fn solid(width: u32, height: u32, yuv: [u8; 3]) -> VideoFrame {
    nv12(width, height, bt709_limited(), |_, _| yuv)
}

/// A test pattern: a horizontal luma ramp with color varying by quadrant, shifted
/// horizontally by `offset` pixels so different times give different frames.
pub fn pattern(width: u32, height: u32, offset: u32) -> VideoFrame {
    nv12(width, height, bt709_limited(), move |x, y| {
        let x = (x + offset) % width;
        let luma = 16 + (219 * x / width.max(1)) as u8;
        let (cb, cr) = match (x * 2 / width, y * 2 / height) {
            (0, 0) => (90, 200),
            (1, 0) => (200, 90),
            (0, _) => (160, 60),
            _ => (60, 160),
        };
        [luma, cb, cr]
    })
}

pub fn asset(n: u64) -> Id {
    Id::new(IdKind::Asset, n)
}

pub fn source(n: u64) -> SourceRef {
    SourceRef {
        asset: asset(n),
        time: Flicks::ZERO,
    }
}

/// Fixed frames per asset, or a pattern that moves with time.
#[derive(Default)]
pub struct Frames {
    pub fixed: HashMap<Id, VideoFrame>,
    pub moving: HashMap<Id, (u32, u32)>,
}

impl Frames {
    pub fn with(mut self, n: u64, frame: VideoFrame) -> Frames {
        self.fixed.insert(asset(n), frame);
        self
    }
}

impl FrameProvider for Frames {
    fn frame(&mut self, source: &SourceRef) -> Result<VideoFrame, RenderError> {
        if let Some(frame) = self.fixed.get(&source.asset) {
            return Ok(frame.clone());
        }
        if let Some((w, h)) = self.moving.get(&source.asset) {
            let shift = (Rate::FPS_30.frame_at(source.time).max(0) as u32 * 7) % w;
            return Ok(pattern(*w, *h, shift));
        }
        Err(RenderError::MissingFrame {
            asset: source.asset.to_string(),
            message: "not in the test provider".into(),
        })
    }
}
