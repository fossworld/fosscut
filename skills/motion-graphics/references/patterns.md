# Worked patterns

Exact arguments for each step of the process, each kind of scene and each finishing pass. Arguments are what `edit` takes for each command (`{"command": "shape.add", "args": {...}}`); `library`, `export` and `query` calls are the MCP tools' arguments. Capitals stand for ids from earlier results. Sizes assume a 1080x1920 ad; times are on the timeline, so add a scene's start to every `start` and keyframe time inside it. Put commands that do not need each other's ids in one `edit`, so each step is one undo.

## The process

**Storyboard markers.** After the music and its bars (`auto_edit` `{"action": "beat_markers", "music": M, "every": 4}`), one `edit` with a `marker.add` per scene: `{"at": "0s", "name": "scene 1: logo draws on with sparks", "color": "#FF7A45"}`, `{"at": "2s", "name": "scene 2: phone rises, the app searches"}` and so on. `timeline_read` returns them under `markers` with their times, so a scene's range is from its marker to the next.

**Style frame.** Build the scene's hero moment without motion, then `frame_grab` `{"time": "1.6s", "width": 1080}`. After approval, the motion is added to the same clips: entrances (`animation.set`), keyframes and effects.

**Scene preview.** `export` `{"output": "scene-2-v1.mp4", "range": {"start": "2s", "end": "4.5s"}, "resolution": "360", "fps": "15", "expected_revision": R, "request_id": "scene-2-v1"}`, then `job_status` `{"job": J, "wait_seconds": 60}` until `state` is `succeeded`; `report.verified.video` says 360x640. The next version gets a new name and request id (`scene-2-v2.mp4`).

**Whole preview, then final.** The same without `range`; the final export has no `resolution` or `fps` on a 1080x1920 project at 30 fps (`"resolution": "1080p"` when the project is smaller), and `"cover": "11.5s"` writes a still of the end card beside the file.

## Scenes from the 3D templates

The templates (`references/library.md`) are complete scenes rendered by Three.js: `library` `{"action": "add", "name": "route-map-3d", "start": "4.5s", "duration": "2.5s", "params": {...}}` places one as a web composition clip. Their timing parameters are in seconds of their own clock, so time their moves to the music by subtracting the clip's start from the beat: a route that must arrive on the beat at 6.5 s in a clip starting at 4.5 s needs `draw_start` plus `draw_duration` equal to 2. They bring their own bloom, depth of field and motion blur, so they need no `motion.blur` of their own unless the clip itself moves. With `{"transparent": true}` a template draws over the clips below (a brand gradient, a 2D card behind it). Text, UI cards and the user's logo usually go on top as native layers, in the scene's compound clip with the template.

## Native scenes

### Logo whose arrow draws on with a glow (2 s)

1. One `edit`:
   - `shape.add` `{"item": "ui-screen-dark", "duration": "2s"}` (the floor);
   - `shape.add` `{"d": "M 330 1260 L 510 1080 L 630 1170 L 780 990", "stroke": {"color": "#FF7A45", "width": 0.014, "cap": "round", "join": "round"}, "duration": "2s"}` gives the rising line `C`;
   - `shape.add` `{"d": "M 696 984 L 786 984 L 786 1074", "stroke": {"color": "#FF7A45", "width": 0.014, "cap": "round", "join": "round"}, "start": "0.6s", "duration": "1.4s"}` gives the head `H`;
   - `text.add` `{"text": "Your brand", "start": "0.7s", "duration": "1.3s", "style": {"size": 0.075, "bold": true}, "transform": {"position": [0, -0.08]}, "reveal": {"unit": "letter", "motion": "blur", "duration": "0.5s", "overlap": 0.7}}`;
   - `particles.add` `{"preset": "burst", "start": "0.6s", "duration": "1.4s"}` as the arrow lands (or `spark-gather` before it, gathering where the logo appears).
2. One `edit`: `animation.set` `{"clip": C, "in": {"preset": "draw_on", "duration": "0.6s"}}`, `animation.set` `{"clip": H, "in": {"preset": "draw_on", "duration": "0.2s"}}`, and `effect.add` `{"clip": C, "type": "light.glow", "params": {"color": "#FF7A45", "tint": 1, "radius": 0.05, "intensity": 1.5}}` with the same on `H`.
3. `clip.compound` with the five clips and `{"name": "Scene 1 logo"}`.

**From the user's logo file:** `media_import` the `.svg`, then `vector.explode` `{"asset": A, "start": "0s", "duration": "2s"}`: one compound clip whose timeline holds a shape clip per path. Draw stroked parts on with the preset. A filled part keeps its fill while its outline draws, so draw an outline copy (`clips.copy` and `clips.paste`, then `shape.set` on the copy with `{"fill": null, "stroke": {"color": "#FFFFFF", "width": 0.004}}`) and fade the filled part in after it. For a solid 3D logo, `logo-extrude-3d` with the same `.svg` in its `logo` slot.

**Check:** `frame_grab` at 0.3 s (half drawn, glowing) and 1.8 s (whole logo, name readable).

