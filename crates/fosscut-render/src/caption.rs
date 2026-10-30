//! Captions (ADR-0021): every word's look evaluated from word times alone, and caption pages
//! composed on the CPU at the output resolution. Words are laid out once at rest, so
//! animation never reflows text; each word is then drawn with its outline, fill or karaoke
//! fill, scale and box, and the page is placed inside the canvas's safe area.

use cosmic_text::{
    Attrs, Buffer, Color as GlyphColor, Family, Metrics, Shaping, Style as FontStyle, Weight,
};
use fosscut_model::{
    Animator, CaptionContent, CaptionStyle, FontWeight, Placement, TextContent, TextStyle,
    safe_margins,
};
use fosscut_time::Flicks;

use crate::raster::{outline, over, rounded_rect};
use crate::text::{DEFAULT_FAMILY, LINE_HEIGHT, TextEngine, TextLayer, TextRaster, linear_color};

/// A caption page at one moment: what the plan hands the renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionLayer {
    /// The track's look.
    pub style: CaptionStyle,
    /// The page's words, timed from the clip's start.
    pub content: CaptionContent,
    /// The moment shown, from the clip's start.
    pub time: Flicks,
    /// The page's length.
    pub duration: Flicks,
}

/// One word's look at a moment.
#[derive(Clone, Debug, PartialEq)]
pub struct WordLook {
    /// Opacity, 0 to 1.
    pub alpha: f32,
    /// Fill colour, linear RGB with alpha.
    pub color: [f32; 4],
    /// Size relative to rest, around the word's centre.
    pub scale: f32,
    /// Karaoke fill: its colour and how far it has reached, 0 to 1, from the left.
    pub fill: Option<([f32; 4], f32)>,
    /// Colour of a box behind the word.
    pub box_color: Option<[f32; 4]>,
}

/// A page's look at a moment.
#[derive(Clone, Debug, PartialEq)]
pub struct PageLook {
    /// Opacity of the whole page.
    pub alpha: f32,
    /// Size of the whole page relative to rest.
    pub scale: f32,
    /// Every word, line by line.
    pub words: Vec<WordLook>,
}

/// Gaps between words shorter than this keep the previous word spoken (ADR-0021 item 3).
const SHORT_GAP: Flicks = Flicks::from_millis(300);

fn millis(ms: u32) -> Flicks {
    Flicks::from_millis(i64::from(ms))
}

/// `part / whole` clamped to 0..1; 1 when `whole` is not positive.
fn progress(part: Flicks, whole: Flicks) -> f32 {
    if whole.get() <= 0 {
        return if part.get() >= 0 { 1.0 } else { 0.0 };
    }
    (part.get() as f64 / whole.get() as f64).clamp(0.0, 1.0) as f32
}

fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}

/// The word being spoken at `time`: the last word that has started, until it ends or, in a
/// gap shorter than 0.3 s, until the next word starts.
pub fn spoken_word(content: &CaptionContent, time: Flicks) -> Option<usize> {
    let words: Vec<_> = content.words().collect();
    let index = words.iter().rposition(|w| w.start <= time)?;
    let word = words[index];
    let holds = match words.get(index + 1) {
        Some(next) => next.start - word.end < SHORT_GAP,
        None => false,
    };
    (time < word.end || holds).then_some(index)
}

