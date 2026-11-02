// Cross out, made by FossCut for its library (ADR-0042): X marks or strike lines drawn on
// stroke by stroke over whatever lies below, placed by fractions of the canvas.

// fosscut:defaults
const DEFAULTS = {
  "style": "x",
  "marks": [[0, 0]],
  "size": 0.62,
  "thickness": 0.11,
  "color": "#F04438",
  "appear": 0,
  "hold": 2.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, svg, Timeline, clamp } = Overlay;
  // Fluid: the design follows the canvas's proportions, so fractions land where they say.
  const stage = Overlay.stage(1080, 780, { fluid: true });
  const { root, width: W, height: H } = stage;
  const layer = el('div', {}, root);
  layer.id = 'marks';
  const short = Math.min(W, H);
  const points = (Array.isArray(P.marks) && P.marks.length ? P.marks : [[0, 0]]).slice(0, 12);
  const strike = P.style === 'strike';
  const size = clamp(Number(P.size) || 0.6, 0.02, 1) * (strike ? W : short);
  const stroke = Math.max(4, (strike ? short * 0.5 : size) * clamp(Number(P.thickness) || 0.1, 0.01, 0.4));
  const color = P.color || '#F04438';

  const tl = new Timeline();
  points.forEach(([px, py], i) => {
    const cx = W / 2 + Number(px) * W;
    const cy = H / 2 + Number(py) * H;
    const pad = stroke * 2;
    const box = strike ? [size + pad * 2, stroke + pad * 2] : [size + pad * 2, size + pad * 2];
    const mark = svg('svg', {
      class: 'mark',
      width: box[0],
      height: box[1],
      viewBox: `${-box[0] / 2} ${-box[1] / 2} ${box[0]} ${box[1]}`,
    }, layer);
    mark.style.left = `${cx - box[0] / 2}px`;
    mark.style.top = `${cy - box[1] / 2}px`;
    mark.style.filter = `drop-shadow(0 ${(stroke * 0.25).toFixed(1)}px ${(stroke * 0.5).toFixed(1)}px rgba(0, 0, 0, 0.35))`;
    const h = size / 2;
    // A hand-made feel: the strokes lean a little and differ slightly in length.
    const lean = ((i * 37) % 7 - 3) * 0.012 * size;
    const lines = strike
      ? [`M ${-h} ${lean * 0.4} L ${h} ${-lean * 0.4}`]
      : [`M ${-h} ${-h + lean} L ${h} ${h - lean * 0.5}`, `M ${h - lean * 0.6} ${-h} L ${-h} ${h + lean * 0.3}`];
    const start = 0.04 + i * 0.16;
    lines.forEach((d, k) => {
      const under = svg('path', { d, stroke: 'rgba(255,255,255,0.92)', 'stroke-width': stroke * 1.32 }, mark);
      const over = svg('path', { d, stroke: color, 'stroke-width': stroke }, mark);
      const length = over.getTotalLength();
      for (const path of [under, over]) {
        path.style.strokeDasharray = `${length + 1} ${length + 1}`;
      }
      const from = start + k * 0.15;
      tl.tween(from, 0.17, (p) => {
        const offset = ((1 - p) * (length + 1)).toFixed(2);
        under.style.strokeDashoffset = offset;
        over.style.strokeDashoffset = offset;
        const hidden = p <= 0;
        under.style.visibility = hidden ? 'hidden' : '';
        over.style.visibility = hidden ? 'hidden' : '';
      }, 'outQuad');
    });
    // A punch as the last stroke lands, and a slight tilt.
    const landed = start + (lines.length - 1) * 0.15 + 0.17;
    tl.key(mark, [
      { at: start, scale: 0.86, rotate: -4 },
      { at: landed, scale: 1.12, rotate: 2, ease: 'outQuad' },
      { at: landed + 0.28, scale: 1, rotate: 0, ease: 'outBack' },
    ]);
  });
  return { timeline: tl };
});
