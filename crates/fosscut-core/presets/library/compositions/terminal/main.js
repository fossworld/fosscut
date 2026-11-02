// Terminal, made by FossCut for its library (ADR-0042): steps typed one after another
// behind a prompt, each finished with a check mark.

// fosscut:defaults
const DEFAULTS = {
  "title": "Terminal",
  "steps": ["analyze talking-head.mp4", "cut --silences --retakes", "captions --style bold", "export reel.mp4 --vertical"],
  "prompt": "$",
  "done": "done",
  "typing": 40,
  "appear": 0,
  "hold": 5.5,
  "exit": true,
  "speed": 1,
  "theme": "dark",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, Timeline, pop, fade, typed, caretOn } = Overlay;
  const { root } = Overlay.stage(1080, 700);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const bar = el('div', { className: 'window-bar' }, card);
  const dots = [0, 1, 2].map(() => el('div', { className: 'dot' }, bar));
  const name = el('div', { className: 'name nowrap', text: P.title }, bar);

  const lines = el('div', {}, card);
  lines.id = 'lines';
  const steps = (Array.isArray(P.steps) ? P.steps : []).slice(0, 6);
  const cps = Math.max(4, Number(P.typing) || 40);
  // Each step: typed, a short wait, then its check mark.
  let at = 0.45;
  const rows = steps.map((step) => {
    const row = el('div', { className: 'row' }, lines);
    el('div', { className: 'prompt', text: P.prompt }, row);
    const text = el('div', { className: 'text' }, row);
    const shown = el('span', {}, text);
    const caret = el('span', { className: 'caret' }, text);
    const state = el('div', { className: 'state' }, row);
    const wait = el('div', { className: 'wait' }, state);
    const waits = [0, 1, 2].map(() => el('i', {}, wait));
    const done = el('div', { className: 'done' }, state);
    const label = el('span', { text: P.done }, done);
    label.classList.toggle('hidden', !P.done);
    const tick = el('div', { className: 'tick' }, done);
    tick.appendChild(icon('check', 28, 3));
    const start = at;
    const typedEnd = start + Array.from(step).length / cps;
    const finish = typedEnd + 0.3;
    at = finish + 0.16;
    return { row, step, shown, caret, wait, waits, done, start, typedEnd, finish };
  });

  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.76, y: 36, duration: 0.55 }));
  dots.forEach((dot, i) => tl.key(dot, pop(0.2 + i * 0.05, { from: 0.2 })));
  tl.key(name, fade(0.3, 0.3));
  for (const r of rows) {
    tl.key(r.row, fade(r.start - 0.12, 0.12));
    tl.key(r.done, pop(r.finish, { from: 0.4, strong: true }));
    tl.key(r.wait, [{ at: r.typedEnd, opacity: 0 }, { at: r.typedEnd + 0.06, opacity: 1 }, { at: r.finish - 0.02, opacity: 1 }, { at: r.finish + 0.04, opacity: 0 }]);
  }
  tl.each((t) => {
    rows.forEach((r, i) => {
      r.shown.textContent = typed(r.step, t, r.start, cps);
      // The caret sits on the line being typed, and blinks on the last one when all are done.
      const last = i === rows.length - 1;
      const current = t >= r.start && (t < r.finish || last);
      r.caret.style.visibility = current && (t < r.typedEnd || caretOn(t)) ? '' : 'hidden';
      r.waits.forEach((dot, k) => {
        const phase = (t - r.typedEnd) * 3.2 - k * 0.22;
        dot.style.opacity = (0.35 + 0.65 * Math.max(0, Math.sin(phase * Math.PI))).toFixed(3);
      });
    });
  });
  return { timeline: tl };
});
