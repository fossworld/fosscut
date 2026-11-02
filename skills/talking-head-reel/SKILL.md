---
name: talking-head-reel
description: Turn a raw talking-head video (one person speaking to camera) into a polished short-form reel with FossCut, the way top creators' editors do it. Sync a separate microphone, cut dead air and retakes, storyboard every line to a visual from FossCut's library (cards, app windows, terminals, phones, lists, counters, arrows, calls to action), add punch-ins, split screens and background swaps on key lines, one-word captions with boxed keywords, a sound effect on every pop-in, and a ducked music bed with levelled loudness. Use when the user hands over raw footage of themselves talking and asks to edit it, make it a reel, TikTok or Short, add visuals or b-roll, or make it look like a creator's edit.
---

# Talking-head reel

Follow the safe editing loop of the `fosscut-basics` skill: read, plan with a dry run, apply with `expected_revision` and a `request_id`, check, export, report. This skill turns a person talking to camera into a reel where something changes every two or three seconds, always in service of what they are saying.

Read `references/visuals.md` before the storyboard: what each of the 22 explainer overlays shows, when to use it, its parameters and slots, and where it sits on a vertical talking head.

## What a good result looks like

- Every line of the script has a reason to keep watching: a visual that shows what is being said, a punch-in on the point, a background swap, a split screen, or a keyword caption. No two neighbouring beats use the same treatment.
- The hook (first two seconds) has the strongest visual; the call to action at the end has `comment-cta` or `lower-third-card`.
- Visuals sit above the head (centre at about 21% of the height) or in the top half; the face (about 30 to 48% of the height) is never covered; captions are one word at a time at about 60% of the height, with one or two keywords per sentence boxed.
- Visuals appear on the word they illustrate and leave when the thought ends; every entrance has a short sound (pop, whoosh, click) under the voice.
- The voice is clean and at -14 LUFS; music plays under it all the way, ducked while they speak.
- The user approved the storyboard before you built it (unless they asked you to go ahead on your own).

## 1. Brief

Settle in one message, or take sensible defaults when the user is not there to answer and list them in your report:

- What the video is for and its one call to action (comment a word, follow, link in bio).
- The user's own material for the visuals: screenshots, logos, app screen recordings, photos, earlier videos. Import them with `media_import`. Without them, templates show tasteful placeholders: prefer texts that carry the meaning.
- Brand colours (the templates' `accent`), and the feel of the music.
- A separate microphone recording, if they made one.

## 2. Project, footage and sound

1. `project_new` at the footage's size and rate (vertical footage: its own size, for example 720x1280 or 1080x1920), `media_import` the footage and `edit` with `clip.add` on the main video track.
2. **Separate microphone:** `media_import` it, then `auto_edit` with action `sync_audio`, `camera` the footage's asset and `recording` the microphone's. It places the recording under every camera clip, mutes the camera and groups each pair; a `fosscut.audio.sync_uncertain` error means the two do not match well: ask the user.
3. `media_analyze` with `["transcript"]` on the asset whose sound you keep (the recording when there is one), and `job_status` while it runs. Ask before installing a speech model with `models`.

## 3. Tighten

Unless the footage is already cut, follow the `talking-head-cleanup` skill's steps 3 to 6: cut fillers, pauses over 0.6 s (keep 0.12 s each side) and abandoned retakes with one `timeline.remove_ranges`, after the user agreed to the list. Then `audio.denoise` or `audio.enhance_voice` only if the sound needs it.

## 4. Storyboard (approval)

1. Split the transcript into beats: one idea each, usually a sentence or a clause, 1.5 to 4 seconds.
2. Give every beat one treatment, from the strongest to the plainest:
   - **A visual from the catalogue** that shows the idea literally: a tool named becomes an `app-window` or `terminal`; "I record on my phone" becomes an `arrow-pointer` at the phone; a number becomes a `stat-counter` or `clock-card`; a list of steps becomes `storyboard-list` or `progress-card`; proof becomes `reel-fan`, `social-post` or `video-card`; "not this" becomes `cross-out`; a comparison becomes `before-after`.
   - **A split screen** for a longer explanation with something to look at (a timeline being cut, a list, a grid): the visual in the top half, the speaker cropped in the bottom half.
   - **A background swap** when they mention a look, a place or a trick: `key.subject` behind them (green, a colour, `grid-stage`, an image).
   - **A punch-in** on the punchline or an emphatic word.
   - **A keyword caption** alone for a short line.
3. Vary them: change something every 2 to 3 seconds, never the same treatment twice in a row, at most one visual on screen at a time (two only when one is a small chip).
4. Record it: one `edit` with a `marker.add` per beat at its first word, named `beat N: "words" -> treatment`. Show the user the table (time, words, treatment, texts on screen) and wait for approval or changes.

## 5. Build the beats

Work in order; check each with `frame_grab` at the beat's middle before going on.

- **Visuals:** `library` add the overlay with `start` at the first word it illustrates, `params` with the user's words (short: 2 to 6 words per text), `accent` and `hold` (seconds until the thought ends; raise `speed` when the phrase is shorter than the overlay's build time), and `media` for its slots. Then `clip.set` its `transform.scale` and `transform.position` from `references/visuals.md`. Overlays animate themselves in and out: do not add entrance animations or `motion.blur` to them.
- **Sound on entrances:** `library` add `pop`, `whoosh`, `click` or `swipe` at each visual's start (a `notification` or `ding` for notification cards, `typewriter` under a terminal), then turn them all down with one `audio_normalize` to -26 LUFS (`target_lufs`) so they sit under the voice.
- **Split screens:** use at least one for the longest explanation (a process, a list, a timeline, a menu), as creators do. `clip.split` the speaker's clip at the beat's edges, then on the middle part `clip.set` `crop.top` and `crop.bottom` adding up to 0.5 (for example 0.2 and 0.3, so the face stays in the kept half) and `transform.position` [0, 0.25], and put the visual above it at `transform.position` [0, -0.25] filling the top half. Check the join with `frame_grab`.
- **Never shrink the speaker** or add bars to make room for a visual: use a smaller visual, move it up, or make the beat a split screen.
- **Background swaps:** `media_analyze` with `["matte"]` on the footage once, `effect.add` `key.subject` (`background` "color" with the colour, or "none" with `grid-stage` or an image on a track below) on the speaker clip, and keyframe its `effects.<id>.mix` from 0 to 1 at the beat's start and back to 0 at its end (`keyframe.set` with `hold` easing for a hard switch). The fast person matte can leave soft edges around hands, props and cables: keep swaps short, and prefer a busy background such as `grid-stage` over flat green, which shows edges most.
- **Punch-ins:** `auto_edit` with action `punch_in`, `clip` the speaker clip and `moments` with one `{"start", "end", "scale"}` per punchline (`scale` 1.15 to 1.3, held to the end of the phrase); after `media_analyze` with `["pose"]` it centres on the face. Alternate them with wide moments; never punch in during a split screen or a background swap.

