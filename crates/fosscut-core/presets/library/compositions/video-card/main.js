// Video card, made by FossCut for its library (ADR-0042): a thumbnail with a play button,
// length and watched bar, then the channel, title, views and age.

// fosscut:defaults
const DEFAULTS = {
  "thumbnail": null,
  "avatar": null,
  "title": "How I edit a week of videos in one afternoon",
  "channel": "Your Channel",
  "views": "1.2M views",
  "age": "2 days ago",
  "length": "12:48",
  "watched": 0.35,
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, media, Timeline, pop, slide, fade, countAmount } = Overlay;
  const { root } = Overlay.stage(1080, 860);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';

  const thumb = el('div', {}, card);
  thumb.id = 'thumb';
  const zoom = el('div', { style: { position: 'absolute', inset: '0' } }, thumb);
  const picture = media(el('div', {}, zoom), P.thumbnail, { seed: 11, width: 912, height: 513 });
  el('div', {}, thumb).id = 'shade';
  const ring = el('div', {}, thumb);
  ring.id = 'ring';
  const play = el('div', {}, thumb);
  play.id = 'play';
  play.appendChild(icon('play', 60));
  const length = el('div', { text: P.length }, thumb);
  length.id = 'length';
  length.classList.toggle('hidden', !P.length);
  const track = el('div', {}, thumb);
  track.id = 'track';
  track.classList.toggle('hidden', !(P.watched > 0));
  const watched = el('div', {}, track);
  watched.id = 'watched';

  const info = el('div', {}, card);
  info.id = 'info';
  const avatar = el('div', { className: 'avatar', text: Overlay.initials(P.channel) }, info);
  avatar.style.background = Overlay.gradient(3);
  const face = P.avatar ? media(el('div', {}, avatar), P.avatar, { seed: 3 }) : null;
  const text = el('div', {}, info);
  text.id = 'text';
  const title = el('div', { className: 'heading clip-2', text: P.title }, text);
  title.id = 'title';
  const meta = el('div', {}, text);
  meta.id = 'meta';
  const channel = el('b', { text: P.channel }, meta);
  const views = el('span', {}, meta);
  const rest = el('span', { text: P.age ? ` \u00B7 ${P.age}` : '' }, meta);
  channel.classList.toggle('hidden', !P.channel);

  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.72, y: 40, duration: 0.55 }));
  tl.key(zoom, [{ at: 0, scale: 1.12 }, { at: 1.1, scale: 1, ease: 'outCubic' }]);
  tl.key(play, pop(0.28, { from: 0.3, duration: 0.5, strong: true }));
  tl.key(length, slide(0.36, { x: 24, y: 0 }));
  tl.key(avatar, pop(0.3, { from: 0.4 }));
  tl.key(title, slide(0.34, { y: 22 }));
  tl.key(meta, fade(0.46, 0.3));
  tl.key(watched, [{ at: 0.6, sx: 0 }, { at: 1.6, sx: Math.min(1, Math.max(0, P.watched)), ease: 'inOutCubic' }]);
  tl.tween(0.46, 1.0, (p) => {
    const counted = countAmount(P.views, p);
    views.textContent = P.views ? `${P.channel ? ' \u00B7 ' : ''}${counted}` : '';
  }, 'outCubic');
  tl.each((t) => {
    // After the build, a ring pulses out of the play button now and then.
    const cycle = (t - 1.6) % 1.6;
    if (t < 1.6 || cycle > 0.9) {
      ring.style.visibility = 'hidden';
      return;
    }
    const p = cycle / 0.9;
    ring.style.visibility = '';
    ring.style.opacity = (0.8 * (1 - p)).toFixed(4);
    ring.style.transform = `scale(${(1 + 0.55 * Overlay.ease.outCubic(p)).toFixed(4)})`;
  });
  return { timeline: tl, media: face ? [picture, face] : [picture] };
});
