// Bar chart, made by FossCut for its library (ADR-0042): bars growing one after another
// with their values counting up, the highlighted one in the accent colour.

// fosscut:defaults
const DEFAULTS = {
  "title": "Views per video",
  "subtitle": "Last five uploads",
  "labels": ["Mon", "Tue", "Wed", "Thu", "Fri"],
  "values": [12, 18, 9, 26, 41],
  "highlight": 0,
  "badge": "Best",
  "prefix": "",
  "suffix": "K",
  "decimals": 0,
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, Timeline, pop, fade, slide, clamp, ease, formatNumber } = Overlay;
  const { root, scale } = Overlay.stage(1080, 820);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const title = el('div', { className: 'title nowrap', text: P.title }, card);
  title.id = 'title';
  const subtitle = el('div', { className: 'nowrap', text: P.subtitle }, card);
  subtitle.id = 'subtitle';
  subtitle.classList.toggle('hidden', !P.subtitle);
  const plot = el('div', {}, card);
  plot.id = 'plot';

  const values = (Array.isArray(P.values) ? P.values : []).slice(0, 12).map((v) => Math.max(0, Number(v) || 0));
  const labels = Array.isArray(P.labels) ? P.labels : [];
  const n = Math.max(1, values.length);
  const top = Math.max(...values, 1e-9);
  // Bars count from 1; 0 highlights the largest.
  let highlight = Math.round(Number(P.highlight)) - 1;
  if (!(highlight >= 0 && highlight < values.length)) {
    highlight = values.indexOf(Math.max(...values));
  }
  const decimals = clamp(Math.round(Number(P.decimals) || 0), 0, 3);
  const width = 880;
  const gap = Math.min(40, (width / n) * 0.3);
  const barWidth = (width - gap * (n - 1)) / n;
  const chartTop = 100;
  const chartHeight = 470 - 58 - chartTop;
  const grids = [0, 1, 2, 3].map((i) => {
    const line = el('div', { className: 'grid' }, plot);
    line.style.top = `${chartTop + (chartHeight * i) / 3}px`;
    return line;
  });
  const cols = values.map((value, i) => {
    const col = el('div', { className: `col${i === highlight ? ' top' : ''}` }, plot);
    col.style.left = `${i * (barWidth + gap)}px`;
    col.style.width = `${barWidth}px`;
    const height = Math.max(8, (value / top) * chartHeight);
    const bar = el('div', { className: 'bar', style: { height: `${height}px` } }, col);
    const shown = el('div', { className: 'value tnum' }, col);
    shown.style.bottom = `${height + 12}px`;
    el('div', { className: 'name', text: labels[i] || '' }, col);
    let flag = null;
    if (i === highlight && P.badge) {
      flag = el('div', { className: 'flag' }, col);
      flag.appendChild(icon('star', 20));
      el('span', { text: P.badge }, flag);
      flag.style.bottom = `${height + 58}px`;
      flag.style.marginLeft = `${-flag.getBoundingClientRect().width / 2 / scale}px`;
    }
    return { col, bar, shown, value, flag };
  });

  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.76, y: 36, duration: 0.55 }));
  tl.key(title, slide(0.16, { y: 18 }));
  tl.key(subtitle, fade(0.26, 0.3));
  grids.forEach((g, i) => tl.key(g, [{ at: 0.25 + i * 0.04, opacity: 0, sx: 0.4 }, { at: 0.55 + i * 0.04, opacity: 1, sx: 1, ease: 'outCubic' }]));
  cols.forEach(({ col, bar, shown, value, flag }, i) => {
    const at = 0.4 + i * 0.08;
    tl.key(bar, [{ at, sy: 0 }, { at: at + 0.6, sy: 1, ease: 'outBack' }]);
    tl.key(shown, [{ at: at + 0.1, opacity: 0, y: 16 }, { at: at + 0.3, opacity: 1, ease: 'outQuad' }, { at: at + 0.6, y: 0, ease: 'outBack' }]);
    tl.key(col.querySelector('.name'), fade(at, 0.3));
    tl.tween(at, 0.7, (p) => {
      shown.textContent = `${P.prefix}${formatNumber(value * p, decimals)}${P.suffix}`;
    }, 'outCubic');
    if (flag) {
      tl.key(flag, pop(0.4 + n * 0.08 + 0.45, { from: 0.3, strong: true }));
    }
  });
  return { timeline: tl };
});