## 6. Captions

`captions` with action `generate`, `preset` "reel", `skip_fillers` true, and `emphasis` with one or two keywords per sentence (the word that carries the meaning, never "the" or "and"; give `at` when the word occurs more than once). Check a frame where a visual and a caption overlap: the caption must not cover the visual's text; move the visual up rather than the caption.

Caption pages are clips on the caption track, so one shot can differ: where the face sits low or the hands fill the middle, or on a split screen, move just those pages with one `edit` of `clip.set` `transform.position` per page (for example [0, -0.04] to sit on a split screen's dividing line). Keep the style the same everywhere.

## 7. Music and loudness

1. `audio_normalize` every speech clip to -14 LUFS.
2. `library` add kind music with `duration` the timeline's length (pick the feel from the brief: `upbeat-loop`, `chill-loop`, `corporate-loop`, `trap-loop`), then `audio_normalize` it to -20 LUFS.
3. `auto_edit` with action `duck`, `clips` the music clip, `speech` the speech clips and `depth_db` -6: it dips while they talk and rises in pauses. Level first, then duck (levelling keeps the ducking's shape either way). Creators' reels keep the bed clearly audible, about 10 to 12 dB under the voice the whole time: with fast talkers who never pause, a deeper duck only makes the music vanish.

## 8. Check and deliver

1. `contact_sheet` of 12 frames and `frame_grab` at every beat: faces uncovered, texts readable, captions not hidden, visuals on the right words.
2. `export` a preview at `resolution` "360" for the user to watch; revise what they ask.
3. Final `export` at the footage's size (or 1080x1920), 30 fps, with a `cover` at the hook, then `contact_sheet` with `out` "sheet.png". Report the beats and their treatments, the sound levels, and that `undo` reverses each step.

## When something fails

| Problem | What to do |
|---|---|
| `fosscut.web.browser_missing` | Templates need Google Chrome, Chromium, Edge or Brave installed: ask the user, or use text, shapes and the UI kit stickers instead. |
| `fosscut.audio.sync_uncertain` | The recording and the camera do not share a sound: check they are the same take; place the recording by hand only if the user confirms. |
| Key or matte errors | Run `media_analyze` with `["matte"]` on the footage first. |
| A visual covers the face | Lower its `transform.scale` or move it up; use a split screen instead. |
| An overlay ends before the thought does | Its clip lasts `hold` plus 0.4 s: add it again with a longer `hold`, or `exit` false and a `duration`. |
| Captions sit on a visual's text | Move the visual up, or give that beat a split screen. |
