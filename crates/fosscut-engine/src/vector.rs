//! SVG drawings (ADR-0036): probing `.svg` files when they are imported, loading drawings
//! for the renderer, which draws them at the size they show, and reading a drawing's paths
//! into the parts `vector.explode` turns into shape clips. Front ends call [`with_parts`]
//! before running a request, so agents give `vector.explode` only the asset.

use std::path::Path;
use std::sync::Arc;

use fosscut_core::{ApiError, Request, codes};
use fosscut_model::{
    AssetKind, Color, FillRule, Gradient, Id, LineCap, LineJoin, MaskShape, MediaInfo, Paint,
    Project, ShapeContent, ShapeStroke, VectorInfo, VideoInfo,
};
use fosscut_render::DecoderFrames;
use fosscut_render::vector::VectorImage;
use serde_json::{Value, json};

use crate::EngineError;
use crate::import::Probed;

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::VECTOR_INVALID, message)
}

/// Reads an SVG file and checks it parses as a drawing: its size, view box and number of
/// paths.
pub(crate) fn probe(file: &Path) -> Result<Probed, ApiError> {
    let data = std::fs::read(file).map_err(|e| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("cannot read {}: {e}", file.display()),
        )
    })?;
    let tree = parse(&data).map_err(|e| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("{} is not a usable SVG drawing: {e}", file.display()),
        )
        .with_hint("FossCut reads SVG drawings (.svg) with a width and height or a viewBox.")
    })?;
    let size = tree.size();
    let (width, height) = (f64::from(size.width()), f64::from(size.height()));
    Ok(Probed {
        kind: AssetKind::Vector,
        info: MediaInfo {
            container: Some("svg".into()),
            video: Some(VideoInfo {
                width: (width.round() as u32).max(1),
                height: (height.round() as u32).max(1),
                rate: None,
                codec: Some("svg".into()),
                rotation: 0,
            }),
            vector: Some(VectorInfo {
                view_box: view_box(&data).unwrap_or([0.0, 0.0, width, height]),
                paths: count_paths(tree.root()),
            }),
            ..MediaInfo::default()
        },
    })
}

fn parse(data: &[u8]) -> Result<usvg::Tree, String> {
    usvg::Tree::from_data(data, &usvg::Options::default()).map_err(|e| e.to_string())
}

/// The `viewBox` of the root `svg` element, when it has one.
fn view_box(data: &[u8]) -> Option<[f64; 4]> {
    let text = std::str::from_utf8(data).ok()?;
    let root = &text[text.find("<svg")?..];
    let tag = &root[..root.find('>')?];
    let at = tag.find("viewBox")?;
    let rest = tag[at + "viewBox".len()..].trim_start().strip_prefix('=')?;
    let rest = rest.trim_start();
    let quote = rest.chars().next()?;
    let body = &rest[1..];
    let value = &body[..body.find(quote)?];
    let numbers: Vec<f64> = value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    match numbers[..] {
        [x, y, w, h] if w > 0.0 && h > 0.0 => Some([x, y, w, h]),
        _ => None,
    }
}

fn count_paths(group: &usvg::Group) -> u32 {
    group
        .children()
        .iter()
        .map(|node| match node {
            usvg::Node::Group(g) => count_paths(g),
            usvg::Node::Path(p) if p.is_visible() => 1,
            _ => 0,
        })
        .sum()
}

/// Loads the drawing of `asset` into `frames` when it is a vector asset, so the renderer
/// draws it instead of decoding frames. False for other assets.
pub(crate) fn open_drawing(
    project: &Project,
    bundle: &Path,
    asset: &Id,
    frames: &mut DecoderFrames,
) -> Result<bool, EngineError> {
    if project.media.get(asset).map(|a| a.kind) != Some(AssetKind::Vector) {
        return Ok(false);
    }
    if frames.has_vector(asset) {
        return Ok(true);
    }
    let path = crate::export::resolve(project, bundle, asset)?;
    let failed = |message: String| EngineError::Failed {
        code: codes::VECTOR_INVALID.into(),
        message,
    };
    let data = std::fs::read(&path)
        .map_err(|e| failed(format!("cannot read the drawing {}: {e}", path.display())))?;
    let image =
        VectorImage::parse(&data).map_err(|e| failed(format!("{}: {e}", path.display())))?;
    frames.insert_vector(asset.clone(), Arc::new(image));
    Ok(true)
}

