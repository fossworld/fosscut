---
name: motion-graphics
description: Make motion-graphics ads and promos with FossCut the way a motion designer works with a client: agree a storyboard of scenes, show a style frame of each scene, build and preview the scenes one by one for approval, then finish them (glow passes, motion blur, grade, grain, camera shake, transitions and sound on the beats) and export at 1080x1920. Use when the user asks for a motion-graphics ad, an app or product promo, an animated logo, kinetic typography, a route on a map or an animated end card.
---

# Motion graphics

Follow the safe editing loop of the `fosscut-basics` skill: read, plan with a dry run, apply with `expected_revision` and a `request_id`, check, export, report. This skill adds a director's process with four approvals by the user: the storyboard, a style frame per scene, a preview per scene, and the finished cut. Never build a whole ad in one run: what nobody approved usually has to be rebuilt.

Read `references/quality.md` before the storyboard (timing, easing, type, colour, safe areas, readable phone screens). For an app, delivery or product ad, `references/reference-ad.md` is a worked plan of a 12-second ad to adapt. `references/patterns.md` has exact arguments for each kind of scene and for the finishing passes, and `references/library.md` lists the 3D templates, the UI kit, the device frames and the app screen.

## What a good result looks like

- The user approved the storyboard, every style frame and every scene preview before the final export.
- One idea per scene, 1 to 3 seconds each, scenes changing on bars of the music; every move eases, and the hero moment of each scene holds long enough to read.
- Everything that moves has motion blur; light has its own glow pass; the whole piece shares one light grade, a little grain and a little camera shake; sound hits the beats.
- Text and UI read on a phone: inside the safe area, large enough, with strong contrast.
- The export report shows 1080x1920 at 30 fps for the agreed length, and the user has the file and a contact sheet.

## 1. Brief and storyboard (approval 1)