/// Every word's look at the layer's moment, from the style's animators in order (a later
/// animator's colour wins).
pub fn evaluate(layer: &CaptionLayer) -> PageLook {
    let style = &layer.style;
    let t = layer.time;
    let base = linear_color(style.color.as_str());
    let spoken = spoken_word(&layer.content, t);
    let mut page = PageLook {
        alpha: 1.0,
        scale: 1.0,
        words: layer
            .content
            .words()
            .map(|_| WordLook {
                alpha: 1.0,
                color: base,
                scale: 1.0,
                fill: None,
                box_color: None,
            })
            .collect(),
    };
    let words: Vec<_> = layer.content.words().collect();
    for animator in style.animators.iter().filter(|a| a.enabled()) {
        match animator {
            Animator::PageFade(a) => {
                let fade_in = millis(a.params.in_ms);
                let fade_out = millis(a.params.out_ms);
                if fade_in.is_positive() {
                    page.alpha *= progress(t, fade_in);
                }
                if fade_out.is_positive() {
                    page.alpha *= progress(layer.duration - t, fade_out);
                }
            }
            Animator::PagePop(a) => {
                let length = millis(a.params.duration_ms);
                let x = ease_out(progress(t, length));
                page.scale *= a.params.from as f32 + (1.0 - a.params.from as f32) * x;
            }
            Animator::WordReveal(a) => {
                for (look, word) in page.words.iter_mut().zip(&words) {
                    look.alpha *= if t < word.start {
                        0.0
                    } else {
                        progress(t - word.start, millis(a.params.fade_ms))
                    };
                }
            }
            Animator::WordHighlight(a) => {
                if let Some(look) = spoken.and_then(|i| page.words.get_mut(i)) {
                    look.color = linear_color(a.params.color.as_str());
                    look.scale *= a.params.scale as f32;
                    look.box_color = a
                        .params
                        .box_color
                        .as_ref()
                        .map(|c| linear_color(c.as_str()));
                }
            }
            Animator::WordPop(a) => {
                let length = millis(a.params.duration_ms);
                for (look, word) in page.words.iter_mut().zip(&words) {
                    if word.start <= t && t < word.start + length {
                        let phase = progress(t - word.start, length);
                        let bump = (phase * std::f32::consts::PI).sin();
                        look.scale *= 1.0 + (a.params.scale as f32 - 1.0) * bump;
                    }
                }
            }
            Animator::WordKaraoke(a) => {
                let color = linear_color(a.params.color.as_str());
                for (look, word) in page.words.iter_mut().zip(&words) {
                    let reached = if t < word.start {
                        0.0
                    } else {
                        progress(t - word.start, word.end - word.start)
                    };
                    look.fill = Some((color, reached));
                }
            }
            Animator::WordEmphasis(a) => {
                for (look, word) in page.words.iter_mut().zip(&words) {
                    if word.emphasis {
                        look.color = linear_color(a.params.color.as_str());
                        look.scale *= a.params.scale as f32;
                    }
                }
            }
        }
    }
    page
}

/// A composed caption page: premultiplied linear RGBA at the output resolution.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionRaster {
    /// Width in output pixels.
    pub width: u32,
    /// Height in output pixels.
    pub height: u32,
    /// `width * height` premultiplied linear RGBA pixels, rows top to bottom.
    pub pixels: Vec<[f32; 4]>,
    /// Height of the page's lines in output pixels; the page is centred in the raster.
    pub lines_height: f32,
}

impl CaptionRaster {
    /// Nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The pixel at `x`, `y`.
    pub fn at(&self, x: u32, y: u32) -> [f32; 4] {
        self.pixels[(y * self.width + x) as usize]
    }
}

/// Coverage of one word drawn alone, with room around it.
struct Ink {
    width: i32,
    height: i32,
    coverage: Vec<f32>,
    /// The advance box's left edge and width inside the bitmap; its top is `pad`.
    pad: i32,
    advance: f32,
}

impl Ink {
    fn at(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            0.0
        } else {
            self.coverage[(y * self.width + x) as usize]
        }
    }
}

fn weight(weight: FontWeight) -> Weight {
    match weight {
        FontWeight::Regular => Weight::NORMAL,
        FontWeight::Bold => Weight::BOLD,
        FontWeight::ExtraBold => Weight::EXTRA_BOLD,
        FontWeight::Black => Weight::BLACK,
    }
}

