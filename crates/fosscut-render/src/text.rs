//! Text layers, first step of the text engine (ADR-0011, item 5): shaping, wrapping,
//! alignment and rasterization with `cosmic-text`, using only the bundled Inter fonts so
//! text renders identically on every machine.
//!
//! Text is rasterized on the CPU at the output resolution, so preview and export are both
//! sharp; the renderer colors the coverage, uploads it and composites it like any layer.

use cosmic_text::{
    Align as TextAlign, Attrs, Buffer, Color as GlyphColor, Family, FontSystem, Metrics, Shaping,
    Style, SwashCache, Weight, fontdb,
};
use fosscut_model::{Align, TextContent};

/// The bundled font family (SIL Open Font License; see `assets/fonts/inter/README.md`).
pub const DEFAULT_FAMILY: &str = "Inter";

const FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Italic.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-BoldItalic.ttf"),
];

/// Line height as a multiple of the font size.
pub const LINE_HEIGHT: f32 = 1.2;
/// Text wraps at this fraction of the canvas width.
pub const WRAP_WIDTH: f32 = 0.9;
/// Transparent border around the glyphs, so filtering never clips them, in output pixels.
const PADDING: u32 = 2;

/// Rasterized text: coverage from 0 (transparent) to 255 (covered), rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRaster {
    /// Width in output pixels, including padding; 0 when there is nothing to draw.
    pub width: u32,
    /// Height in output pixels, including padding; 0 when there is nothing to draw.
    pub height: u32,
    /// `width * height` coverage values.
    pub coverage: Vec<u8>,
    /// The text's layout box inside the raster, in raster pixels: x, y, width, height.
    /// Placement centres this box, not the pixel-cropped raster, so text sits in the same
    /// place at every output size.
    pub layout_box: [f32; 4],
}

impl TextRaster {
    /// Nothing to draw.
    pub fn empty() -> TextRaster {
        TextRaster {
            width: 0,
            height: 0,
            coverage: Vec::new(),
            layout_box: [0.0; 4],
        }
    }

    /// True when no glyph was drawn.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Coverage at a pixel.
    pub fn at(&self, x: u32, y: u32) -> u8 {
        self.coverage[(y * self.width + x) as usize]
    }
}

/// Shapes and rasterizes text with the bundled fonts.
pub struct TextEngine {
    fonts: FontSystem,
    glyphs: SwashCache,
}

impl Default for TextEngine {
    fn default() -> TextEngine {
        TextEngine::new()
    }
}

impl TextEngine {
    /// An engine that knows only the bundled fonts, never system fonts.
    pub fn new() -> TextEngine {
        let mut database = fontdb::Database::new();
        for data in FONTS {
            database.load_font_data(data.to_vec());
        }
        TextEngine {
            fonts: FontSystem::new_with_locale_and_db("en-US".to_string(), database),
            glyphs: SwashCache::new(),
        }
    }

    /// The font families the engine can use.
    pub fn families(&self) -> Vec<String> {
        let mut families: Vec<String> = self
            .fonts
            .db()
            .faces()
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .collect();
        families.sort();
        families.dedup();
        families
    }

