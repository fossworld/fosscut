// Review frame, made by FossCut for its library (ADR-0042): a full-screen review app
// around a vertical video, with versions, comment markers, a comment and approve buttons.

// fosscut:defaults
const DEFAULTS = {
  "media": null,
  "open": false,
  "file_name": "Final cut.mp4",
  "subtitle": "Edited 2 min ago",
  "share": "Share",
  "versions": ["V1", "V2", "V3"],
  "current": 0,
  "notes": "2 comments",
  "commenter": "Alex Morgan",
  "timestamp": "0:12",
  "comment": "Love the pacing. Can we trim the intro a little?",
  "approve": "Approve",
  "changes": "Request changes",
  "appear": 0,
  "hold": 3.5,
  "exit": true,
  "speed": 1,
  "theme": "light",
  "accent": "#2F6BFF"
};
// fosscut:end

Overlay.compose(DEFAULTS, async (P) => {
  const { el, icon, media, Timeline, pop, fade, slide, gradient, initials } = Overlay;
  const { root } = Overlay.stage(1080, 1920);
  const page = el('div', { className: P.open ? 'open' : 'solid' }, root);
  page.id = 'page';
  const frame = el('div', {}, page);
  frame.id = 'frame';
  const zoom = el('div', { style: { position: 'absolute', inset: '0' } }, frame);
  const items = [];
  if (!P.open) {
    items.push(media(el('div', {}, zoom), P.media, { seed: 13, width: 621, height: 1104 }));
  }
  const edge = el('div', {}, root);
  edge.id = 'edge';
  edge.classList.toggle('hidden', !P.open);

  const bar = el('div', { className: 'card' }, root);
  bar.id = 'bar';
  const back = el('div', {}, bar);
  back.id = 'back';
  const chevron = icon('right', 32, 2.6);
  chevron.style.transform = 'scaleX(-1)';
  back.appendChild(chevron);
  const file = el('div', {}, bar);
  file.id = 'file';
  el('div', { className: 'nowrap', text: P.file_name }, file).id = 'name';
  const sub = el('div', { className: 'nowrap', text: P.subtitle }, file);
  sub.id = 'sub';
  sub.classList.toggle('hidden', !P.subtitle);
  const people = el('div', {}, bar);
  people.id = 'people';
  for (const [name, seed] of [[P.commenter, 2], ['You', 6]]) {
    const face = el('div', { className: 'avatar', text: initials(name) }, people);
    face.style.background = gradient(seed);
  }
  const share = el('div', { className: 'button', text: P.share }, bar);
  share.id = 'share';
  share.classList.toggle('hidden', !P.share);

  const versions = el('div', {}, root);
  versions.id = 'versions';
  const names = (Array.isArray(P.versions) ? P.versions : []).slice(0, 6);
  let current = Math.round(Number(P.current)) - 1;
  if (!(current >= 0 && current < names.length)) {
    current = names.length - 1;
  }
  const pills = names.map((name, i) => el('div', { className: `pill${i === current ? ' on' : ''}`, text: name }, versions));
  const notes = el('div', { className: 'pill' }, versions);
  notes.id = 'notes';
  notes.appendChild(icon('comment', 28, 2.4));
  el('span', { text: ` ${P.notes}` }, notes);
  notes.classList.toggle('hidden', !P.notes);

  const scrub = el('div', {}, root);
  scrub.id = 'scrub';
  const track = el('div', {}, scrub);
  track.id = 'track';
  const played = el('div', {}, track);
  played.id = 'played';
  const markers = [[0.28, P.commenter, 2], [0.62, 'You', 6]].map(([at, name, seed]) => {
    const marker = el('div', { className: 'marker', style: { left: `${at * 100}%` } }, scrub);
    const face = el('div', { className: 'avatar', text: initials(name) }, marker);
    face.style.background = gradient(seed);
    return marker;
  });

  const comment = el('div', { className: 'card' }, root);
  comment.id = 'comment';
  const face = el('div', { className: 'avatar', text: initials(P.commenter) }, comment);
  face.style.background = gradient(2);
  const words = el('div', {}, comment);
  words.id = 'words';
  const who = el('div', {}, words);
  who.id = 'who';
  el('span', { className: 'nowrap', text: P.commenter }, who);
  const stamp = el('span', { className: 'chip accent', text: P.timestamp }, who);
  stamp.classList.toggle('hidden', !P.timestamp);
  const text = el('div', { className: 'clip-2', text: P.comment }, words);
  text.id = 'text';
  comment.classList.toggle('hidden', !P.comment);

  const buttons = el('div', {}, root);
  buttons.id = 'buttons';
  const changes = el('div', { className: 'button', text: P.changes }, buttons);
  changes.id = 'changes';
  changes.classList.toggle('hidden', !P.changes);
  const approve = el('div', { className: 'button' }, buttons);
  approve.appendChild(icon('check', 32, 3));
  el('span', { text: P.approve }, approve);
  approve.classList.toggle('hidden', !P.approve);

  const tl = new Timeline();
  tl.key(page, fade(0, 0.35));
  tl.key(edge, fade(0.15, 0.3));
  tl.key(zoom, [{ at: 0, scale: 1.1 }, { at: 1.1, scale: 1, ease: 'outCubic' }]);
  tl.key(bar, slide(0.08, { y: -60, duration: 0.55, easing: 'outBack' }));
  pills.forEach((pill, i) => tl.key(pill, pop(0.25 + i * 0.07, { from: 0.5 })));
  tl.key(notes, pop(0.25 + pills.length * 0.07, { from: 0.5 }));
  tl.key(scrub, fade(0.45, 0.25));
  tl.key(played, [{ at: 0.5, '--w': 0 }, { at: 1.6, '--w': 0.45, ease: 'inOutCubic' }]);
  played.style.width = 'calc(var(--w) * 100%)';
  markers.forEach((marker, i) => tl.key(marker, pop(0.85 + i * 0.12, { from: 0.3, strong: true })));
  tl.key(comment, slide(0.6, { y: 60, duration: 0.55, easing: 'outBack' }));
  tl.key(buttons, slide(0.8, { y: 40, duration: 0.5, easing: 'outBack' }));
  // The approve button is pressed once everything is in.
  tl.key(approve, [{ at: 1.9, scale: 1 }, { at: 2.0, scale: 0.94, ease: 'outQuad' }, { at: 2.3, scale: 1, ease: 'outBack' }]);
  // The page fades without moving, so an open window stays over the clip below.
  return { timeline: tl, media: items, exit: 'dissolve' };
});
