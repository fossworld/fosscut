//! Vector shapes and drawings (ADR-0036): fills and strokes of shape outlines rasterized
//! with tiny-skia into antialiased coverage, painted in linear light like every other shape,
//! clips placed along shape paths, and SVG drawings rendered by resvg at the size they are
//! shown, so they stay sharp at any output resolution.

use fosscut_model::{Clip, FillRule, LineCap, LineJoin, Project, ShapeStroke, Transform};
use fosscut_time::Flicks;
use kurbo::{BezPath, PathEl};
use resvg::{tiny_skia, usvg};

use crate::text::TextRaster;

/// The longest side a drawing is rendered at, in pixels: the largest texture every GPU
/// takes.
pub const MAX_DRAWING_SIDE: u32 = 8192;

/// `path` (in raster pixels) as a tiny-skia path, or `None` when it is empty.
fn skia_path(path: &BezPath) -> Option<tiny_skia::Path> {
    let mut builder = tiny_skia::PathBuilder::new();
    let point = |p: kurbo::Point| (p.x as f32, p.y as f32);
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                let (x, y) = point(p);
                builder.move_to(x, y);
            }
            PathEl::LineTo(p) => {
                let (x, y) = point(p);
                builder.line_to(x, y);
            }
            PathEl::QuadTo(c, p) => {
                let ((cx, cy), (x, y)) = (point(c), point(p));
                builder.quad_to(cx, cy, x, y);
            }
            PathEl::CurveTo(c1, c2, p) => {
                let ((ax, ay), (bx, by), (x, y)) = (point(c1), point(c2), point(p));
                builder.cubic_to(ax, ay, bx, by, x, y);
            }
            PathEl::ClosePath => builder.close(),
        }
    }
    builder.finish()
}

/// Antialiased coverage, 0 to 1, of `path` filled by `rule` over a `size` raster.
pub(crate) fn fill_coverage(path: &BezPath, rule: FillRule, size: (u32, u32)) -> Vec<f32> {
    let mut coverage = vec![0.0f32; (size.0 * size.1) as usize];
    let (Some(path), Some(mut mask)) = (skia_path(path), tiny_skia::Mask::new(size.0, size.1))
    else {
        return coverage;
    };
    let rule = match rule {
        FillRule::EvenOdd => tiny_skia::FillRule::EvenOdd,
        FillRule::Nonzero => tiny_skia::FillRule::Winding,
    };
    mask.fill_path(&path, rule, true, tiny_skia::Transform::identity());
    for (c, &m) in coverage.iter_mut().zip(mask.data()) {
        *c = f32::from(m) / 255.0;
    }
    coverage
}

/// How far a stroke `width` pixels wide can reach past its path, in pixels: half its width,
/// further at miter corners and square caps.
pub(crate) fn stroke_reach(stroke: &ShapeStroke, width: f64) -> f64 {
    let mut reach: f64 = 0.5;
    if stroke.join == LineJoin::Miter {
        reach = reach.max(0.5 * stroke.miter_limit.max(1.0));
    }
    if stroke.cap == LineCap::Square {
        reach = reach.max(std::f64::consts::FRAC_1_SQRT_2);
    }
    width * reach
}

