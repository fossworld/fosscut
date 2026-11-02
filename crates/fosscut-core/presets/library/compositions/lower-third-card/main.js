// Lower third card, made by FossCut for its library (ADR-0042): a thumbnail, a tag, a
// title, a subtitle and a call to action, sliding in from the left.

// fosscut:defaults
const DEFAULTS = {
  "thumbnail": null,
  "tag": "New video",
  "title": "Full breakdown",
  "subtitle": "The whole workflow, step by step",
  "cta": "Watch now",
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, media, Timeline, pop, fade, slide } = Overlay;
  const { root } = Overlay.stage(1080, 360);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const thumb = el('div', {}, card);
  thumb.id = 'thumb';
  const zoom = el('div', { style: { position: 'absolute', inset: '0' } }, thumb);
  const picture = media(el('div', {}, zoom), P.thumbnail, { seed: 41, width: 340, height: 192 });
  const play = el('div', {}, thumb);
  play.id = 'play';
  play.appendChild(icon('play', 34));
  const words = el('div', {}, card);
  words.id = 'words';
  const tag = el('div', { className: 'chip accent', text: P.tag }, words);
  tag.id = 'tag';
  tag.classList.toggle('hidden', !P.tag);
  const title = el('div', { className: 'title nowrap', text: P.title }, words);
  title.id = 'title';
  const subtitle = el('div', { className: 'nowrap', text: P.subtitle }, words);
  subtitle.id = 'subtitle';
  subtitle.classList.toggle('hidden', !P.subtitle);
  const cta = el('div', { className: 'button' }, words);
  cta.id = 'cta';
  el('span', { text: P.cta }, cta);
  const arrow = el('span', { className: 'arrow' }, cta);
  arrow.appendChild(icon('right', 26, 2.8));
  cta.classList.toggle('hidden', !P.cta);

  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, [
    { at: 0, opacity: 0, x: -140, scale: 0.94 },
    { at: 0.16, opacity: 1, ease: 'outQuad' },
    { at: 0.6, x: 0, scale: 1, ease: 'outBack' },
  ]);
  tl.key(zoom, [{ at: 0.05, scale: 1.25 }, { at: 0.9, scale: 1, ease: 'outCubic' }]);
  tl.key(play, pop(0.4, { from: 0.3, strong: true }));
  tl.key(tag, pop(0.22, { from: 0.5 }));
  tl.key(title, slide(0.26, { y: 20 }));
  tl.key(subtitle, slide(0.36, { y: 16 }));
  tl.key(cta, pop(0.5, { from: 0.5, y: 10 }));
  tl.each((t) => {
    // The arrow nudges forward now and then once everything is in.
    const settled = Math.max(0, t - 1.1);
    const push = t < 1.1 ? 0 : Math.max(0, Math.sin(settled * Math.PI * 1.5)) ** 2 * 7;
    arrow.style.transform = `translateX(${push.toFixed(2)}px)`;
  });
  return { timeline: tl, media: [picture], exit: 'left' };
});
