# Explainer visuals for a talking-head reel

FossCut's library has 22 explainer overlays made for vertical talking-head videos: cards, windows, charts, pointers and two full-screen backgrounds, each a web composition drawn by FossCut and free to use, also commercially. They share one look (white or near-black cards, soft shadows, rounded corners, Inter, one accent colour) and one motion style (a pop with a little overshoot, then small staggered details), so several in one reel look like a set. No platform's name, logo or interface is copied; the placeholder art is abstract.

## Adding one

1. Add it with the `library` tool: `{"action": "add", "name": "video-card", "start": "12.3s", "params": {"title": "...", "hold": 2.4}, "media": ["asset_0007"]}`, or `fosscut library add video-card --at 12.3s --params '{"hold": 2.4}' --media asset_0007`. `library` with action list and kind composition shows every parameter with its type, range and default.
2. Place the new clip with `clip.set`: `{"transform.scale": 0.85, "transform.position": [0, -0.29]}`. The placements below are written for that call.
3. Give it a sound on its entrance from the library (a pop or a whoosh at the clip's start).

Different parameters make another copy of the composition (about 1.3 MB with its fonts), so set them when adding rather than adding twice.

## Timing parameters (every overlay)

| Parameter | Default | What it does |
|---|---|---|
| `appear` | 0 | Seconds from the clip's start to the entrance; the canvas is clear before it |
| `hold` | per overlay | Seconds from the entrance's start until the exit starts |
| `exit` | true | Leave with a 0.4 s exit after `hold` (cards fade as they sink a little, or slide away; the full-screen pages fade in place); false keeps it until the clip ends |
| `speed` | 1 | Plays the entrance and inner motion (typing, counting, building) faster above 1; `hold` and the exit keep their seconds |
| `theme` | light (dark for `terminal` and `grid-stage`) | White cards with dark text, or near-black cards with light text |
| `accent` | `#2F6BFF` | Highlights, bars, buttons, selections |
| `width`, `height` | the overlay's canvas | Render size; the design scales to fit, so a larger canvas is sharper |
| `duration` | `appear` + `hold` (+ 0.4 s with `exit`) | The clip's length; leave it out |

To match a phrase: start the clip at the phrase's first word with `appear` 0, and set `hold` to the time until the phrase ends (or until the next visual), so the exit starts as the speaker moves on. The clip then lasts `hold` plus 0.4 s. If the phrase is shorter than the overlay's build time (listed below), raise `speed` so the build fits: build time divided by `speed` should be under `hold`.

## Placement on a vertical video

A web clip is fitted inside the canvas at scale 1, centred: an overlay wider than 9:16 (every card) spans the full frame width at scale 1. `transform.position` moves its centre by fractions of the frame from the middle (x right, y down), and `transform.scale` multiplies its size. An overlay W x H pixels at scale s covers s of the frame's width and 0.5625 x s x H / W of its height, on any 9:16 frame (1080x1920 or 720x1280). Each overlay's canvas includes clear margins for its shadow, so the card itself is a little smaller.

The usual zones with the speaker framed in the middle, head at about 30 to 48% of the height and captions at about 60%:

| Zone | Position | Height it may take |
|---|---|---|
| Above the head | [0, -0.29] (centre at 21% of the height) | about 0.38 of the frame |
| Top half of a split screen (the speaker moved into the bottom half) | [0, -0.25] | up to 0.5 |
| Low, clear of captions | [0, 0.3] | about 0.18 |
| Full screen (`review-frame`, `grid-stage`) | [0, 0] at scale 1 | all |

## The overlays

### `video-card`

- **Shows:** a video card: a 16:9 thumbnail with a play button, length badge and watched bar, the channel's avatar, the title and a line of views and age. It pops in, the views count up, the watched bar fills, then a ring pulses from the play button.
- **Use for:** "I posted a video about...", "my long-form video", "this upload got...".
- **Params:** `title`, `channel`, `views` (counted up, "1.2M views"), `age`, `length` ("12:48", empty hides), `watched` (0 to 1).
- **Slots:** `thumbnail` (picture or video), `avatar` (picture).
- **Canvas:** 1080x860. Above the head: scale 0.85, position [0, -0.29]. Top half: scale 1, position [0, -0.25].
- **Build:** 1.6 s. **Typical length:** 2.5 to 4 s.

### `app-window`

- **Shows:** a generic editing app window: title bar with an export button, a media panel, a viewer and a timeline of clips building in under a running playhead.
- **Use for:** "in my editor", "the old way, hours on the timeline", "software like this". Lay `cross-out` over it with the same placement to reject it.
- **Params:** `title` (project name), `button`, `panel` (media panel heading), `timeline` (show the timeline).
- **Slots:** `media` (picture or video in the viewer).
- **Canvas:** 1080x780. Above the head: scale 0.9, position [0, -0.29]. Top half: scale 1, position [0, -0.25].
- **Build:** 1.4 s, then the playhead keeps running. **Typical length:** 2.5 to 4 s.

### `terminal`

- **Shows:** a dark terminal window typing steps behind a prompt, each finished with a green check mark.
- **Use for:** a sequence of steps or an automated process: "it analyzes the video, cuts the silences, adds captions and exports".
- **Params:** `title`, `steps` (up to 6 lines), `prompt`, `done` (word beside each check), `typing` (characters per second).
- **Slots:** none.
- **Canvas:** 1080x700. Above the head: scale 0.9, position [0, -0.29].
- **Build:** about 0.45 s plus, per step, its characters divided by `typing` and 0.46 s; 4.7 s for the default four steps. **Typical length:** 3 to 6 s; raise `speed` for short phrases.

### `review-frame`

- **Shows:** a full-screen review app around a vertical video: a bar with the file's name and a share button, version pills, the video in a rounded window, a scrubber with comment markers, a comment and Approve and Request changes buttons.
- **Use for:** "I send the cut to my client", "feedback", "version three", "approved".
- **Params:** `file_name`, `subtitle`, `share`, `versions` (pills), `current` (counted from 1, 0 is the last), `notes`, `commenter`, `timestamp`, `comment`, `approve`, `changes`, `open`.
- **Slots:** `media` (picture or video in the window). With `open: true` the window is clear instead: put the speaker's own clip on a track below the overlay at scale 0.575, position [0, -0.05], and it fills the window exactly.
- **Canvas:** 1080x1920, full screen at scale 1, position [0, 0].
- **Build:** 1.4 s (the Approve button is pressed at 2 s). **Typical length:** 3 to 5 s.

### `social-post`

- **Shows:** a generic social post: avatar, name, handle and time, the text, a picture, and like, comment and share counts; the heart fills with a burst while the counts rise.
- **Use for:** "this post went viral", "people commented...", engagement, a reaction.
- **Params:** `name`, `handle`, `time`, `text` (up to three lines), `likes`, `comments`, `shares` (counted up), `liked`, `show_image`.
- **Slots:** `avatar` (picture), `image` (picture or video).
- **Canvas:** 1080x1240. Above the head: scale 0.58, position [0, -0.29]. Top half: scale 0.77, position [0, -0.25].
- **Build:** 1.8 s. **Typical length:** 2.5 to 4 s.

### `quote-panel`

- **Shows:** a quote revealed word by word under a large quotation mark, one phrase marked in the accent colour, and the author with an avatar and role.
- **Use for:** the key sentence of the video, a rule of thumb, a testimonial, "as my mentor said...".
- **Params:** `quote`, `highlight` (words of the quote to mark), `author`, `role`.
- **Slots:** `avatar` (picture).
- **Canvas:** 1080x760. Above the head: scale 0.9, position [0, -0.29].
- **Build:** 0.4 s plus 0.07 s per word, then the mark (about 1.8 s for 13 words). **Typical length:** 2.5 to 4 s.

### `before-after`

- **Shows:** two pictures or videos in one frame, split by a handle that sweeps across to show the after side and springs back to the split, with Before and After labels.
- **Use for:** "raw versus edited", "before I fixed the audio", any transformation or improvement.
- **Params:** `before_label`, `after_label`, `split` (where the handle settles, 0.05 to 0.95).
- **Slots:** `before`, `after` (pictures or videos; empty slots show the same art dull and soft before, vivid after).
- **Canvas:** 1080x760. Above the head: scale 0.9, position [0, -0.29]. Top half: scale 1, position [0, -0.25].
- **Build:** 2.2 s. **Typical length:** 3 to 4 s.

### `reel-fan`

- **Shows:** three to five vertical video cards rising as a stack and fanning out, each with a view count that counts up.
- **Use for:** "short videos", "my last five reels", "they all hit a million".
- **Params:** `count` (3 to 5), `views` (one per card, left to right), `icon` (play, eye or heart), `spread` (degrees between cards).
- **Slots:** `reel_1` to `reel_5` (pictures or videos, left to right).
- **Canvas:** 1080x960. Above the head: scale 0.76, position [0, -0.29]. Top half: scale 1, position [0, -0.25].
- **Build:** 1.9 s. **Typical length:** 2.5 to 4 s.

### `menu-grid`

- **Shows:** tiles filling a grid one by one while a counter counts them, then one tile chosen with a ring and a check while the others dim.
- **Use for:** "pick a template", "dozens of styles", "choose the one you like".
- **Params:** `title`, `unit` (word after the count), `columns` (2 to 4), `rows` (1 to 4), `labels`, `selected` (counted from 1, 0 for none).
- **Slots:** `tile_1` to `tile_9` (pictures or videos in reading order).
- **Canvas:** 1080x1080. Above the head: scale 0.68, position [0, -0.29]. Top half: scale 0.88, position [0, -0.25].
- **Build:** 1.9 s for nine tiles. **Typical length:** 3 to 4 s.

### `storyboard-list`

- **Shows:** numbered lines, each with a 16:9 thumbnail, a quoted line and its time, sliding in one after another under a title and a count.
- **Use for:** "I plan every line first", "here is the structure", a script or an outline.
- **Params:** `title`, `unit`, `quotes` (up to 5), `times`.
- **Slots:** `thumb_1` to `thumb_5` (pictures or videos).
- **Canvas:** 1080x1180. Top half: scale 0.8, position [0, -0.25]. Above the head: scale 0.62, position [0, -0.29].
- **Build:** 0.85 s plus 0.16 s per line (1.5 s for four). **Typical length:** 3 to 5 s.

### `cross-out`

- **Shows:** red X marks (or strike lines) drawn stroke by stroke with a punch, edged in white with a shadow so they read on any picture.
- **Use for:** "stop doing this", "not this way", rejecting what another overlay or the video shows.
- **Params:** `style` (x or strike), `marks` (centres as [x, y] fractions of the canvas from its middle), `size`, `thickness`, `color`.
- **Slots:** none.
- **Canvas:** 1080x780, the same as `app-window`. To cross out another overlay, give it the same `transform` and the other overlay's `width` and `height`; on a later track. To cross out something in the video, keep scale 1 and set `marks` (with `width` 1080, `height` 1920 the marks are fractions of the frame).
- **Build:** 0.7 s for one mark, 0.16 s more per mark. **Typical length:** 1 to 2.5 s.

### `arrow-pointer`

- **Shows:** a curved arrow that draws on towards the middle of its canvas, where its tip lands, with an optional label at its tail; it then nudges at its target.
- **Use for:** "look at this", "right here", pointing at the phone in the speaker's hand or at another overlay.
- **Params:** `label`, `from` (the side it comes from), `length`, `curve`, `color`.
- **Slots:** none.
- **Canvas:** 800x800. Scale 0.5 to 0.7; position is the target's own position, because the tip sits at the canvas's middle (a target at 70% across and 40% down is position [0.2, -0.1]).
- **Build:** 1 s. **Typical length:** 1.5 to 2.5 s.

### `audio-chip`

- **Shows:** a pill with a microphone, a label, a blinking recording dot with a running timer and a live waveform.
- **Use for:** "my microphone", "I record the voice separately", sound, voice-over.
- **Params:** `label`, `status` (word by the dot), `timer`, `start_time`, `bars`, `level`.
- **Slots:** none.
- **Canvas:** 760x280. Above the head: scale 0.6, position [0, -0.33].
- **Build:** 0.6 s, then it stays live. **Typical length:** 2 to 3 s.

### `waveform-sync`

- **Shows:** two sound waves of the same recording, the lower one out of step, sliding into line; a marker flashes across both and the chip turns from the offset to In sync.
- **Use for:** "sync the microphone with the camera", "it lines the audio up automatically".
- **Params:** `title`, `top_label`, `bottom_label`, `offset_label`, `synced_label`, `offset`, `top_color`.
- **Slots:** none.
- **Canvas:** 1080x640. Above the head: scale 0.9, position [0, -0.29].
- **Build:** 2.5 s. **Typical length:** 3 to 4 s.

### `timeline-cut`

- **Shows:** a timeline whose dead parts are marked in red with scissors and cut away while the kept clips close up and the total length counts down, ending green with a done chip.
- **Use for:** "it cut 6 minutes down to 1:15", "no more dead air", silences, filler and retakes removed.
- **Params:** `label`, `from` and `to` (lengths such as "6:24" and "1:15"), `status`, `done`, `segments` (odd count of parts), `seed`.
- **Slots:** none.
- **Canvas:** 1080x620. Above the head: scale 0.9, position [0, -0.29]. Top half: scale 1, position [0, -0.25].
- **Build:** 3.1 s. **Typical length:** 3.5 to 5 s.

### `progress-card`

- **Shows:** a title and status line, a percentage counting up, a bar filling with a running sheen, and step chips ticked off one by one as the bar passes them, the current one spinning.
- **Use for:** "while it renders", "it is already halfway through", a pipeline of steps.
- **Params:** `title`, `status`, `steps` (up to 8), `progress` (where the bar stops, 0 to 1).
- **Slots:** none.
- **Canvas:** 1080x460. Above the head: scale 0.9, position [0, -0.3].
- **Build:** 2.5 s. **Typical length:** 2.5 to 4 s.

### `stat-counter`

- **Shows:** a big number counting up with a prefix and suffix in the accent colour, an accent line, a label and a trend chip.
- **Use for:** any figure the speaker says: "10x faster", "87% of viewers", "$4,200", "12 hours saved".
- **Params:** `value`, `from`, `decimals`, `prefix`, `suffix`, `kicker`, `label`, `trend` (empty hides), `trend_up`.
- **Slots:** none.
- **Canvas:** 900x560. Above the head: scale 0.75, position [0, -0.29].
- **Build:** 2.2 s. **Typical length:** 2 to 3 s.

### `bar-chart`

- **Shows:** a title and subtitle and up to twelve bars growing one after another with their values counting up, the highlighted bar in the accent colour with a badge.
- **Use for:** "views per video", growth over weeks, comparing a few numbers.
- **Params:** `title`, `subtitle`, `labels`, `values`, `highlight` (counted from 1, 0 is the largest), `badge`, `prefix`, `suffix`, `decimals`.
- **Slots:** none.
- **Canvas:** 1080x820. Above the head: scale 0.88, position [0, -0.29]. Top half: scale 1, position [0, -0.25].
- **Build:** 1.8 s for five bars. **Typical length:** 2.5 to 4 s.

### `clock-card`

- **Shows:** a clock whose hands sweep from one time to another while an accent arc fills the time that passed, the time counting under it, a label and a chip with the total.
- **Use for:** "it used to take me hours", "done in 20 minutes", a deadline.
- **Params:** `from`, `to` (clock times such as "9:00" and "11:30"), `label`, `spent`.
- **Slots:** none.
- **Canvas:** 760x900. Above the head: scale 0.57, position [0, -0.29].
- **Build:** 2.6 s. **Typical length:** 2.5 to 4 s.

### `lower-third-card`

- **Shows:** a card with a thumbnail and play button, a small tag, a title, a subtitle and a call-to-action button whose arrow nudges; it slides in from the left and leaves to the left.
- **Use for:** "full breakdown on my channel", "link in bio", naming the speaker or pointing to a longer video.
- **Params:** `tag`, `title`, `subtitle`, `cta`.
- **Slots:** `thumbnail` (picture or video).
- **Canvas:** 1080x360. Low: scale 0.92, position [0, 0.3].
- **Build:** 1 s. **Typical length:** 3 to 5 s.

### `comment-cta`

- **Shows:** a comment notification that drops in from the top: avatar, name, time and a message whose keyword sits in an accent pill that pops and pulses; it leaves upwards.
- **Use for:** the closing call to action: "comment EDIT and I'll send you the workflow".
- **Params:** `header`, `name`, `time`, `message` (with `{keyword}` where the pill goes), `keyword`.
- **Slots:** `avatar` (picture).
- **Canvas:** 1080x380. Above the head: scale 0.9, position [0, -0.3]; or low: scale 0.9, position [0, 0.3].
- **Build:** 1 s, then the keyword pulses. **Typical length:** 3 to 5 s, usually to the end of the reel with `exit: false`.

### `grid-stage`

- **Shows:** a full-screen stage: a glowing perspective grid floor (optionally a ceiling) running towards a lit horizon, with picture cards arcing over the speaker's head and swaying. It fades in from clear, so the real background shows until it has arrived.
- **Use for:** an emphatic line or a section change, swapping the background behind the speaker cut out with `key.subject`.
- **Params:** `cards` (0 to 6), `background`, `glow`, `ceiling`, `grid_speed`, `sway`, `head_y` (the head's height as a fraction of the frame from the top; the cards arc over it), `radius`.
- **Slots:** `card_1` to `card_6` (pictures or videos).
- **Canvas:** 1080x1920, full screen at scale 1, position [0, 0], on a track below the speaker's clip, with `key.subject` on that clip for the same time range.
- **Build:** 2 s, then the grid runs and the cards sway. **Typical length:** 2 to 5 s.

## Rendering

The overlays render in a headless browser in software (SwiftShader), about 45 to 100 ms per frame for a card on an M3 Pro, 130 to 220 ms for `before-after` and `reel-fan`, and about 80 ms for `review-frame` and 180 ms for `grid-stage` at 1080x1920; preview frames are cached. Frames depend only on the time, so the preview, frame grabs and the export match.
