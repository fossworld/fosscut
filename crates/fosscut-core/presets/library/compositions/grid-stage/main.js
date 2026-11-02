// Grid stage, made by FossCut for its library (ADR-0042): a glowing perspective grid
// running towards a lit horizon, with picture cards arcing around a speaker's head.

// fosscut:defaults
const DEFAULTS = {
  "cards": 5,
  "card_1": null,
  "card_2": null,
  "card_3": null,
  "card_4": null,
  "card_5": null,
  "card_6": null,
  "background": "#060A18",
  "glow": 1,
  "ceiling": false,
  "grid_speed": 0.6,
  "sway": 4,
  "head_y": 0.34,
  "radius": 0.4,
  "appear": 0,
  "hold": 4,
  "exit": true,
  "speed": 1,
  "theme": "dark",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, media, Timeline, fade, clamp, ease, spring, rgba, luminance, blend } = Overlay;
  const { root } = Overlay.stage(1080, 1920);
  const accent = typeof P.accent === 'string' ? P.accent.slice(0, 7) : '#2F6BFF';
  const base = typeof P.background === 'string' ? P.background.slice(0, 7) : '#060A18';
  const light = luminance(base) > 0.4;
  const glow = clamp(Number(P.glow), 0, 2);
  const horizonY = 0.47 * 1920;

  const back = el('div', {}, root);
  back.id = 'back';
  back.style.background = [
    `radial-gradient(ellipse 70% 30% at 50% 47%, ${rgba(accent, (light ? 0.28 : 0.55) * glow)} 0%, ${rgba(accent, 0)} 100%)`,
    `radial-gradient(ellipse 120% 70% at 50% 30%, ${rgba(blend(base, accent, 0.35), 0.9)} 0%, ${rgba(base, 0)} 70%)`,
    `linear-gradient(180deg, ${blend(base, '#000000', light ? 0 : 0.35)} 0%, ${base} 45%, ${blend(base, accent, 0.12)} 100%)`,
  ].join(', ');

  // The grid is drawn in perspective on a canvas kept in memory (software), which paints
  // the same pixels for the same lines every time.
  const world = el('canvas', {}, root);
  world.id = 'world';
  world.width = 1080;
  world.height = 1920;
  const ctx = world.getContext('2d', { willReadFrequently: true });
  const line = light ? blend(accent, '#000000', 0.1) : blend(accent, '#FFFFFF', 0.25);
  const strength = light ? 0.55 : 0.85;
  const FOCAL = 900;
  const SPACING = 120;
  /** One grid plane, below (`side` 1) or above (-1) the horizon, `lift` from 0 (flat on
   * the horizon) to 1, scrolled `run` squares towards the viewer. */
  const plane = (side, lift, run) => {
    const height = 900 * lift;
    if (height < 1) {
      return;
    }
    const edge = side > 0 ? 1920 : 0;
    const reach = Math.abs(edge - horizonY);
    const nearest = (FOCAL * height) / reach;
    const fadeTo = (y) => Math.min(1, Math.max(0, (Math.abs(y - horizonY) - 10) / 460)) ** 1.6;
    // Lines across, nearer ones thicker.
    for (let k = -1; k < 160; k += 1) {
      const z = nearest + (k + 1 - (run % 1)) * SPACING;
      if (z <= nearest * 0.6) {
        continue;
      }
      const y = horizonY + (side * FOCAL * height) / z;
      const alpha = strength * fadeTo(y);
      // Far lines closer together than a few pixels would merge into a band.
      const gap = (FOCAL * height) / z - (FOCAL * height) / (z + SPACING);
      if (alpha <= 0.004 || gap < 5) {
        break;
      }
      const width = Math.max(1, Math.min(4, (3.2 * nearest) / z));
      ctx.strokeStyle = rgba(line, alpha * 0.22);
      ctx.lineWidth = width + 7;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(1080, y);
      ctx.stroke();
      ctx.strokeStyle = rgba(line, alpha);
      ctx.lineWidth = width;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(1080, y);
      ctx.stroke();
    }
    // Lines along, meeting at the vanishing point, fading towards it.
    const along = (a) => {
      const g = ctx.createLinearGradient(0, horizonY, 0, edge);
      g.addColorStop(0, rgba(line, 0));
      g.addColorStop(Math.min(1, 40 / reach), rgba(line, 0));
      g.addColorStop(Math.min(1, 460 / reach), rgba(line, a * 0.75));
      g.addColorStop(1, rgba(line, a));
      return g;
    };
    for (const [width, a] of [[10, strength * 0.22], [3, strength]]) {
      ctx.strokeStyle = along(a);
      ctx.lineWidth = width;
      for (let i = -90; i <= 90; i += 1) {
        const x = 540 + (i * SPACING * FOCAL) / nearest;
        ctx.beginPath();
        ctx.moveTo(x, edge);
        ctx.lineTo(540, horizonY);
        ctx.stroke();
      }
    }
  };
  const haze = el('div', {}, root);
  haze.id = 'haze';
  haze.style.top = `${horizonY - 260}px`;
  haze.style.background = `radial-gradient(ellipse 70% 50% at 50% 50%, ${rgba(blend(accent, '#FFFFFF', 0.3), 0.55 * glow)} 0%, ${rgba(accent, 0)} 100%)`;
  const horizon = el('div', {}, root);
  horizon.id = 'horizon';
  horizon.style.top = `${horizonY - 2}px`;
  horizon.style.background = `linear-gradient(90deg, ${rgba(accent, 0)} 0%, ${rgba(blend(accent, '#FFFFFF', 0.6), Math.min(1, 0.9 * glow))} 50%, ${rgba(accent, 0)} 100%)`;
  // The line's glow is a gradient band rather than a blurred shadow, which paints the same
  // every frame.
  const shine = el('div', {}, root);
  shine.id = 'shine';
  shine.style.top = `${horizonY - 40}px`;
  shine.style.background = `radial-gradient(ellipse 52% 50% at 50% 50%, ${rgba(accent, 0.55 * Math.min(1, glow))} 0%, ${rgba(accent, 0.18 * Math.min(1, glow))} 45%, ${rgba(accent, 0)} 100%)`;

  // Cards on an arc round the head, over it from one side to the other.
  const arc = el('div', {}, root);
  arc.id = 'arc';
  const count = clamp(Math.round(Number(P.cards) || 0), 0, 6);
  const cx = 540;
  const cy = clamp(Number(P.head_y), 0.1, 0.8) * 1920;
  const radius = clamp(Number(P.radius), 0.15, 0.6) * 1080;
  const span = count <= 1 ? 0 : Math.min(150, 34 * (count - 1));
  const items = [];
  const cards = Array.from({ length: count }, (_, i) => {
    const tile = el('div', { className: 'tile' }, arc);
    const plate = el('div', { className: 'plate' }, tile);
    plate.style.background = `radial-gradient(closest-side, rgba(0, 0, 0, 0.5) 0%, rgba(0, 0, 0, 0.32) 55%, ${rgba(accent, 0.2 * glow)} 80%, ${rgba(accent, 0)} 100%)`;
    const body = el('div', { className: 'body' }, tile);
    const face = el('div', { className: 'face' }, body);
    items.push(media(el('div', {}, face), P[`card_${i + 1}`], { seed: [100, 104, 102, 106, 87, 108][i], width: 224, height: 304 }));
    // Angles in degrees, 0 straight above the head, negative to the left.
    const angle = count <= 1 ? 0 : -span / 2 + (span * i) / (count - 1);
    return { tile, angle };
  });

  const tl = new Timeline();
  tl.key(back, fade(0, 0.5));
  tl.key(world, fade(0.05, 0.5));
  tl.key(haze, fade(0.3, 0.6));
  tl.key([horizon, shine], [{ at: 0.25, opacity: 0, sx: 0.2 }, { at: 0.45, opacity: 1 }, { at: 0.95, sx: 1, ease: 'outCubic' }]);
  tl.each((t) => {
    // The floor unfolds from the horizon, then its grid runs towards the viewer.
    const fold = ease.outCubic(clamp((t - 0.05) / 0.9));
    const run = Math.max(0, t) * clamp(Number(P.grid_speed), 0, 4);
    ctx.clearRect(0, 0, 1080, 1920);
    plane(1, fold, run);
    if (P.ceiling) {
      plane(-1, fold, run);
    }
    const sway = Number(P.sway) || 0;
    cards.forEach(({ tile, angle }, i) => {
      const out = spring(t - 0.35 - i * 0.08, 1.4, 0.55);
      // The arc sways gently back and forth round the head.
      const a = ((angle + sway * Math.sin(Math.max(0, t - 0.35) * 0.9)) * Math.PI) / 180;
      const r = radius * (0.35 + 0.65 * out);
      const x = cx + Math.sin(a) * r * 1.05;
      const y = cy - Math.cos(a) * r * 0.92 + Math.sin(t * 1.3 + i * 1.7) * 7;
      const turn = Math.sin(a) * -28;
      const lean = Math.sin(a) * 9;
      const shown = clamp((t - 0.35 - i * 0.08) / 0.2);
      tile.style.left = `${x.toFixed(2)}px`;
      tile.style.top = `${y.toFixed(2)}px`;
      tile.style.transform = `translateZ(${(-120 * Math.abs(Math.sin(a))).toFixed(2)}px) rotateY(${turn.toFixed(3)}deg) rotateZ(${lean.toFixed(3)}deg) scale(${(0.4 + 0.6 * out).toFixed(4)})`;
      tile.style.opacity = shown.toFixed(4);
      tile.style.visibility = shown <= 0 ? 'hidden' : '';
    });
  });
  return { timeline: tl, media: items, exit: 'dissolve' };
});