impl TextEngine {
    /// `text` shaped on one line at `size` pixels in the caption's font.
    fn shaped(&mut self, text: &str, size: f32, style: &CaptionStyle) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * LINE_HEIGHT));
        let attrs = Attrs::new()
            .family(Family::Name(DEFAULT_FAMILY))
            .weight(weight(style.weight))
            .style(if style.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            });
        buffer.set_size(None, None);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    /// The advance width of `text` at `size` pixels.
    fn measure(&mut self, text: &str, size: f32, style: &CaptionStyle) -> f32 {
        self.shaped(text, size, style)
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0, f32::max)
    }

    /// Draws `text` alone at `size` pixels with `pad` pixels of room on every side.
    fn word_ink(&mut self, text: &str, size: f32, style: &CaptionStyle, pad: i32) -> Ink {
        let mut buffer = self.shaped(text, size, style);
        let advance = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0, f32::max);
        let width = advance.ceil() as i32 + 2 * pad;
        let height = (size * LINE_HEIGHT).ceil() as i32 + 2 * pad;
        let mut coverage = vec![0.0f32; (width.max(0) * height.max(0)) as usize];
        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            GlyphColor::rgb(255, 255, 255),
            |x, y, gw, gh, color| {
                let alpha = f32::from(color.a()) / 255.0;
                if alpha == 0.0 {
                    return;
                }
                for dy in 0..gh as i32 {
                    for dx in 0..gw as i32 {
                        let (px, py) = (x + dx + pad, y + dy + pad);
                        if px < 0 || py < 0 || px >= width || py >= height {
                            continue;
                        }
                        let cell = &mut coverage[(py * width + px) as usize];
                        *cell = alpha + *cell * (1.0 - alpha);
                    }
                }
            },
        );
        Ink {
            width,
            height,
            coverage,
            pad,
            advance,
        }
    }

    /// Composes a caption page at `scale` output pixels per canvas pixel for a canvas of
    /// `canvas` pixels.
    pub fn caption(
        &mut self,
        layer: &CaptionLayer,
        look: &PageLook,
        canvas: (u32, u32),
        scale: f32,
    ) -> CaptionRaster {
        let style = &layer.style;
        let size = (style.size as f32 * canvas.1 as f32 * scale).max(1.0);
        let line_height = size * LINE_HEIGHT;
        let [left, _, right, _] = safe_margins(canvas.0, canvas.1);
        let safe_width = (canvas.0 as f32 * (1.0 - left as f32 - right as f32) * scale).max(size);
        let shown = |text: &str| {
            if style.uppercase {
                text.to_uppercase()
            } else {
                text.to_string()
            }
        };

        // Lay out at rest: measure words, wrap lines wider than the safe area.
        let stroke = if style.stroke.enabled {
            (style.stroke.width as f32 * size).max(0.0)
        } else {
            0.0
        };
        // Outlines grow into the gaps between words; widen them so words stay apart.
        let space = {
            let pair = self.measure("x x", size, style);
            (pair - 2.0 * self.measure("x", size, style)).max(size * 0.2) + 1.5 * stroke
        };
        let mut lines: Vec<Vec<(usize, String, f32)>> = Vec::new();
        let mut index = 0;
        for line in &layer.content.lines {
            let mut current: Vec<(usize, String, f32)> = Vec::new();
            let mut used = 0.0;
            for word in &line.words {
                let text = shown(&word.text);
                let width = self.measure(&text, size, style);
                if !current.is_empty() && used + space + width > safe_width {
                    lines.push(std::mem::take(&mut current));
                    used = 0.0;
                }
                used += if current.is_empty() {
                    width
                } else {
                    space + width
                };
                current.push((index, text, width));
                index += 1;
            }
            if !current.is_empty() {
                lines.push(current);
            }
        }
        let line_widths: Vec<f32> = lines
            .iter()
            .map(|line| {
                line.iter().map(|(_, _, w)| w).sum::<f32>()
                    + space * line.len().saturating_sub(1) as f32
            })
            .collect();
        let page_width = line_widths.iter().copied().fold(0.0, f32::max);
        let lines_height = line_height * lines.len() as f32;
        if lines.is_empty() || page_width <= 0.0 {
            return CaptionRaster {
                width: 0,
                height: 0,
                pixels: Vec::new(),
                lines_height: 0.0,
            };
        }

        let padding = if style.background.enabled {
            style.background.padding as f32 * size
        } else {
            0.0
        };
        let margin = (size * 0.6 + stroke + padding).ceil();
        let width = (page_width + 2.0 * margin).ceil() as u32;
        let height = (lines_height + 2.0 * margin).ceil() as u32;
        let mut pixels = vec![[0.0f32; 4]; (width * height) as usize];
        let origin = (
            (width as f32 - page_width) / 2.0,
            (height as f32 - lines_height) / 2.0,
        );

        // Rest boxes of every word: x, y, width.
        let mut boxes = vec![(0.0f32, 0.0f32, 0.0f32); index];
        for (row, line) in lines.iter().enumerate() {
            let mut x = origin.0
                + match style.align {
                    fosscut_model::Align::Left => 0.0,
                    fosscut_model::Align::Center => (page_width - line_widths[row]) / 2.0,
                    fosscut_model::Align::Right => page_width - line_widths[row],
                };
            let y = origin.1 + row as f32 * line_height;
            for (word, _, w) in line {
                boxes[*word] = (x, y, *w);
                x += w + space;
            }
        }

        // Line boxes behind the words shown so far, fading with them. Each is drawn into
        // a mask first, so translucent colours do not double up where boxes touch.
        if style.background.enabled {
            let color = linear_color(style.background.color.as_str());
            let mut mask = vec![0.0f32; pixels.len()];
            for line in &lines {
                let shown: Vec<usize> = line
                    .iter()
                    .map(|(i, _, _)| *i)
                    .filter(|&i| look.words.get(i).is_some_and(|w| w.alpha > 0.0))
                    .collect();
                let (Some(&first), Some(&last)) = (shown.first(), shown.last()) else {
                    continue;
                };
                let strength = shown
                    .iter()
                    .filter_map(|&i| look.words.get(i))
                    .map(|w| w.alpha)
                    .fold(0.0, f32::max);
                let (x0, y, _) = boxes[first];
                let (x1, _, w1) = boxes[last];
                let rect = [
                    x0 - padding,
                    y + line_height * 0.08 - padding * 0.5,
                    x1 + w1 + padding,
                    y + line_height * 0.92 + padding * 0.5,
                ];
                let radius = style.background.radius as f32 * (rect[3] - rect[1]);
                let mut line_mask = vec![0.0f32; pixels.len()];
                rounded_rect(&mut line_mask, width, height, rect, radius);
                for (cell, coverage) in mask.iter_mut().zip(line_mask) {
                    *cell = cell.max(coverage * strength);
                }
            }
            for (pixel, &coverage) in pixels.iter_mut().zip(&mask) {
                over(pixel, color, coverage);
            }
        }

        let stroke_color = linear_color(style.stroke.color.as_str());
        for (word_index, text, _) in lines.iter().flatten() {
            let Some(look) = look.words.get(*word_index) else {
                continue;
            };
            if look.alpha <= 0.0 {
                continue;
            }
            let (x, y, w) = boxes[*word_index];
            let center = (x + w / 2.0, y + line_height / 2.0);
            let word_size = size * look.scale.max(0.01);
            let word_stroke = stroke * look.scale.max(0.01);
            if let Some(color) = look.box_color {
                let half = (
                    w * look.scale / 2.0 + size * 0.15,
                    line_height * look.scale * 0.42,
                );
                let rect = [
                    center.0 - half.0,
                    center.1 - half.1,
                    center.0 + half.0,
                    center.1 + half.1,
                ];
                let mut mask = vec![0.0f32; pixels.len()];
                rounded_rect(&mut mask, width, height, rect, (rect[3] - rect[1]) * 0.25);
                for (pixel, &coverage) in pixels.iter_mut().zip(&mask) {
                    over(pixel, color, coverage * look.alpha);
                }
            }
            let pad = word_stroke.ceil() as i32 + 2;
            let ink = self.word_ink(text, word_size, style, pad);
            let outline = (word_stroke > 0.0).then(|| {
                outline(
                    &ink.coverage,
                    ink.width as usize,
                    ink.height as usize,
                    word_stroke,
                )
            });
            // Place the word's advance box centre on its rest centre.
            let left = center.0 - (ink.pad as f32 + ink.advance / 2.0);
            let top = center.1 - (ink.pad as f32 + word_size * LINE_HEIGHT / 2.0);
            let (ox, oy) = (left.round() as i32, top.round() as i32);
            let karaoke_edge = look
                .fill
                .map(|(_, reached)| ink.pad as f32 + ink.advance * reached);
            for by in 0..ink.height {
                let py = oy + by;
                if py < 0 || py >= height as i32 {
                    continue;
                }
                for bx in 0..ink.width {
                    let px = ox + bx;
                    if px < 0 || px >= width as i32 {
                        continue;
                    }
                    let pixel = &mut pixels[(py as u32 * width + px as u32) as usize];
                    if let Some(outline) = &outline {
                        over(
                            pixel,
                            stroke_color,
                            outline[(by * ink.width + bx) as usize] * look.alpha,
                        );
                    }
                    let coverage = ink.at(bx, by);
                    if coverage > 0.0 {
                        let color = match (look.fill, karaoke_edge) {
                            (Some((fill, _)), Some(edge)) if (bx as f32 + 0.5) < edge => fill,
                            _ => look.color,
                        };
                        over(pixel, color, coverage * look.alpha);
                    }
                }
            }
        }
        let page = CaptionRaster {
            width,
            height,
            pixels,
            lines_height: lines_height / scale.max(f32::EPSILON),
        };
        match layer.content.translation.as_deref().map(str::trim) {
            Some(translation) if !translation.is_empty() => {
                let shown = look.words.iter().map(|w| w.alpha).fold(0.0, f32::max);
                let line = self.translation(style, translation, canvas, scale);
                stack(page, margin, &line, size * 0.15, shown, scale)
            }
            _ => page,
        }
    }

    /// The translation line of a bilingual page, composed as text in the caption's font,
    /// outline and box.
    fn translation(
        &mut self,
        style: &CaptionStyle,
        translation: &str,
        canvas: (u32, u32),
        scale: f32,
    ) -> TextRaster {
        let look = &style.translation;
        let text = TextStyle {
            font: style.font.clone(),
            size: style.size * look.size,
            color: look.color.clone(),
            bold: style.weight != FontWeight::Regular,
            italic: look.italic,
            align: style.align,
            stroke: style.stroke.clone(),
            background: style.background.clone(),
            ..TextStyle::default()
        };
        self.rasterize(
            &TextLayer::still(TextContent::styled(translation, text)),
            canvas,
            scale,
        )
    }
}

