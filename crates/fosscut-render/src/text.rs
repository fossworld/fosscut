//! Text layers (ADR-0011 item 5, ADR-0027): shaping, wrapping, alignment and spans with
//! `cosmic-text`, using only the bundled Inter fonts and the project's font assets so text
//! renders identically on every machine. The CPU then composes each layer at the output
//! resolution, back to front: line boxes, shadow, glow, outline and fill, unit by unit when
//! the text reveals.

use std::collections::BTreeMap;

use cosmic_text::{
    Align as TextAlign, Attrs, Buffer, Color as GlyphColor, Family, FontSystem, LayoutRun, Metrics,
    Shaping, Style, SwashCache, UnderlineStyle, Weight, fontdb,
};
use fosscut_model::{Align, BUNDLED_FONT, RevealUnit, TextContent};
use fosscut_time::Flicks;

use crate::raster::{blur, outline, over, rounded_rect};

/// The bundled font family (SIL Open Font License; see `assets/fonts/inter/README.md`).
pub const DEFAULT_FAMILY: &str = BUNDLED_FONT;

const FONTS: [&[u8]; 6] = [
    include_bytes!("../assets/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Italic.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-BoldItalic.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-ExtraBold.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Black.ttf"),
];

/// Line height of captions as a multiple of the font size; text styles set their own.
pub const LINE_HEIGHT: f32 = 1.2;
/// Text wraps at this fraction of the canvas width.
pub const WRAP_WIDTH: f32 = 0.9;
/// Transparent border around the glyphs, so filtering never clips them, in output pixels.
const PADDING: u32 = 2;

/// A text clip at one moment: what the plan hands the renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayer {
    /// The text and its style.
    pub content: TextContent,
    /// The moment shown, from the clip's start; only reveals depend on it.
    pub time: Flicks,
    /// The clip's length.
    pub duration: Flicks,
}

impl TextLayer {
    /// Text that does not change over time: fully shown.
    pub fn still(content: TextContent) -> TextLayer {
        TextLayer {
            content,
            time: Flicks::ZERO,
            duration: Flicks::ZERO,
        }
    }

    /// The moment when it matters for the picture: during a reveal's entry or exit.
    pub fn moment(&self) -> Option<Flicks> {
        let reveal = self.content.reveal.as_ref()?;
        let entering = self.time < reveal.duration;
        let leaving = reveal.exit && self.time > self.duration - reveal.duration;
        (entering || leaving).then_some(self.time)
    }
}

/// Composed text: premultiplied linear RGBA, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRaster {
    /// Width in output pixels, including margins; 0 when there is nothing to draw.
    pub width: u32,
    /// Height in output pixels, including margins; 0 when there is nothing to draw.
    pub height: u32,
    /// `width * height` pixels.
    pub pixels: Vec<[f32; 4]>,
    /// The text's layout box inside the raster, in raster pixels: x, y, width, height.
    /// Placement centres this box, not the pixel-cropped raster, so text sits in the same
    /// place at every output size and with every effect.
    pub layout_box: [f32; 4],
}

impl TextRaster {
    /// Nothing to draw.
    pub fn empty() -> TextRaster {
        TextRaster {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            layout_box: [0.0; 4],
        }
    }

    /// True when nothing was drawn.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The pixel at `x`, `y`.
    pub fn at(&self, x: u32, y: u32) -> [f32; 4] {
        self.pixels[(y * self.width + x) as usize]
    }

    /// Opacity at `x`, `y`, from 0 to 1.
    pub fn alpha(&self, x: u32, y: u32) -> f32 {
        self.at(x, y)[3]
    }

