// Timeline cut, made by FossCut for its library (ADR-0042): the dead parts of a timeline
// are marked and cut away while the total length counts down.

// fosscut:defaults
const DEFAULTS = {
  "label": "Total length",
  "from": "6:24",
  "to": "1:15",
  "status": "Cutting dead air",
  "done": "Tightened",
  "segments": 11,
  "seed": 4,
  "appear": 0,
  "hold": 4,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, svg, icon, Timeline, pop, fade, slide, random, clamp, ease, clockSeconds, clockText, noise } = Overlay;
  const { root } = Overlay.stage(1080, 620);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const head = el('div', {}, card);
  head.id = 'head';
  const left = el('div', {}, head);
  const label = el('div', { text: P.label }, left);
  label.id = 'label';
  label.classList.toggle('hidden', !P.label);
  const count = el('div', { className: 'tnum' }, left);
  count.id = 'count';
  const status = el('div', {}, head);
  status.id = 'status';
  const working = el('div', { className: 'chip bad' }, status);
  working.appendChild(icon('scissors', 26, 2.4));
  el('span', { text: P.status }, working);
  working.classList.toggle('hidden', !P.status);
  const done = el('div', { className: 'chip good' }, status);
  done.appendChild(icon('check', 26, 3));
  el('span', { text: P.done }, done);
  done.classList.toggle('hidden', !P.done);

  const tracks = el('div', {}, card);
  tracks.id = 'tracks';
  const ruler = el('div', {}, tracks);
  ruler.id = 'ruler';
  for (let i = 0; i <= 44; i += 1) {
    const tick = el('i', { className: i % 4 === 0 ? 'major' : '' }, ruler);
    tick.style.left = `${16 + i * 20}px`;
  }
  const from = clockSeconds(P.from);
  const to = clockSeconds(P.to);
  // The kept clips take more of the lane than their share of time, so they stay readable;
  // the counter tells the real lengths.
  const kept = 0.5 + 0.3 * (from > 0 ? clamp(to / from, 0, 1) : 0.5);
  // Kept and cut parts alternate, starting and ending with kept ones.
  const total = clamp(Math.round(Number(P.segments) || 11), 3, 25) | 1;
  const next = random(Number(P.seed) || 0);
  const weights = Array.from({ length: total }, () => 0.55 + next() * 0.9);
  const keptSum = weights.filter((_, i) => i % 2 === 0).reduce((a, b) => a + b, 0);
  const cutSum = weights.filter((_, i) => i % 2 === 1).reduce((a, b) => a + b, 0);
  const width = 888;
  const parts = [];
  let x = 0;
  weights.forEach((w, i) => {
    const keep = i % 2 === 0;
    const size = keep ? (w / keptSum) * kept * width : (w / cutSum) * (1 - kept) * width;
    parts.push({ keep, x, size });
    x += size;
  });
  // Where every kept part ends up once the cut parts are gone.
  let packed = 0;
  for (const part of parts) {
    part.to = packed;
    if (part.keep) {
      packed += part.size;
    }
  }
  const rows = [
    { top: 48, height: 64, kind: 'video' },
    { top: 124, height: 56, kind: 'sound' },
  ];
  const segments = [];
  rows.forEach((row, r) => {
    const lane = el('div', { className: 'row', style: { top: `${row.top}px`, height: `${row.height}px` } }, tracks);
    parts.forEach((part, i) => {
      const seg = el('div', { className: `seg ${part.keep ? 'keep' : 'cut'} ${row.kind}` }, lane);
      const fill = el('div', { className: 'fill' }, seg);
      let flag = null;
      let snip = null;
      if (part.keep && row.kind === 'video') {
        el('b', {}, fill);
      }
      if (row.kind === 'sound') {
        const wave = svg('svg', { class: 'wave', viewBox: '0 0 100 40', preserveAspectRatio: 'none' }, fill);
        let d = '';
        const n = Math.max(3, Math.round(part.size / 9));
        for (let k = 0; k < n; k += 1) {
          const quiet = part.keep ? 1 : 0.12;
          const h = 2 + Math.abs(noise(k * 0.8 + i * 3.1, 9)) * 34 * quiet;
          d += `M${((k + 0.5) / n * 100).toFixed(2)} ${(20 - h / 2).toFixed(2)}v${h.toFixed(2)}`;
        }
        svg('path', { d, stroke: part.keep ? 'var(--accent)' : 'var(--faint)', 'stroke-width': 2.2, 'vector-effect': 'non-scaling-stroke', 'stroke-linecap': 'round' }, wave);
      }
      if (!part.keep) {
        flag = el('div', { className: 'flag' }, seg);
        if (r === 0 && part.size > 30) {
          snip = el('div', { className: 'snip' }, seg);
          snip.appendChild(icon('scissors', 24, 2.4));
        }
      }
      segments.push({ seg, flag, snip, part, row: r });
    });
  });

  const markAt = 0.7;
  const cutAt = 1.25;
  const cutEnd = 2.55;
  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.76, y: 36, duration: 0.55 }));
  tl.key(left, slide(0.18, { x: -18, y: 0 }));
  tl.key(working, [...pop(0.45, { from: 0.5 }), { at: cutEnd, opacity: 1 }, { at: cutEnd + 0.12, opacity: 0, ease: 'linear' }]);
  tl.key(done, pop(cutEnd + 0.08, { from: 0.4, strong: true }));
  tl.key(tracks, slide(0.22, { y: 24 }));
  segments.forEach(({ seg, flag, snip, part, row }) => {
    tl.key(seg, fade(0.3 + (part.x / width) * 0.35 + row * 0.06, 0.2));
    if (flag) {
      tl.key(flag, [{ at: markAt + (part.x / width) * 0.3, opacity: 0 }, { at: markAt + (part.x / width) * 0.3 + 0.16, opacity: 1, ease: 'outQuad' }]);
    }
    if (snip) {
      tl.key(snip, pop(markAt + (part.x / width) * 0.3 + 0.05, { from: 0.3, strong: true }));
    }
  });
  tl.each((t) => {
    const p = ease.inOutCubic(clamp((t - cutAt) / (cutEnd - cutAt)));
    for (const { seg, part } of segments) {
      if (part.keep) {
        seg.style.left = `${(part.x + (part.to - part.x) * p).toFixed(2)}px`;
        seg.style.width = `${part.size.toFixed(2)}px`;
      } else {
        // A cut part shrinks into its place between the kept ones as they close up.
        const size = part.size * (1 - p);
        seg.style.left = `${(part.x + (part.to - part.x) * p).toFixed(2)}px`;
        seg.style.width = `${Math.max(0, size).toFixed(2)}px`;
        seg.style.visibility = size < 0.5 ? 'hidden' : '';
      }
    }
    const shown = from + (to - from) * p;
    count.textContent = clockText(shown, P.from);
    count.style.color = t >= cutEnd ? 'var(--good)' : '';
  });
  return { timeline: tl };
});
