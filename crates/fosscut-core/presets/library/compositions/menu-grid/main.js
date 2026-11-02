// Menu grid, made by FossCut for its library (ADR-0042): tiles fill a grid one by one
// while a counter counts them, then one is chosen with a ring and a check.

// fosscut:defaults
const DEFAULTS = {
  "title": "Pick a style",
  "unit": "styles",
  "columns": 3,
  "rows": 3,
  "labels": ["Bold", "Clean", "Retro", "Neon", "Minimal", "Pop", "Film", "Mono", "Vivid"],
  "selected": 5,
  "tile_1": null,
  "tile_2": null,
  "tile_3": null,
  "tile_4": null,
  "tile_5": null,
  "tile_6": null,
  "tile_7": null,
  "tile_8": null,
  "tile_9": null,
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, media, Timeline, pop, fade, slide, clamp } = Overlay;
  const { root } = Overlay.stage(1080, 1080);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const head = el('div', {}, card);
  head.id = 'head';
  const glyph = el('div', {}, head);
  glyph.id = 'glyph';
  glyph.appendChild(icon('grid', 32, 2.2));
  const title = el('div', { className: 'heading nowrap', text: P.title }, head);
  title.id = 'title';
  const counter = el('div', { className: 'chip accent tnum' }, head);
  counter.id = 'counter';

  const columns = clamp(Math.round(Number(P.columns) || 3), 2, 4);
  const rows = clamp(Math.round(Number(P.rows) || 3), 1, 4);
  const total = columns * rows;
  const labels = Array.isArray(P.labels) ? P.labels : [];
  const gap = 18;
  const width = 892;
  const tileW = (width - gap * (columns - 1)) / columns;
  const tileH = Math.round(tileW * (columns >= 4 ? 1 : 0.82));
  const grid = el('div', { style: { height: `${rows * tileH + (rows - 1) * gap}px` } }, card);
  grid.id = 'grid';
  // Tiles count from 1; 0 chooses none.
  const selected = Math.round(Number(P.selected)) - 1;
  // Seeds whose art differs from tile to tile.
  const SEEDS = [70, 71, 72, 73, 87, 79, 84, 92, 86];
  const tiles = [];
  const items = [];
  for (let i = 0; i < total; i += 1) {
    const r = Math.floor(i / columns);
    const c = i % columns;
    const tile = el('div', { className: 'tile' }, grid);
    Object.assign(tile.style, { left: `${c * (tileW + gap)}px`, top: `${r * (tileH + gap)}px`, width: `${tileW}px`, height: `${tileH}px` });
    const face = el('div', { className: 'face' }, tile);
    items.push(media(el('div', {}, face), i < 9 ? P[`tile_${i + 1}`] : null, { seed: SEEDS[i % SEEDS.length] + 100 * Math.floor(i / SEEDS.length), width: tileW, height: tileH }));
    if (labels[i]) {
      el('div', { className: 'shade' }, face);
      el('div', { className: 'name', text: labels[i] }, face);
    }
    let ring = null;
    let tick = null;
    if (i === selected) {
      ring = el('div', { className: 'ring' }, tile);
      tick = el('div', { className: 'tick' }, tile);
      tick.appendChild(icon('check', 28, 3.2));
    }
    tiles.push({ tile, face, ring, tick });
  }

  const fillFrom = 0.3;
  const step = Math.min(0.08, 0.9 / total);
  const chooseAt = fillFrom + total * step + 0.45;
  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.8, y: 40, duration: 0.55 }));
  tl.key(glyph, pop(0.16, { from: 0.4, strong: true }));
  tl.key(title, slide(0.2, { x: -16, y: 0 }));
  tl.key(counter, pop(0.3, { from: 0.5 }));
  tiles.forEach(({ tile, face, ring, tick }, i) => {
    const at = fillFrom + i * step;
    tl.key(tile, pop(at, { from: 0.5, duration: 0.45 }));
    if (ring) {
      tl.key(tile, [{ at: chooseAt, scale: 1 }, { at: chooseAt + 0.35, scale: 1.05, ease: 'outBack' }]);
      tl.key(ring, [{ at: chooseAt, opacity: 0, scale: 1.12 }, { at: chooseAt + 0.12, opacity: 1 }, { at: chooseAt + 0.35, scale: 1, ease: 'outBack' }]);
      tl.key(tick, pop(chooseAt + 0.12, { from: 0.2, strong: true }));
    } else if (selected >= 0 && selected < total) {
      tl.key(face, [{ at: chooseAt, opacity: 1, scale: 1 }, { at: chooseAt + 0.3, opacity: 0.72, scale: 0.97 }]);
    }
  });
  tl.each((t) => {
    const shown = clamp(Math.floor((t - fillFrom) / step) + 1, 0, total);
    counter.textContent = P.unit ? `${shown} ${P.unit}` : String(shown);
  });
  return { timeline: tl, media: items };
});