    /// Opacity as bytes, one per pixel, as masks take coverage.
    pub fn coverage(&self) -> Vec<u8> {
        self.pixels
            .iter()
            .map(|p| (p[3].clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect()
    }
}

/// Shapes, rasterizes and composes text with the bundled fonts and added font files.
pub struct TextEngine {
    pub(crate) fonts: FontSystem,
    pub(crate) glyphs: SwashCache,
}

impl Default for TextEngine {
    fn default() -> TextEngine {
        TextEngine::new()
    }
}

/// The resolved look of one written character.
#[derive(Clone, Debug, PartialEq)]
struct CharLook {
    font: String,
    size: f32,
    color: [f32; 4],
    bold: bool,
    italic: bool,
    underline: bool,
}

/// Glyphs of one reveal look (opacity and rise), composed together.
struct Group {
    alpha: f32,
    ink: Vec<f32>,
    fill: Vec<[f32; 4]>,
}

/// A copy of the bundled Inter Regular whose family is `family`: a font file of a family
/// FossCut does not bundle, for tests of font assets. Its `name` table is replaced by one
/// naming `family` (TrueType/OpenType `name` format 0, Windows platform, US English).
#[doc(hidden)]
pub fn fixture_font(family: &str) -> Vec<u8> {
    let font = FONTS[0];
    let u16_at = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]);
    let tables = usize::from(u16_at(4));
    let utf16: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let regular: Vec<u8> = "Regular"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    // Family, subfamily, full name, PostScript name, typographic family and subfamily.
    let strings: [(u16, &[u8]); 6] = [
        (1, &utf16),
        (2, &regular),
        (4, &utf16),
        (6, &utf16),
        (16, &utf16),
        (17, &regular),
    ];
    let mut name = Vec::new();
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&(strings.len() as u16).to_be_bytes());
    name.extend_from_slice(&((6 + 12 * strings.len()) as u16).to_be_bytes());
    let mut storage = Vec::new();
    for (id, text) in strings {
        for value in [3u16, 1, 0x409, id, text.len() as u16, storage.len() as u16] {
            name.extend_from_slice(&value.to_be_bytes());
        }
        storage.extend_from_slice(text);
    }
    name.extend_from_slice(&storage);
    let mut out = font.to_vec();
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    let offset = out.len() as u32;
    out.extend_from_slice(&name);
    for table in 0..tables {
        let record = 12 + 16 * table;
        if &font[record..record + 4] == b"name" {
            out[record + 8..record + 12].copy_from_slice(&offset.to_be_bytes());
            out[record + 12..record + 16].copy_from_slice(&(name.len() as u32).to_be_bytes());
        }
    }
    out
}