1. **Brief.** Settle the product, the one message, the call to action, the length (6 to 20 seconds), the size (1080x1920 unless asked otherwise), the brand's colours and fonts, the logo (an `.svg` imports as shapes), and the feel of the music. Ask for what is missing in one message.
2. **Project and music.** `project_new` at 1080x1920. Put the music in first (`library` with kind music, or the user's track through `media_import`), then `auto_edit` with action `beat_markers` and `every` 4: scenes start on these bars, key moves land on beats.
3. **Storyboard.** Write the scenes as a table: number, intent (what the viewer must get, in one sentence), start and length, the beat it starts on, the hero moment (the one frame that sells it), what moves, the primitive (section 3), and the words on screen.
4. **Record it.** One `edit` with a `marker.add` per scene at its start, named `scene N: intent`, for example `{"at": "2s", "name": "scene 2: phone rises, app search types", "color": "#3D6BF5"}`. `timeline_read` lists them under `markers`.
5. **Show the storyboard and wait.** Change it until the user approves. Do not build before.

## 2. Style frames (approval 2)

For each scene, build only its hero moment at its place on the timeline: background, colours, type, the main object, already at the final look and position, without motion. `frame_grab` at that time with `width` 1080 (full size) and send it to the user with the scene's name. Ask for approval or comments, revise and grab again. Settle palette, type scale, contrast and framing here: they are cheap to change now and expensive after the motion is built.

## 3. Build each scene (approval 3, per scene)

Build one scene at a time, in storyboard order, from its approved style frame:

1. **Pick the primitive for each idea.** Before using a command the first time, ask `capabilities` for its arguments.

   | Idea | Primitive |
   |---|---|
   | A 3D city with a route, a parcel dropping, a phone turning with a live app, a logo extruded in 3D, a product on a podium | The 3D templates (`library` add `route-map-3d`, `parcel-drop-3d`, `phone-3d`, `logo-extrude-3d`, `product-podium-3d` with `params` and `media`): FossCut's own Three.js scenes with bloom, depth of field and motion blur; `references/library.md` lists their parameters |
   | An app screen: search, cards, product page, button, toast | `shop-app` with `lines` (its defaults are placeholders: always set them), inside `phone-3d` or the flat `phone` frame; push in on the part that matters, or lift it out at a readable size with the UI kit (`ui-*` stickers) |
   | Lines, underlines, routes and logos that draw themselves | `shape.add` paths with a `stroke`, `animation.set` with `{"in": {"preset": "draw_on"}}`; `vector.explode` for the user's SVG logo |
   | Type with energy | `text.add` with a `reveal`, the `kinetic-type` title, `clips.stagger` |
   | Halos, sheens, streaks, sparks | `light.glow`, `light.sweep` (with `bloom` on white type), `light.streaks`; `particles.add` with a burst on the beat |
   | Depth and camera moves over flat layers | `transform.three_d`, `camera.add` with keyframes on `camera.position_z` or `camera.orbit` |

2. **Build in place, then make it one clip.** Lay the scene's layers over its time on the main timeline (background at the bottom, light and particles on top), then `edit` with `clip.compound` (`clips`, and `name` "Scene N"). Animate the scene clip as a whole for a push, a rise or a 3D turn.
3. **Motion.** Entrances overshoot a little and settle, the hero moment holds, and the scene hands over to the next on its last beat (`references/quality.md`). Add `motion.blur` (`{"amount": 0.5}`) to every clip that moves, inside the scene and on the scene clip when it moves; 3D layers under a moving camera streak with it too.
4. **Preview.** `export` the scene alone: `range` from its marker to the next, `resolution` "360" and `fps` "15", a versioned file name such as `scene-2-v1.mp4`, then `job_status` until it succeeds. Give the user the file and a `frame_grab` of the hero moment, wait for approval, revise, and export the next version (`scene-2-v2.mp4`).

## 4. Finish the whole piece (approval 4)

In this order, then preview the whole piece at `resolution` "360" for the last approval (`references/patterns.md` has the arguments):

1. **Joins.** `transition.set` of 0.3 to 0.6 seconds centred on the bar where scenes meet (push, zoom or blur), or a hard cut exactly on a beat. A transition joins clips that touch on one track: `clip.move` a scene clip onto the previous scene's track first.
2. **Glow passes.** For scenes with light (lines, routes, logo strokes, sparks, neon type): copy the scene clip onto a new track above it at the same time (`query` with `clips.copy`, `track.add`, `clips.paste`), then give the copy `{"blend": "add"}` and a `light.glow` with `{"glow_only": true, "threshold": 0.5}`. Its `opacity` keyframes make the light pulse on hits.
3. **Motion blur check.** `frame_grab` in the middle of the fastest moves: a soft streak, never a sharp double image.
4. **Grade, grain and shake.** One `adjustment.add` over the whole piece with `look.apply` of a gentle look (or the user's LUT with `color.lut`), `stylize.grain` (amount 0.08 to 0.15) and `motion.shake` (amount 0.003 to 0.006, speed 1); end it before an end card that must stay still.
5. **Sound design on the beats.** The music bed, a `whoosh` 2 to 4 frames before each transition, an `impact` or `drum-hit` on each hero landing, `pop` or `click` on taps, `ding` or `notification` on toasts, a `riser` into the end card; sounds under the music, then `audio_normalize` to -14 LUFS.

## 5. Final export and delivery

`export` at 1080x1920 and 30 fps (`resolution` "1080p" when the project is smaller) with `expected_revision` and a `cover` on the end card, `job_status` until it succeeds, and compare the report with the brief. Send the file, a `contact_sheet` of 12 frames, the scenes with what changed at each approval, and a note that `undo` reverses each step.

## When the user is not there to approve

If the user asked you to go ahead without stopping, still make every step's result (markers, style frames, previews) and judge each against `references/quality.md` before moving on; list them in the report so the user can review them afterwards.

## When something fails

| Problem | What to do |
|---|---|
| `fosscut.web.browser_missing` | The 3D templates and `shop-app` need Google Chrome, Chromium, Microsoft Edge or Brave installed. Ask the user to install one, or build the scene from native layers (`references/patterns.md`). |
| A 3D template renders slowly | Preview at `resolution` "360" and `fps` "15"; it renders full size only for the final export. |
| A 3D template cannot render on this machine | The templates need the graphics card renderer (macOS for now): build the scene from native layers and 3D layers instead. |
| `fosscut.web.remote_reference` | A page loads something from the network; put the file in its folder. |
| `fosscut.shape.invalid` | Path data, stroke width or trims out of range; path data is in canvas pixels of the timeline it goes on. |
| A draw-on shows nothing | The shape has no `stroke`, or its trims already hide it. |
| `fosscut.clip.property_animated` | That setting has keyframes: change it with `keyframe.set`. |
| 3D layers drawn in the wrong order | Layers sort as wholes by depth: give overlapping cards different `transform.position_z`, and keep text a 2D layer above them. |
| A sweep does not show on white type | Give the `light.sweep` a `bloom` of 0.6 to 1, or a coloured light. |
| `fosscut.library.unknown_item` | Check the name with `library` and action list. |
| `fosscut.revision.conflict` | Read again, re-plan, apply with the new revision. |
