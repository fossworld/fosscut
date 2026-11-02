// Storyboard list, made by FossCut for its library (ADR-0042): numbered lines with a
// thumbnail, a quote and a time, sliding in one after another.

// fosscut:defaults
const DEFAULTS = {
  "title": "Storyboard",
  "unit": "scenes",
  "quotes": ["Nobody watches past the first three seconds", "So I cut every pause and retake", "Then captions do the heavy lifting", "And the music carries the ending"],
  "times": ["0:04", "0:12", "0:27", "0:41"],
  "thumb_1": null,
  "thumb_2": null,
  "thumb_3": null,
  "thumb_4": null,
  "thumb_5": null,
  "appear": 0,
  "hold": 4,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, media, Timeline, pop, fade, slide } = Overlay;
  const { root } = Overlay.stage(1080, 1180);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const head = el('div', {}, card);
  head.id = 'head';
  const glyph = el('div', {}, head);
  glyph.id = 'glyph';
  glyph.appendChild(icon('film', 32, 2.2));
  const title = el('div', { className: 'heading nowrap', text: P.title }, head);
  title.id = 'title';
  const quotes = (Array.isArray(P.quotes) ? P.quotes : []).slice(0, 5);
  const times = Array.isArray(P.times) ? P.times : [];
  const count = el('div', { className: 'chip accent', text: P.unit ? `${quotes.length} ${P.unit}` : String(quotes.length) }, head);
  count.id = 'count';
  const rows = [];
  const items = [];
  quotes.forEach((quote, i) => {
    const item = el('div', { className: 'item' }, card);
    const num = el('div', { className: 'num', text: i + 1 }, item);
    const thumb = el('div', { className: 'thumb' }, item);
    items.push(media(el('div', {}, thumb), P[`thumb_${i + 1}`], { seed: [80, 82, 84, 86, 87][i], width: 208, height: 117 }));
    const words = el('div', { className: 'words' }, item);
    el('div', { className: 'quote clip-2', text: `\u201C${quote}\u201D` }, words);
    if (times[i]) {
      const time = el('div', { className: 'time' }, words);
      time.appendChild(icon('clock', 22, 2.4));
      el('span', { text: times[i] }, time);
    }
    rows.push({ item, num, thumb });
  });

  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.8, y: 40, duration: 0.55 }));
  tl.key(glyph, pop(0.16, { from: 0.4, strong: true }));
  tl.key(title, slide(0.2, { x: -16, y: 0 }));
  tl.key(count, pop(0.3, { from: 0.5 }));
  rows.forEach(({ item, num, thumb }, i) => {
    const at = 0.35 + i * 0.16;
    tl.key(item, slide(at, { x: 60, y: 0, duration: 0.5, easing: 'outBack' }));
    tl.key(num, pop(at + 0.1, { from: 0.3, strong: true }));
    tl.key(thumb, [{ at: at + 0.05, scale: 0.85 }, { at: at + 0.45, scale: 1, ease: 'outBack' }]);
  });
  return { timeline: tl, media: items };
});