/// Fills in the parts of every `vector.explode` command of `request` that gives none, from
/// its drawing's file; `project` is called for the project only when one needs them.
pub fn with_parts(
    bundle: &Path,
    mut request: Request,
    project: impl FnOnce() -> Result<Project, ApiError>,
) -> Result<Request, ApiError> {
    let needs = |args: &Value| args.get("parts").is_none_or(Value::is_null);
    if !request
        .commands
        .iter()
        .any(|c| c.command == "vector.explode" && needs(&c.args))
    {
        return Ok(request);
    }
    let project = project()?;
    for call in &mut request.commands {
        if call.command != "vector.explode" || !needs(&call.args) {
            continue;
        }
        let Some(asset) = call
            .args
            .get("asset")
            .and_then(Value::as_str)
            .and_then(|a| Id::parse(a).ok())
        else {
            // The command reports the missing asset itself.
            continue;
        };
        if project.media.get(&asset).map(|a| a.kind) != Some(AssetKind::Vector) {
            continue;
        }
        let parts = drawing_parts(&project, bundle, &asset)?;
        if let Value::Object(args) = &mut call.args {
            args.insert("parts".into(), parts);
        }
    }
    Ok(request)
}

/// The parts of the drawing of vector asset `asset`, as `vector.explode` takes them.
pub fn drawing_parts(project: &Project, bundle: &Path, asset: &Id) -> Result<Value, ApiError> {
    let path = crate::export::resolve(project, bundle, asset).map_err(ApiError::from)?;
    let data = std::fs::read(&path)
        .map_err(|e| invalid(format!("cannot read the drawing {}: {e}", path.display())))?;
    let tree = parse(&data).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
    Ok(Value::Array(parts(&tree)))
}

/// One part per visible path of `tree`, in drawing order.
pub fn parts(tree: &usvg::Tree) -> Vec<Value> {
    let size = tree.size();
    let canvas = (f64::from(size.width()), f64::from(size.height()));
    let mut out = Vec::new();
    walk(tree.root(), 1.0, canvas, &mut out);
    out
}

fn walk(group: &usvg::Group, opacity: f64, canvas: (f64, f64), out: &mut Vec<Value>) {
    let opacity = opacity * f64::from(group.opacity().get());
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => walk(g, opacity, canvas, out),
            usvg::Node::Path(p) if p.is_visible() => {
                if let Some(part) = part(p, opacity, canvas) {
                    out.push(part);
                }
            }
            _ => {}
        }
    }
}

/// A usvg transform as a kurbo affine map.
fn affine(t: usvg::Transform) -> kurbo::Affine {
    kurbo::Affine::new([
        f64::from(t.sx),
        f64::from(t.ky),
        f64::from(t.kx),
        f64::from(t.sy),
        f64::from(t.tx),
        f64::from(t.ty),
    ])
}

/// The path of `p` placed on the drawing's canvas, in its pixels.
fn placed_path(p: &usvg::Path) -> kurbo::BezPath {
    let mut path = kurbo::BezPath::new();
    let point = |q: usvg::tiny_skia_path::Point| kurbo::Point::new(f64::from(q.x), f64::from(q.y));
    for segment in p.data().segments() {
        use usvg::tiny_skia_path::PathSegment;
        match segment {
            PathSegment::MoveTo(a) => path.move_to(point(a)),
            PathSegment::LineTo(a) => path.line_to(point(a)),
            PathSegment::QuadTo(a, b) => path.quad_to(point(a), point(b)),
            PathSegment::CubicTo(a, b, c) => path.curve_to(point(a), point(b), point(c)),
            PathSegment::Close => path.close_path(),
        }
    }
    path.apply_affine(affine(p.abs_transform()));
    path
}

