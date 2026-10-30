//! Raster helpers shared by captions and text: painting, rounded boxes, outlines from an
//! exact distance transform, and Gaussian blur from three box blurs.

/// Paints `color` (linear, straight alpha) over a premultiplied pixel with `coverage`.
pub(crate) fn over(pixel: &mut [f32; 4], color: [f32; 4], coverage: f32) {
    let alpha = (color[3] * coverage).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    for channel in 0..3 {
        pixel[channel] = color[channel] * alpha + pixel[channel] * (1.0 - alpha);
    }
    pixel[3] = alpha + pixel[3] * (1.0 - alpha);
}

/// Adds an anti-aliased rounded rectangle (`x0, y0, x1, y1`) to a coverage mask.
pub(crate) fn rounded_rect(mask: &mut [f32], width: u32, height: u32, rect: [f32; 4], radius: f32) {
    let [x0, y0, x1, y1] = rect;
    let radius = radius.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hx, hy) = ((x1 - x0) / 2.0 - radius, (y1 - y0) / 2.0 - radius);
    let rows = (y0.floor().max(0.0) as u32)..(y1.ceil().min(height as f32) as u32);
    for y in rows {
        for x in (x0.floor().max(0.0) as u32)..(x1.ceil().min(width as f32) as u32) {
            // Signed distance to the rounded rectangle, sampled at the pixel centre.
            let (px, py) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let (qx, qy) = (px.abs() - hx, py.abs() - hy);
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            let distance = outside + qx.max(qy).min(0.0) - radius;
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            let cell = &mut mask[(y * width + x) as usize];
            *cell = cell.max(coverage);
        }
    }
}

