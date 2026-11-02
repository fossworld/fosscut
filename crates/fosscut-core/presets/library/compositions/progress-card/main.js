// Progress card, made by FossCut for its library (ADR-0042): a percentage counting up, a
// filling bar and step chips ticked off as the bar passes them.

// fosscut:defaults
const DEFAULTS = {
  "title": "Building your edit",
  "status": "About 30 seconds left",
  "steps": ["Cut", "Captions", "Music", "Color", "Export"],
  "progress": 0.72,
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, svg, icon, Timeline, pop, fade, slide, clamp, ease } = Overlay;
  const { root } = Overlay.stage(1080, 460);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const head = el('div', {}, card);
  head.id = 'head';
  const glyph = el('div', {}, head);
  glyph.id = 'glyph';
  glyph.appendChild(icon('spark', 36));
  const words = el('div', {}, head);
  words.id = 'words';
  const title = el('div', { className: 'heading nowrap', text: P.title }, words);
  title.id = 'title';
  const status = el('div', { className: 'nowrap', text: P.status }, words);
  status.id = 'status';
  status.classList.toggle('hidden', !P.status);
  const percent = el('div', { className: 'tnum' }, head);
  percent.id = 'percent';
  const bar = el('div', {}, card);
  bar.id = 'bar';
  const fill = el('div', {}, bar);
  fill.id = 'fill';
  const sheen = el('div', {}, fill);
  sheen.id = 'sheen';

  const list = el('div', {}, card);
  list.id = 'steps';
  const names = (Array.isArray(P.steps) ? P.steps : []).slice(0, 8);
  list.classList.toggle('hidden', names.length === 0);
  const target = clamp(Number(P.progress), 0, 1);
  const steps = names.map((name, i) => {
    const step = el('div', { className: 'step' }, list);
    const mark = el('div', { className: 'mark' }, step);
    el('div', { className: 'ring' }, mark);
    const spin = svg('svg', { class: 'spin', viewBox: '0 0 38 38' }, mark);
    svg('circle', { cx: 19, cy: 19, r: 16.5, fill: 'none', stroke: 'currentColor', 'stroke-width': 3.4, 'stroke-linecap': 'round', 'stroke-dasharray': '26 200' }, spin);
    const tick = el('div', { className: 'tick' }, mark);
    tick.appendChild(icon('check', 24, 3.2));
    el('span', { text: name }, step);
    // Step i is done once the bar passes the end of its share.
    return { step, spin, tick, end: (i + 1) / names.length, startShare: i / names.length };
  });

  const fillFrom = 0.55;
  const fillLength = 1.6;
  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.76, y: 36, duration: 0.55 }));
  tl.key(glyph, pop(0.18, { from: 0.4, strong: true, rotate: -20 }));
  tl.key(title, slide(0.22, { x: -16, y: 0 }));
  tl.key(status, fade(0.32, 0.3));
  tl.key(percent, pop(0.3, { from: 0.6 }));
  tl.key(bar, fade(0.3, 0.25));
  steps.forEach(({ step }, i) => tl.key(step, pop(0.4 + i * 0.07, { from: 0.6, y: 10 })));
  tl.each((t) => {
    const p = target * ease.inOutCubic(clamp((t - fillFrom) / fillLength));
    fill.style.width = `${(p * 100).toFixed(3)}%`;
    percent.textContent = `${Math.round(p * 100)}%`;
    // A light runs along the filled part.
    const run = ((t * 0.9) % 1.4) / 1.4;
    sheen.style.left = `${(-160 + run * (888 + 320)).toFixed(1)}px`;
    for (const s of steps) {
      const isDone = p >= s.end - 1e-6;
      const isActive = !isDone && p >= s.startShare && (target < s.end || t < fillFrom + fillLength);
      s.step.classList.toggle('done', isDone);
      s.step.classList.toggle('active', isActive && t >= fillFrom);
      s.spin.style.visibility = isActive && t >= fillFrom ? '' : 'hidden';
      s.spin.style.transform = `rotate(${(t * 400) % 360}deg)`;
      s.spin.style.transformOrigin = '50% 50%';
      // The check pops when the bar reaches the step's end.
      const since = t - (fillFrom + fillLength * inverse(s.end / Math.max(target, 1e-6)));
      const shown = isDone ? clamp(since / 0.32) : 0;
      s.tick.style.opacity = isDone ? '1' : '0';
      s.tick.style.transform = `scale(${(isDone ? ease.outBackStrong(shown) : 0).toFixed(4)})`;
    }
  });
  // The time (0 to 1 of the fill) at which the eased fill reaches `share` of its target.
  function inverse(share) {
    if (share >= 1) {
      return 1;
    }
    let lo = 0;
    let hi = 1;
    for (let k = 0; k < 30; k += 1) {
      const mid = (lo + hi) / 2;
      if (ease.inOutCubic(mid) < share) {
        lo = mid;
      } else {
        hi = mid;
      }
    }
    return hi;
  }
  return { timeline: tl };
});
