---
name: vertical-reframe
description: Turn a 16:9 FossCut edit into a 9:16 (or 1:1) version where each shot keeps its subject in frame, without black bars, leaving the original edit untouched. Use when the user asks for a vertical, portrait, Reels, Shorts or TikTok version of a video, or to reframe or crop it for another aspect ratio.
---

# Vertical reframe

Follow the safe editing loop of the `fosscut-basics` skill (read, plan with a dry run, apply with `expected_revision` and a `request_id`, check, export). This skill adds the reframing steps.

## What a good result looks like

- A new timeline at the target size (1080x1920 for 9:16, 1080x1080 for 1:1). The original wide timeline is unchanged.
- No black bars: every picture covers the canvas.
- In every shot, the subject stays well inside the frame: a face, a speaker, the product.
- Captions sit inside the vertical safe area.

## Steps

1. **Read** the wide timeline with `timeline_read` (`detail: "full"`): clips, their `start`, `source_in` and assets.
2. **Copy onto the new canvas.** `edit` with `timeline.duplicate`, for example `{"width": 1080, "height": 1920, "fit": "fill", "name": "Vertical"}`.
   - `fill` scales video and image clips to cover the canvas, and each clip's `transform.scale` shows how much: about 3.16 for 16:9 into 9:16.
   - Note the new timeline id. Pass it as `timeline` to every later read, edit, frame and export; the original stays primary.
3. **Find the shots.** `media_analyze` with `analyzers: ["scenes"]` for each video asset gives the cuts in asset time. A cut at asset time `c` falls in a clip at `start + (c - source_in)`, when it lies inside the clip.
4. **Split at the cuts.** On the new timeline, `clip.split` each clip at its cut times, as one `edit`. Split at the latest time first: the left part keeps the clip's id, so all splits can name the same clip. Read the new timeline to get each shot's clip id.
5. **Frame each shot.**
   - Look at the original wide picture in the middle of the shot: a `frame_grab` of the wide timeline, or a `contact_sheet`.
   - Judge where the subject is horizontally, as a fraction `u` of the width (0 left edge, 0.5 centre, 1 right edge).
   - With the clip's scale `s`, the position that centres the subject is `x = -(u - 0.5) * s`, kept within `±(s - 1) / 2` so the picture still covers the canvas.
   - Apply all shots in one `edit` with `clip.set` and `{"transform.position": [x, 0]}`.
   - A subject that moves across the shot: centre the middle of its path. Two subjects that do not both fit: favour the one speaking.
6. **Captions.**
   - If the wide edit had captions, the copy has them; check their look on the new canvas and adjust it with `captions.style` and `{"placement": ...}` if needed.
   - Or caption the vertical timeline with the `viral-captions` skill.
7. **Check.**
   - `contact_sheet` of the new timeline: no bars, every subject in frame.
   - `frame_grab` in each shot that was hard to frame.
   - Show the user, and adjust positions they do not like.
8. **Export** the new timeline: `export` with its `timeline`. Report the shots and how each was framed.

## When something fails

- **Black bars:** the timeline was copied without `fill`. Set `transform.scale` on those clips with `clip.set`, or copy again with `fill`.
- **No scene cuts found:** it is one shot. Frame it once, or split it yourself where the subject moves a lot.
- `fosscut.revision.conflict`: read the new timeline again and re-plan.
