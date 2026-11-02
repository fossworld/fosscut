# Worked patterns

Each pattern builds one scene. Arguments are what `edit` takes for each command (`{"command": "shape.add", "args": {...}}`); `library` calls are the MCP tool's arguments. Capitals stand for ids from earlier results. Sizes assume a 1080x1920 vertical ad and a scene starting at 0; add the scene's start to every `start` and keyframe time when it starts later. Put commands that do not need each other's ids in one `edit`, so each step is one undo.

Every scene ends the same way: `clip.compound` with all of its clips and a `name` turns them into one compound clip in their place. Its result's `compound.clip` is the scene clip to animate, and its `timeline` is the scene's own timeline, which `timeline_read`, the commands' `timeline` argument and the `library` tool's `timeline` reach later.

## Logo whose arrow draws on with a glow (2 s)

1. One `edit`:
   - `shape.add` `{"item": "ui-screen-dark", "duration": "2s"}` (the floor);
   - `shape.add` `{"d": "M 330 1260 L 510 1080 L 630 1170 L 780 990", "stroke": {"color": "#FF7A45", "width": 0.014, "cap": "round", "join": "round"}, "duration": "2s"}` gives the rising line `C`;
   - `shape.add` `{"d": "M 696 984 L 786 984 L 786 1074", "stroke": {"color": "#FF7A45", "width": 0.014, "cap": "round", "join": "round"}, "start": "0.6s", "duration": "1.4s"}` gives the head `H`;
   - `text.add` `{"text": "Your brand", "start": "0.7s", "duration": "1.3s", "style": {"size": 0.075, "bold": true}, "transform": {"position": [0, -0.08]}, "reveal": {"unit": "letter", "motion": "blur", "duration": "0.5s", "overlap": 0.7}}`;
   - `particles.add` `{"preset": "burst", "start": "0.6s", "duration": "1.4s"}` as the arrow lands (or `spark-gather` before it, gathering where the logo appears).
2. One `edit`: `animation.set` `{"clip": C, "in": {"preset": "draw_on", "duration": "0.6s"}}`, `animation.set` `{"clip": H, "in": {"preset": "draw_on", "duration": "0.2s"}}`, and `effect.add` `{"clip": C, "type": "light.glow", "params": {"color": "#FF7A45", "tint": 1, "radius": 0.05, "intensity": 1.5}}` with the same on `H`.
3. `clip.compound` with the five clips and `{"name": "Logo"}`.

**From the user's logo file:** `media_import` the `.svg`, then `vector.explode` `{"asset": A, "start": "0s", "duration": "2s"}`: one compound clip whose timeline holds a shape clip per path (`timeline_read` with its `timeline` lists them). Draw stroked parts on with the preset. A filled part keeps its fill while its outline draws, so draw an outline copy (`clips.copy` and `clips.paste`, then `shape.set` on the copy with `{"fill": null, "stroke": {"color": "#FFFFFF", "width": 0.004}}`) and fade the filled part in after it.

**Check:** `frame_grab` at 0.3 s (half drawn, glowing) and 1.8 s (whole logo, name readable).

## Phone rising with an app screen that types (3.5 s)

1. `library` `{"action": "add", "name": "shop-app", "start": "0s", "duration": "3.5s", "lines": ["running shoes", "Trail Runner", "$89", "Add to cart", "Added to cart"]}` gives the screen clip `S`. The composition runs 9 s; show a later part by trimming its start with `clip.trim`, or speed it up with `clip.speed`.
2. `library` `{"action": "add", "name": "phone", "start": "0s", "duration": "3.5s"}`: it lands on a track above `S`, and the result's `screen` has `position`, `size` and `scale`.
3. `clip.set` `{"clip": S, "set": {"transform.position": <screen.position>, "transform.scale": <screen.scale>}}`.
4. `clip.compound` with `S` and the phone, `{"name": "Phone"}`, gives the scene clip `P`; then `animation.set` `{"clip": P, "in": {"preset": "rise_in", "duration": "0.6s"}}`. For a turn in depth: `clip.set` `{"clip": P, "set": {"transform.three_d": true, "transform.rotation_y": -28}}` and `keyframe.set` on `transform.rotation_y` from -28 to 0 with `ease_out`.

**Cards flying out in 3D:** above `P`, add three `ui-card` stickers (each with a `ui-image-slot` and a `ui-price-tag` if they must read as products), then `clip.set` each `{"transform.three_d": true, "transform.position": [x, y], "transform.position_z": z}` with z from -0.05 to -0.2 so they sort in depth, and `clips.stagger` `{"clips": [...], "preset": "fly_in_3d", "step": "0.12s"}`. A `camera.add` with `keyframe.set` on `camera.position_z` from 0 to 0.15 pushes in; `camera.set` `{"depth_of_field": true, "aperture": 0.4}` softens what is far from the point of interest.

**Without a browser:** build the screen natively at the screen's proportions (a 720x1560 work timeline, as the map pattern does for 16:9): `ui-screen-light`, `ui-search-bar`, the `ui-search-text` title over its placeholder (remove the placeholder with `shape.set` `{"label": null}`), and a `ui-caret` after the text that blinks with `keyframe.set` on `opacity` (1, then 0, every 0.5 s, `hold`) and moves with the typing (keyframes on `transform.position`, checked with `frame_grab`).