/// `#RRGGBB`, or `#RRGGBBAA` below full opacity.
fn hex(color: usvg::Color, alpha: f64) -> Color {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    let text = if a == 255 {
        format!("#{:02X}{:02X}{:02X}", color.red, color.green, color.blue)
    } else {
        format!(
            "#{:02X}{:02X}{:02X}{a:02X}",
            color.red, color.green, color.blue
        )
    };
    Color::parse(&text).unwrap_or_default()
}

/// A paint as a colour or a two-colour gradient: a gradient keeps its first and last stops
/// and its direction; a pattern becomes a middle grey.
fn paint(paint: &usvg::Paint, opacity: f64, transform: kurbo::Affine) -> Paint {
    let stops = |gradient: &usvg::BaseGradient| {
        let stops = gradient.stops();
        let color = |s: &usvg::Stop| hex(s.color(), opacity * f64::from(s.opacity().get()));
        match (stops.first(), stops.last()) {
            (Some(first), Some(last)) => (color(first), color(last)),
            _ => (Color::default(), Color::default()),
        }
    };
    match paint {
        usvg::Paint::Color(c) => Paint {
            color: hex(*c, opacity),
            gradient: None,
        },
        usvg::Paint::LinearGradient(g) => {
            let (from, to) = stops(g);
            let map = transform * affine(g.transform());
            let a = map * kurbo::Point::new(f64::from(g.x1()), f64::from(g.y1()));
            let b = map * kurbo::Point::new(f64::from(g.x2()), f64::from(g.y2()));
            Paint {
                color: from,
                gradient: Some(Gradient {
                    to,
                    angle: (b.y - a.y).atan2(b.x - a.x).to_degrees(),
                    radial: false,
                }),
            }
        }
        usvg::Paint::RadialGradient(g) => {
            let (from, to) = stops(g);
            Paint {
                color: from,
                gradient: Some(Gradient {
                    to,
                    angle: 0.0,
                    radial: true,
                }),
            }
        }
        usvg::Paint::Pattern(_) => Paint {
            color: Color::parse("#808080").unwrap_or_default(),
            gradient: None,
        },
    }
}