/// Antialiased coverage, 0 to 1, of `stroke` along `path` over a `size` raster, its width
/// `width` pixels and its dash lengths in units of `unit` pixels (the canvas height at the
/// output's scale).
pub(crate) fn stroke_coverage(
    path: &BezPath,
    stroke: &ShapeStroke,
    width: f64,
    unit: f64,
    size: (u32, u32),
) -> Vec<f32> {
    let mut coverage = vec![0.0f32; (size.0 * size.1) as usize];
    if width <= 0.0 {
        return coverage;
    }
    let Some(mut path) = skia_path(path) else {
        return coverage;
    };
    if !stroke.dashes.is_empty() {
        // An odd list repeats to make an even one, as in SVG.
        let mut lengths: Vec<f32> = stroke.dashes.iter().map(|d| (d * unit) as f32).collect();
        if lengths.len() % 2 == 1 {
            lengths.extend(lengths.clone());
        }
        match tiny_skia::StrokeDash::new(lengths, (stroke.dash_offset * unit) as f32)
            .and_then(|dash| path.dash(&dash, 1.0))
        {
            Some(dashed) => path = dashed,
            None => return coverage,
        }
    }
    let skia = tiny_skia::Stroke {
        width: width as f32,
        miter_limit: stroke.miter_limit.max(1.0) as f32,
        line_cap: match stroke.cap {
            LineCap::Butt => tiny_skia::LineCap::Butt,
            LineCap::Round => tiny_skia::LineCap::Round,
            LineCap::Square => tiny_skia::LineCap::Square,
        },
        line_join: match stroke.join {
            LineJoin::Miter => tiny_skia::LineJoin::Miter,
            LineJoin::Round => tiny_skia::LineJoin::Round,
            LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
        },
        dash: None,
    };
    let (Some(outline), Some(mut mask)) = (
        path.stroke(&skia, 1.0),
        tiny_skia::Mask::new(size.0, size.1),
    ) else {
        return coverage;
    };
    mask.fill_path(
        &outline,
        tiny_skia::FillRule::Winding,
        true,
        tiny_skia::Transform::identity(),
    );
    for (c, &m) in coverage.iter_mut().zip(mask.data()) {
        *c = f32::from(m) / 255.0;
    }
    coverage
}

/// Moves `transform` onto the path `clip` follows at `at` from its start (ADR-0036): its
/// position becomes the point on the path plus the follow's offset, and the path's
/// direction turns it when it orients. Unchanged when it follows nothing.
pub(crate) fn follow(project: &Project, clip: &Clip, at: Flicks, transform: &mut Transform) {
    if let Some(placed) = fosscut_model::follow_placement(project, clip, at) {
        transform.position = placed.position;
        transform.rotation += placed.rotation;
    }
}

/// A parsed SVG drawing (ADR-0036), ready to render at any size.
pub struct VectorImage {
    tree: usvg::Tree,
    key: u64,
}

impl std::fmt::Debug for VectorImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (width, height) = self.size();
        f.debug_struct("VectorImage")
            .field("width", &width)
            .field("height", &height)
            .field("key", &self.key)
            .finish()
    }
}

impl VectorImage {
    /// Parses SVG data. Text and embedded raster images are left out: FossCut draws only
    /// with its own fonts, and drawings are paths.
    pub fn parse(data: &[u8]) -> Result<VectorImage, String> {
        let tree = usvg::Tree::from_data(data, &usvg::Options::default())
            .map_err(|e| format!("not a usable SVG drawing: {e}"))?;
        // A key of the content, so cached renders of equal drawings are shared and those
        // of different projects' assets with the same id are not.
        let mut hasher = std::hash::DefaultHasher::new();
        std::hash::Hash::hash(data, &mut hasher);
        Ok(VectorImage {
            tree,
            key: std::hash::Hasher::finish(&hasher),
        })
    }

    /// Its size in pixels, as the drawing states it.
    pub fn size(&self) -> (f64, f64) {
        let size = self.tree.size();
        (f64::from(size.width()), f64::from(size.height()))
    }

    /// What distinguishes its content.
    pub fn key(&self) -> u64 {
        self.key
    }

    /// The drawing rendered to fill `width` x `height` pixels (stretched when the
    /// proportions differ) as premultiplied linear RGBA.
    pub fn rasterize(&self, width: u32, height: u32) -> TextRaster {
        let (width, height) = (
            width.clamp(1, MAX_DRAWING_SIDE),
            height.clamp(1, MAX_DRAWING_SIDE),
        );
        let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
            return TextRaster::empty();
        };
        let (w, h) = self.size();
        let transform = tiny_skia::Transform::from_scale(
            (f64::from(width) / w.max(1e-9)) as f32,
            (f64::from(height) / h.max(1e-9)) as f32,
        );
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        let decode = srgb_table();
        let pixels = pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let alpha = f32::from(p[3]) / 255.0;
                if alpha <= 0.0 {
                    return [0.0; 4];
                }
                // Straight sRGB from the premultiplied bytes, to linear light, premultiplied.
                let channel = |v: u8| {
                    let straight = (f32::from(v) / 255.0 / alpha).clamp(0.0, 1.0);
                    decode[(straight * 4095.0).round() as usize] * alpha
                };
                [channel(p[0]), channel(p[1]), channel(p[2]), alpha]
            })
            .collect();
        TextRaster {
            width,
            height,
            pixels,
            layout_box: [0.0, 0.0, width as f32, height as f32],
        }
    }
}

