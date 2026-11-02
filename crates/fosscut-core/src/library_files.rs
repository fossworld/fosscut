//! Library items that become media files in a project (ADR-0028): FossCut's own SVG
//! drawings, device frames whose screens are open and a street map (ADR-0036), and web
//! compositions, HTML pages animated by time (ADR-0039). Front ends write them into the
//! bundle's `library/` folder and import them like any file, so a project keeps the copy
//! it uses. This module holds the data; it reads no files.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::composition_params::{self, CompositionKit, CompositionParam, ParamType, SlotKind};
use crate::{ApiError, codes};

const DRAWINGS: &str = include_str!("../presets/library/drawings.json");
const COMPOSITIONS: &str = include_str!("../presets/library/compositions.json");

/// The SVG text of every drawing, by file name.
const DRAWING_FILES: [(&str, &str); 5] = [
    (
        "phone.svg",
        include_str!("../presets/library/drawings/phone.svg"),
    ),
    (
        "tablet.svg",
        include_str!("../presets/library/drawings/tablet.svg"),
    ),
    (
        "laptop.svg",
        include_str!("../presets/library/drawings/laptop.svg"),
    ),
    (
        "browser.svg",
        include_str!("../presets/library/drawings/browser.svg"),
    ),
    (
        "street-map.svg",
        include_str!("../presets/library/drawings/street-map.svg"),
    ),
];

/// The page of every composition, by folder.
const COMPOSITION_PAGES: [(&str, &str); 28] = [
    (
        "shop-app",
        include_str!("../presets/library/compositions/shop-app/index.html"),
    ),
    (
        "route-map-3d",
        include_str!("../presets/library/compositions/route-map-3d/index.html"),
    ),
    (
        "parcel-drop-3d",
        include_str!("../presets/library/compositions/parcel-drop-3d/index.html"),
    ),
    (
        "phone-3d",
        include_str!("../presets/library/compositions/phone-3d/index.html"),
    ),
    (
        "logo-extrude-3d",
        include_str!("../presets/library/compositions/logo-extrude-3d/index.html"),
    ),
    (
        "product-podium-3d",
        include_str!("../presets/library/compositions/product-podium-3d/index.html"),
    ),
    // The explainer overlays, made with the overlay kit.
    (
        "video-card",
        include_str!("../presets/library/compositions/video-card/index.html"),
    ),
    (
        "app-window",
        include_str!("../presets/library/compositions/app-window/index.html"),
    ),
    (
        "terminal",
        include_str!("../presets/library/compositions/terminal/index.html"),
    ),
    (
        "review-frame",
        include_str!("../presets/library/compositions/review-frame/index.html"),
    ),
    (
        "social-post",
        include_str!("../presets/library/compositions/social-post/index.html"),
    ),
    (
        "quote-panel",
        include_str!("../presets/library/compositions/quote-panel/index.html"),
    ),
    (
        "before-after",
        include_str!("../presets/library/compositions/before-after/index.html"),
    ),
    (
        "reel-fan",
        include_str!("../presets/library/compositions/reel-fan/index.html"),
    ),
    (
        "menu-grid",
        include_str!("../presets/library/compositions/menu-grid/index.html"),
    ),
    (
        "storyboard-list",
        include_str!("../presets/library/compositions/storyboard-list/index.html"),
    ),
    (
        "cross-out",
        include_str!("../presets/library/compositions/cross-out/index.html"),
    ),
    (
        "arrow-pointer",
        include_str!("../presets/library/compositions/arrow-pointer/index.html"),
    ),
    (
        "audio-chip",
        include_str!("../presets/library/compositions/audio-chip/index.html"),
    ),
    (
        "waveform-sync",
        include_str!("../presets/library/compositions/waveform-sync/index.html"),
    ),
    (
        "timeline-cut",
        include_str!("../presets/library/compositions/timeline-cut/index.html"),
    ),
    (
        "progress-card",
        include_str!("../presets/library/compositions/progress-card/index.html"),
    ),
    (
        "stat-counter",
        include_str!("../presets/library/compositions/stat-counter/index.html"),
    ),
    (
        "bar-chart",
        include_str!("../presets/library/compositions/bar-chart/index.html"),
    ),
    (
        "clock-card",
        include_str!("../presets/library/compositions/clock-card/index.html"),
    ),
    (
        "lower-third-card",
        include_str!("../presets/library/compositions/lower-third-card/index.html"),
    ),
    (
        "comment-cta",
        include_str!("../presets/library/compositions/comment-cta/index.html"),
    ),
    (
        "grid-stage",
        include_str!("../presets/library/compositions/grid-stage/index.html"),
    ),
];

