//! Grey images for the motion analyzers ([`crate::motion`] and [`crate::track`]): a luma
//! plane box-averaged to about 320 pixels wide, halved into pyramids and sampled with
//! bilinear interpolation.
//!
//! Pixel `(i, j)` of an image has its centre at `(i, j)`. A frame's analysis copy spans
//! `extent` of its pixels across the source frame, so the fraction `f` of the frame's width
//! is at `f * extent.0 - 0.5` in pixel coordinates.

use crate::scenes::Luma;

/// About how wide, in pixels, the motion analyzers' copy of a frame is.
pub(crate) const ANALYSIS_WIDTH: u32 = 320;

/// A grey image with samples from 0 to 255, rows top to bottom without padding.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Gray {
    /// Width in pixels.
    pub(crate) width: usize,
    /// Height in pixels.
    pub(crate) height: usize,
    /// `width * height` samples.
    pub(crate) data: Vec<f32>,
    /// How many of this image's pixels the whole source frame spans, across and down: the
    /// source size divided by the downscaling factor, which can exceed `width` and `height`
    /// by less than a pixel when the factor does not divide the source size.
    pub(crate) extent: (f64, f64),
}

impl Gray {
    /// An image of `data`, `width` by `height`, that is a whole frame.
    pub(crate) fn new(width: usize, height: usize, data: Vec<f32>) -> Gray {
        Gray {
            width,
            height,
            data,
            extent: (width as f64, height as f64),
        }
    }

    /// `luma` averaged over square blocks of the whole number of pixels that brings its width
    /// closest to [`ANALYSIS_WIDTH`]. Rows missing from a short plane read as black.
    pub(crate) fn from_luma(luma: Luma<'_>) -> Gray {
        let factor = (luma.width.saturating_add(ANALYSIS_WIDTH / 2) / ANALYSIS_WIDTH).max(1);
        let factor = factor as usize;
        let width = luma.width as usize / factor;
        let height = luma.height as usize / factor;
        let mut sums = vec![0u64; width * height];
        for y in 0..height * factor {
            let row = y
                .checked_mul(luma.stride)
                .and_then(|start| luma.data.get(start..start.checked_add(width * factor)?));
            let Some(row) = row else {
                continue;
            };
            let out = &mut sums[y / factor * width..][..width];
            for (sum, block) in out.iter_mut().zip(row.chunks_exact(factor)) {
                *sum += block.iter().map(|&v| u64::from(v)).sum::<u64>();
            }
        }
        let scale = 1.0 / (factor * factor) as f64;
        Gray {
            width,
            height,
            data: sums
                .into_iter()
                .map(|sum| (sum as f64 * scale) as f32)
                .collect(),
            extent: (
                f64::from(luma.width) / factor as f64,
                f64::from(luma.height) / factor as f64,
            ),
        }
    }

    /// Whether it has no pixels.
    pub(crate) fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Halved in both directions by averaging 2 by 2 blocks.
    pub(crate) fn half(&self) -> Gray {
        let (width, height) = (self.width / 2, self.height / 2);
        let mut data = Vec::with_capacity(width * height);
        for y in 0..height {
            let top = &self.data[2 * y * self.width..][..self.width];
            let bottom = &self.data[(2 * y + 1) * self.width..][..self.width];
            for x in 0..width {
                let sum = top[2 * x] + top[2 * x + 1] + bottom[2 * x] + bottom[2 * x + 1];
                data.push(sum * 0.25);
            }
        }
        Gray {
            width,
            height,
            data,
            extent: (self.extent.0 / 2.0, self.extent.1 / 2.0),
        }
    }

