// Waveform sync, made by FossCut for its library (ADR-0042): two waves of the same sound,
// one out of step, slide into line, and a chip says they are in sync.

// fosscut:defaults
const DEFAULTS = {
  "title": "Syncing audio",
  "top_label": "Camera",
  "bottom_label": "Microphone",
  "offset_label": "Offset 0.42 s",
  "synced_label": "In sync",
  "offset": 0.16,
  "top_color": "#8C96A8",
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, svg, icon, Timeline, pop, fade, slide, random, clamp, ease } = Overlay;
  const { root } = Overlay.stage(1080, 640);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const head = el('div', {}, card);
  head.id = 'head';
  const glyph = el('div', {}, head);
  glyph.id = 'glyph';
  glyph.appendChild(icon('wave', 34, 2.4));
  const title = el('div', { className: 'heading nowrap', text: P.title }, head);
  title.id = 'title';
  const status = el('div', {}, head);
  status.id = 'status';
  const before = el('div', { className: 'chip', text: P.offset_label }, status);
  const after = el('div', { className: 'chip good' }, status);
  after.appendChild(icon('check', 26, 3));
  el('span', { text: P.synced_label }, after);
  before.classList.toggle('hidden', !P.offset_label);

  // One recording's loudness: syllables in words, with pauses between words.
  const next = random(23);
  const bursts = [];
  for (let x = -0.6; x < 1.6;) {
    const syllables = 2 + Math.floor(next() * 4);
    for (let s = 0; s < syllables; s += 1) {
      bursts.push({ at: x, width: 0.012 + next() * 0.016, height: 0.35 + next() * 0.65 });
      x += 0.026 + next() * 0.02;
    }
    x += 0.035 + next() * 0.05;
  }
  const loud = (x) => {
    let a = 0.04;
    for (const b of bursts) {
      const d = (x - b.at) / b.width;
      if (d > -4 && d < 4) {
        a += b.height * Math.exp(-d * d);
      }
    }
    return clamp(a, 0, 1);
  };
  const laneWidth = 888;
  const step = 8;
  const makeWave = (lane, color) => {
    // Drawn from -0.6 to 1.6 lane widths, so sliding never shows an end.
    const span = laneWidth * 2.2;
    const wave = svg('svg', { class: 'wave', width: span, height: 150, viewBox: `0 0 ${span} 150` }, lane);
    wave.style.left = `${-laneWidth * 0.6}px`;
    let d = '';
    for (let x = 0; x <= span; x += step) {
      const f = x / laneWidth - 0.6;
      const h = 4 + loud(f) * 84;
      d += `M${x.toFixed(1)} ${(92 - h / 2).toFixed(1)}v${h.toFixed(1)}`;
    }
    svg('path', { d, stroke: color, 'stroke-width': 5, 'stroke-linecap': 'round' }, wave);
    return wave;
  };
  const lanes = [];
  for (const [text, color] of [[P.top_label, P.top_color || '#8C96A8'], [P.bottom_label, 'var(--accent)']]) {
    const lane = el('div', { className: 'lane' }, card);
    const name = el('div', { className: 'name' }, lane);
    el('i', { style: { background: color } }, name);
    el('span', { text }, name);
    name.classList.toggle('hidden', !text);
    const wave = makeWave(lane, color === 'var(--accent)' ? getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() : color);
    lanes.push({ lane, name, wave });
  }
  // The marker crosses both lanes at the loudest syllable in view.
  let peak = bursts.filter((b) => b.at > 0.3 && b.at < 0.7).sort((a, b) => b.height - a.height)[0];
  if (!peak) {
    peak = { at: 0.5 };
  }
  const marker = el('div', {}, card);
  marker.id = 'marker';
  marker.style.left = `${32 + peak.at * laneWidth}px`;
  marker.style.height = `${150 * 2 + 22}px`;

  const offset = clamp(Number(P.offset) || 0, -0.5, 0.5) * laneWidth;
  const slideStart = 0.85;
  const slideEnd = 1.9;
  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.76, y: 36, duration: 0.55 }));
  tl.key(glyph, pop(0.2, { from: 0.4, strong: true }));
  tl.key(title, slide(0.24, { x: -16, y: 0 }));
  lanes.forEach(({ lane, name }, i) => {
    tl.key(lane, slide(0.3 + i * 0.1, { y: 24 }));
    tl.key(name, pop(0.45 + i * 0.1, { from: 0.6 }));
  });
  tl.key(before, [...pop(0.5, { from: 0.6 }), { at: slideEnd, opacity: 1 }, { at: slideEnd + 0.1, opacity: 0, ease: 'linear' }]);
  tl.key(after, pop(slideEnd + 0.06, { from: 0.4, strong: true }));
  tl.key(marker, [{ at: slideEnd, opacity: 0, sy: 0.3 }, { at: slideEnd + 0.12, opacity: 1, ease: 'outQuad' }, { at: slideEnd + 0.4, sy: 1, ease: 'outBack' }]);
  tl.each((t) => {
    // The waves draw on from the left, then the lower one slides into line.
    lanes.forEach(({ wave }, i) => {
      const drawn = ease.outCubic(clamp((t - 0.35 - i * 0.1) / 0.6));
      wave.style.clipPath = `inset(0 ${((1 - drawn) * 100).toFixed(2)}% 0 0)`;
    });
    const p = ease.inOutCubic(clamp((t - slideStart) / (slideEnd - slideStart)));
    lanes[1].wave.style.transform = `translateX(${(offset * (1 - p)).toFixed(2)}px)`;
    // A soft glow pulses on the marker once aligned.
    const glow = t > slideEnd ? 0.5 + 0.5 * Math.cos((t - slideEnd) * Math.PI * 1.5) : 1;
    marker.style.boxShadow = `0 0 ${(10 + 16 * glow).toFixed(1)}px rgba(18, 183, 106, ${(0.45 + 0.35 * glow).toFixed(3)})`;
  });
  return { timeline: tl };
});
