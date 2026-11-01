//! Shape clips (ADR-0028): a shape's coverage from the mask geometry at the output
//! resolution, filled with a colour or gradient, with an inner outline and a centred label
//! composed by the text engine. Strokes, open paths, contours and trims (ADR-0036) are
//! drawn from the shape's outline as a Bezier path through [`crate::vector`].

use fosscut_model::{Id, IdKind, Mask, MaskShape, Paint, ShapeContent, trim_path};

use crate::mask::coverage_exact;
use crate::raster::{outline, over};
use crate::text::{TextEngine, TextLayer, TextRaster, linear_color};

/// Paints `paint` over `pixels` (a `width` wide raster) by `coverage`; a gradient runs
/// across the shape's box, `box_size` pixels large at `pad` pixels from the raster's
/// top-left corner.
fn paint_over(
    pixels: &mut [[f32; 4]],
    coverage: &[f32],
    paint: &Paint,
    width: u32,
    pad: u32,
    box_size: (f32, f32),
) {
    let start = linear_color(paint.color.as_str());
    let end = paint
        .gradient
        .as_ref()
        .map(|g| (linear_color(g.to.as_str()), g));
    let (fw, fh) = box_size;
    for (index, (pixel, &c)) in pixels.iter_mut().zip(coverage).enumerate() {
        if c <= 0.0 {
            continue;
        }
        let color = match &end {
            None => start,
            Some((to, gradient)) => {
                let x = (index as u32 % width) as f32 - pad as f32;
                let y = (index as u32 / width) as f32 - pad as f32;
                let (u, v) = ((x + 0.5) / fw - 0.5, (y + 0.5) / fh - 0.5);
                let t = if gradient.radial {
                    ((u * u + v * v).sqrt() / std::f32::consts::FRAC_1_SQRT_2).min(1.0)
                } else {
                    // Along the direction, scaled so the box's far corners reach 0 and 1.
                    let (sin, cos) = (gradient.angle as f32).to_radians().sin_cos();
                    let reach = 0.5 * (cos.abs() + sin.abs());
                    ((u * cos + v * sin) / (2.0 * reach.max(1e-6)) + 0.5).clamp(0.0, 1.0)
                };
                let mut mixed = [0.0f32; 4];
                for channel in 0..4 {
                    mixed[channel] = start[channel] + (to[channel] - start[channel]) * t;
                }
                mixed
            }
        };
        over(pixel, color, c);
    }
}

