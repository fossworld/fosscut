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
use fosscut_render::subject::{Landmark, Matte, MatteKind, Pose};
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

/// Fixed frames per asset, or a pattern that moves with time, with mattes and poses for
/// the subject effects.
#[derive(Default)]
pub struct Frames {
    pub fixed: HashMap<Id, VideoFrame>,
    pub moving: HashMap<Id, (u32, u32)>,
    pub mattes: HashMap<(Id, MatteKind), Arc<Matte>>,
    pub poses: HashMap<Id, Arc<Pose>>,
}

impl Frames {
    pub fn with(mut self, n: u64, frame: VideoFrame) -> Frames {
        self.fixed.insert(asset(n), frame);
        self
    }
}

impl Frames {
    /// Uses `matte` as asset `n`'s matte of `kind` at every time.
    pub fn with_matte(mut self, n: u64, kind: MatteKind, matte: Matte) -> Frames {
        self.mattes.insert((asset(n), kind), Arc::new(matte));
        self
    }

    /// Uses `pose` as asset `n`'s pose at every time.
    pub fn with_pose(mut self, n: u64, pose: Pose) -> Frames {
        self.poses.insert(asset(n), Arc::new(pose));
        self
    }
}

/// A drawn figure: head, neck, torso, arms and legs as capsules and boxes, in fractions of a
/// 16:9 frame's width and height. Returns how much of the pixel at `(u, v)` it covers, from
/// a 4 by 4 grid of samples inside the pixel of size `(du, dv)`.
pub fn figure_coverage(u: f32, v: f32, du: f32, dv: f32) -> f32 {
    let aspect = 16.0 / 9.0;
    let capsule = |x: f32, y: f32, a: (f32, f32), b: (f32, f32), r: f32| {
        // Distances in units of the frame's height.
        let (px, py) = ((x - a.0) * aspect, y - a.1);
        let (vx, vy) = ((b.0 - a.0) * aspect, b.1 - a.1);
        let t = ((px * vx + py * vy) / (vx * vx + vy * vy).max(1e-9)).clamp(0.0, 1.0);
        let (dx, dy) = (px - t * vx, py - t * vy);
        dx * dx + dy * dy <= r * r
    };
    let inside = |x: f32, y: f32| {
        capsule(x, y, (0.5, 0.2), (0.5, 0.2), 0.07)
            || capsule(x, y, (0.5, 0.26), (0.5, 0.32), 0.025)
            || ((0.44..0.56).contains(&x) && (0.3..0.6).contains(&y))
            || capsule(x, y, (0.44, 0.33), (0.39, 0.6), 0.025)
            || capsule(x, y, (0.56, 0.33), (0.61, 0.6), 0.025)
            || capsule(x, y, (0.47, 0.6), (0.46, 0.9), 0.03)
            || capsule(x, y, (0.53, 0.6), (0.54, 0.9), 0.03)
    };
    let mut hits = 0;
    for j in 0..4 {
        for i in 0..4 {
            let x = u + (i as f32 + 0.5) / 4.0 * du;
            let y = v + (j as f32 + 0.5) / 4.0 * dv;
            hits += u32::from(inside(x, y));
        }
    }
    hits as f32 / 16.0
}

/// The figure's matte at `width` by `height`.
pub fn figure_matte(width: u32, height: u32) -> Matte {
    let (du, dv) = (1.0 / width as f32, 1.0 / height as f32);
    let alpha = (0..width * height)
        .map(|i| {
            let (x, y) = (i % width, i / width);
            let c = figure_coverage(x as f32 * du, y as f32 * dv, du, dv);
            (c * 255.0).round() as u8
        })
        .collect();
    Matte {
        width,
        height,
        alpha,
    }
}

/// The figure as a frame: a warm shirt and skin on a blue-grey background with a gentle
/// gradient and checker, so effects behind it show.
pub fn figure_frame(width: u32, height: u32) -> VideoFrame {
    let (du, dv) = (1.0 / width as f32, 1.0 / height as f32);
    nv12(width, height, bt709_limited(), move |x, y| {
        let (u, v) = (x as f32 * du, y as f32 * dv);
        let c = figure_coverage(u, v, du, dv);
        // A gradient with a soft checker, so effects behind the figure and blurs show.
        let checker = ((x / 8 + y / 8) % 2) as u8 * 24;
        let background = [80 + (v * 40.0) as u8 + checker, 150, 112];
        let person = if v < 0.28 {
            [150, 100, 160]
        } else {
            [100, 90, 190]
        };
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * c).round() as u8;
        [
            mix(background[0], person[0]),
            mix(background[1], person[1]),
            mix(background[2], person[2]),
        ]
    })
}

