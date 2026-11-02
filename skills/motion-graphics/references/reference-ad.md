# A worked plan: a 12-second delivery app ad

The plan of a vertical spec ad in the class motion designers make, built from FossCut's 3D templates and native layers. Adapt the scenes, words and colours to the brief; keep the structure: six scenes on a 120 bpm grid, each approved as a style frame and a preview before the finish.

**Project.** `project_new` at 1080x1920, 30 fps. Music: `library` add `upbeat-loop` (120 bpm) from 0 s for 12 s, `audio.fade_out` of 1 s by `clip.set`; `auto_edit` `beat_markers` with `every` 4 puts the bars at 0, 2, 4, 6, 8 and 10 s. A beat is 0.5 s. Palette: ink `#0B0E14` to navy `#1C2230` backgrounds, accent orange `#FF7A2F`, highlight blue `#5B8CFF`, white type; the user's logo as an `.svg`.

## Storyboard

| Scene | Time | Intent | Hero moment | Primitives |
|---|---|---|---|---|
| 1 | 0 to 2 s | The brand arrives with energy | 1.5 s: logo complete, glowing, name in | Native strokes drawn on, `light.glow`, sparks, name blurring in |
| 2 | 2 to 4.5 s | Ordering is easy | 3.6 s: phone facing us, search typed, cards rising | `phone-3d` with `shop-app` on its screen, a large search bar lifted out of it |
| 3 | 4.5 to 7 s | You can follow it across the city | 6.6 s: route arrived, pin down, "Arriving today" card | `route-map-3d`, `ui-panel` with the tracker |
| 4 | 7 to 9 s | It arrives, safely | 8.4 s: parcel settled, "Delivered" notification in | `parcel-drop-3d` with the logo, `ui-notification`, `ui-check-badge` |
| 5 | 9 to 10.5 s | The promise in words | 10.2 s: both lines in, underline drawn | `text.add` words on beats, `swoosh-underline` |
| 6 | 10.5 to 12 s | Act now | 11.5 s: logo, name and button still, sweep passed | Logo again, `ui-button-primary`, `light.sweep` with `bloom`, `sparkle` |

Markers: `scene 1: the brand arrives` at 0 s, `scene 2: ordering is easy` at 2 s, `scene 3: follow it across the city` at 4.5 s, `scene 4: delivered safely` at 7 s, `scene 5: the promise in words` at 9 s, `scene 6: act now` at 10.5 s.

## Scene 1: logo draw-on with glow and sparks (0 to 2 s)

- **Style frame at 1.5 s:** a `night` background (or `ui-screen-dark`), the logo's strokes complete in orange with `light.glow` (`{"color": "#FF7A2F", "tint": 1, "radius": 0.05, "intensity": 1.5}`), the brand name in white bold at 0.075 under it.
- **Motion:** the strokes draw on from 0.2 to 0.9 s (`draw_on`, eased out), the arrow head snaps on in 0.15 s to land on the beat at 1 s; `particles.add` `spark-gather` from 0.2 s converging on the logo, then `burst` at 1 s; the name blurs in (`blur_in`, 0.4 s) from 1 s. The scene clip pushes in slowly (`transform.scale` 1 to 1.06, eased in and out) with `motion.blur`.
- **Finish:** a glow pass (the scene copied with `light.glow` `glow_only` on `add` blend) whose `opacity` peaks at 1 s. Sound: `riser` from 0 to 1 s, `impact` at 1 s.
- **Out:** `transition.set` `zoom` of 0.4 s centred at 2 s; `whoosh` at 1.85 s.

Alternative: `logo-extrude-3d` with the logo in its `logo` slot, `side_color` orange and `in_at` 0.1, for a solid 3D logo.

## Scene 2: phone with the app (2 to 4.5 s)

- **Screen:** `shop-app` on a work timeline with the user's `lines`, for example `["desk lamp", "Arc Desk Lamp", "$59", "Add to cart", "Added to cart"]`, then `phone-3d` from 2 s for 2.5 s with `{"screen": <the shop-app asset>, "screen_start": 0.3, "rise_duration": 0.8, "turn_at": 0.2, "turn_duration": 1.1, "push_at": 0.9, "push_duration": 1.6, "glow_color": "#5B8CFF", "background": "#0B0E14", "duration": 2.5}`.
- **Readable search:** the phone's screen alone is too small to read, so a `ui-search-bar` at 0.9 scale rises out in front of the phone at 2.5 s (`rise_in`, `{"bezier": [0.34, 1.56, 0.64, 1]}` on its scale for the overshoot), with `ui-search-text` typing the same query and a blinking `ui-caret`; it leaves upward at 3.9 s as the cards rise on the screen.
- **Style frame at 3.6 s**; preview `range` 2 to 4.5 s.
- **Sound:** a `swipe` as the phone rises and one `pop` when the search bar lands (a click per letter is too busy).
- **Out:** `push` up, 0.4 s, centred at 4.5 s.