/// A page with its translation under it, `gap` pixels apart, the translation at opacity
/// `shown`. The result is padded so the words and translation together sit in its middle,
/// as placement expects.
fn stack(
    page: CaptionRaster,
    page_margin: f32,
    line: &TextRaster,
    gap: f32,
    shown: f32,
    scale: f32,
) -> CaptionRaster {
    if line.is_empty() || shown <= 0.0 {
        return page;
    }
    let page_top = page_margin.ceil() as u32;
    let page_lines = (page.lines_height * scale).round() as u32;
    let [_, line_top, _, line_lines] = line.layout_box;
    let (line_top, line_lines) = (line_top.round() as u32, line_lines.round() as u32);
    let line_bottom = line.height.saturating_sub(line_top + line_lines);
    let gap = gap.round() as u32;
    // Rows: top margin, page lines, gap, translation lines, bottom margin; equal margins.
    let margin = page_top.max(line_top).max(line_bottom);
    let content = page_lines + gap + line_lines;
    let width = page.width.max(line.width);
    let height = 2 * margin + content;
    let mut pixels = vec![[0.0f32; 4]; (width * height) as usize];
    let mut paste = |source: &[[f32; 4]], w: u32, h: u32, top: i64, alpha: f32| {
        let left = (width - w) / 2;
        for y in 0..h {
            let row = top + i64::from(y);
            if row < 0 || row >= i64::from(height) {
                continue;
            }
            for x in 0..w {
                let p = source[(y * w + x) as usize];
                let target = &mut pixels[(row as u32 * width + left + x) as usize];
                let a = p[3] * alpha;
                for c in 0..3 {
                    target[c] = p[c] * alpha + target[c] * (1.0 - a);
                }
                target[3] = a + target[3] * (1.0 - a);
            }
        }
    };
    paste(
        &page.pixels,
        page.width,
        page.height,
        i64::from(margin) - i64::from(page_top),
        1.0,
    );
    paste(
        &line.pixels,
        line.width,
        line.height,
        i64::from(margin + page_lines + gap) - i64::from(line_top),
        shown,
    );
    CaptionRaster {
        width,
        height,
        pixels,
        lines_height: content as f32 / scale.max(f32::EPSILON),
    }
}