/// One path as a part: its contours in their box, the box as fractions of the drawing,
/// its fill and stroke with lengths in the drawing's height.
fn part(p: &usvg::Path, opacity: f64, canvas: (f64, f64)) -> Option<Value> {
    let (width, height) = (canvas.0.max(1e-9), canvas.1.max(1e-9));
    let transform = affine(p.abs_transform());
    let fitted = fosscut_model::fit_path(&placed_path(p), width.max(height) * 1e-3)?;
    let mut contours = fitted
        .contours
        .into_iter()
        .filter(|c| c.points.len() >= if c.closed { 3 } else { 2 });
    let first = contours.next()?;
    let [x, y, w, h] = fitted.bounds;
    let mut shape = ShapeContent::new(MaskShape::Path, [w / width, h / height], Color::default());
    shape.points = first.points;
    shape.closed = first.closed;
    shape.contours = contours.collect();
    shape.fill = p
        .fill()
        .map(|f| paint(f.paint(), f64::from(f.opacity().get()), transform));
    if let Some(fill) = p.fill() {
        shape.fill_rule = match fill.rule() {
            usvg::FillRule::NonZero => FillRule::Nonzero,
            usvg::FillRule::EvenOdd => FillRule::EvenOdd,
        };
    } else {
        shape.fill_rule = FillRule::Nonzero;
    }
    if let Some(s) = p.stroke() {
        // Widths scale with the path's transform; lengths are fractions of the drawing's
        // height.
        let scale = transform.determinant().abs().sqrt();
        let length = |v: f32| f64::from(v) * scale / height;
        let painted = paint(s.paint(), f64::from(s.opacity().get()), transform);
        shape.stroke = Some(ShapeStroke {
            color: painted.color,
            gradient: painted.gradient,
            width: length(s.width().get()).clamp(0.0, 0.5),
            cap: match s.linecap() {
                usvg::LineCap::Butt => LineCap::Butt,
                usvg::LineCap::Round => LineCap::Round,
                usvg::LineCap::Square => LineCap::Square,
            },
            join: match s.linejoin() {
                usvg::LineJoin::Round => LineJoin::Round,
                usvg::LineJoin::Bevel => LineJoin::Bevel,
                _ => LineJoin::Miter,
            },
            miter_limit: f64::from(s.miterlimit().get()).max(1.0),
            dashes: s
                .dasharray()
                .unwrap_or_default()
                .iter()
                .take(32)
                .map(|d| length(*d).clamp(0.0, 4.0))
                .collect(),
            dash_offset: length(s.dashoffset()),
        });
    }
    let name = (!p.id().is_empty()).then(|| p.id().to_string());
    Some(json!({
        "name": name,
        "shape": shape,
        "position": [(x + w / 2.0) / width - 0.5, (y + h / 2.0) / height - 0.5],
        "opacity": opacity.clamp(0.0, 1.0),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 400 200">
        <g opacity="0.5" transform="translate(20 0)">
            <path id="arrow" d="M 20 180 C 100 20 200 20 300 100" fill="none" stroke="#FF6600" stroke-width="20" stroke-linecap="round" stroke-dasharray="40 20"/>
        </g>
        <path id="ring" fill-rule="evenodd" fill="#0066ff" d="M 300 0 h 100 v 100 h -100 z M 320 20 h 60 v 60 h -60 z"/>
        <linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff"/></linearGradient>
        <ellipse cx="50" cy="150" rx="40" ry="40" fill="url(#g)"/>
    </svg>"##;

    #[test]
    fn view_boxes_parse_from_the_root_element() {
        assert_eq!(view_box(LOGO.as_bytes()), Some([0.0, 0.0, 400.0, 200.0]));
        assert_eq!(view_box(br#"<svg width="10" height="10"/>"#), None);
        assert_eq!(
            view_box(br#"<svg viewBox='-5,-5 10 10'/>"#),
            Some([-5.0, -5.0, 10.0, 10.0])
        );
    }

    #[test]
    fn paths_become_parts_on_the_drawings_canvas() {
        let tree = parse(LOGO.as_bytes()).unwrap();
        assert_eq!(count_paths(tree.root()), 3);
        let parts = parts(&tree);
        assert_eq!(parts.len(), 3);
        let arrow: ShapeContent = serde_json::from_value(parts[0]["shape"].clone()).unwrap();
        assert_eq!(parts[0]["name"], "arrow");
        assert!(!arrow.closed && arrow.fill.is_none());
        let stroke = arrow.stroke.unwrap();
        // 20 units of a 200-unit view box shown 100 pixels high: a tenth of the height.
        assert!((stroke.width - 0.1).abs() < 1e-6, "{}", stroke.width);
        assert_eq!(stroke.cap, LineCap::Round);
        assert_eq!(stroke.dashes.len(), 2);
        assert!((stroke.dashes[0] - 0.2).abs() < 1e-6);
        assert_eq!(parts[0]["opacity"], 0.5);
        // Moved by the group's translation: its box starts at 40 of 400 across.
        let size = &parts[0]["shape"]["size"];
        let left =
            parts[0]["position"][0].as_f64().unwrap() + 0.5 - size[0].as_f64().unwrap() / 2.0;
        assert!((left - 0.1).abs() < 1e-6, "{left}");
        let ring: ShapeContent = serde_json::from_value(parts[1]["shape"].clone()).unwrap();
        assert_eq!(ring.contours.len(), 1, "a hole");
        assert_eq!(ring.fill_rule, FillRule::EvenOdd);
        assert_eq!(ring.fill.as_ref().unwrap().color.as_str(), "#0066FF");
        assert_eq!(parts[1]["position"], json!([0.375, -0.25]));
        let ball: ShapeContent = serde_json::from_value(parts[2]["shape"].clone()).unwrap();
        let gradient = ball.fill.unwrap().gradient.unwrap();
        assert!((gradient.angle - 90.0).abs() < 1e-6, "top to bottom");
        assert_eq!(gradient.to.as_str(), "#FFFFFF");
        assert!(parts[2]["name"].is_null());
    }
}