    /// The sample at `(x, y)`, which must be inside.
    pub(crate) fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }

    /// The sample at `(x, y)`, repeating the nearest edge pixel outside; 0 when empty.
    pub(crate) fn clamped(&self, x: isize, y: isize) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let x = x.clamp(0, self.width as isize - 1) as usize;
        let y = y.clamp(0, self.height as isize - 1) as usize;
        self.at(x, y)
    }

    /// Bilinear interpolation at `(x, y)`, repeating edge pixels outside; 0 when empty.
    pub(crate) fn bilinear(&self, x: f64, y: f64) -> f32 {
        if self.is_empty() || x.is_nan() || y.is_nan() {
            return 0.0;
        }
        let x = x.clamp(0.0, (self.width - 1) as f64);
        let y = y.clamp(0.0, (self.height - 1) as f64);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let top = self.at(x0, y0) + (self.at(x1, y0) - self.at(x0, y0)) * fx;
        let bottom = self.at(x0, y1) + (self.at(x1, y1) - self.at(x0, y1)) * fx;
        top + (bottom - top) * fy
    }

    /// Resampled to `width` by `height` with bilinear interpolation, keeping its extent.
    pub(crate) fn resized(&self, width: usize, height: usize) -> Gray {
        let (sx, sy) = (
            self.width as f64 / width.max(1) as f64,
            self.height as f64 / height.max(1) as f64,
        );
        let mut data = Vec::with_capacity(width * height);
        for y in 0..height {
            let v = (y as f64 + 0.5) * sy - 0.5;
            data.extend((0..width).map(|x| self.bilinear((x as f64 + 0.5) * sx - 0.5, v)));
        }
        Gray {
            width,
            height,
            data,
            extent: self.extent,
        }
    }
}

/// `base` and up to `levels - 1` halvings of it, stopping before a level narrower or shorter
/// than `min_size` pixels.
pub(crate) fn pyramid(base: Gray, levels: usize, min_size: usize) -> Vec<Gray> {
    let mut pyramid = vec![base];
    while pyramid.len() < levels {
        let Some(last) = pyramid.last() else {
            break;
        };
        if last.width / 2 < min_size.max(1) || last.height / 2 < min_size.max(1) {
            break;
        }
        let half = last.half();
        pyramid.push(half);
    }
    pyramid
}

/// Where a parabola through `(-1, before)`, `(0, at)` and `(1, after)` has its lowest point,
/// within half a step of 0; 0 when it does not open upwards.
pub(crate) fn parabola_minimum(before: f64, at: f64, after: f64) -> f64 {
    let curvature = before - 2.0 * at + after;
    if curvature > 0.0 && curvature.is_finite() {
        (0.5 * (before - after) / curvature).clamp(-0.5, 0.5)
    } else {
        0.0
    }
}

/// Deterministic test pictures: smooth value noise and soft discs, defined everywhere so
/// that frames can be rendered at any position, angle and size.
#[cfg(test)]
pub(crate) mod testing {
    /// A hash of a lattice point, from 0 to 1.
    pub(crate) fn lattice(x: i64, y: i64, seed: u64) -> f64 {
        let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
        h ^= h >> 33;
        h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        h ^= h >> 33;
        h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
        h ^= h >> 33;
        (h >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Value noise from 0 to 1 with features about `period` pixels apart.
    pub(crate) fn noise(u: f64, v: f64, period: f64, seed: u64) -> f64 {
        let (x, y) = (u / period, v / period);
        let (x0, y0) = (x.floor(), y.floor());
        let ease = |t: f64| t * t * (3.0 - 2.0 * t);
        let (fx, fy) = (ease(x - x0), ease(y - y0));
        let (i, j) = (x0 as i64, y0 as i64);
        let top = lattice(i, j, seed) + (lattice(i + 1, j, seed) - lattice(i, j, seed)) * fx;
        let bottom =
            lattice(i, j + 1, seed) + (lattice(i + 1, j + 1, seed) - lattice(i, j + 1, seed)) * fx;
        top + (bottom - top) * fy
    }

    /// A grey texture at `(u, v)`, from 0 to 255: noise at three scales plus a soft disc,
    /// dark or bright, in every 32-pixel cell.
    pub(crate) fn texture(u: f64, v: f64, seed: u64) -> f64 {
        let mut value = 128.0
            + 90.0 * (noise(u, v, 24.0, seed) - 0.5)
            + 60.0 * (noise(u, v, 8.0, seed + 1) - 0.5)
            + 40.0 * (noise(u, v, 4.0, seed + 2) - 0.5);
        let (cell_x, cell_y) = ((u / 32.0).floor(), (v / 32.0).floor());
        let (i, j) = (cell_x as i64, cell_y as i64);
        let centre_x = cell_x * 32.0 + 8.0 + 16.0 * lattice(i, j, seed + 3);
        let centre_y = cell_y * 32.0 + 8.0 + 16.0 * lattice(i, j, seed + 4);
        let radius = 3.0 + 4.0 * lattice(i, j, seed + 5);
        let shade = if lattice(i, j, seed + 6) < 0.5 {
            30.0
        } else {
            225.0
        };
        let distance = (u - centre_x).hypot(v - centre_y);
        let cover = (radius - distance + 0.5).clamp(0.0, 1.0) * 0.8;
        value += (shade - value) * cover;
        value.clamp(0.0, 255.0)
    }

    /// A `width` by `height` plane of `shade(x, y)` at each pixel, rounded.
    pub(crate) fn plane(width: u32, height: u32, shade: impl Fn(f64, f64) -> f64) -> Vec<u8> {
        let mut data = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                data.push(shade(f64::from(x), f64::from(y)).round().clamp(0.0, 255.0) as u8);
            }
        }
        data
    }