/// The other files of compositions, by folder and path in it.
const COMPOSITION_FILES: [(&str, &str, &[u8]); 28] = [
    (
        "route-map-3d",
        "scene.js",
        include_bytes!("../presets/library/compositions/route-map-3d/scene.js"),
    ),
    (
        "parcel-drop-3d",
        "scene.js",
        include_bytes!("../presets/library/compositions/parcel-drop-3d/scene.js"),
    ),
    (
        "phone-3d",
        "scene.js",
        include_bytes!("../presets/library/compositions/phone-3d/scene.js"),
    ),
    (
        "logo-extrude-3d",
        "scene.js",
        include_bytes!("../presets/library/compositions/logo-extrude-3d/scene.js"),
    ),
    (
        "logo-extrude-3d",
        "mark.svg",
        include_bytes!("../presets/library/compositions/logo-extrude-3d/mark.svg"),
    ),
    (
        "product-podium-3d",
        "scene.js",
        include_bytes!("../presets/library/compositions/product-podium-3d/scene.js"),
    ),
    (
        "video-card",
        "main.js",
        include_bytes!("../presets/library/compositions/video-card/main.js"),
    ),
    (
        "app-window",
        "main.js",
        include_bytes!("../presets/library/compositions/app-window/main.js"),
    ),
    (
        "terminal",
        "main.js",
        include_bytes!("../presets/library/compositions/terminal/main.js"),
    ),
    (
        "review-frame",
        "main.js",
        include_bytes!("../presets/library/compositions/review-frame/main.js"),
    ),
    (
        "social-post",
        "main.js",
        include_bytes!("../presets/library/compositions/social-post/main.js"),
    ),
    (
        "quote-panel",
        "main.js",
        include_bytes!("../presets/library/compositions/quote-panel/main.js"),
    ),
    (
        "before-after",
        "main.js",
        include_bytes!("../presets/library/compositions/before-after/main.js"),
    ),
    (
        "reel-fan",
        "main.js",
        include_bytes!("../presets/library/compositions/reel-fan/main.js"),
    ),
    (
        "menu-grid",
        "main.js",
        include_bytes!("../presets/library/compositions/menu-grid/main.js"),
    ),
    (
        "storyboard-list",
        "main.js",
        include_bytes!("../presets/library/compositions/storyboard-list/main.js"),
    ),
    (
        "cross-out",
        "main.js",
        include_bytes!("../presets/library/compositions/cross-out/main.js"),
    ),
    (
        "arrow-pointer",
        "main.js",
        include_bytes!("../presets/library/compositions/arrow-pointer/main.js"),
    ),
    (
        "audio-chip",
        "main.js",
        include_bytes!("../presets/library/compositions/audio-chip/main.js"),
    ),
    (
        "waveform-sync",
        "main.js",
        include_bytes!("../presets/library/compositions/waveform-sync/main.js"),
    ),
    (
        "timeline-cut",
        "main.js",
        include_bytes!("../presets/library/compositions/timeline-cut/main.js"),
    ),
    (
        "progress-card",
        "main.js",
        include_bytes!("../presets/library/compositions/progress-card/main.js"),
    ),
    (
        "stat-counter",
        "main.js",
        include_bytes!("../presets/library/compositions/stat-counter/main.js"),
    ),
    (
        "bar-chart",
        "main.js",
        include_bytes!("../presets/library/compositions/bar-chart/main.js"),
    ),
    (
        "clock-card",
        "main.js",
        include_bytes!("../presets/library/compositions/clock-card/main.js"),
    ),
    (
        "lower-third-card",
        "main.js",
        include_bytes!("../presets/library/compositions/lower-third-card/main.js"),
    ),
    (
        "comment-cta",
        "main.js",
        include_bytes!("../presets/library/compositions/comment-cta/main.js"),
    ),
    (
        "grid-stage",
        "main.js",
        include_bytes!("../presets/library/compositions/grid-stage/main.js"),
    ),
];