/// Composes `shape` for a `canvas` (timeline pixels) at `scale` output pixels per canvas
/// pixel: premultiplied linear RGBA as large as the shape's box, and wider by a margin on
/// every side when its stroke reaches past the box; the layout box is the shape's box.
pub fn rasterize_shape(
    engine: &mut TextEngine,
    shape: &ShapeContent,
    canvas: (u32, u32),
    scale: f32,
) -> TextRaster {
    let size = |fraction: f64, side: u32| {
        ((fraction * f64::from(side) * f64::from(scale)).round() as u32).max(1)
    };
    let (w, h) = (size(shape.size[0], canvas.0), size(shape.size[1], canvas.1));
    let (fw, fh) = (w as f32, h as f32);
    // Lengths along strokes are fractions of the canvas height (ADR-0036).
    let unit = f64::from(canvas.1) * f64::from(scale);
    let stroke = shape
        .stroke
        .as_ref()
        .filter(|s| s.width > 0.0)
        .map(|s| (s, s.width * unit));
    let pad = stroke.map_or(0, |(s, px)| {
        crate::vector::stroke_reach(s, px).ceil() as u32 + 1
    });
    let (rw, rh) = (w + 2 * pad, h + 2 * pad);
    // The outline as a path in raster pixels, for shapes the vector renderer draws.
    let vector = shape.is_vector().then(|| {
        let mut path = shape.outline([f64::from(w), f64::from(h)]);
        path.apply_affine(kurbo::Affine::translate((
            f64::from(pad) + f64::from(w) / 2.0,
            f64::from(pad) + f64::from(h) / 2.0,
        )));
        path
    });
    // The area the shape covers: an open path's fill keeps to what its trims show.
    let coverage: Vec<f32> = match &vector {
        Some(path) => {
            let area = if shape.shape == MaskShape::Path && !shape.closed {
                trim_path(path, shape.trim_start, shape.trim_end, shape.trim_offset)
            } else {
                path.clone()
            };
            crate::vector::fill_coverage(&area, shape.fill_rule, (rw, rh))
        }
        None => {
            let mask = Mask {
                roundness: shape.roundness,
                points: shape.points.clone(),
                size: [1.0, 1.0],
                ..Mask::new(Id::new(IdKind::Mask, 0), shape.shape)
            };
            coverage_exact(&mask, (w, h))
                .data
                .iter()
                .map(|&c| f32::from(c) / 255.0)
                .collect()
        }
    };
    let mut pixels = vec![[0.0f32; 4]; (rw * rh) as usize];
    if let Some(fill) = &shape.fill {
        paint_over(&mut pixels, &coverage, fill, rw, pad, (fw, fh));
    }
    // The outline runs inside the edge: what lies within its width of the outside.
    if shape.outline.enabled && shape.outline.width > 0.0 {
        let width = (shape.outline.width as f32 * fw.min(fh)).max(0.5);
        // Past the raster's edges is outside too: measure on a padded copy.
        let pad = width.ceil() as usize + 1;
        let (pw, ph) = (rw as usize + 2 * pad, rh as usize + 2 * pad);
        let mut outside = vec![1.0f32; pw * ph];
        for y in 0..rh as usize {
            for x in 0..rw as usize {
                outside[(y + pad) * pw + x + pad] = 1.0 - coverage[y * rw as usize + x];
            }
        }
        let band = outline(&outside, pw, ph, width);
        let color = linear_color(shape.outline.color.as_str());
        for y in 0..rh as usize {
            for x in 0..rw as usize {
                let index = y * rw as usize + x;
                let edge = band[(y + pad) * pw + x + pad];
                over(&mut pixels[index], color, edge * coverage[index]);
            }
        }
    }
    // The stroke along the part of the outline its trims show (ADR-0036).
    if let (Some((stroke, px)), Some(path)) = (stroke, &vector) {
        let shown = trim_path(path, shape.trim_start, shape.trim_end, shape.trim_offset);
        let coverage = crate::vector::stroke_coverage(&shown, stroke, px, unit, (rw, rh));
        paint_over(&mut pixels, &coverage, &stroke.paint(), rw, pad, (fw, fh));
    }
    if let Some(label) = &shape.label {
        let text = engine.rasterize(&TextLayer::still(label.clone()), canvas, scale);
        if !text.is_empty() {
            // Centre the label's layout box where `label_at` puts it in the shape's box.
            let [bx, by, bw, bh] = text.layout_box;
            let (cx, cy) = (
                pad as f32 + fw / 2.0 * (1.0 + shape.label_at[0] as f32),
                pad as f32 + fh / 2.0 * (1.0 + shape.label_at[1] as f32),
            );
            let left = (cx - (bx + bw / 2.0)).round() as i32;
            let top = (cy - (by + bh / 2.0)).round() as i32;
            for ty in 0..text.height {
                for tx in 0..text.width {
                    let (px, py) = (left + tx as i32, top + ty as i32);
                    if px < 0 || py < 0 || px >= rw as i32 || py >= rh as i32 {
                        continue;
                    }
                    let source = text.at(tx, ty);
                    let target = &mut pixels[(py as u32 * rw + px as u32) as usize];
                    let alpha = source[3];
                    for channel in 0..3 {
                        target[channel] = source[channel] + target[channel] * (1.0 - alpha);
                    }
                    target[3] = alpha + target[3] * (1.0 - alpha);
                }
            }
        }
    }
    TextRaster {
        width: rw,
        height: rh,
        pixels,
        layout_box: [pad as f32, pad as f32, fw, fh],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_model::{Color, Gradient, MaskShape, ShapeOutline, TextContent, TextStyle};

    const CANVAS: (u32, u32) = (1920, 1080);

    #[test]
    fn shapes_fill_their_box_with_colours_gradients_outlines_and_labels() {
        let mut engine = TextEngine::new();
        let red = Color::parse("#FF0000").unwrap();
        let mut ellipse = ShapeContent::new(MaskShape::Ellipse, [0.2, 0.2], red.clone());
        let raster = rasterize_shape(&mut engine, &ellipse, CANVAS, 0.5);
        assert_eq!((raster.width, raster.height), (192, 108));
        let centre = raster.at(96, 54);
        assert!(centre[3] > 0.99 && centre[0] > 0.99 && centre[1] < 0.01);
        assert_eq!(raster.alpha(0, 0), 0.0, "outside the ellipse");

        // A full-canvas gradient background runs from one colour to the other.
        let mut background = ShapeContent::new(MaskShape::Rectangle, [1.0, 1.0], red.clone());
        background.fill.as_mut().unwrap().gradient = Some(Gradient {
            to: Color::parse("#0000FF").unwrap(),
            angle: 0.0,
            radial: false,
        });
        let raster = rasterize_shape(&mut engine, &background, CANVAS, 0.25);
        assert_eq!((raster.width, raster.height), (480, 270));
        let (left, right) = (raster.at(0, 135), raster.at(479, 135));
        assert!(left[0] > 0.95 && left[2] < 0.05, "{left:?}");
        assert!(right[2] > 0.95 && right[0] < 0.05, "{right:?}");
        assert!(
            raster.pixels.iter().all(|p| p[3] > 0.99),
            "covers everything"
        );
        background
            .fill
            .as_mut()
            .unwrap()
            .gradient
            .as_mut()
            .unwrap()
            .radial = true;
        let radial = rasterize_shape(&mut engine, &background, CANVAS, 0.25);
        assert!(radial.at(240, 135)[0] > 0.95 && radial.at(0, 0)[2] > 0.95);

        // An outline darkens the edge, not the middle.
        ellipse.outline = ShapeOutline {
            enabled: true,
            width: 0.1,
            ..ShapeOutline::default()
        };
        let outlined = rasterize_shape(&mut engine, &ellipse, CANVAS, 0.5);
        assert!(outlined.at(96, 4)[0] < 0.2, "black at the top edge");
        assert!(outlined.at(96, 54)[0] > 0.99, "red inside");

        // A label is drawn in the middle.
        let mut badge = ShapeContent::new(MaskShape::Rectangle, [0.3, 0.12], red);
        badge.roundness = 1.0;
        badge.label = Some(TextContent::styled(
            "NEW",
            TextStyle {
                size: 0.06,
                bold: true,
                ..TextStyle::default()
            },
        ));
        let labelled = rasterize_shape(&mut engine, &badge, CANVAS, 0.5);
        let white = labelled
            .pixels
            .iter()
            .filter(|p| p[1] > 0.9 && p[3] > 0.9)
            .count();
        assert!(white > 200, "white letters: {white}");
        assert_eq!(labelled.alpha(0, 0), 0.0, "round ends");
    }
}
