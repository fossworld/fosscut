// Stat counter, made by FossCut for its library (ADR-0042): a big number counting up to
// its value, a label, an accent line and a trend chip.

// fosscut:defaults
const DEFAULTS = {
  "value": 10,
  "from": 0,
  "decimals": 0,
  "prefix": "",
  "suffix": "x",
  "kicker": "This month",
  "label": "faster editing",
  "trend": "+24% vs last month",
  "trend_up": true,
  "appear": 0,
  "hold": 3,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, Timeline, pop, fade, slide, clamp, ease, formatNumber } = Overlay;
  const { root, scale } = Overlay.stage(900, 560);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const kicker = el('div', {}, card);
  kicker.id = 'kicker';
  el('i', { className: 'dot' }, kicker);
  el('span', { text: P.kicker }, kicker);
  kicker.classList.toggle('hidden', !P.kicker);
  const number = el('div', { className: 'tnum' }, card);
  number.id = 'number';
  const prefix = el('span', { className: 'affix', text: P.prefix }, number);
  const digits = el('span', {}, number);
  const suffix = el('span', { className: 'affix', text: P.suffix }, number);
  const line = el('div', {}, card);
  line.id = 'line';
  const label = el('div', { text: P.label }, card);
  label.id = 'label';
  label.classList.toggle('hidden', !P.label);
  const trend = el('div', { className: `chip ${P.trend_up ? 'good' : 'bad'}` }, card);
  trend.id = 'trend';
  trend.appendChild(icon(P.trend_up ? 'up' : 'down', 26, 3));
  el('span', { text: P.trend }, trend);
  trend.classList.toggle('hidden', !P.trend);
  const decimals = clamp(Math.round(Number(P.decimals) || 0), 0, 3);
  const from = Number(P.from) || 0;
  const value = Number(P.value) || 0;
  // The widest the number gets fixes its size, so the card does not jump while counting.
  digits.textContent = formatNumber(Math.max(Math.abs(from), Math.abs(value)) * (value < 0 || from < 0 ? -1 : 1), decimals);
  const full = number.getBoundingClientRect().width / scale;
  const room = 676;
  if (full > room) {
    number.style.fontSize = `${Math.floor(168 * room / full)}px`;
  }

  const countFrom = 0.25;
  const countLength = 1.5;
  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.72, y: 36, duration: 0.55 }));
  tl.key(kicker, slide(0.15, { x: -14, y: 0 }));
  tl.key(number, slide(0.18, { y: 30, duration: 0.5, easing: 'outBack' }));
  tl.key(line, [{ at: 0.4, sx: 0 }, { at: 1.3, sx: 1, ease: 'outQuint' }]);
  tl.key(label, slide(0.42, { y: 18 }));
  tl.key(trend, pop(countFrom + countLength - 0.1, { from: 0.4, strong: true }));
  tl.each((t) => {
    const p = ease.outExpo(clamp((t - countFrom) / countLength));
    digits.textContent = formatNumber(from + (value - from) * p, decimals);
  });
  prefix.classList.toggle('hidden', !P.prefix);
  suffix.classList.toggle('hidden', !P.suffix);
  return { timeline: tl };
});
