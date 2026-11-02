// Reel fan, made by FossCut for its library (ADR-0042): vertical video cards rise as a
// stack and fan out, each with a view count counting up.

// fosscut:defaults
const DEFAULTS = {
  "count": 5,
  "views": ["845K", "1.2M", "2.4M", "530K", "1.7M"],
  "icon": "play",
  "spread": 10,
  "reel_1": null,
  "reel_2": null,
  "reel_3": null,
  "reel_4": null,
  "reel_5": null,
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, media, Timeline, clamp, ease, spring, countAmount } = Overlay;
  const { root } = Overlay.stage(1080, 960);
  const count = clamp(Math.round(Number(P.count) || 5), 3, 5);
  const views = Array.isArray(P.views) ? P.views : [];
  const spread = clamp(Number(P.spread), 0, 25);
  const glyph = ['play', 'eye', 'heart'].includes(P.icon) ? P.icon : 'play';
  const middle = (count - 1) / 2;
  // The middle card sits on top; the outer ones lie behind.
  const order = Array.from({ length: count }, (_, i) => i).sort((a, b) => Math.abs(b - middle) - Math.abs(a - middle));
  const reels = [];
  const items = [];
  for (const i of order) {
    const reel = el('div', { className: 'reel' }, root);
    el('div', { className: 'plate' }, reel);
    const body = el('div', { className: 'body' }, reel);
    const screen = el('div', { className: 'screen' }, body);
    items.push(media(el('div', {}, screen), P[`reel_${i + 1}`], { seed: [60, 62, 63, 64, 65][i], width: 276, height: 502 }));
    el('div', { className: 'shade' }, screen);
    const bars = el('div', { className: 'bars' }, screen);
    el('i', {}, bars);
    el('i', {}, bars);
    const chip = el('div', { className: 'views' }, screen);
    chip.appendChild(icon(glyph, 26, 2.4));
    const label = el('span', { className: 'tnum' }, chip);
    chip.classList.toggle('hidden', !views[i]);
    // Chips sit on each card's outer side, which the cards beside it leave in view.
    if (i > middle) {
      chip.style.left = 'auto';
      chip.style.right = '14px';
    }
    reels[i] = { reel, chip, label, text: views[i] || '' };
  }

  const tl = new Timeline();
  tl.each((t) => {
    reels.forEach(({ reel }, i) => {
      const offset = i - middle;
      // Rise together as a stack, then fan out on a spring, outer cards a beat later.
      const rise = ease.outCubic(clamp(t / 0.45));
      const fan = spring(t - 0.3 - Math.abs(offset) * 0.04, 1.5, 0.48);
      const angle = offset * spread * fan;
      const x = offset * 95 * fan;
      const y = (1 - rise) * 260 + Math.abs(offset) * 30 * fan;
      // A gentle float once out.
      const float = t > 1.2 ? Math.sin((t - 1.2) * 2 + i) * 5 : 0;
      reel.style.transform = `translate(${x.toFixed(2)}px, ${(y + float).toFixed(2)}px) rotate(${angle.toFixed(3)}deg) scale(${(0.9 + 0.1 * rise).toFixed(4)})`;
      reel.style.opacity = t >= 0.2 ? '' : clamp(t / 0.2).toFixed(4);
      reel.style.visibility = t <= 0 ? 'hidden' : '';
    });
  });
  reels.forEach(({ chip, label, text }, i) => {
    const at = 0.75 + i * 0.07;
    tl.key(chip, [{ at, opacity: 0, scale: 0.5 }, { at: at + 0.1, opacity: 1 }, { at: at + 0.4, scale: 1, ease: 'outBackStrong' }]);
    tl.tween(at, 0.9, (p) => {
      label.textContent = countAmount(text, p);
    }, 'outCubic');
  });
  return { timeline: tl, media: items };
});
