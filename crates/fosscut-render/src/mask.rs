//! Mask coverage (ADR-0011, item 4): a clip's masks rasterized on the CPU into one 8-bit
//! coverage image of its picture, which compositing multiplies into the layer's alpha.
//! Working on the CPU keeps every shape exact and testable without a GPU, and masks are
//! small: the longest side is at most [`MAX_SIDE`] pixels, sampled smoothly when drawn.
//!
//! Shapes are signed distance functions in units of the picture's height, so feather and
//! expansion mean the same on any picture. Distances to curves come from polygons: stars
//! and hearts are traced on their outlines, paths flattened from their Bezier segments.

use fosscut_model::{Mask, MaskMode, MaskShape};

/// Longest side of a coverage image, in pixels.
pub const MAX_SIDE: u32 = 768;

/// Coverage of a picture: 0 hides, 255 shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// One byte per pixel, row by row.
    pub data: Vec<u8>,
}

impl Coverage {
    /// The coverage at pixel (x, y), 0 to 1.
    pub fn at(&self, x: u32, y: u32) -> f32 {
        f32::from(self.data[(y * self.width + x) as usize]) / 255.0
    }

    /// The coverage at (u, v) in 0 to 1, bilinear; zero outside.
    pub fn sample(&self, u: f64, v: f64) -> f64 {
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) || self.width == 0 {
            return 0.0;
        }
        let x = (u * f64::from(self.width) - 0.5).clamp(0.0, f64::from(self.width - 1));
        let y = (v * f64::from(self.height) - 0.5).clamp(0.0, f64::from(self.height - 1));
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - f64::from(x0), y - f64::from(y0));
        let get = |x, y| f64::from(self.at(x, y));
        let top = get(x0, y0) * (1.0 - fx) + get(x1, y0) * fx;
        let bottom = get(x0, y1) * (1.0 - fx) + get(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// The size of the coverage image for a picture `size` pixels large.
pub fn coverage_size(size: (u32, u32)) -> (u32, u32) {
    let (w, h) = (size.0.max(1), size.1.max(1));
    let longest = w.max(h);
    if longest <= MAX_SIDE {
        return (w, h);
    }
    let scale = f64::from(MAX_SIDE) / f64::from(longest);
    (
        ((f64::from(w) * scale).round() as u32).max(1),
        ((f64::from(h) * scale).round() as u32).max(1),
    )
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    if edge1 <= edge0 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (pa, ba) = ([p[0] - a[0], p[1] - a[1]], [b[0] - a[0], b[1] - a[1]]);
    let length = ba[0] * ba[0] + ba[1] * ba[1];
    let t = if length > 0.0 {
        ((pa[0] * ba[0] + pa[1] * ba[1]) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (dx, dy) = (pa[0] - ba[0] * t, pa[1] - ba[1] * t);
    (dx * dx + dy * dy).sqrt()
}

/// Signed distance to a closed polygon, negative inside (even-odd rule).
fn polygon_distance(p: [f64; 2], points: &[[f64; 2]]) -> f64 {
    let mut distance = f64::INFINITY;
    let mut inside = false;
    let n = points.len();
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        distance = distance.min(segment_distance(p, a, b));
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    if inside { -distance } else { distance }
}

/// A five-pointed star, point up, inscribed in the unit box.
fn star() -> Vec<[f64; 2]> {
    // The inner radius of a regular star: the outer radius times sin(18) / sin(126).
    let inner = (18f64.to_radians().sin()) / (126f64.to_radians().sin());
    (0..10)
        .map(|i| {
            let radius = if i % 2 == 0 { 1.0 } else { inner };
            let angle = (-90.0 + 36.0 * f64::from(i)).to_radians();
            [radius * angle.cos(), radius * angle.sin()]
        })
        .collect()
}

/// A heart, point down, inscribed in the unit box: the classic parametric heart curve.
fn heart() -> Vec<[f64; 2]> {
    let raw: Vec<[f64; 2]> = (0..96)
        .map(|i| {
            let t = f64::from(i) / 96.0 * std::f64::consts::TAU;
            let x = 16.0 * t.sin().powi(3);
            let y =
                13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            [x, -y]
        })
        .collect();
    let (min_y, max_y) = raw
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p[1]), hi.max(p[1]))
        });
    let middle = (min_y + max_y) / 2.0;
    let half = (max_y - min_y) / 2.0;
    raw.into_iter()
        .map(|[x, y]| [x / 16.0, (y - middle) / half])
        .collect()
}

