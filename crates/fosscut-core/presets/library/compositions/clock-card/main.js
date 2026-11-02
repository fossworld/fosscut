// Clock card, made by FossCut for its library (ADR-0042): hands sweep from one time to
// another while an arc fills the time that passed and the time counts under it.

// fosscut:defaults
const DEFAULTS = {
  "from": "9:00",
  "to": "11:30",
  "label": "Editing time",
  "spent": "2.5 hours",
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, svg, icon, Timeline, pop, fade, slide, clamp, ease, clockSeconds } = Overlay;
  const { root } = Overlay.stage(760, 900);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const face = el('div', {}, card);
  face.id = 'face';
  const C = 220;
  const R = 200;
  const dial = svg('svg', { width: 440, height: 440, viewBox: '0 0 440 440' }, face);
  svg('circle', { cx: C, cy: C, r: R, fill: 'var(--card-2)' }, dial);
  const arc = svg('path', { fill: 'rgba(var(--accent-rgb), 0.18)', stroke: 'none' }, dial);
  const rim = svg('path', { fill: 'none', stroke: 'var(--accent)', 'stroke-width': 14, 'stroke-linecap': 'round' }, dial);
  const ticks = [];
  for (let i = 0; i < 60; i += 1) {
    const hour = i % 5 === 0;
    const a = (i / 60) * Math.PI * 2;
    const r1 = R - (hour ? 40 : 24);
    const r2 = R - 14;
    const tick = svg('line', {
      x1: C + Math.sin(a) * r1, y1: C - Math.cos(a) * r1, x2: C + Math.sin(a) * r2, y2: C - Math.cos(a) * r2,
      stroke: hour ? 'var(--ink)' : 'var(--faint)', 'stroke-width': hour ? 7 : 3, 'stroke-linecap': 'round',
    }, dial);
    ticks.push(tick);
  }
  const hourHand = svg('line', { x1: C, y1: C + 18, x2: C, y2: C - 100, stroke: 'var(--ink)', 'stroke-width': 16, 'stroke-linecap': 'round' }, dial);
  const minuteHand = svg('line', { x1: C, y1: C + 24, x2: C, y2: C - 152, stroke: 'var(--ink)', 'stroke-width': 10, 'stroke-linecap': 'round' }, dial);
  svg('circle', { cx: C, cy: C, r: 15, fill: 'var(--accent)', stroke: 'var(--card)', 'stroke-width': 5 }, dial);

  const time = el('div', { className: 'tnum' }, card);
  time.id = 'time';
  const label = el('div', { text: P.label }, card);
  label.id = 'label';
  label.classList.toggle('hidden', !P.label);
  const spent = el('div', { className: 'chip accent' }, card);
  spent.id = 'spent';
  spent.appendChild(icon('clock', 28, 2.4));
  el('span', { text: P.spent }, spent);
  spent.classList.toggle('hidden', !P.spent);

  // Clock times as minutes of a 12 hour face; "to" earlier than "from" means the next day.
  const start = clockSeconds(P.from);
  let end = clockSeconds(P.to);
  if (end < start) {
    end += 12 * 60;
  }
  const show = (minutes) => {
    const m = ((Math.round(minutes) % 720) + 720) % 720;
    const h = Math.floor(m / 60) || 12;
    return `${h}:${String(m % 60).padStart(2, '0')}`;
  };
  const point = (minutes, r) => {
    const a = ((minutes % 720) / 720) * Math.PI * 2;
    return [C + Math.sin(a) * r, C - Math.cos(a) * r];
  };
  const sector = (from, to, r) => {
    const span = Math.min(to - from, 719.9);
    if (span <= 0.01) {
      return '';
    }
    const [x1, y1] = point(from, r);
    const [x2, y2] = point(from + span, r);
    const large = span > 360 ? 1 : 0;
    return { x1, y1, x2, y2, large };
  };

  const sweepFrom = 0.45;
  const sweepLength = 1.7;
  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.74, y: 36, duration: 0.55 }));
  tl.key(face, pop(0.12, { from: 0.6, rotate: -30, duration: 0.6 }));
  ticks.forEach((tick, i) => tl.key(tick, fade(0.2 + (i / 60) * 0.3, 0.12)));
  tl.key(time, slide(0.3, { y: 20 }));
  tl.key(label, fade(0.38, 0.3));
  tl.key(spent, pop(sweepFrom + sweepLength - 0.05, { from: 0.4, strong: true }));
  tl.each((t) => {
    const p = ease.inOutCubic(clamp((t - sweepFrom) / sweepLength));
    const now = start + (end - start) * p;
    const minuteAngle = (now % 60) / 60 * 360;
    const hourAngle = (now % 720) / 720 * 360;
    minuteHand.setAttribute('transform', `rotate(${minuteAngle.toFixed(3)} ${C} ${C})`);
    hourHand.setAttribute('transform', `rotate(${hourAngle.toFixed(3)} ${C} ${C})`);
    const s = sector(start, now, R - 7);
    if (s) {
      arc.setAttribute('d', `M ${C} ${C} L ${s.x1.toFixed(2)} ${s.y1.toFixed(2)} A ${R - 7} ${R - 7} 0 ${s.large} 1 ${s.x2.toFixed(2)} ${s.y2.toFixed(2)} Z`);
      rim.setAttribute('d', `M ${s.x1.toFixed(2)} ${s.y1.toFixed(2)} A ${R - 7} ${R - 7} 0 ${s.large} 1 ${s.x2.toFixed(2)} ${s.y2.toFixed(2)}`);
    } else {
      arc.setAttribute('d', '');
      rim.setAttribute('d', '');
    }
    time.textContent = show(now);
  });
  return { timeline: tl };
});