/// Where a composition's lines are: a JSON array of strings in this element.
const LINES_OPEN: &str = r#"<script id="fosscut-lines" type="application/json">"#;
const LINES_CLOSE: &str = "</script>";

/// What a library drawing is.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DrawingKind {
    /// A device frame (phone, tablet, laptop, browser window) with an open screen.
    Device,
    /// Any other drawing, such as a map.
    Drawing,
}

/// The open screen of a device frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceScreen {
    /// Left, top, width and height in the drawing's pixels.
    pub rect: [f64; 4],
    /// Corner radius in the drawing's pixels.
    pub radius: f64,
    /// A canvas with the screen's proportions for what shows on it: a compound timeline or
    /// a web composition of this size fills the screen exactly.
    pub canvas: [u32; 2],
}

/// Where a device's screen lies on a canvas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScreenPlacement {
    /// Its centre as a clip's `transform.position` (fractions of the canvas from its
    /// centre).
    pub position: [f64; 2],
    /// Its width and height as fractions of the canvas.
    pub size: [f64; 2],
    /// The `transform.scale` that makes a picture with the screen's proportions (a
    /// compound clip or composition of the screen's `canvas`) fill it exactly.
    pub scale: f64,
}

impl DeviceScreen {
    /// Where the screen lies on a `canvas` (width, height) when a device drawing of
    /// `drawing` pixels is fitted to it, centred at scale 1 as clips are placed by
    /// default. A device clip moved to `p` and scaled by `s` puts the screen at `p` plus
    /// `position` times `s`, `s` times as large.
    pub fn on_canvas(&self, drawing: [u32; 2], canvas: (u32, u32)) -> ScreenPlacement {
        let (cw, ch) = (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1)));
        let (dw, dh) = (f64::from(drawing[0].max(1)), f64::from(drawing[1].max(1)));
        let fit = (cw / dw).min(ch / dh);
        let [x, y, w, h] = self.rect;
        let centre = (x + w / 2.0 - dw / 2.0, y + h / 2.0 - dh / 2.0);
        // A picture with the screen's proportions fits the canvas by the tighter side.
        let picture = (cw / w).min(ch / h);
        let round = |v: f64| (v * 1e5).round() / 1e5;
        ScreenPlacement {
            position: [round(centre.0 * fit / cw), round(centre.1 * fit / ch)],
            size: [round(w * fit / cw), round(h * fit / ch)],
            scale: round(fit / picture),
        }
    }
}

/// A drawing made by FossCut: a device frame or a map, free to use anywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibraryDrawing {
    /// Stable name, for example `phone`.
    pub name: String,
    /// What it is.
    pub kind: DrawingKind,
    /// Display title.
    pub title: String,
    /// What it shows and how to use it.
    pub description: String,
    /// Search words.
    pub tags: Vec<String>,
    /// Its file name.
    pub file: String,
    /// Its width and height in pixels.
    pub size: [u32; 2],
    /// A device's open screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<DeviceScreen>,
}

impl LibraryDrawing {
    /// The SVG text.
    pub fn svg(&self) -> &'static str {
        DRAWING_FILES
            .iter()
            .find(|(file, _)| *file == self.file)
            .map(|(_, text)| *text)
            .unwrap_or_default()
    }
}

/// Every built-in drawing.
pub fn drawings() -> Vec<LibraryDrawing> {
    serde_json::from_str(DRAWINGS).unwrap_or_default()
}