    /// Rasterizes `content` for a `canvas` (timeline pixels) drawn at `scale` output pixels
    /// per canvas pixel. The font size is `style.size` times the canvas height; lines wrap
    /// at [`WRAP_WIDTH`] of the canvas width and align within the widest line.
    pub fn rasterize(
        &mut self,
        content: &TextContent,
        canvas: (u32, u32),
        scale: f32,
    ) -> TextRaster {
        let style = &content.style;
        let size = (style.size as f32 * canvas.1 as f32 * scale).max(1.0);
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * LINE_HEIGHT));
        let attrs = Attrs::new()
            .family(Family::Name(DEFAULT_FAMILY))
            .weight(if style.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            })
            .style(if style.italic {
                Style::Italic
            } else {
                Style::Normal
            });
        let align = match style.align {
            Align::Left => TextAlign::Left,
            Align::Center => TextAlign::Center,
            Align::Right => TextAlign::Right,
        };

        // Lay out at the wrap width, which aligns lines relative to each other, draw with
        // room for overhanging glyphs (italics), then crop to the ink horizontally. The
        // height stays the line boxes, so a title does not move when descenders change.
        let wrap = (canvas.0 as f32 * WRAP_WIDTH * scale).max(1.0);
        buffer.set_size(Some(wrap), None);
        buffer.set_text(&content.text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.fonts, false);
        let height = line_boxes_height(&buffer);
        let Some((ink_left, ink_right)) = glyph_extent(&buffer) else {
            return TextRaster::empty();
        };
        if height <= 0.0 {
            return TextRaster::empty();
        }
        let margin = (size * 0.5).ceil() as i32 + PADDING as i32;
        let w = wrap.ceil() as i32 + 2 * margin;
        let h = height.ceil() as i32 + 2 * PADDING as i32;
        let mut ink = vec![0u8; (w * h) as usize];
        let white = GlyphColor::rgb(255, 255, 255);
        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            white,
            |x, y, gw, gh, color| {
                let alpha = u32::from(color.a());
                if alpha == 0 {
                    return;
                }
                for dy in 0..gh as i32 {
                    for dx in 0..gw as i32 {
                        let (px, py) = (x + dx + margin, y + dy + PADDING as i32);
                        if px < 0 || py < 0 || px >= w || py >= h {
                            continue;
                        }
                        // Overlapping glyph pixels combine like "over".
                        let cell = &mut ink[(py * w + px) as usize];
                        let existing = u32::from(*cell);
                        *cell = (alpha + existing * (255 - alpha) / 255).min(255) as u8;
                    }
                }
            },
        );
        let inked = |x: i32| (0..h).any(|y| ink[(y * w + x) as usize] > 0);
        let (Some(first), Some(last)) =
            ((0..w).find(|&x| inked(x)), (0..w).rev().find(|&x| inked(x)))
        else {
            return TextRaster::empty();
        };
        let left = (first - PADDING as i32).max(0);
        let right = (last + PADDING as i32).min(w - 1);
        let width = (right - left + 1) as u32;
        let mut coverage = Vec::with_capacity(width as usize * h as usize);
        for y in 0..h {
            let row = (y * w) as usize;
            coverage.extend_from_slice(&ink[row + left as usize..=row + right as usize]);
        }
        TextRaster {
            width,
            height: h as u32,
            coverage,
            layout_box: [
                ink_left + (margin - left) as f32,
                PADDING as f32,
                ink_right - ink_left,
                height,
            ],
        }
    }
}

/// Horizontal extent of the visible (non-whitespace) glyphs' advance boxes, in layout
/// units, or `None` when there are none.
fn glyph_extent(buffer: &Buffer) -> Option<(f32, f32)> {
    let mut extent: Option<(f32, f32)> = None;
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let visible = run
                .text
                .get(glyph.start..glyph.end)
                .is_some_and(|text| !text.trim().is_empty());
            if visible {
                let (left, right) = (glyph.x, glyph.x + glyph.w);
                extent = Some(extent.map_or((left, right), |(l, r)| (l.min(left), r.max(right))));
            }
        }
    }
    extent
}

/// Height of all laid-out lines, or 0 when the text has no glyphs.
fn line_boxes_height(buffer: &Buffer) -> f32 {
    let mut height = 0.0_f32;
    let mut glyphs = false;
    for run in buffer.layout_runs() {
        glyphs |= !run.glyphs.is_empty();
        height = height.max(run.line_top + run.line_height);
    }
    if glyphs { height } else { 0.0 }
}