/// The families in a font file, sorted, and its number of faces; an error for data that
/// is not a font.
pub fn font_families(data: &[u8]) -> Result<(Vec<String>, u32), String> {
    let mut database = fontdb::Database::new();
    database.load_font_data(data.to_vec());
    let mut families: Vec<String> = database
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
        .collect();
    let faces = database.faces().count() as u32;
    families.sort();
    families.dedup();
    if faces == 0 || families.is_empty() {
        return Err("it holds no usable font face".to_string());
    }
    Ok((families, faces))
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

    /// Adds the faces of a font file (a project's font asset) and returns its families.
    pub fn add_font(&mut self, data: Vec<u8>) -> Vec<String> {
        let ids = self
            .fonts
            .db_mut()
            .load_font_source(fontdb::Source::Binary(std::sync::Arc::new(data)));
        let mut families: Vec<String> = ids
            .iter()
            .filter_map(|id| self.fonts.db().face(*id))
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .collect();
        families.sort();
        families.dedup();
        families
    }

    /// Composes `layer` for a `canvas` (timeline pixels) drawn at `scale` output pixels per
    /// canvas pixel. The font size is `style.size` times the canvas height; lines wrap at
    /// [`WRAP_WIDTH`] of the canvas width and align within the widest line.
    pub fn rasterize(&mut self, layer: &TextLayer, canvas: (u32, u32), scale: f32) -> TextRaster {
        let content = &layer.content;
        let style = &content.style;
        let per_canvas_height = canvas.1 as f32 * scale;
        let size = (style.size as f32 * per_canvas_height).max(1.0);
        let line_height = style.line_height as f32;

        // Every written character's look, spans applied in order; then runs of equal looks
        // in the shown text, with each shown byte's written character.
        let base = CharLook {
            font: style.family().to_string(),
            size,
            color: linear_color(style.color.as_str()),
            bold: style.bold,
            italic: style.italic,
            underline: style.underline,
        };
        let written: Vec<char> = content.text.chars().collect();
        let shown = style.case.shown(&content.text);
        let mut looks: Vec<CharLook> = Vec::new();
        let mut look_of: Vec<usize> = Vec::with_capacity(written.len());
        for index in 0..written.len() {
            let mut look = base.clone();
            for span in content
                .spans
                .iter()
                .filter(|s| s.start <= index && index < s.end)
            {
                if let Some(font) = &span.font {
                    look.font = font.clone();
                }
                if let Some(size) = span.size {
                    look.size = (size as f32 * per_canvas_height).max(1.0);
                }
                if let Some(color) = &span.color {
                    look.color = linear_color(color.as_str());
                }
                look.bold = span.bold.unwrap_or(look.bold);
                look.italic = span.italic.unwrap_or(look.italic);
                look.underline = span.underline.unwrap_or(look.underline);
            }
            let found = looks.iter().position(|l| *l == look);
            look_of.push(found.unwrap_or_else(|| {
                looks.push(look);
                looks.len() - 1
            }));
        }
        let mut text = String::new();
        let mut owner: Vec<usize> = Vec::new();
        let mut runs: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
        for (index, part) in shown.iter().enumerate() {
            let start = text.len();
            text.push_str(part);
            owner.extend(std::iter::repeat_n(index, part.len()));
            match runs.last_mut() {
                Some((range, look)) if *look == look_of[index] => range.end = text.len(),
                _ => runs.push((start..text.len(), look_of[index])),
            }
        }
        let spacing = style.letter_spacing as f32;
        let attrs = |look, index| attrs_of(look, index, spacing, line_height);
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * line_height));
        let align = match style.align {
            Align::Left => TextAlign::Left,
            Align::Center => TextAlign::Center,
            Align::Right => TextAlign::Right,
        };

        // Lay out at the wrap width, which aligns lines relative to each other, draw with
        // room for overhanging glyphs (italics) and effects, then crop to the ink
        // horizontally. The height stays the line boxes plus margins, so a title does not
        // move when descenders change.
        let wrap = (canvas.0 as f32 * WRAP_WIDTH * scale).max(1.0);
        buffer.set_size(Some(wrap), None);
        let default_attrs = attrs(&base, usize::MAX);
        buffer.set_rich_text(
            runs.iter()
                .map(|(range, look)| (&text[range.clone()], attrs(&looks[*look], *look))),
            &default_attrs,
            Shaping::Advanced,
            Some(align),
        );
        buffer.shape_until_scroll(&mut self.fonts, false);
        let height = line_boxes_height(&buffer);
        let Some((ink_left, ink_right)) = glyph_extent(&buffer) else {
            return TextRaster::empty();
        };
        if height <= 0.0 {
            return TextRaster::empty();
        }

        // Room for every effect around the line boxes.
        let em = |v: f64| v as f32 * size;
        let stroke = if style.stroke.enabled {
            em(style.stroke.width).max(0.0)
        } else {
            0.0
        };
        let shadow_sigma = em(style.shadow.blur) / 2.0;
        let shadow_reach = if style.shadow.enabled {
            stroke
                + em(style.shadow.offset[0]
                    .abs()
                    .max(style.shadow.offset[1].abs()))
                + 3.0 * shadow_sigma
        } else {
            0.0
        };
        let glow_sigma = em(style.glow.radius) / 2.0;
        let glow_reach = if style.glow.enabled {
            3.0 * glow_sigma
        } else {
            0.0
        };
        let padding = if style.background.enabled {
            em(style.background.padding)
        } else {
            0.0
        };
        let reach = stroke.max(shadow_reach).max(glow_reach).max(padding);
        let margin_x = (size * 0.5 + reach).ceil() as i32 + PADDING as i32;
        let margin_y = reach.ceil() as i32 + PADDING as i32;
        let w = wrap.ceil() as i32 + 2 * margin_x;
        let h = height.ceil() as i32 + 2 * margin_y;
        let (wu, hu) = (w as usize, h as usize);
        let origin = (margin_x as f32, margin_y as f32);

        // Each glyph's reveal look, by the unit it belongs to.
        let lines: Vec<LayoutRun> = buffer.layout_runs().collect();
        // Glyph positions count bytes from the start of their paragraph (hard line).
        let mut paragraphs = vec![0usize];
        paragraphs.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        let unit_of = |line: usize, byte: usize| -> usize {
            let paragraph = paragraphs.get(lines[line].line_i).copied().unwrap_or(0);
            let index = owner.get(paragraph + byte).copied().unwrap_or(0);
            match content.reveal.as_ref().map(|r| r.unit) {
                Some(RevealUnit::Line) => line,
                Some(RevealUnit::Word) => words_before(&written, index),
                Some(RevealUnit::Letter) | None => written[..index]
                    .iter()
                    .filter(|c| !c.is_whitespace())
                    .count(),
            }
        };
        let count = match content.reveal.as_ref().map(|r| r.unit) {
            Some(RevealUnit::Line) => lines.len(),
            Some(RevealUnit::Word) => words_before(&written, written.len()),
            _ => written.iter().filter(|c| !c.is_whitespace()).count(),
        };
        let look_at = |unit: usize| -> (f32, f32) {
            match &content.reveal {
                Some(reveal) => reveal.unit_at(unit, count, layer.time, layer.duration),
                None => (1.0, 0.0),
            }
        };
        let key = |(alpha, rise): (f32, f32)| {
            (
                (alpha * 1024.0).round() as i32,
                (rise * 1024.0).round() as i32,
            )
        };

        let mut groups: BTreeMap<(i32, i32), Group> = BTreeMap::new();
        let mut line_alpha = vec![0.0f32; lines.len()];
        let TextEngine { fonts, glyphs } = self;
        for (line_index, run) in lines.iter().enumerate() {
            for glyph in run.glyphs {
                let visible = run
                    .text
                    .get(glyph.start..glyph.end)
                    .is_some_and(|t| !t.trim().is_empty());
                let look = look_at(unit_of(line_index, glyph.start));
                if look.0 <= 0.0 {
                    continue;
                }
                if visible {
                    line_alpha[line_index] = line_alpha[line_index].max(look.0);
                }
                let group = groups.entry(key(look)).or_insert_with(|| Group {
                    alpha: look.0,
                    ink: vec![0.0; wu * hu],
                    fill: vec![[0.0; 4]; wu * hu],
                });
                let color = looks.get(glyph.metadata).map_or(base.color, |l| l.color);
                let physical =
                    glyph.physical((origin.0, origin.1 + run.line_y + look.1 * size), 1.0);
                glyphs.with_pixels(
                    fonts,
                    physical.cache_key,
                    GlyphColor::rgb(255, 255, 255),
                    |x, y, pixel| {
                        let coverage = f32::from(pixel.a()) / 255.0;
                        let (px, py) = (physical.x + x, physical.y + y);
                        if coverage <= 0.0 || px < 0 || py < 0 || px >= w || py >= h {
                            return;
                        }
                        let cell = (py * w + px) as usize;
                        group.ink[cell] = coverage + group.ink[cell] * (1.0 - coverage);
                        over(&mut group.fill[cell], color, coverage);
                    },
                );
            }
            // Underlines, glyph by glyph so they reveal with their letters.
            for span in run.decorations {
                if span.data.text_decoration.underline == UnderlineStyle::None {
                    continue;
                }
                let metrics = span.data.underline_metrics;
                let thickness = (metrics.thickness * span.font_size).max(1.0);
                let top = run.line_y - metrics.offset * span.font_size;
                for glyph in run.glyphs.get(span.glyph_range.clone()).unwrap_or_default() {
                    let look = look_at(unit_of(line_index, glyph.start));
                    if look.0 <= 0.0 {
                        continue;
                    }
                    let Some(group) = groups.get_mut(&key(look)) else {
                        continue;
                    };
                    let color = looks.get(glyph.metadata).map_or(base.color, |l| l.color);
                    let y0 = (origin.1 + top + look.1 * size).round() as i32;
                    let x0 = (origin.0 + glyph.x).round() as i32;
                    let x1 = (origin.0 + glyph.x + glyph.w).round() as i32;
                    for py in y0.max(0)..(y0 + thickness.ceil() as i32).min(h) {
                        for px in x0.max(0)..x1.min(w) {
                            let cell = (py * w + px) as usize;
                            group.ink[cell] = 1.0;
                            over(&mut group.fill[cell], color, 1.0);
                        }
                    }
                }
            }
        }
        if groups.is_empty() {
            return TextRaster::empty();
        }

        let mut pixels = vec![[0.0f32; 4]; wu * hu];
        // Boxes behind each line shown so far, fading with it, drawn into one mask so
        // translucent colours do not double up where boxes touch.
        if style.background.enabled {
            let mut mask = vec![0.0f32; wu * hu];
            for (line_index, run) in lines.iter().enumerate() {
                let alpha = line_alpha[line_index];
                let extent = run
                    .glyphs
                    .iter()
                    .filter(|g| {
                        run.text
                            .get(g.start..g.end)
                            .is_some_and(|t| !t.trim().is_empty())
                    })
                    .fold(None, |e: Option<(f32, f32)>, g| {
                        let (l, r) = (g.x, g.x + g.w);
                        Some(e.map_or((l, r), |(a, b)| (a.min(l), b.max(r))))
                    });
                let Some((left, right)) = extent.filter(|_| alpha > 0.0) else {
                    continue;
                };
                let font = run.glyphs.iter().map(|g| g.font_size).fold(size, f32::max);
                let rect = [
                    origin.0 + left - padding,
                    origin.1 + run.line_y - font * 0.92 - padding * 0.6,
                    origin.0 + right + padding,
                    origin.1 + run.line_y + font * 0.26 + padding * 0.6,
                ];
                let radius = style.background.radius as f32 * (rect[3] - rect[1]);
                let mut line_mask = vec![0.0f32; wu * hu];
                rounded_rect(&mut line_mask, w as u32, h as u32, rect, radius);
                for (cell, coverage) in mask.iter_mut().zip(line_mask) {
                    *cell = cell.max(coverage * alpha);
                }
            }
            let color = linear_color(style.background.color.as_str());
            for (pixel, &coverage) in pixels.iter_mut().zip(&mask) {
                over(pixel, color, coverage);
            }
        }

        // Each group's outline coverage (ink included), shared by the shadow and stroke.
        let outlines: Vec<Option<Vec<f32>>> = groups
            .values()
            .map(|g| (stroke > 0.0).then(|| outline(&g.ink, wu, hu, stroke)))
            .collect();
        if style.shadow.enabled {
            let color = linear_color(style.shadow.color.as_str());
            let (dx, dy) = (
                em(style.shadow.offset[0]).round() as i32,
                em(style.shadow.offset[1]).round() as i32,
            );
            for (group, outlined) in groups.values().zip(&outlines) {
                let source = outlined.as_ref().unwrap_or(&group.ink);
                let mut shadow = vec![0.0f32; wu * hu];
                for y in 0..h {
                    for x in 0..w {
                        let (sx, sy) = (x - dx, y - dy);
                        if sx >= 0 && sy >= 0 && sx < w && sy < h {
                            shadow[(y * w + x) as usize] = source[(sy * w + sx) as usize];
                        }
                    }
                }
                blur(&mut shadow, wu, hu, shadow_sigma);
                for (pixel, &coverage) in pixels.iter_mut().zip(&shadow) {
                    over(pixel, color, coverage * group.alpha);
                }
            }
        }
        if style.glow.enabled {
            let color = linear_color(style.glow.color.as_str());
            for (group, outlined) in groups.values().zip(&outlines) {
                let mut glow = outlined.clone().unwrap_or_else(|| group.ink.clone());
                blur(&mut glow, wu, hu, glow_sigma);
                for (pixel, &coverage) in pixels.iter_mut().zip(&glow) {
                    // Doubled so the light stays bright next to the letters.
                    over(pixel, color, (coverage * 2.0).min(1.0) * group.alpha);
                }
            }
        }
        let stroke_color = linear_color(style.stroke.color.as_str());
        for (group, outlined) in groups.values().zip(&outlines) {
            if let Some(outlined) = outlined {
                for (pixel, &coverage) in pixels.iter_mut().zip(outlined) {
                    over(pixel, stroke_color, coverage * group.alpha);
                }
            }
        }
        for group in groups.values() {
            for (pixel, fill) in pixels.iter_mut().zip(&group.fill) {
                let alpha = fill[3] * group.alpha;
                if alpha <= 0.0 {
                    continue;
                }
                for channel in 0..3 {
                    pixel[channel] = fill[channel] * group.alpha + pixel[channel] * (1.0 - alpha);
                }
                pixel[3] = alpha + pixel[3] * (1.0 - alpha);
            }
        }

        // Crop to the columns that show anything, keeping a little transparent border.
        let shows = |x: i32| (0..h).any(|y| pixels[(y * w + x) as usize][3] > 0.0);
        let (Some(first), Some(last)) =
            ((0..w).find(|&x| shows(x)), (0..w).rev().find(|&x| shows(x)))
        else {
            return TextRaster::empty();
        };
        let left = (first - PADDING as i32).max(0);
        let right = (last + PADDING as i32).min(w - 1);
        let width = (right - left + 1) as u32;
        let mut cropped = Vec::with_capacity(width as usize * hu);
        for y in 0..h {
            let row = (y * w) as usize;
            cropped.extend_from_slice(&pixels[row + left as usize..=row + right as usize]);
        }
        TextRaster {
            width,
            height: h as u32,
            pixels: cropped,
            layout_box: [
                ink_left + (margin_x - left) as f32,
                margin_y as f32,
                ink_right - ink_left,
                height,
            ],
        }
    }
}