### Phone rising with an app screen that types (2.5 s)

First choice: `phone-3d`, whose screen shows `shop-app` live (`references/library.md`), with `lines` set to the user's product. Built from flat layers:

1. `library` `{"action": "add", "name": "shop-app", "start": "0s", "duration": "2.5s", "lines": ["desk lamp", "Arc Desk Lamp", "$59", "Add to cart", "Added to cart"]}` gives the screen clip `S`. The composition runs 9 s; show a later part by trimming its start with `clip.trim`, or speed it up with `clip.speed`.
2. `library` `{"action": "add", "name": "phone", "start": "0s", "duration": "2.5s"}`: it lands on a track above `S`, and the result's `screen` has `position`, `size` and `scale`.
3. `clip.set` `{"clip": S, "set": {"transform.position": <screen.position>, "transform.scale": <screen.scale>}}`.
4. `clip.compound` with `S` and the phone, `{"name": "Scene 2 phone"}`, gives the scene clip `P`; then `animation.set` `{"clip": P, "in": {"preset": "rise_in", "duration": "0.6s"}}`. For a turn in depth: `clip.set` `{"clip": P, "set": {"transform.three_d": true, "transform.rotation_y": -28}}` and `keyframe.set` on `transform.rotation_y` from -28 to 0 with `ease_out`. Push in on the search bar while it types: keyframes on `transform.scale` (1 to 1.6) and `transform.position` (towards the bar), so its words read on a phone.

**Cards flying out in 3D:** above `P`, add three `ui-card` stickers (each with a `ui-image-slot` and a `ui-price-tag` if they must read as products), then `clip.set` each `{"transform.three_d": true, "transform.position": [x, y], "transform.position_z": z}` with z from -0.05 to -0.2 so they sort in depth, and `clips.stagger` `{"clips": [...], "preset": "fly_in_3d", "step": "0.12s"}`. A `camera.add` with `keyframe.set` on `camera.position_z` from 0 to 0.15 pushes in; `camera.set` `{"depth_of_field": true, "aperture": 0.4}` softens what is far from the point of interest.

**Without a browser:** build the screen natively at the screen's proportions (a 720x1560 work timeline, as the map pattern does for 16:9): `ui-screen-light`, `ui-search-bar`, the `ui-search-text` title over its placeholder (remove the placeholder with `shape.set` `{"label": null}`), and a `ui-caret` after the text that blinks with `keyframe.set` on `opacity` (1, then 0, every 0.5 s, `hold`) and moves with the typing.

### Route drawing on a tilted map, with a dot riding it (2.5 s)

First choice: `route-map-3d` with the route, colours and timing in `params`. Built from flat layers, on a work timeline of the map's own 16:9 size:

1. `timeline.add` `{"name": "Map work", "width": 1920, "height": 1080}` gives `W`.
2. `library` `{"action": "add", "name": "street-map", "timeline": W, "start": "0s", "duration": "2.5s"}` gives `MAP`.
3. One `edit`:
   - `shape.add` `{"timeline": W, "d": "M 360 1000 L 360 810 L 840 810 L 840 330 L 1560 330 L 1560 200", "stroke": {"color": "#4D8BFF", "width": 0.012, "cap": "round", "join": "round"}, "duration": "2.5s"}` gives the route `R`, along the map's streets;
   - `shape.add` `{"timeline": W, "item": "ui-location-dot", "duration": "2.5s"}` gives the dot `D`;
   - `shape.add` `{"timeline": W, "item": "ui-map-pin", "start": "1.7s", "duration": "0.8s", "transform": {"position": [0.3125, -0.348]}}` gives `PIN`: the route's end, (1560, 200), is at x 1560 / 1920 - 0.5 and y 200 / 1080 - 0.5, raised by half the pin's height so its tip stands on it.
4. One `edit`: `clip.follow` `{"clip": D, "path": R}`; `keyframe.set` on `R` for `shape.trim_end` with value 0 at 0.2 s and 1 at 1.6 s, and on `D` for `follow.progress` with the same times, values and `ease_in_out` easing, so the dot rides the line's tip; `effect.add` of `light.glow` (`{"color": "#4D8BFF", "tint": 1, "radius": 0.04}`) on `R` and `D`; `animation.set` `{"clip": PIN, "in": {"preset": "drop", "duration": "0.4s"}}`.
5. `clip.compound` with `MAP`, `R`, `D` and `PIN`, `{"name": "Map"}`: its result's `timeline` is the 16:9 map scene `M`.
6. On the main timeline: `clip.add` `{"compound": M, "start": <the scene's start>}` gives `C`; `clip.set` `{"clip": C, "set": {"transform.three_d": true, "transform.rotation_x": 50, "transform.scale": 1.3}}` lays it back like a map on a table (put `ui-screen-dark` below it); a `camera.add` over it with `camera.position_z` from 0 to 0.1 drifts in, and `motion.blur` on `C` streaks it with the camera.

