//! Colour quantization for GIF output: at most 255 colours per frame by median cut, mapped
//! with ordered (Bayer) dithering. Deterministic: the same frames always give the same
//! palettes and indices.
//!
//! A frame with at most 255 distinct colours (titles, flat graphics) keeps them exactly,
//! undithered. Other frames get their own median-cut palette, so a scene change or a fade
//! never has to live with colours picked from earlier frames. A palette that changes every
//! frame makes flat areas shimmer, though, and FFmpeg's GIF encoder only saves the
//! unchanged pixels of a frame (and skips the frame's own colour table) when the palette
//! equals the first frame's. So the previous palette is kept while it still fits: when the
//! frame's error with it is at most [`REUSE_TOLERANCE`] times the error with the frame's own
//! palette. Static shots then share one palette and stay small; new content gets new
//! colours.
//!
//! One palette entry stays free (255 colours, not 256) so the encoder always has an unused
//! index to mark unchanged pixels transparent.

/// Colours a palette holds; the 256th entry is left for the encoder's transparency.
const MAX_COLORS: usize = 255;

/// How much worse (in summed squared error) the previous palette may be than a fresh one
/// before the frame gets the fresh one.
const REUSE_TOLERANCE: (u64, u64) = (5, 4);

/// Bits per channel of the histogram that median cut works on.
const HISTOGRAM_BITS: u32 = 5;

/// Bits per channel of the cells whose nearest entry a palette remembers while mapping.
const CELL_BITS: u32 = 6;

/// Range of the ordered dither offset in 8-bit levels: offsets run from -8 to +8, about
/// the spacing of 255 colours spread over a video frame, as FFmpeg's `paletteuse` does.
const DITHER_SPREAD: f32 = 16.0;

/// The 8x8 Bayer threshold matrix (values 0 to 63).
const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// One frame as palette indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IndexedFrame {
    /// 256 opaque entries as FFmpeg's `pal8` stores them: `0xAARRGGBB` in native byte
    /// order. Entries past the frame's colours are opaque black and unused.
    pub palette: [u32; 256],
    /// `width * height` indices, rows top to bottom, no padding.
    pub indices: Vec<u8>,
}

/// Turns RGBA frames into palette frames, keeping the previous palette while it fits.
#[derive(Debug, Default)]
pub(crate) struct Quantizer {
    current: Option<Palette>,
}

impl Quantizer {
    /// A quantizer with no palette yet.
    pub(crate) fn new() -> Quantizer {
        Quantizer::default()
    }

    /// Indexes one frame of `width * height` RGBA pixels (`rgba` holds at least
    /// `width * height * 4` bytes). Alpha is ignored: GIF frames here are opaque.
    pub(crate) fn index(&mut self, rgba: &[u8], width: usize, height: usize) -> IndexedFrame {
        let pixels = &rgba[..(width * height * 4).min(rgba.len())];
        let previous = self.current.take();
        let (mut palette, exact) = match distinct_colors(pixels) {
            // Few colours are kept exactly, by the previous palette if it has them all.
            Some(colors) => match previous {
                Some(previous) if colors.iter().all(|&c| previous.nearest(c).1 == 0) => {
                    (previous, true)
                }
                _ => (Palette::new(colors), true),
            },
            None => {
                let mut bins = histogram(pixels);
                // The fresh palette's error is taken as that of the boxes it came from,
                // which is at least its nearest-entry error, so close calls keep the
                // previous palette.
                let (colors, fresh_error) = median_cut(&mut bins);
                let fresh = Palette::new(colors);
                let (num, den) = REUSE_TOLERANCE;
                match previous {
                    Some(mut previous) => {
                        if previous.error(&bins) * den <= fresh_error * num {
                            (previous, false)
                        } else {
                            (fresh, false)
                        }
                    }
                    None => (fresh, false),
                }
            }
        };
        let indices = if exact {
            palette.map_exact(pixels)
        } else {
            palette.map_dithered(pixels, width)
        };
        let frame = IndexedFrame {
            palette: palette.entries(),
            indices,
        };
        self.current = Some(palette);
        frame
    }
}