/// A `#RRGGBB` or `#RRGGBBAA` color as linear-light RGB and alpha: the hex value is sRGB,
/// as on the web and in design tools.
pub fn linear_color(hex: &str) -> [f32; 4] {
    let digits = hex.trim_start_matches('#');
    let channel = |i: usize| {
        digits
            .get(i..i + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .map_or(1.0, |v| f32::from(v) / 255.0)
    };
    let decode = |v: f32| crate::color::TransferFunction::Srgb.decode(v);
    let alpha = if digits.len() >= 8 { channel(6) } else { 1.0 };
    [
        decode(channel(0)),
        decode(channel(2)),
        decode(channel(4)),
        alpha,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_model::TextStyle;

    fn content(text: &str, edit: impl FnOnce(&mut TextStyle)) -> TextContent {
        let mut style = TextStyle::default();
        edit(&mut style);
        TextContent {
            text: text.into(),
            style,
        }
    }

    const CANVAS: (u32, u32) = (1920, 1080);

    #[test]
    fn only_the_bundled_fonts_are_known() {
        let engine = TextEngine::new();
        assert_eq!(engine.families(), vec![DEFAULT_FAMILY.to_string()]);
    }

    #[test]
    fn size_follows_the_canvas_height_and_scale() {
        let mut engine = TextEngine::new();
        let title = content("FossCut", |s| s.size = 0.1);
        let full = engine.rasterize(&title, CANVAS, 1.0);
        let half = engine.rasterize(&title, CANVAS, 0.5);
        // One line: 0.1 x 1080 px font with 1.2 line height, plus padding.
        assert_eq!(
            full.height,
            (108.0 * LINE_HEIGHT).ceil() as u32 + 2 * PADDING
        );
        let ratio = f64::from(full.width) / f64::from(half.width);
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "width scales with output: {ratio}"
        );
        assert!(
            full.coverage.contains(&255),
            "glyph interiors are fully covered"
        );
    }

    #[test]
    fn long_text_wraps_at_ninety_percent_of_the_canvas() {
        let mut engine = TextEngine::new();
        let long = content(&"wrap me please ".repeat(20), |s| s.size = 0.06);
        let raster = engine.rasterize(&long, CANVAS, 1.0);
        let line = (0.06 * 1080.0 * LINE_HEIGHT) as u32;
        assert!(raster.height > 3 * line, "several lines: {}", raster.height);
        assert!(
            raster.width <= (1920.0 * WRAP_WIDTH) as u32 + 2 * PADDING,
            "{}",
            raster.width
        );
        let explicit = engine.rasterize(&content("one\ntwo\nthree", |_| {}), CANVAS, 1.0);
        let single = engine.rasterize(&content("one", |_| {}), CANVAS, 1.0);
        assert!(explicit.height >= 3 * (single.height - 2 * PADDING));
    }

    #[test]
    fn weight_slant_and_alignment_change_the_raster() {
        let mut engine = TextEngine::new();
        let text = "Bold move";
        let regular = engine.rasterize(&content(text, |_| {}), CANVAS, 1.0);
        let bold = engine.rasterize(&content(text, |s| s.bold = true), CANVAS, 1.0);
        let italic = engine.rasterize(&content(text, |s| s.italic = true), CANVAS, 1.0);
        let ink = |r: &TextRaster| r.coverage.iter().map(|&c| u64::from(c)).sum::<u64>();
        assert!(ink(&bold) > ink(&regular) * 11 / 10, "bold uses more ink");
        assert_ne!(italic.coverage, regular.coverage);

        // Two lines of different length: the short one sits left, centred or right.
        let lines = "a much longer first line\nshort";
        let first_ink_column = |raster: &TextRaster| {
            let bottom = raster.height - 1 - PADDING - raster.height / 8;
            (0..raster.width).find(|&x| (raster.height / 2..bottom).any(|y| raster.at(x, y) > 128))
        };
        let left = engine.rasterize(&content(lines, |s| s.align = Align::Left), CANVAS, 1.0);
        let center = engine.rasterize(&content(lines, |s| s.align = Align::Center), CANVAS, 1.0);
        let right = engine.rasterize(&content(lines, |s| s.align = Align::Right), CANVAS, 1.0);
        let (l, c, r) = (
            first_ink_column(&left).unwrap(),
            first_ink_column(&center).unwrap(),
            first_ink_column(&right).unwrap(),
        );
        assert!(l < c && c < r, "short line starts at {l}, {c}, {r}");
    }

    #[test]
    fn rendering_is_deterministic_and_empty_text_draws_nothing() {
        let mut first = TextEngine::new();
        let mut second = TextEngine::new();
        let title = content("Same pixels everywhere", |s| s.bold = true);
        assert_eq!(
            first.rasterize(&title, CANVAS, 1.0),
            second.rasterize(&title, CANVAS, 1.0)
        );
        for blank in ["", "   ", "\n"] {
            assert!(
                first
                    .rasterize(&content(blank, |_| {}), CANVAS, 1.0)
                    .is_empty(),
                "{blank:?}"
            );
        }
        assert!(TextEngine::default().rasterize(&title, CANVAS, 0.25).width > 0);
    }

    #[test]
    fn the_layout_box_scales_exactly_with_the_output() {
        let mut engine = TextEngine::new();
        let title = content("Steady title", |s| s.bold = true);
        let full = engine.rasterize(&title, CANVAS, 1.0);
        let half = engine.rasterize(&title, CANVAS, 0.5);
        let [_, _, fw, fh] = full.layout_box;
        let [_, _, hw, hh] = half.layout_box;
        assert!((fw / hw - 2.0).abs() < 0.02, "width {fw} vs {hw}");
        assert!((fh / hh - 2.0).abs() < 0.001, "height {fh} vs {hh}");
        // The box sits on the raster; advance boxes may overhang the ink crop slightly.
        let [x, y, w, h] = full.layout_box;
        assert!(x < full.width as f32 && x + w > 0.0 && y >= 0.0 && y + h <= full.height as f32);
    }

    #[test]
    fn colors_are_srgb_hex_decoded_to_linear() {
        assert_eq!(linear_color("#FFFFFF"), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(linear_color("#000000"), [0.0, 0.0, 0.0, 1.0]);
        let mid = linear_color("#808080");
        assert!((mid[0] - 0.2158605).abs() < 1e-5, "{mid:?}");
        let translucent = linear_color("#FF000080");
        assert_eq!(translucent[1], 0.0);
        assert!((translucent[3] - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(linear_color("bad")[3], 1.0);
    }
}