**Tracker card** in 2D above either version, from the route's arrival: `ui-panel` at `[0, 0.28]` (its label already says "Arriving today"), `ui-tracker` and `ui-tracker-progress` both at `[0, 0.31]`; `animation.set` `{"in": {"preset": "rise_in"}}` on the panel and `{"in": {"preset": "draw_on", "duration": "0.6s", "delay": "0.3s"}}` on the progress.

**Check:** `frame_grab` mid-route (line half drawn, dot on its tip) and at the end (pin down, card readable).

### Box with a delivered notification (2 s)

First choice: `parcel-drop-3d` with the user's logo in its `logo` slot and `drop_at` and `fall` timed so it lands on a beat. Native: `model.add` `{"preset": "shipping-box", "duration": "2s"}` with a `camera.add` whose `camera.orbit` keyframes turn around it. Either way: `particles.add` `{"preset": "burst", "start": "0.4s", "duration": "1.6s"}` as it lands; at the top, `ui-notification` at `[0.03, -0.3]` with `ui-app-icon` at `[-0.33, -0.3]`, both `{"in": {"preset": "slide_down", "duration": "0.4s"}}` a beat after the landing, and a `ui-check-badge` that pops in with `{"in": {"preset": "pop_in"}}`. Compound the clips into one scene.

### Kinetic type with an underline swoosh (1.5 s)

`library` add `kinetic-type` with `lines` `["Ordered today", "Delivered today"]`; then `swoosh-underline` starting 0.6 s later under the second line (`clip.set` `{"transform.position": [0, 0.07]}`) with `{"in": {"preset": "draw_on", "duration": "0.35s"}}`. For one word per beat, make one `text.add` clip per word at the beat times and give them `clips.stagger` `{"preset": "pop_in", "step": "0.08s"}`, or a `reveal` with `{"unit": "word", "motion": "drop", "order": "center"}`.

### End card with a light sweep (1.5 to 2 s)

A background (a `library` background such as a gradient), the logo scene again (`clip.add` with the logo scene's `compound` timeline), a call to action (`text.add` `{"text": "Order today", "style": {"size": 0.08, "bold": true}}`, or a `ui-button-primary` with its label changed), then `effect.add` `{"clip": X, "type": "light.sweep", "params": {"position": 0, "angle": 20, "intensity": 1.2, "bloom": 0.8}}` on the logo or the title: `bloom` lets the sheen glow past white letters, which the band alone cannot brighten. Its result's `effect` id `E` names the sweep: `keyframe.set` on `effects.E.position` with 0 at 0.3 s and 1 at 1.1 s, `ease_in_out`. A `particles.add` with the `sparkle` preset adds a few glints. Hold the last second still.

## Finishing passes

**Joins.** A compound scene clip may land on a track of its own, so first put the later scene on the earlier one's track: `clip.move` `{"clip": <the later scene>, "track": <the earlier scene's track>, "start": <its start>}`; then, in the same `edit`, `transition.set` `{"clip": <the earlier scene>, "type": "push", "duration": "0.4s", "params": {"direction": "up"}}` (or `zoom`, `blur`), centred on the bar where they meet. Or cut exactly on a beat.

**Glow pass of a scene.** `query` `{"name": "clips.copy", "args": {"clips": [SCENE]}}` returns `clips`; one `edit` with `track.add` `{"kind": "video"}` gives the track `T`; then `clips.paste` `{"clips": <the copied clips>, "at": <the scene's start>, "track": T}` gives the copy `G`, and one `edit` with `clip.set` `{"clip": G, "set": {"blend": "add"}}` and `effect.add` `{"clip": G, "type": "light.glow", "params": {"glow_only": true, "threshold": 0.5, "radius": 0.05, "intensity": 1.2, "tint": 0.6, "color": "#FF7A45"}}`. Only the bright parts glow, in their own pass; `keyframe.set` on `G`'s `opacity` (0.6, then 1 on a hit, back to 0.6) pulses it, and removing `G` removes the glow alone.

**Motion blur.** `effect.add` `{"clip": X, "type": "motion.blur", "params": {"amount": 0.5}}` on every clip that moves, and on the scene clip when it moves as a whole or is a 3D layer under a moving camera.

**Grade, grain and shake.** One `edit`: `adjustment.add` `{"start": "0s", "duration": <up to the end card>, "effects": [{"type": "stylize.grain", "params": {"amount": 0.1}}, {"type": "motion.shake", "params": {"amount": 0.004, "speed": 1}}]}` gives `ADJ`; then `look.apply` `{"clip": ADJ, "look": "vivid"}` (or `cool`, `warm`, `teal_orange`; or `effect.add` of `color.lut` with the user's `.cube` asset). Keep looks gentle: brand colours must still match their hex values after the grade.

**Sound design.** `library` `{"action": "add", "name": "whoosh", "start": <3 frames before a transition>}`; `impact` or `drum-hit` on hero landings, `pop` or `click` on taps, `ding` or `notification` on toasts, `riser` over the last second before the end card. Each sound lands on its own audio track; `clip.set` `{"volume": 0.7}` keeps it under the music, and `audio_normalize` with the music and the sounds evens the mix at -14 LUFS.
