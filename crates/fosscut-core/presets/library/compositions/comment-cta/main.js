// Comment call to action, made by FossCut for its library (ADR-0042): a comment
// notification drops in, and its keyword pops in an accent pill.

// fosscut:defaults
const DEFAULTS = {
  "avatar": null,
  "header": "New comment",
  "name": "your.handle",
  "time": "now",
  "message": "Comment {keyword} and I'll send you the full workflow",
  "keyword": "EDIT",
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
  const { root } = Overlay.stage(1080, 380);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const top = el('div', {}, card);
  top.id = 'top';
  const bell = el('div', {}, top);
  bell.id = 'bell';
  bell.appendChild(icon('comment', 26, 2.4));
  el('span', { text: P.header }, top);
  const when = el('span', { className: 'when', text: P.time }, top);
  when.classList.toggle('hidden', !P.time);

  const body = el('div', {}, card);
  body.id = 'body';
  const avatar = el('div', { className: 'avatar', text: Overlay.initials(P.name) }, body);
  avatar.style.background = Overlay.gradient(8);
  const face = P.avatar ? media(el('div', {}, avatar), P.avatar, { seed: 8 }) : null;
  const text = el('div', {}, body);
  text.id = 'text';
  const name = el('div', { className: 'nowrap', text: P.name }, text);
  name.id = 'name';
  name.classList.toggle('hidden', !P.name);
  const message = el('div', { className: 'clip-3' }, text);
  message.id = 'message';
  // The keyword goes where {keyword} is, or after the message when it is missing.
  const raw = String(P.message || '');
  const parts = raw.includes('{keyword}') ? raw.split('{keyword}') : [raw ? `${raw} ` : '', ''];
  message.appendChild(document.createTextNode(parts[0]));
  let keyword = null;
  if (P.keyword) {
    keyword = el('span', { text: P.keyword }, message);
    keyword.id = 'keyword';
  }
  message.appendChild(document.createTextNode(parts.slice(1).join(P.keyword || '')));

  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, [
    { at: 0, opacity: 0, y: -90, scale: 0.92 },
    { at: 0.14, opacity: 1, ease: 'outQuad' },
    { at: 0.62, y: 0, scale: 1, ease: 'outBack' },
  ]);
  tl.key(bell, pop(0.2, { from: 0.3, rotate: -25, strong: true }));
  tl.key(avatar, pop(0.26, { from: 0.4 }));
  tl.key(name, slide(0.3, { y: 14 }));
  tl.key(message, slide(0.38, { y: 16 }));
  if (keyword) {
    tl.key(keyword, [
      { at: 0.62, scale: 0.3, rotate: -6 },
      { at: 0.95, scale: 1, rotate: 0, ease: 'outBackStrong' },
    ]);
    tl.each((t) => {
      // A gentle pulse invites the comment.
      if (t < 1.3) {
        return;
      }
      const beat = ((t - 1.3) % 1.2) / 1.2;
      const bump = beat < 0.3 ? Math.sin((beat / 0.3) * Math.PI) : 0;
      keyword.style.transform = `scale(${(1 + 0.08 * bump).toFixed(4)})`;
      keyword.style.boxShadow = `0 0 0 ${(10 * bump).toFixed(2)}px rgba(var(--accent-rgb), ${(0.25 * bump).toFixed(3)})`;
    });
  }
  return { timeline: tl, media: face ? [face] : [], exit: 'up' };
});
