---
name: motion-graphics
description: Make motion-graphics videos with FossCut, such as app and product ads, logo reveals, kinetic type, animated map routes and end cards: storyboard the brief into short scenes, build each scene as a compound clip from draw-on paths, light effects, text reveals, 3D layers with a camera, particles, 3D objects, device frames, the UI kit and web compositions, time it to music, check frames and export. Use when the user asks for a motion-graphics ad, an animated logo, an app or product promo, kinetic typography, a route on a map or an animated end card.
---

# Motion graphics

Follow the safe editing loop of the `fosscut-basics` skill: read, plan with a dry run, apply with `expected_revision` and a `request_id`, check with `frame_grab` and `contact_sheet`, export, report. This skill adds how to turn a brief into scenes and which of FossCut's primitives draws each idea. Worked patterns with exact arguments are in `references/patterns.md`; the UI kit, device frames and their screens are in `references/library.md`. Read them before building the scenes they cover.

## What a good result looks like

- One idea per scene, readable at a glance: scenes of 1 to 3 seconds, the whole piece usually 6 to 20 seconds, at the size the user needs (1080x1920 for vertical platforms, 1920x1080 for landscape).
- Everything moves for a reason: it enters, acts and leaves with easing, and scenes change on beats of the music.
- Text stays long enough to read (about 0.3 s per word plus half a second) and inside the safe area: the middle 80% of the width, and clear of the top and bottom 15% on vertical video.
- The frame at the key moment of every scene has been looked at, and the export report matches the request.

## 1. Storyboard the brief

Before touching the project, write the scenes as a numbered list: the idea in one sentence, the length, what moves, the primitive (table below), the words on screen, and the beat it starts on. Show it to the user when there are more than three scenes, and keep each scene to one idea. A 15-second ad for a delivery app could be: a logo whose arrow draws on and glows (2 s); a phone rising while its search bar types and product cards fly out in 3D (3.5 s); a product page with an order toast (2 s); a tilted city map where a route draws and a dot follows it to an "Arriving today" card (3 s); a box landing with a delivered notification (2 s); kinetic type with an underline swoosh (1.5 s); an end card with a light sweep (1 s).

## 2. Pick the primitive for each idea

