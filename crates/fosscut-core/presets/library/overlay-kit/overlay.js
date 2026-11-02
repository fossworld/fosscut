// FossCut's overlay kit for the library's explainer overlays (ADR-0042), written for FossCut
// and free to use, also commercially. A template builds its card once, declares its motion
// as keyframes in seconds on a Timeline, and the kit draws it for any time t:
//
// - timing: `appear` starts the entrance, `speed` scales the entrance and inner motion,
//   and after `hold` seconds the card leaves in EXIT seconds when `exit` is on;
// - motion: keyframes of opacity, position, scale, rotation, blur, reveal and CSS
//   variables, with easing curves, overshooting pops and damped springs;
// - text: typing, counting up and down, amounts such as "1.2M" and clock times such as
//   "6:24";
// - theme: light or dark cards and the accent colour, as CSS variables of overlay.css;
// - media: pictures and videos from slots, a video at the card's own time, and generated
//   abstract placeholders when a slot is empty;
// - icons drawn as strokes.
//
// Every frame is a function of its time only: nothing carries over from one frame to the
// next, randomness comes from seeded generators, and window.fosscut.seek(t) draws time t.

(() => {
  'use strict';

  /** Seconds the exit takes; fosscut-core adds it to a composition's length. */
  const EXIT = 0.4;

  // Numbers.

  const clamp = (value, low = 0, high = 1) => Math.min(high, Math.max(low, value));
  const mix = (a, b, t) => a + (b - a) * t;
  /** How far `t` is through the span from `start` lasting `duration`, from 0 to 1. */
  const progress = (t, start, duration) => {
    if (duration <= 0) {
      return t >= start ? 1 : 0;
    }
    return clamp((t - start) / duration);
  };

  /** Easing curves from 0 to 1. `outBack` overshoots a little, `outBackStrong` more. */
  const ease = {
    linear: (x) => x,
    inQuad: (x) => x * x,
    outQuad: (x) => 1 - (1 - x) * (1 - x),
    inOutQuad: (x) => (x < 0.5 ? 2 * x * x : 1 - (-2 * x + 2) ** 2 / 2),
    inCubic: (x) => x * x * x,
    outCubic: (x) => 1 - (1 - x) ** 3,
    inOutCubic: (x) => (x < 0.5 ? 4 * x * x * x : 1 - (-2 * x + 2) ** 3 / 2),
    outQuart: (x) => 1 - (1 - x) ** 4,
    outQuint: (x) => 1 - (1 - x) ** 5,
    inOutQuint: (x) => (x < 0.5 ? 16 * x ** 5 : 1 - (-2 * x + 2) ** 5 / 2),
    inOutSine: (x) => -(Math.cos(Math.PI * x) - 1) / 2,
    outExpo: (x) => (x >= 1 ? 1 : 1 - 2 ** (-10 * x)),
    inOutExpo: (x) => {
      if (x <= 0 || x >= 1) {
        return x <= 0 ? 0 : 1;
      }
      return x < 0.5 ? 2 ** (20 * x - 10) / 2 : (2 - 2 ** (-20 * x + 10)) / 2;
    },
    outBack: (x) => 1 + 2.2 * (x - 1) ** 3 + 1.2 * (x - 1) ** 2,
    outBackStrong: (x) => 1 + 2.7 * (x - 1) ** 3 + 1.7 * (x - 1) ** 2,
  };
  const curve = (name) => (typeof name === 'function' ? name : ease[name] || ease.outCubic);

  /**
   * A damped spring settling from 0 to 1, `t` seconds after it starts: `frequency`
   * oscillations per second, `damping` from 0 (rings on) to 1 (no overshoot).
   */
  const spring = (t, frequency = 2.2, damping = 0.45) => {
    if (t <= 0) {
      return 0;
    }
    const omega = 2 * Math.PI * frequency;
    const zeta = clamp(damping, 0.01, 0.999);
    const damped = omega * Math.sqrt(1 - zeta * zeta);
    const decay = Math.exp(-zeta * omega * t);
    return 1 - decay * (Math.cos(damped * t) + (zeta * omega / damped) * Math.sin(damped * t));
  };

  /** A generator of numbers from 0 to 1 that gives the same sequence for the same seed. */
  const random = (seed) => {
    let state = (Math.floor(Number(seed) || 0) ^ 0x5f3759df) | 0;
    return () => {
      state = (state + 0x9e3779b9) | 0;
      let z = state;
      z = Math.imul(z ^ (z >>> 16), 0x85ebca6b);
      z = Math.imul(z ^ (z >>> 13), 0xc2b2ae35);
      z ^= z >>> 16;
      return (z >>> 0) / 4294967296;
    };
  };

  /** Smooth noise from -1 to 1 along `x`, the same for the same `seed`. */
  const noise = (x, seed = 0) => {
    const hash = (n) => {
      const s = Math.sin(n * 127.1 + seed * 311.7) * 43758.5453;
      return (s - Math.floor(s)) * 2 - 1;
    };
    const i = Math.floor(x);
    const f = x - i;
    const u = f * f * (3 - 2 * f);
    return mix(hash(i), hash(i + 1), u);
  };

  // Parameters.

  const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
  /** `base` with the values of `over` in place, groups merged field by field. */
  const merge = (base, over) => {
    const out = { ...base };
    if (!isObject(over)) {
      return out;
    }
    for (const [key, value] of Object.entries(over)) {
      out[key] = isObject(value) && isObject(base[key]) ? merge(base[key], value) : value;
    }
    return out;
  };
  /**
   * The composition's parameters: `params.json` from its own folder, which FossCut writes
   * when it adds the composition, over `defaults`, which also apply when it is missing.
   */
  const loadParams = async (defaults) => {
    try {
      const response = await fetch('params.json');
      if (response.ok) {
        return merge(defaults, await response.json());
      }
    } catch (error) {
      // No params.json: the defaults.
    }
    return merge(defaults, {});
  };

  // Colours.

  /** `#RGB`, `#RRGGBB` or `#RRGGBBAA` as [r, g, b] from 0 to 255. */
  const rgb = (hex) => {
    let text = String(hex || '').replace('#', '');
    if (text.length === 3) {
      text = text.split('').map((c) => c + c).join('');
    }
    const n = parseInt(text.slice(0, 6), 16);
    return Number.isFinite(n) ? [(n >> 16) & 255, (n >> 8) & 255, n & 255] : [47, 107, 255];
  };
  const rgba = (hex, alpha) => `rgba(${rgb(hex).join(', ')}, ${alpha})`;
  /** Relative luminance from 0 (black) to 1 (white). */
  const luminance = (hex) => {
    const [r, g, b] = rgb(hex).map((c) => {
      const s = c / 255;
      return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  /** `hex` mixed towards `other` by `t`, as `#RRGGBB`. */
  const blend = (hex, other, t) => {
    const a = rgb(hex);
    const b = rgb(other);
    return `#${a.map((c, i) => Math.round(mix(c, b[i], t)).toString(16).padStart(2, '0')).join('')}`;
  };

  /** Applies the theme (`light` or `dark`) and the accent colour to the page. */
  const theme = (P) => {
    const root = document.documentElement;
    root.dataset.theme = P.theme === 'dark' ? 'dark' : 'light';
    const accent = typeof P.accent === 'string' ? P.accent.slice(0, 7) : '#2F6BFF';
    root.style.setProperty('--accent', accent);
    root.style.setProperty('--accent-rgb', rgb(accent).join(', '));
    root.style.setProperty('--accent-ink', luminance(accent) > 0.45 ? '#0B1220' : '#FFFFFF');
  };

  // Placeholders: abstract art for empty slots, never photographs.

  const PALETTES = [
    ['#4F7BFF', '#A06BFF', '#7CE0FF'],
    ['#22C3A6', '#3A7BFF', '#B6F3E4'],
    ['#FF8A3D', '#FF4F6D', '#FFD27A'],
    ['#E85CC2', '#7A5CFF', '#FFB3E6'],
    ['#14B8D4', '#5161F0', '#A8F0FF'],
    ['#F2B43A', '#F06C33', '#FFF0B8'],
    ['#2FBF71', '#1E9AC8', '#C9F7A8'],
    ['#6B7A99', '#2B3448', '#C8D2E6'],
  ];
  /** A CSS background of soft shapes over a gradient, the same for the same seed. */
  const placeholder = (seed) => {
    const next = random(Number(seed) * 7919 + 17);
    const [a, b, c] = PALETTES[Math.floor(next() * PALETTES.length)];
    const angle = Math.round(110 + next() * 70);
    const blob = (color, size, alpha) => {
      const x = Math.round(10 + next() * 80);
      const y = Math.round(10 + next() * 80);
      return `radial-gradient(circle at ${x}% ${y}%, ${rgba(color, alpha)} 0%, ${rgba(color, 0)} ${size}%)`;
    };
    const band = Math.round(30 + next() * 40);
    // A crisp disc and ring give the soft light a designed shape.
    const discX = Math.round(18 + next() * 64);
    const discY = Math.round(20 + next() * 60);
    const disc = Math.round(12 + next() * 12);
    return [
      `radial-gradient(circle at ${discX}% ${discY}%, rgba(255,255,255,0) ${disc + 7}%, rgba(255,255,255,0.22) ${disc + 7.4}%, rgba(255,255,255,0.22) ${disc + 8.6}%, rgba(255,255,255,0) ${disc + 9}%)`,
      `radial-gradient(circle at ${discX}% ${discY}%, rgba(255,255,255,0.20) ${disc}%, rgba(255,255,255,0) ${disc + 0.4}%)`,
      `linear-gradient(${angle + 90}deg, rgba(255,255,255,0) ${band - 12}%, rgba(255,255,255,0.16) ${band}%, rgba(255,255,255,0) ${band + 12}%)`,
      blob('#FFFFFF', 34, 0.38),
      blob(c, 46, 0.75),
      blob(b, 58, 0.85),
      `linear-gradient(${angle}deg, ${a} 0%, ${b} 100%)`,
    ].join(', ');
  };

  /** A two-colour gradient for avatars and swatches, the same for the same seed. */
  const gradient = (seed, angle = 135) => {
    const next = random(Number(seed) * 104729 + 3);
    const [a, b] = PALETTES[Math.floor(next() * (PALETTES.length - 1))];
    return `linear-gradient(${angle}deg, ${a} 0%, ${b} 100%)`;
  };

  // Icons, drawn on a 24-unit grid with round strokes.

  const ICONS = {
    play: '<path class="f" d="M8.5 5.6c0-.9 1-1.5 1.8-1l9.2 5.9c.7.5.7 1.5 0 2l-9.2 5.9c-.8.5-1.8-.1-1.8-1z"/>',
    check: '<path class="s" d="M5 12.6l4.6 4.6L19 7.6"/>',
    cross: '<path class="s" d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
    mic: '<rect class="s" x="9" y="3" width="6" height="11.5" rx="3"/><path class="s" d="M5.8 11.2a6.2 6.2 0 0 0 12.4 0M12 17.4V21M8.6 21h6.8"/>',
    heart: '<path class="s" d="M12 19.6C6 15.4 3.4 12.3 3.4 9.1c0-2.6 2-4.6 4.5-4.6 1.7 0 3.2.9 4.1 2.4.9-1.5 2.4-2.4 4.1-2.4 2.5 0 4.5 2 4.5 4.6 0 3.2-2.6 6.3-8.6 10.5z"/>',
    comment: '<path class="s" d="M5 5.5h14a1.8 1.8 0 0 1 1.8 1.8v8.4a1.8 1.8 0 0 1-1.8 1.8h-8.2L6.4 20.6v-3.1H5a1.8 1.8 0 0 1-1.8-1.8V7.3A1.8 1.8 0 0 1 5 5.5z"/>',
    share: '<path class="s" d="M13.5 5.2l6.6 6.3-6.6 6.3v-3.6c-4.4 0-7.4 1.3-9.6 4.4.6-4.8 3.3-8.6 9.6-9.3z"/>',
    bell: '<path class="s" d="M6.2 16.6V11a5.8 5.8 0 0 1 11.6 0v5.6l1.7 1.6H4.5zM10 20.5h4"/>',
    up: '<path class="s" d="M12 19V5.5M6.2 11.3L12 5.5l5.8 5.8"/>',
    down: '<path class="s" d="M12 5v13.5M6.2 12.7L12 18.5l5.8-5.8"/>',
    right: '<path class="s" d="M5 12h13.5M12.7 6.2l5.8 5.8-5.8 5.8"/>',
    clock: '<circle class="s" cx="12" cy="12" r="8.6"/><path class="s" d="M12 7.4V12l3.2 2"/>',
    eye: '<path class="s" d="M2.6 12c2.2-4.1 5.4-6.2 9.4-6.2s7.2 2.1 9.4 6.2c-2.2 4.1-5.4 6.2-9.4 6.2S4.8 16.1 2.6 12z"/><circle class="s" cx="12" cy="12" r="2.8"/>',
    scissors: '<circle class="s" cx="6.5" cy="7" r="2.7"/><circle class="s" cx="6.5" cy="17" r="2.7"/><path class="s" d="M8.8 8.6L20 16.4M8.8 15.4L20 7.6"/>',
    spark: '<path class="f" d="M12 3.2l1.9 5.6 5.7 1.9-5.7 1.9L12 18.2l-1.9-5.6-5.7-1.9 5.7-1.9z"/>',
    send: '<path class="s" d="M4.2 11.6L20 4.4l-5.6 15.8-3.1-6.7z"/><path class="s" d="M11.3 13.5L20 4.4"/>',
    film: '<rect class="s" x="3.4" y="5" width="17.2" height="14" rx="2.4"/><path class="s" d="M3.4 9.6h17.2M3.4 14.4h17.2M8.6 5v14M15.4 5v14"/>',
    wave: '<path class="s" d="M3.5 12h2M7.8 8.5v7M12 5v14M16.2 8.5v7M20.5 12h-2"/>',
    star: '<path class="f" d="M12 3.6l2.5 5.3 5.8.7-4.3 4 1.1 5.7L12 16.5l-5.1 2.8 1.1-5.7-4.3-4 5.8-.7z"/>',
    dots: '<circle class="f" cx="6" cy="12" r="1.8"/><circle class="f" cx="12" cy="12" r="1.8"/><circle class="f" cx="18" cy="12" r="1.8"/>',
    sliders: '<path class="s" d="M5 7h9M18 7h1M5 17h3M12 17h7"/><circle class="s" cx="16" cy="7" r="2.2"/><circle class="s" cx="10" cy="17" r="2.2"/>',
    grid: '<rect class="s" x="4" y="4" width="6.6" height="6.6" rx="1.6"/><rect class="s" x="13.4" y="4" width="6.6" height="6.6" rx="1.6"/><rect class="s" x="4" y="13.4" width="6.6" height="6.6" rx="1.6"/><rect class="s" x="13.4" y="13.4" width="6.6" height="6.6" rx="1.6"/>',
  };
  /** An SVG icon `name` of `size` pixels, its strokes `weight` units wide. */
  const icon = (name, size = 24, weight = 2) => {
    const holder = document.createElement('span');
    holder.innerHTML = `<svg class="icon" viewBox="0 0 24 24" style="width:${size}px;height:${size}px;stroke-width:${weight}">${ICONS[name] || ''}</svg>`;
    const svg = holder.firstElementChild;
    for (const part of svg.querySelectorAll('.s')) {
      part.style.strokeWidth = String(weight);
    }
    return svg;
  };

  // Elements.

  /** An element `tag` with `className`, `text` and inline `style`, put in `parent`. */
  const el = (tag, { className = '', text = null, style = null, html = null } = {}, parent = null) => {
    const node = document.createElement(tag);
    if (className) {
      node.className = className;
    }
    if (text !== null && text !== undefined) {
      node.textContent = String(text);
    }
    if (html !== null) {
      node.innerHTML = html;
    }
    if (style) {
      Object.assign(node.style, style);
    }
    if (parent) {
      parent.appendChild(node);
    }
    return node;
  };
  /** An SVG element `tag` with attributes `attrs`, put in `parent`. */
  const svg = (tag, attrs = {}, parent = null) => {
    const node = document.createElementNS('http://www.w3.org/2000/svg', tag);
    for (const [key, value] of Object.entries(attrs)) {
      // Presentation attributes take no CSS variables; styles do.
      if (String(value).includes('var(')) {
        node.style.setProperty(key, String(value));
      } else {
        node.setAttribute(key, String(value));
      }
    }
    if (parent) {
      parent.appendChild(node);
    }
    return node;
  };

  /** Up to two initials of `name`. */
  const initials = (name) => String(name || '').replace(/^@/, '').split(/[\s._-]+/).filter(Boolean)
    .slice(0, 2).map((w) => w[0].toUpperCase()).join('') || '?';

  // Text over time.

  /** The first characters of `text` typed `cps` characters a second from `start`. */
  const typed = (text, t, start, cps = 28) => {
    const chars = Array.from(String(text));
    const count = clamp(Math.floor((t - start) * cps), 0, chars.length);
    return chars.slice(0, count).join('');
  };
  /** True while a caret blinking twice a second is shown. */
  const caretOn = (t) => Math.floor(Math.max(0, t) * 2.2) % 2 === 0;

  /** `value` with `decimals` places and thousands separated by commas. */
  const formatNumber = (value, decimals = 0) => {
    const fixed = Math.abs(value).toFixed(decimals);
    const [whole, part] = fixed.split('.');
    const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
    return `${value < 0 && Number(fixed) !== 0 ? '-' : ''}${grouped}${part ? `.${part}` : ''}`;
  };
  /**
   * An amount written like "1.2M", "$49", "12,400 views" or "87%": what comes before and
   * after the number, the number and its decimal places.
   */
  const amount = (text) => {
    const found = /^(.*?)(-?\d[\d,]*(?:\.\d+)?|-?\.\d+)(.*)$/s.exec(String(text));
    if (!found) {
      return { prefix: String(text), value: 0, decimals: 0, suffix: '', number: false };
    }
    const digits = found[2].replace(/,/g, '');
    const decimals = digits.includes('.') ? digits.split('.')[1].length : 0;
    return { prefix: found[1], value: Number(digits), decimals, suffix: found[3], number: true, grouped: found[2].includes(',') };
  };
  /** `text` (an amount) with its number counted to `fraction` of its value. */
  const countAmount = (text, fraction) => {
    const a = amount(text);
    if (!a.number) {
      return a.prefix;
    }
    const value = a.value * clamp(fraction);
    const shown = a.grouped || Math.abs(a.value) >= 10000 ? formatNumber(value, a.decimals) : value.toFixed(a.decimals);
    return `${a.prefix}${shown}${a.suffix}`;
  };
  /** Seconds of a clock time such as "6:24" or "1:02:03" (a bare number is seconds). */
  const clockSeconds = (text) => {
    const parts = String(text).trim().split(':').map((p) => Number(p));
    if (parts.some((p) => !Number.isFinite(p))) {
      return 0;
    }
    return parts.reduce((total, part) => total * 60 + part, 0);
  };
  /** `seconds` as a clock time with as many fields as `like` ("6:24" or "1:02:03"). */
  const clockText = (seconds, like = '0:00') => {
    const total = Math.max(0, Math.round(seconds));
    const fields = Math.max(2, String(like).split(':').length);
    const s = total % 60;
    const m = Math.floor(total / 60);
    if (fields >= 3) {
      return `${Math.floor(m / 60)}:${String(m % 60).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
    }
    return `${m}:${String(s).padStart(2, '0')}`;
  };

  // Keyframes.

  const TRANSFORM = ['x', 'y', 'scale', 'sx', 'sy', 'rotate'];

  const targets = (target) => {
    if (!target) {
      return [];
    }
    if (typeof target === 'string') {
      return [...document.querySelectorAll(target)];
    }
    if (target instanceof Element) {
      return [target];
    }
    return [...target];
  };

  /** The value of keys `list` (sorted by `at`) at `t`, each eased into by its own curve. */
  const sample = (list, t) => {
    if (t <= list[0].at) {
      return list[0].value;
    }
    for (let i = 1; i < list.length; i += 1) {
      if (t < list[i].at) {
        const a = list[i - 1];
        const b = list[i];
        const p = (t - a.at) / Math.max(1e-9, b.at - a.at);
        return mix(a.value, b.value, curve(b.ease)(p));
      }
    }
    return list[list.length - 1].value;
  };

  const applyValues = (node, values) => {
    const style = node.style;
    if (TRANSFORM.some((k) => k in values)) {
      const x = values.x || 0;
      const y = values.y || 0;
      const scale = 'scale' in values ? values.scale : 1;
      const sx = scale * ('sx' in values ? values.sx : 1);
      const sy = scale * ('sy' in values ? values.sy : 1);
      const rotate = values.rotate || 0;
      const transform = `translate(${x.toFixed(2)}px, ${y.toFixed(2)}px) rotate(${rotate.toFixed(3)}deg) scale(${sx.toFixed(4)}, ${sy.toFixed(4)})`;
      // At rest an element has no transform at all, so it paints the same however it got
      // there.
      style.transform = transform === 'translate(0.00px, 0.00px) rotate(0.000deg) scale(1.0000, 1.0000)' ? '' : transform;
    }
    if ('opacity' in values) {
      const opacity = clamp(values.opacity);
      style.opacity = opacity >= 0.9999 ? '' : opacity.toFixed(4);
      style.visibility = opacity <= 0.001 ? 'hidden' : '';
    }
    if ('blur' in values) {
      style.filter = values.blur > 0.05 ? `blur(${values.blur.toFixed(2)}px)` : '';
    }
    if ('reveal' in values) {
      // Shown from the left edge up to `reveal` of the width.
      style.clipPath = `inset(-200px ${((1 - clamp(values.reveal)) * 100).toFixed(3)}% -200px -200px)`;
    }
    if ('rise' in values) {
      // Shown from the bottom edge up to `rise` of the height.
      style.clipPath = `inset(${((1 - clamp(values.rise)) * 100).toFixed(3)}% -200px -200px -200px)`;
    }
    for (const [key, value] of Object.entries(values)) {
      if (key.startsWith('--')) {
        style.setProperty(key, Number(value).toFixed(5));
      }
    }
  };

  /**
   * Motion declared as keyframes in seconds of the card's own time. `key(target, keys)`
   * takes keys like `{ at: 0.2, opacity: 1, y: 0, ease: 'outBack' }`: each property moves
   * from the key before that names it, with the curve of the key it moves to, and holds
   * before its first and after its last key. Properties: opacity, x and y (pixels), scale,
   * sx and sy, rotate (degrees), blur (pixels), reveal and rise (0 to 1) and CSS variables
   * (`'--name'`). `tween(start, duration, fn, ease)` calls `fn(eased, t)` every frame.
   */
  class Timeline {
    constructor() {
      this.items = new Map();
      this.tweens = [];
      this.calls = [];
    }

    key(target, keys) {
      for (const node of targets(target)) {
        let props = this.items.get(node);
        if (!props) {
          props = new Map();
          this.items.set(node, props);
        }
        for (const k of keys) {
          for (const [name, value] of Object.entries(k)) {
            if (name === 'at' || name === 'ease') {
              continue;
            }
            if (!props.has(name)) {
              props.set(name, []);
            }
            props.get(name).push({ at: k.at, value, ease: k.ease || 'outCubic' });
          }
        }
        for (const list of props.values()) {
          list.sort((a, b) => a.at - b.at);
        }
      }
      return this;
    }

    tween(start, duration, fn, easing = 'outCubic') {
      this.tweens.push({ start, duration, fn, easing });
      return this;
    }

    each(fn) {
      this.calls.push(fn);
      return this;
    }

    seek(t) {
      for (const [node, props] of this.items) {
        const values = {};
        for (const [name, list] of props) {
          values[name] = sample(list, t);
        }
        applyValues(node, values);
      }
      for (const tw of this.tweens) {
        tw.fn(curve(tw.easing)(progress(t, tw.start, tw.duration)), t);
      }
      for (const fn of this.calls) {
        fn(t);
      }
    }
  }

  /** Keys that pop something in at `at`: from small and clear to full size, overshooting. */
  const pop = (at = 0, { from = 0.6, duration = 0.5, y = 0, x = 0, rotate = 0, fade = 0.16, strong = false } = {}) => [
    { at, opacity: 0, scale: from, y, x, rotate },
    { at: at + fade, opacity: 1, ease: 'outQuad' },
    { at: at + duration, scale: 1, y: 0, x: 0, rotate: 0, ease: strong ? 'outBackStrong' : 'outBack' },
  ];
  /** Keys that slide something in by (`x`, `y`) pixels at `at`, fading in. */
  const slide = (at = 0, { x = 0, y = 28, duration = 0.45, fade = 0.25, easing = 'outCubic' } = {}) => [
    { at, opacity: 0, x, y },
    { at: at + fade, opacity: 1, ease: 'outQuad' },
    { at: at + duration, x: 0, y: 0, ease: easing },
  ];
  /** Keys that fade something in at `at`. */
  const fade = (at = 0, duration = 0.3) => [
    { at, opacity: 0 },
    { at: at + duration, opacity: 1, ease: 'outQuad' },
  ];
  /** The start of item `index` of a group starting at `at`, one every `step` seconds. */
  const stagger = (at, index, step = 0.07) => at + index * step;

  // The stage.

  /**
   * Lays the page out at the template's design size (`width` x `height`) and scales it to
   * the canvas, centred, keeping its proportions; `fluid` stretches the design size to the
   * canvas's proportions instead, for templates that place parts by fractions.
   */
  const stage = (width, height, { fluid = false } = {}) => {
    const canvasW = window.innerWidth || width;
    const canvasH = window.innerHeight || height;
    let w = width;
    let h = height;
    if (fluid) {
      // Keep the design's width and follow the canvas's proportions.
      h = Math.round(width * (canvasH / canvasW));
    }
    const scale = Math.min(canvasW / w, canvasH / h);
    const box = el('div', { style: { width: `${w}px`, height: `${h}px` } });
    box.id = 'stage';
    box.style.transform = `translate(${((canvasW - w * scale) / 2).toFixed(3)}px, ${((canvasH - h * scale) / 2).toFixed(3)}px) scale(${scale.toFixed(6)})`;
    const exit = el('div', {}, box);
    exit.id = 'exit';
    document.body.appendChild(box);
    current = { root: exit, box, width: w, height: h, scale };
    return current;
  };
  let current = null;

  /**
   * Centres `node` (a card whose height follows its content) on the stage, a little above
   * the middle so its shadow below has room; `horizontal` centres its width too.
   */
  const center = (node, { horizontal = false, lift = 10 } = {}) => {
    if (!current) {
      return;
    }
    const box = node.getBoundingClientRect();
    const h = box.height / current.scale;
    node.style.top = `${Math.round((current.height - h) / 2 - lift)}px`;
    if (horizontal) {
      node.style.left = `${Math.round((current.width - box.width / current.scale) / 2)}px`;
    }
  };

  // Media.

  /**
   * Fills `host` with slot `slot` (null, or `{src, kind}` from params.json): a picture, a
   * video drawn at the card's own time, or abstract art from `seed` when empty. `width`
   * and `height` size a video's canvas. Gives `{ ready, show(seconds), kind }`.
   */
  const media = (host, slot, { seed = 1, width = 640, height = 360, fit = 'cover' } = {}) => {
    host.classList.add('media');
    const kind = slot && typeof slot === 'object' && slot.src ? slot.kind : null;
    if (kind === 'image' || kind === 'svg') {
      const image = el('img', {}, host);
      image.style.objectFit = fit;
      image.src = slot.src;
      const ready = image.decode().catch(() => {});
      return { ready, show: async () => {}, kind };
    }
    if (kind === 'video') {
      // The video stays out of the document, where FossCut's clock leaves it alone, and
      // is drawn into a canvas once it has sought, so the card shows exactly the frame
      // for its own time.
      const video = document.createElement('video');
      video.muted = true;
      video.playsInline = true;
      video.preload = 'auto';
      const canvas = el('canvas', {}, host);
      canvas.width = Math.max(2, Math.round(width));
      canvas.height = Math.max(2, Math.round(height));
      const context = canvas.getContext('2d');
      const paint = () => {
        const vw = video.videoWidth || canvas.width;
        const vh = video.videoHeight || canvas.height;
        const scale = fit === 'contain'
          ? Math.min(canvas.width / vw, canvas.height / vh)
          : Math.max(canvas.width / vw, canvas.height / vh);
        context.clearRect(0, 0, canvas.width, canvas.height);
        context.drawImage(video, (canvas.width - vw * scale) / 2, (canvas.height - vh * scale) / 2, vw * scale, vh * scale);
      };
      const ready = new Promise((resolve) => {
        video.addEventListener('loadeddata', resolve, { once: true });
        video.addEventListener('error', resolve, { once: true });
        video.src = slot.src;
        video.load();
      });
      let painted = -1;
      const show = (seconds) => new Promise((resolve) => {
        const length = video.duration > 0 ? video.duration : 0;
        // A frame's middle avoids landing on the boundary between two frames.
        const target = length > 0 ? Math.min(Math.max(0, seconds) % length, Math.max(0, length - 0.001)) : 0;
        if (Math.abs(video.currentTime - target) < 0.0005 && video.readyState >= 2) {
          if (painted !== target) {
            paint();
            painted = target;
          }
          resolve();
          return;
        }
        const done = () => {
          paint();
          painted = target;
          resolve();
        };
        video.addEventListener('seeked', done, { once: true });
        video.addEventListener('error', () => resolve(), { once: true });
        video.currentTime = target;
      });
      return { ready, show, kind };
    }
    const art = el('div', { className: 'art' }, host);
    art.style.background = placeholder(seed);
    return { ready: Promise.resolve(), show: async () => {}, kind: null };
  };

  // Composing.

  /**
   * Builds a template: `setup(P, kit)` makes the card from the parameters and gives
   * `{ timeline, update(local, t), media: [...], exit }`; `exit` is `fade` (the default,
   * sinking a little as it fades), `up`, `down`, `left`, `right`, or `dissolve` (opacity
   * only, for full-screen pages that must not move). window.fosscut.seek(t) then draws
   * time t.
   */
  const compose = (defaults, setup) => {
    const ready = (async () => {
      const P = await loadParams(defaults);
      theme(P);
      // Templates measure text while building, so the fonts must be in first.
      await Promise.all(['400', '700', '800'].map((weight) => document.fonts.load(`${weight} 24px Inter`).catch(() => {})));
      const built = await setup(P);
      const items = built.media || [];
      await Promise.all(items.map((m) => m.ready));
      const exitStyle = built.exit || 'fade';
      const exitNode = document.getElementById('exit');
      const box = document.getElementById('stage');
      const appear = Number(P.appear) || 0;
      const hold = Math.max(0, Number(P.hold) || 0);
      const speed = Math.max(0.05, Number(P.speed) || 1);
      return async (t) => {
        const local = (t - appear) * speed;
        const gone = P.exit ? progress(t, appear + hold, EXIT) : 0;
        if (box) {
          box.style.visibility = local < 0 || gone >= 1 ? 'hidden' : '';
        }
        built.timeline.seek(local);
        if (built.update) {
          built.update(local, t);
        }
        if (exitNode) {
          const e = ease.inCubic(gone);
          const lift = ease.inOutCubic(gone);
          const shift = {
            fade: [0, 18 * lift],
            up: [0, -140 * lift],
            down: [0, 140 * lift],
            left: [-160 * lift, 0],
            right: [160 * lift, 0],
          }[exitStyle] || [0, 0];
          exitNode.style.opacity = gone > 0 ? (1 - e).toFixed(4) : '';
          exitNode.style.transform = gone > 0 && exitStyle !== 'dissolve'
            ? `translate(${shift[0].toFixed(2)}px, ${shift[1].toFixed(2)}px) scale(${(1 - 0.08 * lift).toFixed(4)})`
            : '';
        }
        // Videos play at real time from the entrance.
        await Promise.all(items.map((m) => m.show(Math.max(0, t - appear))));
      };
    })();
    window.fosscut = {
      seek: async (seconds) => {
        const draw = await ready;
        await draw(Math.max(0, Number(seconds) || 0));
      },
    };
    // Opened by hand with ?t=seconds, the page shows that time.
    const asked = /[?&]t=([0-9.]+)/.exec(window.location.search);
    if (asked && !window.__fosscut) {
      window.fosscut.seek(Number(asked[1]));
    }
    return ready;
  };

  window.Overlay = Object.freeze({
    EXIT,
    clamp,
    mix,
    progress,
    ease,
    spring,
    random,
    noise,
    merge,
    loadParams,
    rgb,
    rgba,
    luminance,
    blend,
    theme,
    placeholder,
    gradient,
    icon,
    el,
    svg,
    initials,
    typed,
    caretOn,
    formatNumber,
    amount,
    countAmount,
    clockSeconds,
    clockText,
    Timeline,
    pop,
    slide,
    fade,
    stagger,
    stage,
    center,
    media,
    compose,
  });
})();
