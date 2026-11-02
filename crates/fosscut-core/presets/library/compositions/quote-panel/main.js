// Quote panel, made by FossCut for its library (ADR-0042): a quote revealed word by word
// with a phrase marked in the accent colour, and its author.

// fosscut:defaults
const DEFAULTS = {
  "quote": "Your first draft is never the edit. The edit is what you cut.",
  "highlight": "what you cut",
  "author": "Your Name",
  "role": "Video creator",
  "avatar": null,
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, svg, media, Timeline, pop, fade, slide } = Overlay;
  const { root } = Overlay.stage(1080, 760);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  // An opening quotation mark drawn as two rounded commas.
  const quoteMark = (parent, id) => {
    const node = svg('svg', { id, viewBox: '0 0 120 100' }, parent);
    for (const x of [0, 62]) {
      svg('path', {
        d: `M ${x + 22} 96 C ${x + 8} 96 ${x} 84 ${x} 66 C ${x} 36 ${x + 16} 12 ${x + 46} 2 L ${x + 52} 14 C ${x + 34} 22 ${x + 26} 34 ${x + 26} 46 C ${x + 42} 46 ${x + 52} 56 ${x + 52} 70 C ${x + 52} 86 ${x + 40} 96 ${x + 22} 96 Z`,
        fill: 'currentColor',
      }, node);
    }
    return node;
  };
  const glyph = quoteMark(card, 'glyph');
  const quote = el('div', {}, card);
  quote.id = 'quote';
  const line = el('span', { className: 'line' }, quote);
  const words = String(P.quote || '').split(/\s+/).filter(Boolean);
  // The highlighted phrase: the words of `highlight` where they first appear in order.
  const norm = (w) => w.toLowerCase().replace(/[^\p{L}\p{N}']/gu, '');
  const target = String(P.highlight || '').split(/\s+/).map(norm).filter(Boolean);
  let hotFrom = -1;
  if (target.length) {
    for (let i = 0; i + target.length <= words.length; i += 1) {
      if (target.every((w, k) => norm(words[i + k]) === w)) {
        hotFrom = i;
        break;
      }
    }
  }
  const spans = words.map((word, i) => {
    const hot = hotFrom >= 0 && i >= hotFrom && i < hotFrom + target.length;
    const span = el('span', { className: `w${hot ? ' hot' : ''}` }, line);
    let bg = null;
    if (hot) {
      bg = el('span', { className: 'bg' }, span);
      if (i < hotFrom + target.length - 1) {
        span.classList.add('joined');
      }
    }
    span.appendChild(document.createTextNode(word));
    if (i < words.length - 1) {
      line.appendChild(document.createTextNode(' '));
    }
    return { span, bg, hot };
  });
  const by = el('div', {}, card);
  by.id = 'by';
  const avatar = el('div', { className: 'avatar', text: Overlay.initials(P.author) }, by);
  avatar.style.background = Overlay.gradient(14);
  const face = P.avatar ? media(el('div', {}, avatar), P.avatar, { seed: 14 }) : null;
  const who = el('div', {}, by);
  el('div', { text: P.author }, who).id = 'author';
  const role = el('div', { text: P.role }, who);
  role.id = 'role';
  role.classList.toggle('hidden', !P.role);
  by.classList.toggle('hidden', !P.author && !P.role);

  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.76, y: 36, duration: 0.55 }));
  tl.key(glyph, pop(0.15, { from: 0.3, rotate: -15, strong: true }));
  const step = Math.min(0.07, 1.4 / Math.max(1, words.length));
  spans.forEach(({ span }, i) => tl.key(span, slide(0.3 + i * step, { y: 22, duration: 0.4, fade: 0.2, easing: 'outBack' })));
  const marked = 0.3 + words.length * step + 0.1;
  const hot = spans.filter((s) => s.hot);
  hot.forEach(({ bg }, k) => tl.key(bg, [{ at: marked + k * 0.08, sx: 0 }, { at: marked + k * 0.08 + 0.28, sx: 1, ease: 'outCubic' }]));
  hot.forEach(({ span }, k) => tl.each((t) => {
    span.style.color = t >= marked + k * 0.08 + 0.12 ? 'var(--accent-ink)' : 'var(--ink)';
  }));
  tl.key(avatar, pop(0.5, { from: 0.4 }));
  tl.key(who, slide(0.56, { x: -14, y: 0 }));
  return { timeline: tl, media: face ? [face] : [] };
});