/// The frame's distinct colours in ascending order, or `None` when there are more than
/// [`MAX_COLORS`].
fn distinct_colors(rgba: &[u8]) -> Option<Vec<[u8; 3]>> {
    let mut seen = std::collections::HashSet::new();
    let mut last = None;
    for pixel in rgba.as_chunks::<4>().0 {
        let key = (u32::from(pixel[0]) << 16) | (u32::from(pixel[1]) << 8) | u32::from(pixel[2]);
        if last != Some(key) {
            last = Some(key);
            if seen.insert(key) && seen.len() > MAX_COLORS {
                return None;
            }
        }
    }
    let mut keys: Vec<u32> = seen.into_iter().collect();
    keys.sort_unstable();
    Some(
        keys.into_iter()
            .map(|k| [(k >> 16) as u8, (k >> 8) as u8, k as u8])
            .collect(),
    )
}

/// A histogram cell: the pixels whose colour falls in one cell of the 5-bit grid.
#[derive(Clone, Copy, Debug)]
struct Bin {
    /// Grid cell, for deterministic ordering.
    key: u32,
    /// Number of pixels.
    count: u64,
    /// Sum of each channel over the pixels.
    sum: [u64; 3],
    /// The mean colour of the pixels.
    mean: [i32; 3],
}

/// The non-empty cells of the frame's 5-bit-per-channel histogram, in grid order.
fn histogram(rgba: &[u8]) -> Vec<Bin> {
    let cells = 1usize << (3 * HISTOGRAM_BITS);
    let shift = 8 - HISTOGRAM_BITS;
    let mut counts = vec![0u64; cells];
    let mut sums = vec![[0u64; 3]; cells];
    for pixel in rgba.as_chunks::<4>().0 {
        let key = (usize::from(pixel[0] >> shift) << (2 * HISTOGRAM_BITS))
            | (usize::from(pixel[1] >> shift) << HISTOGRAM_BITS)
            | usize::from(pixel[2] >> shift);
        counts[key] += 1;
        for channel in 0..3 {
            sums[key][channel] += u64::from(pixel[channel]);
        }
    }
    counts
        .iter()
        .zip(&sums)
        .enumerate()
        .filter(|(_, (count, _))| **count > 0)
        .map(|(key, (&count, &sum))| Bin {
            key: key as u32,
            count,
            sum,
            mean: sum.map(|s| ((s + count / 2) / count) as i32),
        })
        .collect()
}

/// Summed squared deviation of each channel of `bins` from its mean, weighted by pixels.
fn channel_errors(bins: &[Bin]) -> [f64; 3] {
    let mut count = 0.0;
    let mut sum = [0.0f64; 3];
    let mut squares = [0.0f64; 3];
    for bin in bins {
        let n = bin.count as f64;
        count += n;
        for (channel, c) in bin.mean.into_iter().enumerate() {
            let c = f64::from(c);
            sum[channel] += n * c;
            squares[channel] += n * c * c;
        }
    }
    if count == 0.0 {
        return [0.0; 3];
    }
    [0, 1, 2].map(|channel| (squares[channel] - sum[channel] * sum[channel] / count).max(0.0))
}

