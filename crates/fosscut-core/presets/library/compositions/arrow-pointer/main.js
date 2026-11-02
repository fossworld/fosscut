// Arrow pointer, made by FossCut for its library (ADR-0042): a curved arrow drawing on
// towards the canvas's middle, where its tip lands, with a label at its tail.

// fosscut:defaults
const DEFAULTS = {
  "label": "Look here",
  "from": "top-left",
  "length": 0.36,
  "curve": 0.35,
  "color": "#2F6BFF",
  "appear": 0,
  "hold": 2.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, svg, Timeline, pop, clamp } = Overlay;
  const { root, scale } = Overlay.stage(800, 800);
  const DIRECTIONS = {
    'top-left': [-1, -1], top: [0, -1], 'top-right': [1, -1], right: [1, 0],
    'bottom-right': [1, 1], bottom: [0, 1], 'bottom-left': [-1, 1], left: [-1, 0],
  };
  const [dx0, dy0] = DIRECTIONS[P.from] || DIRECTIONS['top-left'];
  const norm = Math.hypot(dx0, dy0);
  const dir = [dx0 / norm, dy0 / norm];
  const length = clamp(Number(P.length) || 0.36, 0.12, 0.42) * 800;
  const tip = [400, 400];
  const tail = [tip[0] + dir[0] * length, tip[1] + dir[1] * length];
  const bend = clamp(Number(P.curve) || 0, -1, 1) * length * 0.45;
  const normal = [-dir[1], dir[0]];
  const control = [(tip[0] + tail[0]) / 2 + normal[0] * bend, (tip[1] + tail[1]) / 2 + normal[1] * bend];
  const stroke = 18;
  const headLength = 58;
  // The shaft ends where the head begins, along the curve's last direction.
  const end = [tip[0] - control[0], tip[1] - control[1]];
  const endNorm = Math.hypot(end[0], end[1]) || 1;
  const ux = end[0] / endNorm;
  const uy = end[1] / endNorm;
  const neck = [tip[0] - ux * headLength * 0.62, tip[1] - uy * headLength * 0.62];
  const shaft = `M ${tail[0].toFixed(1)} ${tail[1].toFixed(1)} Q ${control[0].toFixed(1)} ${control[1].toFixed(1)} ${neck[0].toFixed(1)} ${neck[1].toFixed(1)}`;
  const side = headLength * 0.62;
  const head = `M ${(tip[0] - ux * headLength - uy * side).toFixed(1)} ${(tip[1] - uy * headLength + ux * side).toFixed(1)} L ${tip[0]} ${tip[1]} L ${(tip[0] - ux * headLength + uy * side).toFixed(1)} ${(tip[1] - uy * headLength - ux * side).toFixed(1)}`;

  const nudge = el('div', { style: { position: 'absolute', inset: '0' } }, root);
  const arrow = svg('svg', { id: 'arrow', width: 800, height: 800, viewBox: '0 0 800 800' }, nudge);
  const color = P.color || '#2F6BFF';
  const parts = [];
  for (const [d, kind] of [[shaft, 'shaft'], [head, 'head']]) {
    const under = svg('path', { d, stroke: '#FFFFFF', 'stroke-width': stroke + 9 }, arrow);
    const over = svg('path', { d, stroke: color, 'stroke-width': stroke }, arrow);
    parts.push({ kind, under, over, length: over.getTotalLength() });
  }
  const headGroup = parts[1];

  const label = el('div', { text: P.label }, root);
  label.id = 'label';
  label.classList.toggle('hidden', !P.label);
  // The label sits beyond the tail, kept inside the canvas.
  const box = label.getBoundingClientRect();
  const lw = box.width / scale;
  const lh = box.height / scale;
  const gap = 26;
  let lx = tail[0] + dir[0] * (gap + lw / 2) - lw / 2;
  let ly = tail[1] + dir[1] * (gap + lh / 2) - lh / 2;
  lx = clamp(lx, 14, 800 - lw - 14);
  ly = clamp(ly, 14, 800 - lh - 14);
  label.style.left = `${lx}px`;
  label.style.top = `${ly}px`;

  const tl = new Timeline();
  tl.key(label, pop(0, { from: 0.5, strong: true }));
  const drawFrom = P.label ? 0.18 : 0.05;
  const shaftPart = parts[0];
  for (const path of [shaftPart.under, shaftPart.over]) {
    path.style.strokeDasharray = `${shaftPart.length + 1} ${shaftPart.length + 1}`;
  }
  tl.tween(drawFrom, 0.42, (p) => {
    const offset = ((1 - p) * (shaftPart.length + 1)).toFixed(2);
    shaftPart.under.style.strokeDashoffset = offset;
    shaftPart.over.style.strokeDashoffset = offset;
    shaftPart.under.style.visibility = p <= 0 ? 'hidden' : '';
    shaftPart.over.style.visibility = p <= 0 ? 'hidden' : '';
  }, 'outCubic');
  // The head snaps open as the shaft arrives.
  const headAt = drawFrom + 0.32;
  for (const path of [headGroup.under, headGroup.over]) {
    path.style.transformBox = 'view-box';
    path.style.transformOrigin = `${tip[0]}px ${tip[1]}px`;
  }
  tl.key([headGroup.under, headGroup.over], [
    { at: headAt, scale: 0, opacity: 0 },
    { at: headAt + 0.06, opacity: 1 },
    { at: headAt + 0.32, scale: 1, ease: 'outBackStrong' },
  ]);
  tl.each((t) => {
    // Once drawn, the whole arrow nudges towards its target now and then.
    const settled = headAt + 0.5;
    const phase = Math.max(0, t - settled);
    const push = t < settled ? 0 : Math.max(0, Math.sin(phase * Math.PI * 1.6)) ** 2 * 12;
    nudge.style.transform = `translate(${(-dir[0] * push).toFixed(2)}px, ${(-dir[1] * push).toFixed(2)}px)`;
  });
  return { timeline: tl };
});
