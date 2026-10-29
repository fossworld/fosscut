//! Captions (ADR-0021): every word's look evaluated from word times alone, and caption pages
//! composed on the CPU at the output resolution. Words are laid out once at rest, so
//! animation never reflows text; each word is then drawn with its outline, fill or karaoke
//! fill, scale and box, and the page is placed inside the canvas's safe area.

use cosmic_text::{
    Attrs, Buffer, Color as GlyphColor, Family, Metrics, Shaping, Style as FontStyle, Weight,
};
use fosscut_model::{Animator, CaptionContent, CaptionStyle, FontWeight, Placement, safe_margins};
use fosscut_time::Flicks;

use crate::text::{DEFAULT_FAMILY, LINE_HEIGHT, TextEngine, linear_color};

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
            let outline = (word_stroke > 0.0).then(|| outline(&ink, word_stroke));
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
        CaptionRaster {
            width,
            height,
            pixels,
            lines_height: lines_height / scale.max(f32::EPSILON),
        }
    }
}

/// Paints `color` (linear, straight alpha) over a premultiplied pixel with `coverage`.
fn over(pixel: &mut [f32; 4], color: [f32; 4], coverage: f32) {
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
fn rounded_rect(mask: &mut [f32], width: u32, height: u32, rect: [f32; 4], radius: f32) {
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

/// Coverage of an outline `radius` pixels wide around a word's ink, from the exact
/// Euclidean distance to the nearest inked pixel (Felzenszwalb and Huttenlocher, "Distance
/// Transforms of Sampled Functions", 2004).
fn outline(ink: &Ink, radius: f32) -> Vec<f32> {
    let (w, h) = (ink.width as usize, ink.height as usize);
    let mut distance: Vec<f32> = ink
        .coverage
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
        .zip(&ink.coverage)
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

    #[test]
    fn the_distance_transform_matches_brute_force() {
        let (w, h) = (23usize, 17usize);
        let inked = |x: usize, y: usize| (x * 7 + y * 3).is_multiple_of(11) || (x == 12 && y > 4);
        let ink = Ink {
            width: w as i32,
            height: h as i32,
            coverage: (0..w * h)
                .map(|i| if inked(i % w, i / w) { 1.0 } else { 0.0 })
                .collect(),
            pad: 0,
            advance: w as f32,
        };
        let outline = outline(&ink, 2.0);
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