/// Median cut: splits the box of bins with the largest error along its widest channel at
/// the pixel-weighted median, until there are [`MAX_COLORS`] boxes or none can be split.
/// Returns each box's mean colour and the summed squared error of the bins against their
/// box's mean. Reorders `bins`.
fn median_cut(bins: &mut [Bin]) -> (Vec<[u8; 3]>, u64) {
    if bins.is_empty() {
        return (vec![[0, 0, 0]], 0);
    }
    // Boxes are ranges of `bins`; each split sorts its range in place.
    let mut boxes = vec![(0usize, bins.len(), channel_errors(bins))];
    while boxes.len() < MAX_COLORS {
        let candidate = boxes
            .iter()
            .enumerate()
            .filter(|(_, (start, end, _))| end - start >= 2)
            .map(|(i, (_, _, errors))| (i, errors.iter().sum::<f64>()))
            .filter(|(_, error)| *error > 0.0)
            .fold(None, |best: Option<(usize, f64)>, (i, error)| match best {
                Some((_, best_error)) if best_error >= error => best,
                _ => Some((i, error)),
            });
        let Some((index, _)) = candidate else {
            break;
        };
        let (start, end, errors) = boxes[index];
        let channel = (0..3).fold(0, |widest, c| {
            if errors[c] > errors[widest] {
                c
            } else {
                widest
            }
        });
        let slice = &mut bins[start..end];
        slice.sort_unstable_by_key(|bin| (bin.mean[channel], bin.key));
        let total: u64 = slice.iter().map(|bin| bin.count).sum();
        let mut running = 0;
        let mut split = slice.len() - 1;
        for (i, bin) in slice.iter().enumerate() {
            running += bin.count;
            if 2 * running >= total {
                split = i + 1;
                break;
            }
        }
        let middle = start + split.clamp(1, slice.len() - 1);
        boxes[index] = (start, middle, channel_errors(&bins[start..middle]));
        boxes.push((middle, end, channel_errors(&bins[middle..end])));
    }
    let error: f64 = boxes.iter().map(|(_, _, e)| e.iter().sum::<f64>()).sum();
    let colors = boxes
        .iter()
        .map(|&(start, end, _)| {
            let range = &bins[start..end];
            let count: u64 = range.iter().map(|bin| bin.count).sum::<u64>().max(1);
            [0, 1, 2].map(|channel| {
                let sum: u64 = range.iter().map(|bin| bin.sum[channel]).sum();
                ((sum + count / 2) / count).min(255) as u8
            })
        })
        .collect();
    (colors, error.round() as u64)
}

/// A palette with a nearest-colour search over its entries sorted by green, and the
/// nearest entry of every 6-bit cell met so far, so a palette kept over many frames maps
/// them fast.
#[derive(Clone, Debug)]
struct Palette {
    colors: Vec<[u8; 3]>,
    /// `(green, index)` of every entry, sorted.
    by_green: Vec<(i32, usize)>,
    /// Nearest entry per cell of [`CELL_BITS`] bits per channel; `u16::MAX` until needed.
    cells: Vec<u16>,
}

impl Palette {
    fn new(colors: Vec<[u8; 3]>) -> Palette {
        let mut by_green: Vec<(i32, usize)> = colors
            .iter()
            .enumerate()
            .map(|(i, c)| (i32::from(c[1]), i))
            .collect();
        by_green.sort_unstable();
        Palette {
            colors,
            by_green,
            cells: Vec::new(),
        }
    }

