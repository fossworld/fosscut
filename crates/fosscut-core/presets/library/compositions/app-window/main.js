// App window, made by FossCut for its library (ADR-0042): a generic editing app with a
// media panel, a viewer and a timeline whose clips build in under a running playhead.

// fosscut:defaults
const DEFAULTS = {
  "media": null,
  "title": "Untitled project",
  "button": "Export",
  "panel": "Media",
  "timeline": true,
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, svg, media, Timeline, pop, slide, fade, random, stagger } = Overlay;
  const { root } = Overlay.stage(1080, 780);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';

  const bar = el('div', { className: 'window-bar' }, card);
  bar.id = 'bar';
  const dots = [0, 1, 2].map(() => el('div', { className: 'dot' }, bar));
  const name = el('div', { className: 'name nowrap', text: P.title }, bar);
  el('div', { className: 'spacer' }, bar);
  const button = el('div', { className: 'button', text: P.button }, bar);
  button.classList.toggle('hidden', !P.button);

  const body = el('div', {}, card);
  body.id = 'body';
  const side = el('div', {}, body);
  side.id = 'side';
  const label = el('div', { className: 'label', text: P.panel }, side);
  label.classList.toggle('hidden', !P.panel);
  const tiles = [];
  const items = [];
  for (let i = 0; i < 3; i += 1) {
    const tile = el('div', { className: 'tile' }, side);
    items.push(media(el('div', {}, tile), null, { seed: [21, 22, 27][i] }));
    tiles.push(tile);
  }
  const viewer = el('div', {}, body);
  viewer.id = 'viewer';
  const inner = el('div', { style: { position: 'absolute', inset: '0' } }, viewer);
  const picture = media(el('div', {}, inner), P.media, { seed: 21, width: 708, height: 398 });
  items.push(picture);
  const timecode = el('div', { className: 'tnum' }, viewer);
  timecode.id = 'timecode';

  // The timeline: a ruler, two video lanes and a sound lane.
  const timeline = el('div', {}, card);
  timeline.id = 'timeline';
  timeline.classList.toggle('hidden', !P.timeline);
  const ruler = el('div', {}, timeline);
  ruler.id = 'ruler';
  for (let i = 0; i <= 44; i += 1) {
    const tick = el('i', { className: i % 4 === 0 ? 'major' : '' }, ruler);
    tick.style.left = `${14 + i * 20}px`;
  }
  const next = random(5);
  const clips = [];
  const lane = (top, kind, count) => {
    const row = el('div', { className: 'lane', style: { top: `${top}px` } }, timeline);
    let x = 0;
    const width = 884;
    for (let i = 0; i < count; i += 1) {
      const w = i === count - 1 ? width - x : Math.round(width / count * (0.7 + next() * 0.6));
      const clip = el('div', { className: `clip ${kind}` }, row);
      clip.style.left = `${x}px`;
      clip.style.width = `${Math.max(30, w - 6)}px`;
      if (kind === 'video') {
        el('b', {}, clip);
      }
      if (kind === 'sound') {
        const wave = svg('svg', { viewBox: '0 0 200 28', preserveAspectRatio: 'none' }, clip);
        let d = '';
        for (let k = 0; k < 50; k += 1) {
          const h = 3 + Math.abs(Overlay.noise(k * 0.55, i + 3)) * 22;
          d += `M${k * 4 + 2} ${14 - h / 2}v${h}`;
        }
        svg('path', { d, stroke: 'rgba(255,255,255,0.75)', 'stroke-width': 2.2, 'stroke-linecap': 'round' }, wave);
      }
      clips.push({ clip, start: x / width });
      x += w;
    }
  };
  lane(38, 'title', 2);
  lane(82, 'video', 4);
  lane(126, 'sound', 3);
  // The title lane is shorter: its clips sit apart.
  const titles = timeline.querySelectorAll('.clip.title');
  if (titles.length === 2) {
    titles[0].style.width = '220px';
    titles[1].style.left = '470px';
    titles[1].style.width = '250px';
  }
  const playhead = el('div', {}, timeline);
  playhead.id = 'playhead';

  if (!P.timeline) {
    body.style.marginBottom = '22px';
  }

  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.74, y: 40, duration: 0.55 }));
  dots.forEach((dot, i) => tl.key(dot, pop(stagger(0.2, i, 0.05), { from: 0.2 })));
  tl.key(name, fade(0.3, 0.3));
  tl.key(button, pop(0.42, { from: 0.5 }));
  tl.key(label, fade(0.28, 0.3));
  tiles.forEach((tile, i) => tl.key(tile, pop(stagger(0.3, i, 0.08), { from: 0.6, y: 12 })));
  tl.key(inner, [{ at: 0.18, opacity: 0, scale: 1.1 }, { at: 0.5, opacity: 1, ease: 'outQuad' }, { at: 1.2, scale: 1, ease: 'outCubic' }]);
  tl.key(timecode, slide(0.5, { y: 12 }));
  clips.forEach(({ clip, start }, i) => tl.key(clip, [
    { at: 0.45 + start * 0.5 + (i % 3) * 0.02, opacity: 0, sx: 0 },
    { at: 0.55 + start * 0.5 + (i % 3) * 0.02, opacity: 1, ease: 'outQuad' },
    { at: 0.85 + start * 0.5 + (i % 3) * 0.02, sx: 1, ease: 'outBack' },
  ]));
  tl.key(playhead, fade(0.8, 0.2));
  tl.each((t) => {
    // The playhead runs across the timeline at a steady pace once the clips are in.
    const run = Math.max(0, t - 0.8);
    const x = 14 + Math.min(884, 120 + run * 120);
    playhead.style.left = `${x.toFixed(2)}px`;
    const seconds = 12 * 60 + 4 + run;
    const frames = Math.floor((seconds % 1) * 30);
    timecode.textContent = `00:${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(Math.floor(seconds % 60)).padStart(2, '0')}:${String(frames).padStart(2, '0')}`;
  });
  return { timeline: tl, media: items };
});