| Idea | Primitive | How |
|---|---|---|
| Lines, logos, underlines and routes that draw themselves | Vector path with a stroke | `edit` with `shape.add` (`d` as SVG path data in canvas pixels, `stroke`, `fill` null for a line), then `animation.set` with `{"in": {"preset": "draw_on"}}` or keyframes on `shape.trim_end`; a logo file: `media_import` the `.svg`, then `vector.explode` to animate its parts |
| Something riding a line: a courier dot, a plane | Path following | `clip.follow` (`path`, `orient`), keyframes on `follow.progress` from 0 to 1 |
| Light: a halo, a sheen across a card, streaks | Light effects | `effect.add` of `light.glow`, `light.sweep` (keyframe `effects.<id>.position` from 0 to 1) or `light.streaks` |
| Type that arrives with energy | Text reveals and staggers | `text.add` with `reveal` (`unit`, `motion` such as blur, pop, zoom, drop or collapse, `overlap`, `easing`, `order`); the `kinetic-type` title template; `clips.stagger` for several clips one after another |
| Depth: cards in space, a tilted map, a turning phone | 3D layers and a camera | `clip.set` with `transform.three_d` true, `transform.position_z`, `transform.rotation_x`, `transform.rotation_y`; `camera.add`, keyframes on `camera.position_z` (push in) or `camera.orbit`, and its depth of field; 3D presets such as `{"in": {"preset": "fly_in_3d"}}` |
| Sparks, embers, dust, snow, confetti, bokeh, a burst | Particles | `particles.add` with a preset (`library` with `kind` particles lists them), bursts on beats |
| A product as an object: a box, city blocks, a podium | 3D objects | `model.add` with a preset from `capabilities` under `library.models` |
| An app: a phone, its screen, buttons, cards, toasts | Device frames, the UI kit, web compositions | `library` add `phone` (its result's `screen` says where the open screen is), `shop-app` with `lines` for a complete animated app screen, `ui-*` stickers for single parts; your own HTML page through `media_import` |
| A background | Library backgrounds | `shape.add` with an `item` such as `ui-screen-dark`, or a gradient |

Before using a command for the first time, ask `capabilities` with its name for the exact arguments.

## 3. Build each scene as a compound clip

1. **Build the scene in place** on the main timeline over its time: background at the bottom, then the subject, text, and light and particles on top. Check it with `frame_grab`.
2. **Make it one clip.** `edit` with `clip.compound` (`clips`: every clip of the scene, `name`) puts one compound clip in their place. Its result's `timeline` is the scene's own timeline: read it with `timeline_read` and its `timeline`, and add to it later with the commands' `timeline` argument or the `library` tool's `timeline`.
3. **A scene with other proportions** (a 16:9 map in a vertical ad, a native 720x1560 phone screen): `timeline.add` a work timeline of that size, build there with `timeline`, and `clip.compound` its clips there. The compound timeline it makes can be placed anywhere with `clip.add` and `compound`, fitted to the canvas like a video of that size.
4. **Move scenes as wholes.** Animate the scene clip (rise, push, 3D turn) instead of every layer inside it. A device and its screen belong in one scene: screen content below, frame above, placed with the `screen` numbers the library returns.
5. **Reuse and join.** `clip.add` with `compound` shows a scene again (the logo on the end card). Join scenes with short transitions (`transition.set`: push, zoom, blur or a dip, 6 to 12 frames) or hard cuts on a beat.

## 4. Time it to music

- Put the music in first: a `library` music loop (kind music) or the user's track through `media_import`.
- Mark its beats with `auto_edit` and action `beat_markers` (`every` 4 for bars), then start scenes on bars and key moves (a card landing, a draw-on finishing) on beats.
- Add small sounds from the `library` on the moves: a whoosh just before a scene change, a pop for each card, a ding for a toast. Keep them under the music.

## 5. Check

`frame_grab` at the key moment of every scene: when a draw-on finishes, when the text has fully arrived, mid camera move, the toast fully in. Then `contact_sheet` over the whole piece. Look for: text inside the safe area and readable over its background; nothing cut by the frame's edge; glows not clipped by a too-small shape; particles not covering words; 3D layers in the right order; screen content lined up with the device's screen. Fix and grab again before exporting.

## 6. Export and report

`export` with `expected_revision`, then `job_status` until it succeeds; compare the report with the request (size, frame rate, length). Tell the user the scenes, what each shows, what you checked, and that `undo` reverses each step.

## When something fails

| Problem | What to do |
|---|---|
| `fosscut.shape.invalid` | Path data, stroke width or trims out of range; the message names the field. Path data is in canvas pixels of the timeline it goes on. |
| A draw-on shows nothing | The shape has no stroke, or its trims already hide it; give it a `stroke` and leave its trims at 0 and 1 for the preset. |
| `fosscut.follow.invalid` | The path must be a shape clip on the same timeline that follows nothing itself. |
| `fosscut.camera.invalid` or `fosscut.particles.invalid` | A value is out of range; the message names it; `capabilities` lists the ranges. |
| `fosscut.clip.property_animated` | That setting has keyframes: change it with `keyframe.set` instead of a patch. |
| 3D layers drawn in the wrong order | Layers sort as wholes by depth; give overlapping cards different `transform.position_z`, and keep text a 2D layer above them. |
| `fosscut.web.browser_missing` | Compositions need a browser. Offer to install the `chrome-headless-shell` model with `models` (it downloads, so ask the user), or build the screen from the UI kit instead. |
| `fosscut.web.remote_reference` | The page loads something from the network; put the file in its folder. |
| `fosscut.library.unknown_item` | Check the name with `library` and action list. |
| `fosscut.revision.conflict` | Read again, re-plan, apply with the new revision. |