    /// The 256 `pal8` entries.
    fn entries(&self) -> [u32; 256] {
        let mut entries = [0xFF00_0000u32; 256];
        for (entry, c) in entries.iter_mut().zip(&self.colors) {
            *entry =
                0xFF00_0000 | (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
        }
        entries
    }

    /// The entry nearest to `color` in squared RGB distance, and that distance; ties go
    /// to the lower index. Searches outwards from the colour's green value and stops once
    /// the green difference alone exceeds the best distance.
    fn nearest(&self, color: [u8; 3]) -> (usize, u32) {
        let color = color.map(i32::from);
        let distance_to = |index: usize| -> u32 {
            let c = self.colors[index];
            (0..3).map(|k| square(color[k] - i32::from(c[k]))).sum()
        };
        // `(distance, index)`, so comparing tuples prefers the lower index on ties.
        let mut best = (u32::MAX, 0);
        let start = self.by_green.partition_point(|&(g, _)| g < color[1]);
        let (mut up, mut down) = (start, start);
        loop {
            let mut moved = false;
            if let Some(&(g, index)) = self.by_green.get(up)
                && square(g - color[1]) <= best.0
            {
                best = best.min((distance_to(index), index));
                up += 1;
                moved = true;
            }
            if let Some(&(g, index)) = down.checked_sub(1).and_then(|i| self.by_green.get(i))
                && square(color[1] - g) <= best.0
            {
                best = best.min((distance_to(index), index));
                down -= 1;
                moved = true;
            }
            if !moved {
                return (best.1, best.0);
            }
        }
    }

    /// The entry nearest to the centre of `color`'s cell, remembered per cell. Within a
    /// cell colours differ by at most 3 levels per channel, well inside the dither offset.
    fn nearest_in_cell(&mut self, color: [u8; 3]) -> usize {
        let shift = 8 - CELL_BITS;
        if self.cells.is_empty() {
            self.cells = vec![u16::MAX; 1 << (3 * CELL_BITS)];
        }
        let cell = color.map(|c| usize::from(c >> shift));
        let key = (cell[0] << (2 * CELL_BITS)) | (cell[1] << CELL_BITS) | cell[2];
        match self.cells[key] {
            u16::MAX => {
                let half = 1u8 << (shift - 1);
                let centre = color.map(|c| ((c >> shift) << shift) | half);
                let index = self.nearest(centre).0;
                self.cells[key] = index as u16;
                index
            }
            index => usize::from(index),
        }
    }

    /// Summed squared error of the bins' mean colours against the entries they map to.
    fn error(&mut self, bins: &[Bin]) -> u64 {
        bins.iter()
            .map(|bin| {
                let mean = bin.mean.map(|c| c.clamp(0, 255) as u8);
                let index = self.nearest_in_cell(mean);
                let entry = self.colors[index];
                let distance: u32 = (0..3)
                    .map(|k| square(bin.mean[k] - i32::from(entry[k])))
                    .sum();
                bin.count * u64::from(distance)
            })
            .sum()
    }

    /// Indexes pixels whose colours are all in the palette.
    fn map_exact(&self, rgba: &[u8]) -> Vec<u8> {
        let mut last = None;
        rgba.as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| {
                let color = [pixel[0], pixel[1], pixel[2]];
                match last {
                    Some((previous, index)) if previous == color => index,
                    _ => {
                        let index = self.nearest(color).0 as u8;
                        last = Some((color, index));
                        index
                    }
                }
            })
            .collect()
    }

    /// Indexes pixels after offsetting each by the Bayer threshold at its position, so
    /// gradients turn into a fixed pattern of neighbouring entries instead of bands.
    fn map_dithered(&mut self, rgba: &[u8], width: usize) -> Vec<u8> {
        let width = width.max(1);
        let offsets: [[i32; 8]; 8] = BAYER.map(|row| {
            row.map(|t| (((f32::from(t) + 0.5) / 64.0 - 0.5) * DITHER_SPREAD).round() as i32)
        });
        rgba.as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .map(|(i, pixel)| {
                let offset = offsets[(i / width) % 8][(i % width) % 8];
                let color = [0, 1, 2].map(|k| (i32::from(pixel[k]) + offset).clamp(0, 255) as u8);
                self.nearest_in_cell(color) as u8
            })
            .collect()
    }
}