## Scene 3: tilted map with a route (4.5 to 7 s)

- `route-map-3d` from 4.5 s for 2.5 s: `{"route": [[-4, -6], [-4, -2], [1, -2], [1, 2], [4, 2], [4, 6]], "route_color": "#FF7A2F", "pin_color": "#FF7A2F", "draw_start": 0.3, "draw_duration": 1.7, "pin_delay": 0.1, "duration": 2.5}`: the route arrives on the beat at 6.5 s and the pin drops just after.
- **Card:** `ui-panel` at `[0, 0.28]` ("Arriving today") rising in at 6 s, `ui-tracker` and `ui-tracker-progress` at `[0, 0.31]`, the progress drawn to its third step (`shape.trim_end` 0.69) by 6.5 s; `motion.blur` on the card.
- **Style frame at 6.6 s.** Sound: `swipe` at 4.5 s, `notification` at 6.5 s.
- **Out:** `zoom` into the pin, 0.4 s, centred at 7 s.

## Scene 4: parcel drop with a delivered notification (7 to 9 s)

- `parcel-drop-3d` from 7 s for 2 s with the logo in its slot: `{"logo": <the logo asset>, "box_color": "#F07A2B", "drop_at": 0.15, "fall": 0.35, "duration": 2}`: it lands on the beat at 7.5 s with its own sparks and shake.
- **Notification:** `ui-notification` at `[0.03, -0.3]` with `ui-app-icon` at `[-0.33, -0.3]` sliding down at 8 s (`slide_down`, 0.35 s), its title "Delivered" and line "Left at your door"; `ui-check-badge` popping in (`pop_in`) at 8.25 s.
- **Style frame at 8.4 s.** Sound: `impact` at 7.5 s, `notification` at 8 s.
- **Out:** a hard cut on the beat at 9 s with a `whoosh` at 8.9 s.

## Scene 5: kinetic type with an underline (9 to 10.5 s)

- On a navy gradient: "Ordered today." at 9 s and "Delivered today." at 9.5 s, one `text.add` per line at 0.085, bold, white, "today" in the accent through `spans`; each pops in with `pop_in` (0.3 s) on its beat, `motion.blur` on both.
- `swoosh-underline` under the second "today", drawn on at 10 s in 0.3 s.
- **Style frame at 10.2 s.** Sound: `drum-hit` at 9 and 9.5 s, `swipe` at 10 s.
- **Out:** `blur`, 0.3 s, centred at 10.5 s; `riser` from 9.5 to 10.5 s.

## Scene 6: end card with a light sweep (10.5 to 12 s)

- The logo scene again (`clip.add` with scene 1's `compound` timeline, `source_in` at its hero moment), the brand name, and a `ui-button-primary` at `[0, 0.25]` labelled "Get the app" popping in at 10.75 s.
- `light.sweep` on the logo and the name with `{"bloom": 0.8, "width": 0.25}`, its position keyed from 0 at 10.8 s to 1 at 11.4 s (`ease_in_out`); a few `sparkle` particles; everything holds still from 11.4 s.
- **Style frame at 11.5 s.** Sound: `chime` at 10.8 s; the music fades out.

## Finish and deliver

1. `motion.blur` on every clip that moves (`frame_grab` mid-move in each scene).
2. Glow passes on scenes 1 and 6.
3. One `adjustment.add` from 0 to 10.5 s with `stylize.grain` 0.1 and `motion.shake` 0.004, and `look.apply` `cool` on it; the end card stays clean and still.
4. `audio_normalize` with the music and all the sounds to -14 LUFS.
5. Whole preview at `resolution` "360" for the last approval, then the final `export` at 1080x1920 with `"cover": "11.5s"`, and a `contact_sheet` of 12 frames saved with `"out": "sheet.png"` for the user.