/// The page's centre in canvas pixels for a page `lines_height` canvas pixels tall, inside
/// the canvas's safe area at the style's placement (ADR-0021 item 5).
pub fn anchor(canvas: (u32, u32), placement: Placement, lines_height: f64) -> (f64, f64) {
    let (w, h) = (f64::from(canvas.0), f64::from(canvas.1));
    let [left, top, right, bottom] = safe_margins(canvas.0, canvas.1);
    let x = w * (1.0 + left - right) / 2.0;
    let y = match placement {
        Placement::Top => h * top + lines_height / 2.0,
        Placement::Middle => h * (1.0 + top - bottom) / 2.0,
        Placement::Bottom => h * (1.0 - bottom) - lines_height / 2.0,
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_model::{
        Animated, CaptionLine, CaptionWord, Color, PageFade, PagePop, WordEmphasis, WordHighlight,
        WordKaraoke, WordPop, WordReveal,
    };

    fn ms(n: i64) -> Flicks {
        Flicks::from_millis(n)
    }

    /// "Big news / today" with "news" emphasised: 0-300, 300-700 and 900-1400 ms.
    fn content() -> CaptionContent {
        let word = |text: &str, start, end, emphasis| CaptionWord {
            text: text.into(),
            start: ms(start),
            end: ms(end),
            emphasis,
        };
        CaptionContent {
            lines: vec![
                CaptionLine {
                    words: vec![word("Big", 0, 300, false), word("news", 300, 700, true)],
                },
                CaptionLine {
                    words: vec![word("today", 900, 1400, false)],
                },
            ],
            ..Default::default()
        }
    }

    fn layer(animators: Vec<Animator>, time: i64) -> CaptionLayer {
        CaptionLayer {
            style: CaptionStyle {
                animators,
                ..CaptionStyle::default()
            },
            content: content(),
            time: ms(time),
            duration: ms(1600),
        }
    }

    fn yellow() -> [f32; 4] {
        linear_color("#FFE14D")
    }

    #[test]
    fn the_spoken_word_holds_through_short_gaps_only() {
        let content = content();
        let at = |t| spoken_word(&content, ms(t));
        assert_eq!(at(0), Some(0));
        assert_eq!(at(299), Some(0));
        assert_eq!(at(300), Some(1));
        // 700-900 ms is a 0.2 s gap: "news" stays spoken until "today" starts.
        assert_eq!(at(800), Some(1));
        assert_eq!(at(900), Some(2));
        assert_eq!(at(1500), None, "after the last word ends");
        let long_gap = CaptionContent {
            lines: vec![CaptionLine {
                words: vec![
                    content.lines[0].words[0].clone(),
                    CaptionWord {
                        start: ms(1000),
                        end: ms(1200),
                        ..content.lines[1].words[0].clone()
                    },
                ],
            }],
            ..Default::default()
        };
        assert_eq!(spoken_word(&long_gap, ms(500)), None, "a 0.7 s pause");
        assert_eq!(
            spoken_word(&long_gap, ms(-1)),
            None,
            "before the first word"
        );
    }

    #[test]
    fn animators_follow_word_times() {
        let white = linear_color("#FFFFFF");

        // Page fade: half in after 60 of 120 ms, half out 60 ms before the end.
        let fade = |t| evaluate(&layer(vec![Animator::PageFade(Animated::default())], t)).alpha;
        assert!((fade(60) - 0.5).abs() < 1e-6 && fade(800) == 1.0);
        assert!((fade(1540) - 0.5).abs() < 1e-6);

        // Page pop: from 0.8, eased, reaching 1 after 150 ms.
        let pop = |t| evaluate(&layer(vec![Animator::PagePop(Animated::default())], t)).scale;
        assert!((pop(0) - 0.8).abs() < 1e-6 && pop(150) == 1.0);
        assert!(
            pop(75) > 0.8 + 0.2 * 0.5,
            "ease out: past halfway at half time"
        );

        // Reveal: words appear at their start, fading in over 80 ms.
        let reveal = evaluate(&layer(vec![Animator::WordReveal(Animated::default())], 340));
        let alphas: Vec<f32> = reveal.words.iter().map(|w| w.alpha).collect();
        assert_eq!(alphas, [1.0, 0.5, 0.0]);

        // Highlight: only the spoken word changes.
        let highlight = Animator::WordHighlight(Animated {
            params: WordHighlight {
                scale: 1.2,
                box_color: Some(Color::parse("#000000").unwrap()),
                ..WordHighlight::default()
            },
            ..Animated::default()
        });
        let look = evaluate(&layer(vec![highlight.clone()], 950));
        assert_eq!(look.words[2].color, yellow());
        assert_eq!(look.words[2].scale, 1.2);
        assert!(look.words[2].box_color.is_some());
        assert_eq!(look.words[0].color, white);
        assert!(look.words[0].box_color.is_none());

        // Pop: up and back over 200 ms from each word's start, largest halfway.
        let word_pop =
            |t| evaluate(&layer(vec![Animator::WordPop(Animated::default())], t)).words[1].scale;
        assert!((word_pop(400) - 1.2).abs() < 1e-6);
        assert_eq!(word_pop(299), 1.0);
        assert_eq!(word_pop(500), 1.0);

        // Karaoke: the fill reaches as far as the word has been spoken.
        let karaoke = evaluate(&layer(
            vec![Animator::WordKaraoke(Animated {
                params: WordKaraoke::default(),
                ..Animated::default()
            })],
            500,
        ));
        let reached: Vec<f32> = karaoke.words.iter().map(|w| w.fill.unwrap().1).collect();
        assert_eq!(reached, [1.0, 0.5, 0.0]);

        // Emphasis, then highlight: the later animator's colour wins on the spoken word.
        let emphasis = Animator::WordEmphasis(Animated {
            params: WordEmphasis {
                color: Color::parse("#FF0000").unwrap(),
                scale: 1.5,
            },
            ..Animated::default()
        });
        let both = evaluate(&layer(vec![emphasis.clone(), highlight], 400));
        assert_eq!(both.words[1].color, yellow());
        assert!((both.words[1].scale - 1.8).abs() < 1e-6, "scales multiply");
        let only = evaluate(&layer(vec![emphasis], 0));
        assert_eq!(only.words[1].color, linear_color("#FF0000"));

        // Disabled animators do nothing.
        let off = evaluate(&layer(
            vec![Animator::PagePop(Animated {
                enabled: false,
                params: PagePop::default(),
                version: 1,
            })],
            0,
        ));
        assert_eq!(off.scale, 1.0);
        let zero = evaluate(&layer(
            vec![
                Animator::PageFade(Animated {
                    params: PageFade {
                        in_ms: 0,
                        out_ms: 0,
                    },
                    ..Animated::default()
                }),
                Animator::WordReveal(Animated {
                    params: WordReveal { fade_ms: 0 },
                    ..Animated::default()
                }),
                Animator::WordPop(Animated {
                    params: WordPop {
                        scale: 2.0,
                        duration_ms: 0,
                    },
                    ..Animated::default()
                }),
            ],
            300,
        ));
        assert_eq!(zero.alpha, 1.0, "no fade");
        assert_eq!(zero.words[1].alpha, 1.0, "shown at once");
        assert_eq!(zero.words[1].scale, 1.0, "no pop");
    }

    fn raster(style: CaptionStyle, look_time: i64, canvas: (u32, u32)) -> CaptionRaster {
        let layer = CaptionLayer {
            style,
            ..layer(Vec::new(), look_time)
        };
        let look = evaluate(&layer);
        TextEngine::new().caption(&layer, &look, canvas, 0.5)
    }

    fn ink(raster: &CaptionRaster) -> f32 {
        raster.pixels.iter().map(|p| p[3]).sum()
    }

    #[test]
    fn pages_are_composed_with_outline_box_and_fill() {
        let canvas = (1920, 1080);
        let plain = raster(
            CaptionStyle {
                stroke: fosscut_model::Stroke {
                    enabled: false,
                    ..Default::default()
                },
                ..CaptionStyle::default()
            },
            0,
            canvas,
        );
        let outlined = raster(CaptionStyle::default(), 0, canvas);
        assert!(!plain.is_empty());
        assert!(outlined.width > plain.width, "the outline needs room");
        assert!(ink(&outlined) > ink(&plain) * 1.3, "the outline adds ink");
        // Two lines, 0.055 x 1080 x 0.5 px tall each, centred in the raster.
        let line = 0.055 * 1080.0 * 0.5 * LINE_HEIGHT;
        assert!((plain.lines_height - 2.0 * line / 0.5).abs() < 0.01);

        let boxed = raster(
            CaptionStyle {
                background: fosscut_model::CaptionBox {
                    enabled: true,
                    ..Default::default()
                },
                ..CaptionStyle::default()
            },
            0,
            canvas,
        );
        // The box fills the gaps between letters with translucent black.
        let centre = boxed.at(boxed.width / 2, boxed.height / 2);
        assert!(centre[3] > 0.6, "{centre:?}");

        // Karaoke halfway through "news": its left part is cyan, its right part white.
        let karaoke = CaptionStyle {
            stroke: fosscut_model::Stroke {
                enabled: false,
                ..Default::default()
            },
            animators: vec![Animator::WordKaraoke(Animated {
                params: WordKaraoke {
                    color: Color::parse("#00FFFF").unwrap(),
                },
                ..Animated::default()
            })],
            ..CaptionStyle::default()
        };
        let half = raster(karaoke, 500, canvas);
        let first_row = (half.height as f32 / 2.0 - line / 2.0) as u32;
        let row: Vec<[f32; 4]> = (0..half.width).map(|x| half.at(x, first_row)).collect();
        let reds: Vec<f32> = row
            .iter()
            .filter(|p| p[3] > 0.9)
            .map(|p| p[0] / p[3])
            .collect();
        assert!(reds.iter().any(|&r| r < 0.05), "cyan pixels: no red");
        assert!(reds.iter().any(|&r| r > 0.95), "white pixels");

        // Capitals, and the same page composes the same pixels every time.
        let upper = raster(
            CaptionStyle {
                uppercase: true,
                ..CaptionStyle::default()
            },
            0,
            canvas,
        );
        assert_ne!(upper.pixels, outlined.pixels);
        assert_eq!(raster(CaptionStyle::default(), 0, canvas), outlined);
    }

    #[test]
    fn bilingual_pages_show_the_translation_under_the_words() {
        let canvas = (1920, 1080);
        let plain = raster(CaptionStyle::default(), 500, canvas);
        let mut layer = CaptionLayer {
            content: CaptionContent {
                translation: Some("Esto lo cambia todo.".into()),
                ..content()
            },
            ..layer(Vec::new(), 500)
        };
        let look = evaluate(&layer);
        let both = TextEngine::new().caption(&layer, &look, canvas, 0.5);
        assert!(both.height > plain.height && both.lines_height > plain.lines_height * 1.3);
        // The translation is light yellow (less blue than red), in the lower part.
        let yellowish = |p: [f32; 4]| p[3] > 0.9 && p[0] / p[3] > 0.9 && p[2] / p[3] < 0.6;
        let rows: Vec<u32> = (0..both.height)
            .filter(|&y| (0..both.width).any(|x| yellowish(both.at(x, y))))
            .collect();
        assert!(!rows.is_empty(), "translation pixels");
        assert!(rows.iter().all(|&y| y > both.height / 2), "under the words");
        // Hidden pages hide it too, and an empty translation changes nothing.
        layer.content.translation = Some("  ".into());
        let look = evaluate(&layer);
        assert_eq!(TextEngine::new().caption(&layer, &look, canvas, 0.5), plain);
    }

    #[test]
    fn a_growing_word_does_not_move_the_others() {
        let canvas = (1920, 1080);
        let pop = |t| {
            raster(
                CaptionStyle {
                    animators: vec![Animator::WordPop(Animated::default())],
                    ..CaptionStyle::default()
                },
                t,
                canvas,
            )
        };
        // At 1000 ms "today" (second line) pops; the first line is untouched.
        let rest = pop(1500);
        let popped = pop(1000);
        assert_eq!((rest.width, rest.height), (popped.width, popped.height));
        let half = rest.height / 2;
        let top = |r: &CaptionRaster| {
            (0..half * r.width)
                .map(|i| r.pixels[i as usize])
                .collect::<Vec<_>>()
        };
        assert_eq!(top(&rest), top(&popped));
        assert_ne!(rest.pixels, popped.pixels);
    }

    #[test]
    fn long_lines_wrap_inside_the_safe_area() {
        let long = CaptionContent {
            lines: vec![CaptionLine {
                words: (0..12)
                    .map(|i| CaptionWord {
                        text: "wonderful".into(),
                        start: ms(i * 100),
                        end: ms(i * 100 + 100),
                        emphasis: false,
                    })
                    .collect(),
            }],
            ..Default::default()
        };
        let layer = CaptionLayer {
            content: long,
            ..layer(Vec::new(), 0)
        };
        let look = evaluate(&layer);
        let raster = TextEngine::new().caption(&layer, &look, (1080, 1920), 0.25);
        let line = 0.055 * 1920.0 * LINE_HEIGHT;
        assert!(raster.lines_height > 2.5 * line, "{}", raster.lines_height);
        // The page is no wider than the safe width plus its margins.
        let safe = 1080.0 * (1.0 - 0.12) * 0.25;
        assert!((raster.width as f32) < safe + 2.0 * (0.055 * 1920.0 * 0.25 * 0.9 + 4.0));
    }

    #[test]
    fn pages_anchor_inside_the_safe_area() {
        let tall = (1080, 1920);
        let (x, y) = anchor(tall, Placement::Bottom, 200.0);
        assert!((x - 540.0).abs() < 1e-9);
        assert!((y - (1920.0 * 0.78 - 100.0)).abs() < 1e-9);
        let (_, top) = anchor(tall, Placement::Top, 200.0);
        assert!((top - (192.0 + 100.0)).abs() < 1e-9);
        let (_, middle) = anchor(tall, Placement::Middle, 200.0);
        assert!((middle - 1920.0 * 0.44).abs() < 1e-9);
        let (wx, wy) = anchor((1920, 1080), Placement::Bottom, 100.0);
        assert!((wx - 960.0).abs() < 1e-9 && (wy - (1026.0 - 50.0)).abs() < 1e-9);
    }
}