/// sRGB-encoded values from 0 to 1 in 4096 steps, decoded to linear light.
fn srgb_table() -> Vec<f32> {
    (0..4096)
        .map(|i| {
            let v = i as f32 / 4095.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_model::Color;

    fn line(from: (f64, f64), to: (f64, f64)) -> BezPath {
        let mut path = BezPath::new();
        path.move_to(from);
        path.line_to(to);
        path
    }

    fn inked(coverage: &[f32]) -> usize {
        coverage.iter().filter(|c| **c > 0.5).count()
    }

    #[test]
    fn strokes_cover_their_width_caps_and_dashes() {
        let size = (100, 40);
        let stroke = ShapeStroke::new(Color::default(), 0.1);
        let path = line((20.0, 20.0), (80.0, 20.0));
        let butt = stroke_coverage(&path, &stroke, 10.0, 100.0, size);
        // 60 long and 10 wide.
        assert!((inked(&butt) as i64 - 600).abs() <= 20, "{}", inked(&butt));
        assert!(butt[20 * 100 + 50] > 0.99 && butt[20 * 100 + 15] == 0.0);
        let round = stroke_coverage(
            &path,
            &ShapeStroke {
                cap: LineCap::Round,
                ..stroke.clone()
            },
            10.0,
            100.0,
            size,
        );
        // Two half circles of radius 5 more.
        let extra = inked(&round) as f64 - inked(&butt) as f64;
        assert!(
            (extra - std::f64::consts::PI * 25.0).abs() < 12.0,
            "{extra}"
        );
        assert!(round[20 * 100 + 17] > 0.5, "past the end");
        let dashed = stroke_coverage(
            &path,
            &ShapeStroke {
                dashes: vec![0.1],
                ..stroke.clone()
            },
            10.0,
            100.0,
            size,
        );
        // Dashes of 10 with gaps of 10: half of it.
        assert!(
            (inked(&dashed) as i64 - 300).abs() <= 20,
            "{}",
            inked(&dashed)
        );
        assert!(
            stroke_coverage(&path, &stroke, 0.0, 100.0, size)
                .iter()
                .all(|c| *c == 0.0)
        );
        assert_eq!(stroke_reach(&stroke, 10.0), 20.0, "miter limit 4");
    }

    #[test]
    fn fills_follow_their_rule() {
        let mut path = BezPath::new();
        for (x0, x1) in [(10.0, 90.0), (30.0, 70.0)] {
            path.move_to((x0, x0));
            path.line_to((x1, x0));
            path.line_to((x1, x1));
            path.line_to((x0, x1));
            path.close_path();
        }
        let even = fill_coverage(&path, FillRule::EvenOdd, (100, 100));
        let nonzero = fill_coverage(&path, FillRule::Nonzero, (100, 100));
        assert_eq!(even[50 * 100 + 50], 0.0, "a hole");
        assert!(nonzero[50 * 100 + 50] > 0.99, "same direction fills");
        assert!(even[20 * 100 + 20] > 0.99);
    }

    #[test]
    fn drawings_render_sharp_at_the_size_asked() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10" viewBox="0 0 20 10">
            <rect x="0" y="0" width="10" height="10" fill="#ff0000"/>
        </svg>"##;
        let image = VectorImage::parse(svg).unwrap();
        assert_eq!(image.size(), (20.0, 10.0));
        let small = image.rasterize(20, 10);
        let large = image.rasterize(400, 200);
        assert_eq!((large.width, large.height), (400, 200));
        assert!(large.at(100, 100)[0] > 0.99 && large.at(300, 100)[3] == 0.0);
        // The edge stays one pixel wide at every size.
        let soft = |r: &TextRaster| {
            (0..r.width)
                .filter(|x| {
                    let a = r.at(*x, r.height / 2)[3];
                    a > 0.01 && a < 0.99
                })
                .count()
        };
        assert!(soft(&small) <= 1 && soft(&large) <= 1);
        assert!(VectorImage::parse(b"not svg").is_err());
        let blue = String::from_utf8_lossy(svg).replace("#ff0000", "#0000ff");
        assert_ne!(
            image.key(),
            VectorImage::parse(blue.as_bytes()).unwrap().key()
        );
    }
}