## Route drawing on a tilted map, with a dot riding it (3 s)

The map is 16:9, so this scene is built on a work timeline of its own size and placed as a compound clip.

1. `timeline.add` `{"name": "Map work", "width": 1920, "height": 1080}` gives `W`.
2. `library` `{"action": "add", "name": "street-map", "timeline": W, "start": "0s", "duration": "3s"}` gives `MAP`.
3. One `edit`:
   - `shape.add` `{"timeline": W, "d": "M 360 1000 L 360 810 L 840 810 L 840 330 L 1560 330 L 1560 200", "stroke": {"color": "#4D8BFF", "width": 0.012, "cap": "round", "join": "round"}, "duration": "3s"}` gives the route `R`, along the map's streets;
   - `shape.add` `{"timeline": W, "item": "ui-location-dot", "duration": "3s"}` gives the dot `D`;
   - `shape.add` `{"timeline": W, "item": "ui-map-pin", "start": "1.7s", "duration": "1.3s", "transform": {"position": [0.3125, -0.348]}}` gives `PIN`: the route's end, (1560, 200), is at x 1560 / 1920 - 0.5 and y 200 / 1080 - 0.5, raised by half the pin's height so its tip stands on it.
4. One `edit`: `clip.follow` `{"clip": D, "path": R}`; `keyframe.set` on `R` for `shape.trim_end` with value 0 at 0.2 s and 1 at 1.8 s, and on `D` for `follow.progress` with the same times, values and `ease_in_out` easing, so the dot rides the line's tip; `effect.add` of `light.glow` (`{"color": "#4D8BFF", "tint": 1, "radius": 0.04}`) on `R` and `D`; `animation.set` `{"clip": PIN, "in": {"preset": "drop", "duration": "0.4s"}}`.
5. `clip.compound` with `MAP`, `R`, `D` and `PIN`, `{"name": "Map"}`: its result's `timeline` is the 16:9 map scene `M`.
6. On the main timeline: `clip.add` `{"compound": M, "start": <the scene's start>}` gives `C`, fitted to the canvas like a 16:9 video; `clip.set` `{"clip": C, "set": {"transform.three_d": true, "transform.rotation_x": 50, "transform.scale": 1.3}}` lays it back like a map on a table with the whole route in view (on a vertical canvas it fills a band across the middle: put `ui-screen-dark` below it); a `camera.add` over it with `camera.position_z` from 0 to 0.1 drifts in.
7. **Tracker card** in 2D above it, from 2 s: `ui-panel` at `[0, 0.33]` (its label already says "Arriving today"), `ui-tracker` and `ui-tracker-progress` both at `[0, 0.36]`; `animation.set` `{"in": {"preset": "rise_in"}}` on the panel and `{"in": {"preset": "draw_on", "duration": "0.6s", "delay": "0.3s"}}` on the progress.

**Check:** `frame_grab` at 1 s (line half drawn, dot on its tip) and 2.6 s (pin down, card readable).

## Box with a delivered notification (2 s)

`model.add` `{"preset": "shipping-box", "duration": "2s"}` (3D objects live in the timeline's 3D space; a `camera.add` with `camera.orbit` keyframes turns around it); `particles.add` `{"preset": "burst", "start": "0.4s", "duration": "1.6s"}` as it lands; at the top, `ui-notification` at `[0.03, -0.38]` with `ui-app-icon` at `[-0.33, -0.38]`, both `{"in": {"preset": "slide_down", "duration": "0.4s"}}` from 0.8 s, and a `ui-check-badge` that pops in with `{"in": {"preset": "pop_in"}}`. Compound the clips into one scene.

## Kinetic type with an underline swoosh (1.5 s)

`library` add `kinetic-type` with `lines` `["Ordered today", "Delivered today"]`; then `swoosh-underline` starting 0.6 s later under the second line (`clip.set` `{"transform.position": [0, 0.07]}`) with `{"in": {"preset": "draw_on", "duration": "0.35s"}}`. For one word per beat, make one `text.add` clip per word at the beat times and give them `clips.stagger` `{"preset": "pop_in", "step": "0.08s"}`, or a `reveal` with `{"unit": "word", "motion": "drop", "order": "center"}`.

## End card with a light sweep (1.5 to 2 s)

A background (a `library` background such as a gradient), the logo scene again (`clip.add` with the logo scene's `compound` timeline), a call to action (`text.add` `{"text": "Order today", "style": {"size": 0.08, "bold": true}}`, or a `ui-button-primary` with its label changed), then `effect.add` `{"clip": X, "type": "light.sweep", "params": {"position": 0, "angle": 20, "intensity": 1.2}}` on the logo or the button. Its result's `effect` id `E` names the sweep: `keyframe.set` on `effects.E.position` with 0 at 0.3 s and 1 at 1.1 s, `ease_in_out`. A `particles.add` with the `sparkle` preset adds a few glints. Hold the last frame at least a second.

## Joining the scenes

Place the scenes in order, each starting on a bar of the music (`auto_edit` with action `beat_markers` and `every` 4 makes markers to read from `timeline_read`). Between them, `transition.set` on the earlier scene clip with a short `push` or `zoom` (0.3 s), or cut on the beat. Then `contact_sheet` the whole ad and `frame_grab` each scene's key moment before exporting.
