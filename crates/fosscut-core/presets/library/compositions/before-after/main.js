// Before and after, made by FossCut for its library (ADR-0042): two pictures or videos in
// one frame, split by a handle that sweeps across and settles.

// fosscut:defaults
const DEFAULTS = {
  "before": null,
  "after": null,
  "before_label": "Before",
  "after_label": "After",
  "split": 0.5,
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, media, Timeline, pop, clamp, ease, spring } = Overlay;
  const { root } = Overlay.stage(1080, 760);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const frame = el('div', {}, card);
  frame.id = 'frame';
  // With no pictures, both sides show the same art: dull and soft before, vivid after.
  const afterSide = el('div', { className: 'side' }, frame);
  afterSide.id = 'after';
  const beforeSide = el('div', { className: 'side' }, frame);
  beforeSide.id = 'before';
  const after = media(el('div', {}, afterSide), P.after, { seed: 52, width: 916, height: 540 });
  const before = media(el('div', {}, beforeSide), P.before, { seed: 52, width: 916, height: 540 });
  beforeSide.classList.toggle('plain', !before.kind);
  const handle = el('div', {}, frame);
  handle.id = 'handle';
  const knob = el('div', {}, handle);
  knob.id = 'knob';
  const left = icon('right', 30, 3);
  left.style.transform = 'scaleX(-1)';
  left.style.marginRight = '-8px';
  knob.append(left, icon('right', 30, 3));
  const beforeLabel = el('div', { className: 'label', text: P.before_label }, frame);
  beforeLabel.id = 'before-label';
  beforeLabel.classList.toggle('hidden', !P.before_label);
  const afterLabel = el('div', { className: 'label', text: P.after_label }, frame);
  afterLabel.id = 'after-label';
  afterLabel.classList.toggle('hidden', !P.after_label);

  const split = clamp(Number(P.split), 0.05, 0.95);
  const W = 916;
  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.76, y: 36, duration: 0.55 }));
  tl.key(knob, pop(0.3, { from: 0.3, strong: true }));
  tl.key(beforeLabel, pop(0.25, { from: 0.5 }));
  tl.key(afterLabel, pop(0.9, { from: 0.5 }));
  tl.each((t) => {
    // All "before" at first; the handle sweeps to show nearly all "after", then springs
    // back to the split.
    const sweep = ease.inOutCubic(clamp((t - 0.45) / 0.65));
    const settle = spring(t - 1.1, 1.6, 0.5);
    const x = t < 1.1 ? 1 - 0.9 * sweep : 0.1 + (split - 0.1) * settle;
    const px = clamp(x, 0, 1) * W;
    beforeSide.style.clipPath = `inset(0 ${(W - px).toFixed(2)}px 0 0)`;
    handle.style.left = `${px.toFixed(2)}px`;
  });
  return { timeline: tl, media: [after, before] };
});