/// The figure's pose: its landmarks where the shapes put the joints, and its mask.
pub fn figure_pose(mask: (u32, u32)) -> Pose {
    let mut landmarks = vec![Landmark::default(); 33];
    let joints = [
        (0, 0.5, 0.2),
        (11, 0.56, 0.33),
        (12, 0.44, 0.33),
        (13, 0.585, 0.46),
        (14, 0.415, 0.46),
        (15, 0.61, 0.6),
        (16, 0.39, 0.6),
        (23, 0.53, 0.6),
        (24, 0.47, 0.6),
        (25, 0.535, 0.75),
        (26, 0.465, 0.75),
        (27, 0.54, 0.9),
        (28, 0.46, 0.9),
    ];
    for (i, x, y) in joints {
        landmarks[i] = Landmark {
            x,
            y,
            visibility: 0.95,
        };
    }
    Pose {
        presence: 0.99,
        landmarks,
        mask: Some(Arc::new(figure_matte(mask.0, mask.1))),
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

    fn matte(
        &mut self,
        source: &SourceRef,
        kind: MatteKind,
    ) -> Result<Option<Arc<Matte>>, RenderError> {
        Ok(self.mattes.get(&(source.asset.clone(), kind)).cloned())
    }

    fn pose(&mut self, source: &SourceRef) -> Result<Option<Arc<Pose>>, RenderError> {
        Ok(self.poses.get(&source.asset).cloned())
    }
}

// ---- Scenes for temporal effects (ADR-0035) ----

/// A hash of three integers to 0 to 1 (PCG3D, as the shaders use).
pub fn hash(x: i32, y: i32, z: u32) -> f32 {
    let mut v =
        [x as u32, y as u32, z].map(|c| c.wrapping_mul(1_664_525).wrapping_add(1_013_904_223));
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
    v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
    v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    v = v.map(|c| c ^ (c >> 16));
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
    (v[0] >> 8) as f32 / 16_777_216.0
}

/// Smooth value noise: hashed values on a grid of `cell` pixels, joined bilinearly.
pub fn value_noise(x: f32, y: f32, cell: f32, seed: u32) -> f32 {
    let (gx, gy) = (x / cell, y / cell);
    let (ix, iy) = (gx.floor(), gy.floor());
    let (fx, fy) = (gx - ix, gy - iy);
    let (fx, fy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let h = |dx: i32, dy: i32| hash(ix as i32 + dx, iy as i32 + dy, seed);
    let top = h(0, 0) + (h(1, 0) - h(0, 0)) * fx;
    let bottom = h(0, 1) + (h(1, 1) - h(0, 1)) * fx;
    top + (bottom - top) * fy
}

/// Linear-light RGB to BT.709 limited-range Y'CbCr codes.
pub fn ycbcr(rgb: [f32; 3]) -> [u8; 3] {
    let e = |l: f32| fosscut_render::color::TransferFunction::Bt709.encode(l.clamp(0.0, 1.0));
    let (r, g, b) = (e(rgb[0]), e(rgb[1]), e(rgb[2]));
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let (cb, cr) = ((b - y) / 1.8556, (r - y) / 1.5748);
    let code = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    [
        code(16.0 + 219.0 * y),
        code(128.0 + 224.0 * cb),
        code(128.0 + 224.0 * cr),
    ]
}

/// A 320x180 frame of a scene given in linear-light RGB at pixel centres.
pub fn frame_of(scene: impl Fn(f32, f32) -> [f32; 3]) -> VideoFrame {
    nv12(320, 180, bt709_limited(), |x, y| {
        ycbcr(scene(x as f32 + 0.5, y as f32 + 0.5))
    })
}

/// A textured background moving left 4 pixels a frame, and a textured disc moving right 10
/// pixels a frame over it, at time `t` in frames.
pub fn scene(x: f32, y: f32, t: f32) -> [f32; 3] {
    let (cx, cy) = (110.0 + 10.0 * t, 90.0);
    let (dx, dy) = (x - cx, y - cy);
    if dx * dx + dy * dy < 34.0 * 34.0 {
        let n =
            0.6 * value_noise(dx + 200.0, dy + 200.0, 5.0, 7) + 0.4 * value_noise(dx, dy, 2.5, 8);
        return [0.6 + 0.35 * n, 0.25 + 0.3 * n, 0.1 + 0.2 * n];
    }
    let bx = x + 4.0 * t;
    let n = 0.6 * value_noise(bx, y, 7.0, 1) + 0.4 * value_noise(bx, y, 3.0, 2);
    let stripe = if (bx / 23.0).floor() as i32 % 2 == 0 {
        0.08
    } else {
        0.0
    };
    [0.1 + 0.5 * n + stripe, 0.15 + 0.55 * n, 0.3 + 0.4 * n * n]
}