fn square(d: i32) -> u32 {
    d.unsigned_abs() * d.unsigned_abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: usize, height: usize, shift: usize) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let r = ((x + shift) * 255 / (width + shift)) as u8;
                let g = (y * 255 / height.max(1)) as u8;
                let b = (((x + y) * 3 + shift) % 256) as u8;
                rgba.extend([r, g, b, 255]);
            }
        }
        rgba
    }

    fn decoded(frame: &IndexedFrame) -> Vec<[u8; 3]> {
        frame
            .indices
            .iter()
            .map(|&i| {
                let v = frame.palette[usize::from(i)];
                [(v >> 16) as u8, (v >> 8) as u8, v as u8]
            })
            .collect()
    }

    #[test]
    fn few_colours_are_kept_exactly_without_dithering() {
        let colors = [[255u8, 0, 0], [0, 128, 255], [10, 10, 10], [12, 12, 12]];
        let mut rgba = Vec::new();
        for i in 0..64 * 32 {
            let c = colors[(i / 7) % colors.len()];
            rgba.extend([c[0], c[1], c[2], 255]);
        }
        let frame = Quantizer::new().index(&rgba, 64, 32);
        let back = decoded(&frame);
        for (pixel, got) in rgba.as_chunks::<4>().0.iter().zip(back) {
            assert_eq!([pixel[0], pixel[1], pixel[2]], got);
        }
        // Unused entries are opaque black.
        assert_eq!(frame.palette[255], 0xFF00_0000);
        assert!(frame.palette.iter().all(|v| v >> 24 == 0xFF));
    }

    #[test]
    fn gradients_stay_close_and_use_at_most_255_entries() {
        let (w, h) = (160, 90);
        let rgba = gradient(w, h, 0);
        let frame = Quantizer::new().index(&rgba, w, h);
        assert_eq!(frame.indices.len(), w * h);
        assert!(
            frame.indices.iter().all(|&i| i < 255),
            "entry 255 stays free"
        );
        let back = decoded(&frame);
        let error: u64 = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(&back)
            .map(|(p, q)| (0..3).map(|k| u64::from(p[k].abs_diff(q[k]))).sum::<u64>())
            .sum();
        let mean = error as f64 / (w * h * 3) as f64;
        assert!(mean < 8.0, "mean absolute error {mean}");
    }

    #[test]
    fn quantizing_is_deterministic_and_reuses_a_fitting_palette() {
        let (w, h) = (96, 54);
        let first = gradient(w, h, 0);
        let mut a = Quantizer::new();
        let mut b = Quantizer::new();
        let one = a.index(&first, w, h);
        assert_eq!(one, b.index(&first, w, h), "same input, same output");
        // The same picture again keeps the palette, so the encoder can skip it.
        let again = a.index(&first, w, h);
        assert_eq!(again.palette, one.palette);
        // Entirely different colours get a new palette.
        let mut green = Vec::new();
        for i in 0..w * h {
            green.extend([0, 100 + (i % 100) as u8, 0, 255]);
        }
        let other = a.index(&green, w, h);
        assert_ne!(other.palette, one.palette);
        let back = decoded(&other);
        assert!(back.iter().all(|c| c[0] == 0 && c[2] == 0 && c[1] >= 92));
    }

    #[test]
    fn nearest_search_matches_a_full_scan() {
        let colors: Vec<[u8; 3]> = (0..200u32)
            .map(|i| {
                [
                    (i * 37 % 256) as u8,
                    (i * 91 % 256) as u8,
                    (i * 53 % 256) as u8,
                ]
            })
            .collect();
        let mut palette = Palette::new(colors.clone());
        for probe in (0..4096u32).map(|i| {
            [
                (i * 7 % 256) as u8,
                (i * 13 % 256) as u8,
                (i * 29 % 256) as u8,
            ]
        }) {
            let full = colors
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let d: i32 = (0..3)
                        .map(|k| (i32::from(probe[k]) - i32::from(c[k])).pow(2))
                        .sum();
                    (d as u32, i)
                })
                .min()
                .unwrap();
            assert_eq!(palette.nearest(probe), (full.1, full.0), "{probe:?}");
            // The remembered cell entry is at most a few levels worse than the nearest.
            let index = palette.nearest_in_cell(probe);
            let cell = palette.colors[index];
            let d: i32 = (0..3)
                .map(|k| (i32::from(probe[k]) - i32::from(cell[k])).pow(2))
                .sum();
            assert!(
                f64::from(d).sqrt() <= f64::from(full.0).sqrt() + 6.0,
                "{probe:?}"
            );
        }
    }

    #[test]
    fn empty_frames_get_a_black_palette() {
        let frame = Quantizer::new().index(&[], 0, 0);
        assert!(frame.indices.is_empty());
        assert_eq!(frame.palette[0], 0xFF00_0000);
    }
}