/// The shaping attributes of a look; `index` comes back on its glyphs as metadata.
fn attrs_of(look: &CharLook, index: usize, spacing: f32, line_height: f32) -> Attrs<'_> {
    let attrs = Attrs::new()
        .family(Family::Name(&look.font))
        .weight(if look.bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        })
        .style(if look.italic {
            Style::Italic
        } else {
            Style::Normal
        })
        .metadata(index)
        .letter_spacing(spacing)
        .metrics(Metrics::new(look.size, look.size * line_height));
    if look.underline {
        attrs.underline(UnderlineStyle::Single)
    } else {
        attrs
    }
}

/// Words that start before written character `index`.
fn words_before(written: &[char], index: usize) -> usize {
    let mut words = 0usize;
    let mut previous_space = true;
    for c in written.iter().take(index) {
        if previous_space && !c.is_whitespace() {
            words += 1;
        }
        previous_space = c.is_whitespace();
    }
    // A character inside a word belongs to the word that started before it.
    match written.get(index) {
        Some(c) if !c.is_whitespace() && !previous_space => words.saturating_sub(1),
        _ => words,
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

    fn content(text: &str, edit: impl FnOnce(&mut TextStyle)) -> TextLayer {
        let mut style = TextStyle::default();
        edit(&mut style);
        TextLayer::still(TextContent::styled(text, style))
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
            full.coverage().contains(&255),
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
        let ink = |r: &TextRaster| r.coverage().iter().map(|&c| u64::from(c)).sum::<u64>();
        assert!(ink(&bold) > ink(&regular) * 11 / 10, "bold uses more ink");
        assert_ne!(italic.pixels, regular.pixels);

        // Two lines of different length: the short one sits left, centred or right.
        let lines = "a much longer first line\nshort";
        let first_ink_column = |raster: &TextRaster| {
            let bottom = raster.height - 1 - PADDING - raster.height / 8;
            (0..raster.width)
                .find(|&x| (raster.height / 2..bottom).any(|y| raster.alpha(x, y) > 0.5))
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

    fn styled_text(text: &str, edit: impl FnOnce(&mut TextContent)) -> TextLayer {
        let mut content = TextContent::styled(
            text,
            TextStyle {
                bold: true,
                ..TextStyle::default()
            },
        );
        edit(&mut content);
        TextLayer::still(content)
    }

    fn covered(raster: &TextRaster) -> usize {
        raster.pixels.iter().filter(|p| p[3] > 0.5).count()
    }

    /// Mean of a channel, weighted by opacity, over pixels with any opacity.
    fn mean_color(raster: &TextRaster, channel: usize) -> f32 {
        let (sum, weight) = raster
            .pixels
            .iter()
            .filter(|p| p[3] > 0.5)
            .fold((0.0, 0.0), |(s, w), p| (s + p[channel] / p[3], w + 1.0));
        sum / weight
    }

    #[test]
    fn outlines_shadows_glows_and_boxes_draw_around_the_letters() {
        use fosscut_model::{Color, Glow, Shadow, Stroke, TextBox};
        let mut engine = TextEngine::new();
        let plain = engine.rasterize(&styled_text("Layers", |_| {}), CANVAS, 1.0);
        let stroked = engine.rasterize(
            &styled_text("Layers", |c| {
                c.style.stroke = Stroke {
                    enabled: true,
                    ..Stroke::default()
                }
            }),
            CANVAS,
            1.0,
        );
        assert!(
            covered(&stroked) > covered(&plain) * 13 / 10,
            "the outline adds ink"
        );
        assert!(
            mean_color(&stroked, 0) < mean_color(&plain, 0) * 0.8,
            "black outline"
        );
        // The layout box keeps the text in place whatever the margins.
        let centre = |r: &TextRaster| {
            let [x, y, w, h] = r.layout_box;
            (
                x + w / 2.0 - r.width as f32 / 2.0,
                y + h / 2.0 - r.height as f32 / 2.0,
            )
        };
        let (a, b) = (centre(&plain), centre(&stroked));
        assert!(
            (a.0 - b.0).abs() < 2.0 && (a.1 - b.1).abs() < 2.0,
            "{a:?} {b:?}"
        );

        let shadowed = engine.rasterize(
            &styled_text("Layers", |c| {
                c.style.shadow = Shadow {
                    enabled: true,
                    color: Color::parse("#FF0000").unwrap(),
                    offset: [0.2, 0.2],
                    blur: 0.0,
                }
            }),
            CANVAS,
            1.0,
        );
        // Red shows only below and right of the white letters.
        let red = |p: [f32; 4]| p[3] > 0.9 && p[0] / p[3] > 0.9 && p[1] / p[3] < 0.1;
        let (mut red_x, mut red_y, mut reds) = (0.0, 0.0, 0.0);
        let (mut white_x, mut white_y, mut whites) = (0.0, 0.0, 0.0);
        for y in 0..shadowed.height {
            for x in 0..shadowed.width {
                let p = shadowed.at(x, y);
                if red(p) {
                    (red_x, red_y, reds) = (red_x + x as f32, red_y + y as f32, reds + 1.0);
                } else if p[3] > 0.9 && p[1] / p[3] > 0.9 {
                    (white_x, white_y, whites) =
                        (white_x + x as f32, white_y + y as f32, whites + 1.0);
                }
            }
        }
        assert!(reds > 100.0 && whites > 100.0);
        let shift = (
            red_x / reds - white_x / whites,
            red_y / reds - white_y / whites,
        );
        let em = 0.08 * 1080.0;
        assert!(
            shift.0 > em * 0.1 && shift.1 > em * 0.1,
            "shadow falls right and down: {shift:?}"
        );

        let glowing = engine.rasterize(
            &styled_text("Layers", |c| {
                c.style.glow = Glow {
                    enabled: true,
                    color: Color::parse("#00FF00").unwrap(),
                    radius: 0.5,
                }
            }),
            CANVAS,
            1.0,
        );
        let faint = |r: &TextRaster| {
            r.pixels
                .iter()
                .filter(|p| p[3] > 0.02 && p[3] < 0.5)
                .count()
        };
        assert!(
            faint(&glowing) > faint(&plain) * 5,
            "the glow spreads softly"
        );
        assert!(glowing.height > plain.height, "with room for the glow");

        let boxed = engine.rasterize(
            &styled_text("Boxed\nlines", |c| {
                c.style.background = TextBox {
                    enabled: true,
                    color: Color::parse("#0000FF").unwrap(),
                    ..TextBox::default()
                }
            }),
            CANVAS,
            1.0,
        );
        let blue = boxed
            .pixels
            .iter()
            .filter(|p| p[3] > 0.99 && p[2] > 0.99 && p[0] < 0.01)
            .count();
        assert!(
            blue > covered(&boxed) / 3,
            "a box behind both lines: {blue}"
        );
    }

    #[test]
    fn spans_case_spacing_underline_and_line_height_change_the_layout() {
        use fosscut_model::{Color, TextCase, TextSpan};
        let mut engine = TextEngine::new();
        let base = engine.rasterize(&styled_text("Gold rush", |_| {}), CANVAS, 1.0);
        let gold = engine.rasterize(
            &styled_text("Gold rush", |c| {
                c.spans = vec![TextSpan {
                    start: 0,
                    end: 4,
                    color: Color::parse("#FFCC00").ok(),
                    bold: None,
                    italic: None,
                    underline: None,
                    font: None,
                    size: None,
                }];
            }),
            CANVAS,
            1.0,
        );
        // The first word turns gold, the second stays white.
        let left = |r: &TextRaster| {
            (0..r.height)
                .map(|y| r.at(r.width / 5, y))
                .find(|p| p[3] > 0.9)
        };
        let right = |r: &TextRaster| {
            (0..r.height)
                .map(|y| r.at(r.width * 4 / 5, y))
                .find(|p| p[3] > 0.9)
        };
        let l = left(&gold).unwrap();
        assert!(l[2] / l[3] < 0.1 && l[0] / l[3] > 0.9, "gold: {l:?}");
        let r = right(&gold).unwrap();
        assert!(r[2] / r[3] > 0.9, "white: {r:?}");
        assert_eq!((gold.width, gold.height), (base.width, base.height));

        let spaced = engine.rasterize(
            &styled_text("Gold rush", |c| c.style.letter_spacing = 0.3),
            CANVAS,
            1.0,
        );
        assert!(
            spaced.width as f32 > base.width as f32 * 1.4,
            "{} vs {}",
            spaced.width,
            base.width
        );
        let upper = engine.rasterize(
            &styled_text("Gold rush", |c| c.style.case = TextCase::Upper),
            CANVAS,
            1.0,
        );
        assert!(upper.width > base.width && covered(&upper) > covered(&base));
        let underlined = engine.rasterize(
            &styled_text("Gold rush", |c| c.style.underline = true),
            CANVAS,
            1.0,
        );
        assert!(covered(&underlined) > covered(&base) + base.width as usize);
        let lines = |height: f64| {
            let mut engine = TextEngine::new();
            engine
                .rasterize(
                    &styled_text("One\nTwo", |c| c.style.line_height = height),
                    CANVAS,
                    1.0,
                )
                .height
        };
        assert!(lines(2.0) as f32 > lines(1.2) as f32 * 1.5);
        let bigger = engine.rasterize(
            &styled_text("Gold rush", |c| {
                c.spans = vec![TextSpan {
                    start: 5,
                    end: 9,
                    color: None,
                    bold: None,
                    italic: None,
                    underline: None,
                    font: None,
                    size: Some(0.16),
                }];
            }),
            CANVAS,
            1.0,
        );
        assert!(bigger.height > base.height && bigger.width > base.width);
    }

    #[test]
    fn reveals_show_letters_words_and_lines_over_time() {
        use fosscut_model::{RevealMotion, RevealUnit, TextReveal};
        let mut engine = TextEngine::new();
        let second = Flicks::from_seconds(1).unwrap();
        let at = |engine: &mut TextEngine, unit, motion, millis| {
            let mut layer = styled_text("one two three\nfour\nfive", |c| {
                c.reveal = Some(TextReveal {
                    unit,
                    motion,
                    duration: second,
                    exit: false,
                });
            });
            layer.time = Flicks::from_millis(millis);
            layer.duration = Flicks::from_seconds(3).unwrap();
            engine.rasterize(&layer, CANVAS, 1.0)
        };
        let full = engine.rasterize(
            &styled_text("one two three\nfour\nfive", |_| {}),
            CANVAS,
            1.0,
        );
        for unit in [RevealUnit::Letter, RevealUnit::Word, RevealUnit::Line] {
            // 17 letters, 5 words or 3 lines over a second: part shows at 0.2 s, more at 0.5 s.
            let early = at(&mut engine, unit, RevealMotion::Appear, 200);
            let late = at(&mut engine, unit, RevealMotion::Appear, 500);
            let done = at(&mut engine, unit, RevealMotion::Appear, 1500);
            assert!(covered(&early) < covered(&late), "{unit:?} grows");
            assert!(
                covered(&late) < covered(&full),
                "{unit:?} not done at 0.5 s"
            );
            assert_eq!(done.pixels, full.pixels, "{unit:?} ends as the plain text");
        }
        // Fading words are partly transparent; rising ones sit lower than at rest.
        let fading = at(&mut engine, RevealUnit::Word, RevealMotion::Fade, 400);
        assert!(fading.pixels.iter().any(|p| p[3] > 0.1 && p[3] < 0.8));
        assert!(at(&mut engine, RevealUnit::Word, RevealMotion::Fade, 0).is_empty());
        let still = TextLayer::still(TextContent::plain("x"));
        assert_eq!(still.moment(), None);
    }

    #[test]
    fn font_files_add_families() {
        let data = fixture_font("Fixture Sans");
        let (families, faces) = font_families(&data).unwrap();
        assert_eq!((families, faces), (vec!["Fixture Sans".to_string()], 1));
        assert!(font_families(b"not a font").is_err());
        let mut engine = TextEngine::new();
        // Bold text in the new family: it has only a regular face, so once added the text
        // draws regular, while Inter would draw it bold.
        let named = |engine: &mut TextEngine| {
            engine.rasterize(
                &TextLayer::still(TextContent::styled(
                    "Fonts",
                    TextStyle {
                        font: Some("Fixture Sans".into()),
                        bold: true,
                        ..TextStyle::default()
                    },
                )),
                CANVAS,
                1.0,
            )
        };
        assert_eq!(engine.add_font(data), vec!["Fixture Sans".to_string()]);
        assert!(engine.families().contains(&"Fixture Sans".to_string()));
        let added = named(&mut engine);
        let regular = engine.rasterize(&content("Fonts", |_| {}), CANVAS, 1.0);
        let bold = engine.rasterize(&content("Fonts", |s| s.bold = true), CANVAS, 1.0);
        assert_eq!(
            added.pixels, regular.pixels,
            "the fixture is Inter Regular renamed"
        );
        assert_ne!(
            added.pixels, bold.pixels,
            "drawn in the added family, not Inter Bold"
        );
    }
}