/// The outline of a path mask in its box (-1 to 1), with Bezier segments flattened.
fn path_outline(mask: &Mask) -> Vec<[f64; 2]> {
    let points = &mask.points;
    let mut outline = Vec::new();
    for i in 0..points.len() {
        let (a, b) = (&points[i], &points[(i + 1) % points.len()]);
        outline.push(a.at);
        if a.out_handle.is_none() && b.in_handle.is_none() {
            continue;
        }
        let add = |p: [f64; 2], d: Option<[f64; 2]>| d.map_or(p, |d| [p[0] + d[0], p[1] + d[1]]);
        let (c1, c2) = (add(a.at, a.out_handle), add(b.at, b.in_handle));
        for step in 1..16 {
            let t = f64::from(step) / 16.0;
            let r = 1.0 - t;
            let point = |k: usize| {
                r * r * r * a.at[k]
                    + 3.0 * r * r * t * c1[k]
                    + 3.0 * r * t * t * c2[k]
                    + t * t * t * b.at[k]
            };
            outline.push([point(0), point(1)]);
        }
    }
    outline
}

/// What a mask needs besides itself: its outline, or its text's coverage.
enum Prepared {
    Outline(Vec<[f64; 2]>),
    Text(Option<Coverage>),
    None,
}

/// Coverage of one mask at picture point `p` (0 to 1), before inversion. `aspect` is the
/// picture's width over its height and `pixel` one coverage pixel in height units.
fn mask_at(mask: &Mask, prepared: &Prepared, p: [f64; 2], aspect: f64, pixel: f64) -> f64 {
    if mask.shape == MaskShape::Brush {
        let point = [p[0] * aspect, p[1]];
        let stroke = |erase: bool| -> f64 {
            let mut best: f64 = 0.0;
            for s in mask.strokes.iter().filter(|s| s.erase == erase) {
                let pts: Vec<[f64; 2]> = s.points.iter().map(|q| [q[0] * aspect, q[1]]).collect();
                let d = if pts.len() == 1 {
                    segment_distance(point, pts[0], pts[0])
                } else {
                    pts.windows(2)
                        .map(|w| segment_distance(point, w[0], w[1]))
                        .fold(f64::INFINITY, f64::min)
                };
                let width = (mask.feather * s.radius * 2.0).max(pixel);
                let edge = d - s.radius * (1.0 + mask.expansion);
                best = best.max(1.0 - smoothstep(-width / 2.0, width / 2.0, edge));
            }
            best
        };
        return stroke(false) * (1.0 - stroke(true));
    }
    let (dx, dy) = ((p[0] - mask.center[0]) * aspect, p[1] - mask.center[1]);
    let (sin, cos) = mask.rotation.to_radians().sin_cos();
    let (x, y) = (dx * cos + dy * sin, -dx * sin + dy * cos);
    let (hx, hy) = (mask.size[0] * aspect / 2.0, mask.size[1] / 2.0);
    let unit = hx.min(hy).max(pixel);
    let distance = match mask.shape {
        MaskShape::Rectangle => {
            let r = mask.roundness * hx.min(hy);
            let (qx, qy) = (x.abs() - (hx - r), y.abs() - (hy - r));
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            outside + qx.max(qy).min(0.0) - r
        }
        MaskShape::Ellipse => {
            let k = ((x / hx.max(1e-9)).powi(2) + (y / hy.max(1e-9)).powi(2)).sqrt();
            (k - 1.0) * hx.min(hy)
        }
        MaskShape::Linear => y,
        MaskShape::Mirror => y.abs() - hy,
        MaskShape::Star | MaskShape::Heart | MaskShape::Path => match prepared {
            Prepared::Outline(outline) if outline.len() >= 3 => {
                let scaled: Vec<[f64; 2]> =
                    outline.iter().map(|q| [q[0] * hx, q[1] * hy]).collect();
                polygon_distance([x, y], &scaled)
            }
            _ => f64::INFINITY,
        },
        MaskShape::Text => {
            let Prepared::Text(Some(text)) = prepared else {
                return 0.0;
            };
            // The text fills the box at its own proportions, centred.
            let (tw, th) = (f64::from(text.width), f64::from(text.height));
            let fit = ((2.0 * hx) / tw).min((2.0 * hy) / th);
            let (u, v) = (x / (tw * fit) + 0.5, y / (th * fit) + 0.5);
            return text.sample(u, v);
        }
        MaskShape::Brush => f64::INFINITY,
    };
    let width = (mask.feather * unit).max(pixel);
    let edge = distance - mask.expansion * unit;
    1.0 - smoothstep(-width / 2.0, width / 2.0, edge)
}