/// Coverage of an outline `radius` pixels wide around ink (`w` by `h` coverage values), from the exact
/// Euclidean distance to the nearest inked pixel (Felzenszwalb and Huttenlocher, "Distance
/// Transforms of Sampled Functions", 2004).
pub(crate) fn outline(coverage: &[f32], w: usize, h: usize, radius: f32) -> Vec<f32> {
    let mut distance: Vec<f32> = coverage
        .iter()
        .map(|&c| if c >= 0.5 { 0.0 } else { FAR })
        .collect();
    let mut column = vec![0.0f32; h.max(w)];
    let mut out = vec![0.0f32; h.max(w)];
    for x in 0..w {
        for y in 0..h {
            column[y] = distance[y * w + x];
        }
        squared_distances(&column[..h], &mut out[..h]);
        for y in 0..h {
            distance[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        column[..w].copy_from_slice(&distance[y * w..(y + 1) * w]);
        squared_distances(&column[..w], &mut out[..w]);
        distance[y * w..(y + 1) * w].copy_from_slice(&out[..w]);
    }
    distance
        .iter()
        .zip(coverage)
        .map(|(&d2, &c)| (radius + 0.5 - d2.sqrt()).clamp(0.0, 1.0).max(c))
        .collect()
}

/// Squared distance of pixels without ink, beyond any outline.
const FAR: f32 = 1e20;

/// One-dimensional squared distance transform of `f` into `d`: the lower envelope of the
/// parabolas rooted at the finite samples of `f`.
fn squared_distances(f: &[f32], d: &mut [f32]) {
    let lift = |q: usize| f64::from(f[q]) + (q * q) as f64;
    // Parabola roots and the left edge of the range where each is lowest.
    let mut roots: Vec<usize> = Vec::new();
    let mut edges: Vec<f64> = Vec::new();
    for q in (0..f.len()).filter(|&q| f[q] < FAR) {
        let mut edge = f64::NEG_INFINITY;
        while let Some(&p) = roots.last() {
            edge = (lift(q) - lift(p)) / (2.0 * (q as f64 - p as f64));
            if edge <= edges.last().copied().unwrap_or(f64::NEG_INFINITY) {
                roots.pop();
                edges.pop();
                edge = f64::NEG_INFINITY;
            } else {
                break;
            }
        }
        roots.push(q);
        edges.push(edge);
    }
    if roots.is_empty() {
        d.fill(FAR);
        return;
    }
    let mut k = 0;
    for (q, out) in d.iter_mut().enumerate() {
        while k + 1 < roots.len() && edges[k + 1] < q as f64 {
            k += 1;
        }
        let p = roots[k];
        let dq = q as f64 - p as f64;
        *out = (dq * dq + f64::from(f[p])) as f32;
    }
}

/// Blurs a coverage mask in place with a Gaussian of `sigma` pixels, approximated by three
/// box blurs (Wells, "Efficient synthesis of Gaussian filters by cascaded uniform filters",
/// 1986); outside the mask counts as empty.
pub(crate) fn blur(mask: &mut [f32], w: usize, h: usize, sigma: f32) {
    if sigma < 0.3 || w == 0 || h == 0 {
        return;
    }
    // Box width for three passes: sqrt(12 sigma^2 / 3 + 1).
    let radius = ((((4.0 * sigma * sigma + 1.0).sqrt()) - 1.0) / 2.0)
        .round()
        .max(1.0) as usize;
    let mut line = vec![0.0f32; w.max(h)];
    for _ in 0..3 {
        for y in 0..h {
            box_line(&mut mask[y * w..(y + 1) * w], radius, &mut line);
        }
        let mut column = vec![0.0f32; h];
        for x in 0..w {
            for y in 0..h {
                column[y] = mask[y * w + x];
            }
            box_line(&mut column, radius, &mut line);
            for y in 0..h {
                mask[y * w + x] = column[y];
            }
        }
    }
}

/// One box blur of `radius` over `values`, with zeros beyond the ends.
fn box_line(values: &mut [f32], radius: usize, scratch: &mut [f32]) {
    let n = values.len();
    let scale = 1.0 / (2 * radius + 1) as f32;
    let mut sum: f64 = values.iter().take(radius + 1).map(|&v| f64::from(v)).sum();
    for i in 0..n {
        scratch[i] = (sum as f32) * scale;
        if let Some(&entering) = values.get(i + radius + 1) {
            sum += f64::from(entering);
        }
        if i >= radius {
            sum -= f64::from(values[i - radius]);
        }
    }
    values.copy_from_slice(&scratch[..n]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distance_transform_matches_brute_force() {
        let (w, h) = (23usize, 17usize);
        let inked = |x: usize, y: usize| (x * 7 + y * 3).is_multiple_of(11) || (x == 12 && y > 4);
        let coverage: Vec<f32> = (0..w * h)
            .map(|i| if inked(i % w, i / w) { 1.0 } else { 0.0 })
            .collect();
        let outline = outline(&coverage, w, h, 2.0);
        for y in 0..h {
            for x in 0..w {
                let nearest = (0..w * h)
                    .filter(|&i| inked(i % w, i / w))
                    .map(|i| {
                        let (dx, dy) = ((i % w) as f32 - x as f32, (i / w) as f32 - y as f32);
                        (dx * dx + dy * dy).sqrt()
                    })
                    .fold(f32::INFINITY, f32::min);
                let expected = (2.5 - nearest).clamp(0.0, 1.0);
                assert!(
                    (outline[y * w + x] - expected).abs() < 1e-4,
                    "({x}, {y}): {} vs {expected}",
                    outline[y * w + x]
                );
            }
        }
        let mut empty = [0.0f32; 4];
        squared_distances(&[FAR; 4], &mut empty);
        assert_eq!(empty, [FAR; 4]);
        squared_distances(&[], &mut []);
    }

    #[test]
    fn blur_spreads_and_keeps_mass() {
        let (w, h) = (41, 41);
        let mut mask = vec![0.0f32; w * h];
        mask[20 * w + 20] = 1.0;
        blur(&mut mask, w, h, 3.0);
        let total: f32 = mask.iter().sum();
        assert!((total - 1.0).abs() < 1e-4, "{total}");
        assert!(mask[20 * w + 20] < 0.05 && mask[20 * w + 26] > 0.0);
        // Symmetric around the centre, and falling off with distance.
        assert!((mask[20 * w + 17] - mask[20 * w + 23]).abs() < 1e-6);
        assert!(mask[20 * w + 21] > mask[20 * w + 24]);
        let mut sharp = vec![0.5f32; 9];
        blur(&mut sharp, 3, 3, 0.1);
        assert_eq!(sharp, vec![0.5; 9], "tiny sigmas change nothing");
    }
}