/// The drawing called `name`.
pub fn drawing(name: &str) -> Result<LibraryDrawing, ApiError> {
    let all = drawings();
    all.iter().find(|d| d.name == name).cloned().ok_or_else(|| {
        ApiError::new(
            codes::LIBRARY_UNKNOWN_ITEM,
            format!("there is no device or drawing {name}"),
        )
        .with_hint(format!(
            "Drawings: {}.",
            all.iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}

/// A web composition made by FossCut, free to use anywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibraryComposition {
    /// Stable name, for example `shop-app`.
    pub name: String,
    /// Display title.
    pub title: String,
    /// What it shows, its lines and how to use it.
    pub description: String,
    /// Search words.
    pub tags: Vec<String>,
    /// Its folder.
    pub folder: String,
    /// The kit of shared files it is made with, whose parameters it has too (ADR-0042).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<CompositionKit>,
    /// Its own parameters (ADR-0042); [`LibraryComposition::all_params`] adds the kit's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<CompositionParam>,
    /// Its defaults for the kit's parameters, groups field by field.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub defaults: Map<String, Value>,
}

impl LibraryComposition {
    /// Every parameter: its own, then its kit's with its defaults.
    pub fn all_params(&self) -> Vec<CompositionParam> {
        composition_params::all_params(&self.params, self.kit, &self.defaults)
    }

    /// Every parameter's value when none is given.
    pub fn default_params(&self) -> Map<String, Value> {
        composition_params::defaults(&self.all_params())
    }

    /// Every parameter with the value in `given` (an object) or its default, checked; the
    /// assets in `media` fill the slots `given` leaves out, in order.
    pub fn resolve_params(
        &self,
        given: Option<&Value>,
        media: &[String],
    ) -> Result<Map<String, Value>, ApiError> {
        composition_params::resolve(&self.name, &self.all_params(), given, media)
    }

    /// True when a slot of it takes another composition: such a composition cannot fill a
    /// composition slot itself.
    pub fn has_composition_slot(&self) -> bool {
        self.params
            .iter()
            .any(|p| p.kind == ParamType::Slot && p.accepts.contains(&SlotKind::Composition))
    }

    /// Every file of its folder except the page, by path: its own and its kit's.
    pub fn files(&self) -> Vec<(String, &'static [u8])> {
        let mut files: Vec<(String, &'static [u8])> = COMPOSITION_FILES
            .iter()
            .filter(|(folder, _, _)| *folder == self.folder)
            .map(|(_, path, bytes)| (path.to_string(), *bytes))
            .collect();
        if let Some(kit) = self.kit {
            files.extend(
                kit.files()
                    .iter()
                    .map(|(path, bytes)| (path.to_string(), *bytes)),
            );
        }
        files
    }

    /// The page with `lines` in place of its placeholder lines and the canvas values of
    /// `params` (`width`, `height`, `duration`, `fps`) in its meta tag. An overlay without a
    /// `duration` lasts until its exit is over (`appear` plus `hold` plus the exit).
    pub fn html_with(&self, lines: &[String], params: &Map<String, Value>) -> String {
        if self.kit == Some(CompositionKit::Overlay)
            && let Some(seconds) = composition_params::overlay_duration(params)
        {
            let mut params = params.clone();
            params.insert("duration".into(), Value::from(seconds));
            return composition_params::with_canvas(&self.html(lines), &params);
        }
        composition_params::with_canvas(&self.html(lines), params)
    }

    /// The page as written, with its placeholder lines.
    pub fn page(&self) -> &'static str {
        COMPOSITION_PAGES
            .iter()
            .find(|(folder, _)| *folder == self.folder)
            .map(|(_, page)| *page)
            .unwrap_or_default()
    }

    /// The lines the page shows when the caller gives none, in order.
    pub fn lines(&self) -> Vec<String> {
        let page = self.page();
        page.find(LINES_OPEN)
            .map(|start| &page[start + LINES_OPEN.len()..])
            .and_then(|rest| rest.find(LINES_CLOSE).map(|end| &rest[..end]))
            .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
            .unwrap_or_default()
    }

    /// The page with `lines` in place of its first placeholder lines; the others keep
    /// their placeholders, and empty lines keep theirs too.
    pub fn html(&self, lines: &[String]) -> String {
        let page = self.page();
        let mut chosen = self.lines();
        for (slot, line) in chosen.iter_mut().zip(lines) {
            if !line.trim().is_empty() {
                slot.clone_from(line);
            }
        }
        let Some(start) = page.find(LINES_OPEN).map(|s| s + LINES_OPEN.len()) else {
            return page.to_string();
        };
        let Some(end) = page[start..].find(LINES_CLOSE).map(|e| start + e) else {
            return page.to_string();
        };
        // A line can hold anything except the end of its script element.
        let json = Value::from(chosen)
            .to_string()
            .replace('<', "\\u003c")
            .replace('>', "\\u003e");
        format!("{}{json}{}", &page[..start], &page[end..])
    }
}

/// Every built-in web composition.
pub fn compositions() -> Vec<LibraryComposition> {
    serde_json::from_str(COMPOSITIONS).unwrap_or_default()
}

/// The composition called `name`.
pub fn composition(name: &str) -> Result<LibraryComposition, ApiError> {
    let all = compositions();
    all.iter().find(|c| c.name == name).cloned().ok_or_else(|| {
        ApiError::new(
            codes::LIBRARY_UNKNOWN_ITEM,
            format!("there is no web composition {name}"),
        )
        .with_hint(format!(
            "Compositions: {}.",
            all.iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_drawing_has_its_file_and_a_screen_inside_it() {
        let all = drawings();
        let names: Vec<&str> = all.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            ["phone", "tablet", "laptop", "browser", "street-map"]
        );
        assert_eq!(all.len(), DRAWING_FILES.len());
        for drawing in &all {
            let svg = drawing.svg();
            assert!(svg.starts_with("<svg"), "{}", drawing.name);
            let size = format!(
                "width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"",
                drawing.size[0], drawing.size[1], drawing.size[0], drawing.size[1]
            );
            assert!(svg.contains(&size), "{}: {size}", drawing.name);
            assert!(!svg.contains("http://") || svg.contains("xmlns=\"http://www.w3.org"));
            assert!(drawing.description.ends_with('.') && !drawing.tags.is_empty());
            assert_eq!(
                drawing.screen.is_some(),
                drawing.kind == DrawingKind::Device,
                "{}",
                drawing.name
            );
            if let Some(screen) = &drawing.screen {
                let [x, y, w, h] = screen.rect;
                assert!(x >= 0.0 && y >= 0.0, "{}", drawing.name);
                assert!(x + w <= f64::from(drawing.size[0]), "{}", drawing.name);
                assert!(y + h <= f64::from(drawing.size[1]), "{}", drawing.name);
                // The suggested canvas has the screen's proportions.
                let ratio = f64::from(screen.canvas[0]) / f64::from(screen.canvas[1]);
                assert!((ratio - w / h).abs() < 1e-3, "{}", drawing.name);
            }
        }
        assert_eq!(drawing("phone").unwrap().kind, DrawingKind::Device);
        let unknown = drawing("watch").unwrap_err();
        assert_eq!(unknown.code, codes::LIBRARY_UNKNOWN_ITEM);
        assert!(unknown.hint.unwrap().contains("laptop"));
    }

    #[test]
    fn screens_are_placed_on_any_canvas() {
        let phone = drawing("phone").unwrap();
        let screen = phone.screen.clone().unwrap();
        // On a vertical canvas the phone fits by its height: 1920 / 832.
        let vertical = screen.on_canvas(phone.size, (1080, 1920));
        let fit = 1920.0 / 832.0;
        assert!((vertical.size[1] - 780.0 * fit / 1920.0).abs() < 1e-4);
        assert!((vertical.size[0] - 360.0 * fit / 1080.0).abs() < 1e-4);
        // The phone's bezels are even, so its screen is in the middle.
        assert!(vertical.position[0].abs() < 1e-4 && vertical.position[1].abs() < 1e-4);
        // A 720x1560 picture fits a 1080x1920 canvas by its height too: the scale is the
        // share of the canvas height the screen takes.
        assert!((vertical.scale - 780.0 * fit / 1920.0).abs() < 1e-4);
        // The browser's page area is below its toolbar.
        let browser = drawing("browser").unwrap();
        let page = browser
            .screen
            .clone()
            .unwrap()
            .on_canvas(browser.size, (1920, 1080));
        assert!(page.position[1] > 0.0);
        let fit = (1920.0f64 / 1296.0).min(1080.0 / 824.0);
        assert!((page.scale - 1280.0 * fit / 1920.0).abs() < 1e-4);
    }

    #[test]
    fn compositions_take_lines_in_place_of_their_placeholders() {
        let all = compositions();
        assert_eq!(all.len(), COMPOSITION_PAGES.len());
        let shop = composition("shop-app").unwrap();
        assert!(shop.page().contains("name=\"fosscut:composition\""));
        let lines = shop.lines();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "your product");
        assert_eq!(shop.html(&[]), shop.page());
        let page = shop.html(&["desk lamps".into(), String::new(), "</script>$5".into()]);
        assert!(page.contains(
            r#"["desk lamps","Product name","\u003c/script\u003e$5","Add to cart","Added to cart"]"#
        ));
        assert_eq!(
            page.matches("</script>").count(),
            shop.page().matches("</script>").count()
        );
        assert!(composition("nothing").is_err());
    }

    /// `a` and `b` are the same JSON, numbers compared by value.
    fn same(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
            (Value::Array(x), Value::Array(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y))
            }
            (Value::Object(x), Value::Object(y)) => {
                x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
            }
            _ => a == b,
        }
    }

    #[test]
    fn scene_templates_carry_their_kit_and_the_defaults_they_declare() {
        let scenes: Vec<LibraryComposition> = compositions()
            .into_iter()
            .filter(|c| c.kit == Some(CompositionKit::Three))
            .collect();
        let names: Vec<&str> = scenes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "route-map-3d",
                "parcel-drop-3d",
                "phone-3d",
                "logo-extrude-3d",
                "product-podium-3d"
            ]
        );
        let shop = composition("shop-app").unwrap();
        assert!(shop.all_params().is_empty() && shop.files().is_empty());
        assert!(!shop.has_composition_slot());
        for scene in &scenes {
            let page = scene.page();
            assert!(
                page.contains("name=\"fosscut:composition\""),
                "{}",
                scene.name
            );
            assert!(
                page.contains(r#"<meta name="fosscut:renderer" content="gpu">"#),
                "{}",
                scene.name
            );
            // Modules load from the folder by relative paths only.
            assert!(
                page.contains(r#""three": "./vendor/three/three.module.min.js""#),
                "{}",
                scene.name
            );
            assert!(
                !page.contains("http://") && !page.contains("https://"),
                "{}",
                scene.name
            );
            let files = scene.files();
            let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
            for needed in [
                "scene.js",
                "kit/stage.js",
                "vendor/three/LICENSE",
                "vendor/three/three.core.min.js",
            ] {
                assert!(paths.contains(&needed), "{}: {needed}", scene.name);
            }
            assert!(scene.description.ends_with('.') && scene.description.contains("Three.js"));
            // The page's own defaults, used when params.json is missing, agree with the
            // declared ones.
            let script = files
                .iter()
                .find(|(p, _)| p == "scene.js")
                .map(|(_, b)| String::from_utf8(b.to_vec()).unwrap())
                .unwrap();
            let start = script.find("// fosscut:defaults").expect("defaults marker");
            let end = script.find("// fosscut:end").expect("end marker");
            let block = &script[start..end];
            let json = &block[block.find('{').unwrap()..=block.rfind('}').unwrap()];
            let page_defaults: Value = serde_json::from_str(json).unwrap();
            let mut declared = scene.default_params();
            for canvas in composition_params::CANVAS_PARAMS {
                assert_eq!(declared.remove(canvas), Some(Value::Null), "{canvas}");
            }
            let declared = Value::Object(declared);
            assert!(
                same(&page_defaults, &declared),
                "{}: the page's defaults\n{page_defaults:#}\ndiffer from the declared\n{declared:#}",
                scene.name
            );
            // Every declared default passes its own check.
            assert_eq!(
                scene.resolve_params(None, &[]).unwrap(),
                scene.default_params(),
                "{}",
                scene.name
            );
        }
        let phone = composition("phone-3d").unwrap();
        assert!(phone.has_composition_slot());
        // A composition with a composition slot cannot fill one.
        let nested = json!({ "screen": "phone-3d" });
        assert_eq!(
            phone.resolve_params(Some(&nested), &[]).unwrap_err().code,
            codes::LIBRARY_INVALID_PARAMS
        );
        let sized = phone
            .resolve_params(
                Some(&json!({ "width": 540, "height": 960, "duration": 2 })),
                &[],
            )
            .unwrap();
        let html = phone.html_with(&[], &sized);
        assert!(
            html.contains("content=\"width=540, height=960, fps=30, duration=2\""),
            "{html}"
        );
    }

    /// The explainer overlays, in catalogue order.
    const OVERLAYS: [&str; 22] = [
        "video-card",
        "app-window",
        "terminal",
        "review-frame",
        "social-post",
        "quote-panel",
        "before-after",
        "reel-fan",
        "menu-grid",
        "storyboard-list",
        "cross-out",
        "arrow-pointer",
        "audio-chip",
        "waveform-sync",
        "timeline-cut",
        "progress-card",
        "stat-counter",
        "bar-chart",
        "clock-card",
        "lower-third-card",
        "comment-cta",
        "grid-stage",
    ];

    /// The `fosscut:composition` meta tag's content in `html`.
    fn meta(html: &str) -> String {
        let start = html
            .find("name=\"fosscut:composition\" content=\"")
            .unwrap()
            + 36;
        html[start..start + html[start..].find('"').unwrap()].to_string()
    }

    #[test]
    fn overlays_carry_their_kit_and_the_defaults_they_declare() {
        let overlays: Vec<LibraryComposition> = compositions()
            .into_iter()
            .filter(|c| c.kit == Some(CompositionKit::Overlay))
            .collect();
        let names: Vec<&str> = overlays.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, OVERLAYS);
        for overlay in &overlays {
            let name = overlay.name.as_str();
            let page = overlay.page();
            assert!(page.starts_with("<!doctype html>"), "{name}");
            for needed in [
                r#"<link rel="stylesheet" href="kit/overlay.css">"#,
                r#"<script src="kit/overlay.js"></script>"#,
                r#"<script src="main.js" defer></script>"#,
            ] {
                assert!(page.contains(needed), "{name}: {needed}");
            }
            assert!(
                !page.contains("fosscut:renderer"),
                "{name} draws in software"
            );
            assert!(overlay.lines().is_empty(), "{name} takes params, not lines");
            assert!(overlay.description.ends_with('.'), "{name}");
            assert!(
                overlay.description.contains("Use it"),
                "{name} says when to use it"
            );
            assert!(!overlay.has_composition_slot(), "{name}");
            let files = overlay.files();
            let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
            for needed in [
                "main.js",
                "kit/overlay.js",
                "kit/overlay.css",
                "kit/fonts/Inter-Regular.ttf",
                "kit/fonts/Inter-Bold.ttf",
                "kit/fonts/Inter-ExtraBold.ttf",
                "kit/fonts/OFL.txt",
            ] {
                assert!(paths.contains(&needed), "{name}: {needed}");
            }
            // FossCut's own text only: plain ASCII (no emoji, no long dashes) and nothing
            // fetched from the network.
            let script = files
                .iter()
                .find(|(p, _)| p == "main.js")
                .map(|(_, b)| String::from_utf8(b.to_vec()).unwrap())
                .unwrap();
            for (path, text) in [("index.html", page), ("main.js", script.as_str())] {
                assert!(text.is_ascii(), "{name}/{path} is not plain ASCII");
                assert!(
                    !text.contains("http://") && !text.contains("https://"),
                    "{name}/{path}"
                );
            }
            let text = serde_json::to_string(overlay).unwrap();
            assert!(!text.contains('\u{2014}'), "{name} has a long dash");
            // The page's own defaults agree with the declared ones.
            let start = script.find("// fosscut:defaults").expect("defaults marker");
            let end = script.find("// fosscut:end").expect("end marker");
            let block = &script[start..end];
            let json = &block[block.find('{').unwrap()..=block.rfind('}').unwrap()];
            let page_defaults: Value = serde_json::from_str(json).unwrap();
            let mut declared = overlay.default_params();
            for canvas in composition_params::CANVAS_PARAMS {
                assert_eq!(declared.remove(canvas), Some(Value::Null), "{canvas}");
            }
            let declared = Value::Object(declared);
            assert!(
                same(&page_defaults, &declared),
                "{name}: the page's defaults\n{page_defaults:#}\ndiffer from the declared\n{declared:#}"
            );
            let resolved = overlay.resolve_params(None, &[]).unwrap();
            assert_eq!(resolved, overlay.default_params(), "{name}");
            // The page's meta tag lasts until its exit is over, as html_with makes it.
            assert_eq!(
                meta(&overlay.html_with(&[], &resolved)),
                meta(page),
                "{name}"
            );
            // Each slot takes pictures, and more of them where the page plays videos.
            for slot in overlay.params.iter().filter(|p| p.kind == ParamType::Slot) {
                assert!(
                    slot.accepts.contains(&SlotKind::Image),
                    "{name}: {}",
                    slot.name
                );
                assert_eq!(slot.default, Value::Null, "{name}: {}", slot.name);
            }
        }
        for kit in [CompositionKit::Overlay, CompositionKit::Three] {
            assert!(!kit.files().is_empty());
        }
        // The kit's ASCII rule holds for its own script and style sheet too.
        for (path, bytes) in CompositionKit::Overlay.files() {
            if path.ends_with(".js") || path.ends_with(".css") {
                assert!(bytes.is_ascii(), "{path}");
            }
        }
    }

    #[test]
    fn overlay_timing_sets_the_length_of_the_page() {
        let card = composition("video-card").unwrap();
        let held = card
            .resolve_params(Some(&json!({ "appear": 0.5, "hold": 4 })), &[])
            .unwrap();
        assert!(
            meta(&card.html_with(&[], &held)).ends_with("duration=4.9"),
            "appear, hold and the exit"
        );
        let staying = card
            .resolve_params(Some(&json!({ "hold": 2, "exit": false })), &[])
            .unwrap();
        assert!(meta(&card.html_with(&[], &staying)).ends_with("duration=2"));
        let given = card
            .resolve_params(
                Some(&json!({ "hold": 2, "duration": 6, "width": 540, "height": 430 })),
                &[],
            )
            .unwrap();
        assert_eq!(
            meta(&card.html_with(&[], &given)),
            "width=540, height=430, fps=30, duration=6"
        );
        // Lists and slots are checked like any parameter.
        let terminal = composition("terminal").unwrap();
        let steps = json!({ "steps": ["one", "two"], "theme": "light" });
        let resolved = terminal.resolve_params(Some(&steps), &[]).unwrap();
        assert_eq!(resolved["steps"], json!(["one", "two"]));
        assert_eq!(resolved["hold"], json!(5.5), "the template's own default");
        let too_many = json!({ "steps": ["1", "2", "3", "4", "5", "6", "7"] });
        assert_eq!(
            terminal
                .resolve_params(Some(&too_many), &[])
                .unwrap_err()
                .code,
            codes::LIBRARY_INVALID_PARAMS
        );
        let fan = composition("reel-fan").unwrap();
        let filled = fan
            .resolve_params(None, &["asset_0001".into(), "asset_0002".into()])
            .unwrap();
        assert_eq!(
            (filled["reel_1"].as_str(), filled["reel_2"].as_str()),
            (Some("asset_0001"), Some("asset_0002"))
        );
        assert_eq!(filled["reel_3"], Value::Null);
        let chart = composition("bar-chart").unwrap();
        let negative = json!({ "values": [3, -1] });
        assert!(
            chart
                .resolve_params(Some(&negative), &[])
                .unwrap_err()
                .message
                .contains("values item must be at least 0")
        );
    }
}