/// The combined coverage of `masks` over a picture `size` pixels large, or `None` when no
/// mask is enabled. `text` rasterizes the text of a text mask into coverage. Masks combine
/// in order: `add` shows what the mask covers too, `subtract` hides it, `intersect` keeps
/// only what both cover. A first mask that subtracts starts from everything shown.
pub fn rasterize(
    masks: &[Mask],
    size: (u32, u32),
    text: &mut dyn FnMut(&str) -> Option<Coverage>,
) -> Option<Coverage> {
    let enabled: Vec<&Mask> = masks.iter().filter(|m| m.enabled).collect();
    let first = enabled.first()?;
    let (width, height) = coverage_size(size);
    let aspect = f64::from(size.0.max(1)) / f64::from(size.1.max(1));
    let pixel = 1.0 / f64::from(height);
    let prepared: Vec<Prepared> = enabled
        .iter()
        .map(|m| match m.shape {
            MaskShape::Star => Prepared::Outline(star()),
            MaskShape::Heart => Prepared::Outline(heart()),
            MaskShape::Path => Prepared::Outline(path_outline(m)),
            MaskShape::Text => Prepared::Text(m.text.as_deref().and_then(&mut *text)),
            _ => Prepared::None,
        })
        .collect();
    let start = if first.mode == MaskMode::Add {
        0.0
    } else {
        1.0
    };
    let mut data = Vec::with_capacity((width * height) as usize);
    for row in 0..height {
        for column in 0..width {
            let p = [
                (f64::from(column) + 0.5) / f64::from(width),
                (f64::from(row) + 0.5) / f64::from(height),
            ];
            let mut c: f64 = start;
            for (mask, prepared) in enabled.iter().zip(&prepared) {
                let mut m = mask_at(mask, prepared, p, aspect, pixel);
                if mask.invert {
                    m = 1.0 - m;
                }
                c = match mask.mode {
                    MaskMode::Add => c.max(m),
                    MaskMode::Subtract => c * (1.0 - m),
                    MaskMode::Intersect => c.min(m),
                };
            }
            data.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Some(Coverage {
        width,
        height,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_model::{BrushStroke, Id, IdKind, PathPoint};

    fn mask(shape: MaskShape) -> Mask {
        Mask::new(Id::new(IdKind::Mask, 1), shape)
    }

    fn cover(masks: &[Mask], size: (u32, u32)) -> Coverage {
        rasterize(masks, size, &mut |_| None).unwrap()
    }

    /// Coverage at picture point (u, v).
    fn at(c: &Coverage, u: f64, v: f64) -> f32 {
        c.at(
            ((u * f64::from(c.width)) as u32).min(c.width - 1),
            ((v * f64::from(c.height)) as u32).min(c.height - 1),
        )
    }

    #[test]
    fn shapes_cover_their_area() {
        let size = (160, 90);
        let rect = cover(&[mask(MaskShape::Rectangle)], size);
        assert_eq!((rect.width, rect.height), size);
        assert_eq!(at(&rect, 0.5, 0.5), 1.0);
        assert_eq!(at(&rect, 0.27, 0.5), 1.0, "inside the left edge at 0.25");
        assert_eq!(at(&rect, 0.2, 0.5), 0.0);
        assert_eq!(at(&rect, 0.5, 0.2), 0.0);
        let ellipse = cover(&[mask(MaskShape::Ellipse)], size);
        assert_eq!(at(&ellipse, 0.5, 0.5), 1.0);
        assert_eq!(
            at(&ellipse, 0.27, 0.27),
            0.0,
            "a corner of its box is outside"
        );
        let linear = cover(&[mask(MaskShape::Linear)], size);
        assert_eq!(
            (at(&linear, 0.5, 0.2), at(&linear, 0.5, 0.8)),
            (1.0, 0.0),
            "above the line"
        );
        let mirror = cover(
            &[Mask {
                size: [1.0, 0.2],
                ..mask(MaskShape::Mirror)
            }],
            size,
        );
        assert_eq!((at(&mirror, 0.1, 0.5), at(&mirror, 0.1, 0.3)), (1.0, 0.0));
        let star = cover(
            &[Mask {
                size: [0.9, 0.9],
                ..mask(MaskShape::Star)
            }],
            (100, 100),
        );
        assert_eq!(at(&star, 0.5, 0.5), 1.0);
        assert_eq!(at(&star, 0.5, 0.08), 1.0, "the top point");
        assert_eq!(at(&star, 0.2, 0.15), 0.0, "between the points");
        let heart = cover(
            &[Mask {
                size: [0.9, 0.9],
                ..mask(MaskShape::Heart)
            }],
            (100, 100),
        );
        assert_eq!(at(&heart, 0.5, 0.5), 1.0);
        assert_eq!(at(&heart, 0.5, 0.1), 0.0, "the dip between the lobes");
        assert_eq!(at(&heart, 0.3, 0.2), 1.0, "a lobe");
    }

    #[test]
    fn rotation_feather_expansion_and_corners() {
        let size = (100, 100);
        let thin = Mask {
            size: [0.8, 0.1],
            ..mask(MaskShape::Rectangle)
        };
        let turned = cover(
            &[Mask {
                rotation: 90.0,
                ..thin.clone()
            }],
            size,
        );
        assert_eq!(
            (at(&turned, 0.5, 0.15), at(&turned, 0.15, 0.5)),
            (1.0, 0.0),
            "now vertical"
        );
        let soft = cover(
            &[Mask {
                feather: 1.0,
                ..mask(MaskShape::Ellipse)
            }],
            size,
        );
        let edge = at(&soft, 0.75, 0.5);
        assert!(edge > 0.3 && edge < 0.7, "half way at the edge: {edge}");
        let grown = cover(
            &[Mask {
                expansion: 0.5,
                ..mask(MaskShape::Rectangle)
            }],
            size,
        );
        assert_eq!(at(&grown, 0.15, 0.5), 1.0, "past the edge when expanded");
        let round = cover(
            &[Mask {
                roundness: 1.0,
                ..mask(MaskShape::Rectangle)
            }],
            size,
        );
        assert_eq!(at(&round, 0.26, 0.26), 0.0, "a rounded corner");
        assert_eq!(
            at(&cover(&[mask(MaskShape::Rectangle)], size), 0.26, 0.26),
            1.0
        );
    }

    #[test]
    fn masks_combine_in_order_and_invert() {
        let size = (100, 100);
        let left = Mask {
            center: [0.25, 0.5],
            size: [0.5, 1.0],
            ..mask(MaskShape::Rectangle)
        };
        let middle = Mask {
            center: [0.5, 0.5],
            size: [0.4, 1.0],
            ..mask(MaskShape::Rectangle)
        };
        let added = cover(
            &[
                left.clone(),
                Mask {
                    mode: MaskMode::Add,
                    ..middle.clone()
                },
            ],
            size,
        );
        assert_eq!(
            (
                at(&added, 0.1, 0.5),
                at(&added, 0.6, 0.5),
                at(&added, 0.9, 0.5)
            ),
            (1.0, 1.0, 0.0)
        );
        let cut = cover(
            &[
                left.clone(),
                Mask {
                    mode: MaskMode::Subtract,
                    ..middle.clone()
                },
            ],
            size,
        );
        assert_eq!((at(&cut, 0.1, 0.5), at(&cut, 0.4, 0.5)), (1.0, 0.0));
        let both = cover(
            &[
                left.clone(),
                Mask {
                    mode: MaskMode::Intersect,
                    ..middle.clone()
                },
            ],
            size,
        );
        assert_eq!(
            (
                at(&both, 0.1, 0.5),
                at(&both, 0.4, 0.5),
                at(&both, 0.6, 0.5)
            ),
            (0.0, 1.0, 0.0)
        );
        let outside = cover(
            &[Mask {
                invert: true,
                ..left.clone()
            }],
            size,
        );
        assert_eq!((at(&outside, 0.1, 0.5), at(&outside, 0.9, 0.5)), (0.0, 1.0));
        let hole = cover(
            &[Mask {
                mode: MaskMode::Subtract,
                ..middle
            }],
            size,
        );
        assert_eq!(
            (at(&hole, 0.1, 0.5), at(&hole, 0.5, 0.5)),
            (1.0, 0.0),
            "a first subtract starts full"
        );
        assert!(
            rasterize(
                &[Mask {
                    enabled: false,
                    ..left
                }],
                size,
                &mut |_| None
            )
            .is_none()
        );
        assert!(rasterize(&[], size, &mut |_| None).is_none());
    }

    #[test]
    fn paths_brushes_and_text() {
        let size = (100, 100);
        let triangle = Mask {
            size: [1.0, 1.0],
            points: vec![
                PathPoint {
                    at: [0.0, -0.8],
                    in_handle: None,
                    out_handle: None,
                },
                PathPoint {
                    at: [0.8, 0.8],
                    in_handle: None,
                    out_handle: None,
                },
                PathPoint {
                    at: [-0.8, 0.8],
                    in_handle: None,
                    out_handle: None,
                },
            ],
            ..mask(MaskShape::Path)
        };
        let shape = cover(std::slice::from_ref(&triangle), size);
        assert_eq!((at(&shape, 0.5, 0.6), at(&shape, 0.15, 0.2)), (1.0, 0.0));
        let mut curved = triangle;
        curved.points[1].out_handle = Some([0.0, 0.6]);
        curved.points[2].in_handle = Some([0.0, 0.6]);
        let bulge = cover(&[curved], size);
        assert_eq!(
            at(&bulge, 0.5, 0.95),
            1.0,
            "the curve bulges below the straight edge"
        );
        assert_eq!(at(&shape, 0.5, 0.95), 0.0);

        let brush = Mask {
            strokes: vec![
                BrushStroke {
                    points: vec![[0.2, 0.5], [0.8, 0.5]],
                    radius: 0.05,
                    erase: false,
                },
                BrushStroke {
                    points: vec![[0.5, 0.5]],
                    radius: 0.08,
                    erase: true,
                },
            ],
            ..mask(MaskShape::Brush)
        };
        let painted = cover(&[brush], size);
        assert_eq!((at(&painted, 0.3, 0.5), at(&painted, 0.3, 0.6)), (1.0, 0.0));
        assert_eq!(at(&painted, 0.5, 0.5), 0.0, "erased in the middle");

        let block = Coverage {
            width: 4,
            height: 2,
            data: vec![255; 8],
        };
        let word = Mask {
            text: Some("A".into()),
            size: [0.8, 0.8],
            ..mask(MaskShape::Text)
        };
        let text = rasterize(std::slice::from_ref(&word), size, &mut |t| {
            assert_eq!(t, "A");
            Some(block.clone())
        })
        .unwrap();
        assert_eq!(at(&text, 0.5, 0.5), 1.0);
        assert_eq!(
            at(&text, 0.5, 0.2),
            0.0,
            "wide text fills the box's width, not its height"
        );
        let missing = rasterize(&[word], size, &mut |_| None).unwrap();
        assert_eq!(at(&missing, 0.5, 0.5), 0.0);
    }

    #[test]
    fn big_pictures_get_a_smaller_coverage_image() {
        assert_eq!(coverage_size((3840, 2160)), (768, 432));
        assert_eq!(coverage_size((320, 180)), (320, 180));
        assert_eq!(coverage_size((0, 0)), (1, 1));
        let c = Coverage {
            width: 2,
            height: 1,
            data: vec![0, 255],
        };
        assert!((c.sample(0.5, 0.5) - 0.5).abs() < 1e-9);
        assert_eq!(c.sample(1.5, 0.5), 0.0);
    }
}
