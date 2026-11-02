// Social post, made by FossCut for its library (ADR-0042): a generic post whose heart
// fills with a burst while likes, comments and shares count up.

// fosscut:defaults
const DEFAULTS = {
  "avatar": null,
  "image": null,
  "show_image": true,
  "name": "Your Name",
  "handle": "@yourname",
  "time": "2h",
  "text": "Recorded one take, let the edit do the rest. Here is the whole workflow.",
  "likes": "2.4K",
  "comments": "186",
  "shares": "92",
  "liked": true,
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, media, Timeline, pop, fade, slide, countAmount } = Overlay;
  const { root } = Overlay.stage(1080, 1240);
  const card = el('div', { className: 'card' }, root);
  card.id = 'card';
  const head = el('div', {}, card);
  head.id = 'head';
  const avatar = el('div', { className: 'avatar', text: Overlay.initials(P.name) }, head);
  avatar.style.background = Overlay.gradient(5);
  const items = [];
  if (P.avatar) {
    items.push(media(el('div', {}, avatar), P.avatar, { seed: 5 }));
  }
  const who = el('div', {}, head);
  who.id = 'who';
  el('div', { className: 'nowrap', text: P.name }, who).id = 'name';
  const handle = el('div', { className: 'nowrap', text: [P.handle, P.time].filter(Boolean).join(' \u00B7 ') }, who);
  handle.id = 'handle';
  const more = el('div', {}, head);
  more.id = 'more';
  more.appendChild(icon('dots', 40));
  const text = el('div', { className: 'clip-3', text: P.text }, card);
  text.id = 'text';
  text.classList.toggle('hidden', !P.text);
  const picture = el('div', {}, card);
  picture.id = 'picture';
  picture.classList.toggle('hidden', !P.show_image);
  const zoom = el('div', { style: { position: 'absolute', inset: '0' } }, picture);
  if (P.show_image) {
    items.push(media(el('div', {}, zoom), P.image, { seed: 31, width: 888, height: 560 }));
  }

  const actions = el('div', {}, card);
  actions.id = 'actions';
  const like = el('div', { className: 'act' }, actions);
  const heart = el('div', {}, like);
  heart.id = 'heart';
  const empty = icon('heart', 44, 2.2);
  const full = icon('heart', 44, 2.2);
  full.classList.add('full');
  heart.append(empty, full);
  const sparks = Array.from({ length: 8 }, () => el('i', { className: 'spark' }, heart));
  const likes = el('span', { className: 'tnum' }, like);
  likes.id = 'liked';
  const comment = el('div', { className: 'act' }, actions);
  comment.appendChild(icon('comment', 42, 2.2));
  const comments = el('span', { className: 'tnum' }, comment);
  const share = el('div', { className: 'act' }, actions);
  share.appendChild(icon('share', 42, 2.2));
  const shares = el('span', { className: 'tnum' }, share);

  const likeAt = 1.15;
  Overlay.center(card);
  const tl = new Timeline();
  tl.key(card, pop(0, { from: 0.8, y: 50, duration: 0.6 }));
  tl.key(avatar, pop(0.18, { from: 0.4 }));
  tl.key(who, slide(0.22, { x: -16, y: 0 }));
  tl.key(more, fade(0.3, 0.3));
  tl.key(text, slide(0.3, { y: 18 }));
  tl.key(zoom, [{ at: 0.3, opacity: 0, scale: 1.12 }, { at: 0.6, opacity: 1, ease: 'outQuad' }, { at: 1.4, scale: 1, ease: 'outCubic' }]);
  [like, comment, share].forEach((node, i) => tl.key(node, slide(0.5 + i * 0.07, { y: 14 })));
  if (P.liked) {
    tl.key(full, [{ at: likeAt, scale: 0, opacity: 0 }, { at: likeAt + 0.05, opacity: 1 }, { at: likeAt + 0.4, scale: 1, ease: 'outBackStrong' }]);
    tl.key(empty, [{ at: likeAt, opacity: 1 }, { at: likeAt + 0.1, opacity: 0, ease: 'linear' }]);
    sparks.forEach((spark, i) => {
      const a = (i / sparks.length) * Math.PI * 2;
      tl.key(spark, [
        { at: likeAt + 0.12, opacity: 0, x: 0, y: 0, scale: 1 },
        { at: likeAt + 0.16, opacity: 1 },
        { at: likeAt + 0.5, x: Math.cos(a) * 46, y: Math.sin(a) * 46, scale: 0.3, ease: 'outCubic' },
        { at: likeAt + 0.6, opacity: 0, ease: 'linear' },
      ]);
    });
  } else {
    tl.key(full, [{ at: 0, opacity: 0 }]);
    sparks.forEach((spark) => tl.key(spark, [{ at: 0, opacity: 0 }]));
  }
  tl.tween(0.55, 1.2, (p, t) => {
    // The likes jump once more when the heart fills.
    const bonus = P.liked && t >= likeAt + 0.1 ? 1 : 0.97;
    likes.textContent = countAmount(P.likes, p * bonus);
    comments.textContent = countAmount(P.comments, p);
    shares.textContent = countAmount(P.shares, p);
  }, 'outCubic');
  likes.classList.toggle('hidden', !P.likes);
  comments.classList.toggle('hidden', !P.comments);
  shares.classList.toggle('hidden', !P.shares);
  return { timeline: tl, media: items };
});