    /// A deterministic generator (SplitMix64).
    pub(crate) struct Random(pub(crate) u64);

    impl Random {
        /// The next number from 0 to 1.
        pub(crate) fn unit(&mut self) -> f64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            (z >> 11) as f64 / (1u64 << 53) as f64
        }

        /// The next number from -1 to 1.
        pub(crate) fn signed(&mut self) -> f64 {
            2.0 * self.unit() - 1.0
        }

        /// An approximately normal number with mean 0 and deviation 1 (sum of four uniforms).
        pub(crate) fn normal(&mut self) -> f64 {
            ((0..4).map(|_| self.unit()).sum::<f64>() - 2.0) * 3f64.sqrt()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_averaged_to_about_320_wide() {
        // 1280 by 720 with a 1296-byte stride: a factor of 4.
        let mut data = vec![0u8; 1296 * 720];
        for y in 0..720 {
            for x in 0..1280 {
                data[y * 1296 + x] = if (x / 4 + y / 4) % 2 == 0 { 200 } else { 100 };
            }
        }
        let gray = Gray::from_luma(Luma {
            width: 1280,
            height: 720,
            stride: 1296,
            data: &data,
        });
        assert_eq!((gray.width, gray.height), (320, 180));
        assert_eq!(gray.extent, (320.0, 180.0));
        assert_eq!(gray.at(0, 0), 200.0);
        assert_eq!(gray.at(1, 0), 100.0);
        // 1000 wide is a factor of 3: 333 pixels, spanning 333.3 of them.
        let small = vec![50u8; 1000 * 10];
        let gray = Gray::from_luma(Luma {
            width: 1000,
            height: 10,
            stride: 1000,
            data: &small,
        });
        assert_eq!((gray.width, gray.height), (333, 3));
        assert!((gray.extent.0 - 1000.0 / 3.0).abs() < 1e-9);
        assert!(gray.data.iter().all(|&v| v == 50.0));
        // A plane shorter than it claims reads as black, without panicking.
        let short = Gray::from_luma(Luma {
            width: 320,
            height: 4,
            stride: 320,
            data: &[255; 640],
        });
        assert_eq!(short.data[..320], [255.0; 320]);
        assert_eq!(short.data[640..], [0.0; 640]);
    }

    #[test]
    fn halving_interpolation_and_resizing() {
        let gray = Gray::new(4, 2, vec![0.0, 2.0, 4.0, 6.0, 2.0, 4.0, 6.0, 8.0]);
        let half = gray.half();
        assert_eq!((half.width, half.height, half.data), (2, 1, vec![2.0, 6.0]));
        assert_eq!(gray.bilinear(0.5, 0.5), 2.0);
        assert_eq!(gray.bilinear(-3.0, 9.0), 2.0);
        assert_eq!(gray.bilinear(f64::NAN, 0.0), 0.0);
        assert_eq!(gray.clamped(10, -1), 6.0);
        assert_eq!(gray.resized(4, 2), gray);
        let levels = pyramid(Gray::new(64, 40, vec![1.0; 64 * 40]), 3, 17);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1].extent, (32.0, 20.0));
        assert_eq!(parabola_minimum(1.0, 0.0, 1.0), 0.0);
        assert!((parabola_minimum(2.0, 0.0, 1.0) - (1.0 / 6.0)).abs() < 1e-12);
        assert_eq!(parabola_minimum(0.0, 1.0, 0.0), 0.0);
        assert!(Gray::default().is_empty());
        assert_eq!(Gray::default().bilinear(1.0, 1.0), 0.0);
    }
}
