// Audio chip, made by FossCut for its library (ADR-0042): a microphone, a label, a
// recording dot with a timer and a live waveform in one pill.

// fosscut:defaults
const DEFAULTS = {
  "label": "Mic on",
  "status": "REC",
  "timer": true,
  "start_time": "0:00",
  "bars": 18,
  "level": 0.8,
  "appear": 0,
  "hold": 3,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, Timeline, pop, fade, slide, noise, clamp, clockSeconds, clockText } = Overlay;
  const { root, scale } = Overlay.stage(760, 280);
  const chip = el('div', { className: 'card' }, root);
  chip.id = 'chip';
  const mic = el('div', {}, chip);
  mic.id = 'mic';
  const halo = el('div', {}, mic);
  halo.id = 'halo';
  mic.appendChild(icon('mic', 54, 2.2));
  const words = el('div', {}, chip);
  words.id = 'words';
  const label = el('div', { text: P.label }, words);
  label.id = 'label';
  label.classList.toggle('hidden', !P.label);
  const status = el('div', {}, words);
  status.id = 'status';
  const rec = el('i', {}, status);
  rec.id = 'rec';
  const statusText = el('span', { text: P.status }, status);
  const timer = el('span', { className: 'tnum' }, status);
  status.classList.toggle('hidden', !P.status && !P.timer);
  const wave = el('div', {}, chip);
  wave.id = 'wave';
  const count = clamp(Math.round(Number(P.bars) || 18), 4, 40);
  const bars = Array.from({ length: count }, () => el('i', {}, wave));
  statusText.classList.toggle('hidden', !P.status);
  const startAt = clockSeconds(P.start_time);
  const timeText = (t) => (P.timer ? `${P.status ? ' ' : ''}${clockText(startAt + Math.max(0, t - 0.3), '0:00')}` : '');
  timer.textContent = timeText(0);
  // Centre the pill now that its width is known.
  const width = chip.getBoundingClientRect().width / scale;
  chip.style.left = `${Math.round((760 - width) / 2)}px`;
  const level = clamp(Number(P.level) || 0.8, 0.05, 1);

  const tl = new Timeline();
  tl.key(chip, [
    { at: 0, opacity: 0, scale: 0.4, sx: 0.5 },
    { at: 0.12, opacity: 1, ease: 'outQuad' },
    { at: 0.5, scale: 1, sx: 1, ease: 'outBack' },
  ]);
  tl.key(mic, pop(0.08, { from: 0.5, strong: true }));
  tl.key(label, slide(0.22, { x: -16, y: 0 }));
  tl.key(status, fade(0.32, 0.25));
  bars.forEach((bar, i) => tl.key(bar, fade(0.3 + i * 0.018, 0.2)));
  tl.each((t) => {
    const live = Math.max(0, t - 0.3);
    bars.forEach((bar, i) => {
      // Speech-like: syllables come and go along the bars.
      const envelope = 0.55 + 0.45 * Math.sin(i / (count - 1) * Math.PI);
      const syllable = Math.abs(noise(live * 5.2 + i * 0.37, 7)) * 0.75 + Math.abs(noise(live * 11 + i * 0.9, 3)) * 0.35;
      const grow = clamp((t - 0.3 - i * 0.018) / 0.3);
      const h = 10 + 74 * level * envelope * clamp(syllable) * grow;
      bar.style.height = `${h.toFixed(2)}px`;
    });
    rec.style.opacity = Math.floor(t * 1.6) % 2 === 0 ? '1' : '0.25';
    timer.textContent = timeText(t);
    const pulse = ((t - 0.6) % 1.4) / 1.4;
    halo.style.visibility = t < 0.6 ? 'hidden' : '';
    halo.style.opacity = (0.55 * (1 - pulse)).toFixed(3);
    halo.style.transform = `scale(${(1 + 0.35 * Overlay.ease.outCubic(pulse)).toFixed(4)})`;
  });
  return { timeline: tl };
});
