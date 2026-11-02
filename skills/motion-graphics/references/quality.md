# Quality rules for motion-graphics ads

Rules of thumb from motion-design practice, in FossCut's units. Check every style frame and preview against them before showing it to the user.

## Timing

- **Scenes.** 1 to 3 seconds each, one idea per scene; a 12-second ad has five or six. The first frame already shows something (never start on black), and the hook lands within the first second.
- **The beat grid.** At 120 bpm a beat is 0.5 seconds (15 frames at 30 fps) and a bar 2 seconds. Scenes start on bars or beats, key landings (a card settling, a draw-on finishing, a parcel touching down) hit beats, small moves fall on half beats. A move starts 2 to 4 frames before its beat so that it lands on it.
- **Durations.** Entrances 0.3 to 0.6 seconds; exits faster, 0.2 to 0.4; staggers 0.06 to 0.12 seconds between items; camera moves span the whole scene.
- **Holds.** The hero moment of a scene holds at least 0.5 seconds after it settles. Text stays at least 0.3 seconds per word plus half a second. The end card holds its logo and call to action perfectly still for at least a second, 1.5 to 2 seconds in all.

## Easing and overshoot

- Nothing important moves at a constant speed. Entrances ease out (fast start, soft landing): `ease_out`, or the stronger Bezier `{"bezier": [0.16, 1, 0.3, 1]}`. Exits ease in: `{"bezier": [0.7, 0, 0.84, 0]}`. Moves between two rests ease in and out: `{"bezier": [0.65, 0, 0.35, 1]}`. Even a slow camera push eases in and out.
- **Overshoot** gives small things weight: a Bezier with a control point above 1, `{"bezier": [0.34, 1.56, 0.64, 1]}`, passes its target by about a tenth and settles; `pop_in` does the same for a whole clip. Use it on cards, badges, pins, buttons and icons, not on large blocks of type or on the camera.
- **Squash and settle** for drops: the object falls with `ease_in`, lands squashed (`transform.stretch` about `[1.12, 0.88]` for 2 frames), springs back past its shape (`[0.96, 1.04]`) and settles over 4 to 6 frames. The `parcel-drop-3d` template does this itself.
- **Follow-through.** Secondary parts trail the main one by 2 to 4 frames: a shadow after its object, sparks after the landing, a toast's check mark after the toast.

## Motion blur

- `motion.blur` with `amount` 0.5 (a half-frame shutter) on every clip that moves, the scene clip included when it moves; 16 to 24 `samples` for whips and fast spins. 3D layers under a moving camera streak with it too, even when they hold still.
- A blur only follows a clip's own placement and the camera: letters moving inside a text reveal stay sharp, so keep those motions short, or move the whole title.
- Check a frame in the middle of the fastest move of every scene: a soft streak along the motion, never two sharp copies.

## Type

- The brand's font (`media_import` a `.ttf` or `.otf`) or the bundled Inter. One family, two weights: bold or black for headlines, regular or medium for supporting lines.
- Sizes at 1080x1920 (`style.size` is a fraction of the canvas height): headlines 0.06 to 0.09 (115 to 170 pixels), supporting lines 0.035 to 0.045, the smallest label 0.022 (about 42 pixels); never below 0.02.
- At most six words a line, two lines a headline, about seven words on screen at once. Kinetic type shows one word or phrase per beat, with the key word in the accent colour or underlined by a swoosh.

## Palette and contrast

- One background family (a dark ink or navy gradient, or a light page), one accent, one highlight, and white or ink text. Use the brand's hex values as given: FossCut keeps hex colours at their code values in the export.
- Text against its background at least 4.5 to 1: white on `#1C2230` reads, white on a bright yellow does not. Prefer dark gradients to flat black, and avoid large pure white areas.
- Light belongs to one or two elements per scene, in the accent colour. Glows and sweeps read best on dark backgrounds; on white or very bright type a `light.sweep` needs `bloom` (0.6 to 1) or a coloured light.

## Safe areas at 1080x1920

- Text, logos and buttons stay within the middle 80% of the width (x 108 to 972) and clear of the top and bottom 15% (y 288 to 1632), where platforms put their buttons and captions. In FossCut's positions (fractions of the canvas from its centre): x from -0.4 to 0.4 and y from -0.35 to 0.35.
- Keep the call to action centred: the right edge of the lower half holds the platforms' buttons.

## Screens that read on a phone

- The `shop-app` screen is 720x1560. In the flat `phone` frame filling the canvas height it shows at about 1.15 times its size, so its 26 to 30 pixel labels are about 32 pixels on the canvas: too small to read on a phone. Push in (scene scale or camera, 1.5 to 2 times) on the part that matters at that moment: the search bar while it types, the card when it is tapped, the button when it is pressed.
- Or lift the key part out of the screen at a readable size: a `ui-toast`, `ui-notification` or `ui-panel` sticker in front of the phone, at least 0.035 of the canvas height for its words.
- Hold each state of the screen at least 0.8 seconds, and show one change at a time.
- Set the composition's `lines` (and the template's `params`) to the user's product: placeholder copy such as "Product name" must never reach a preview the user sees.

## Scene craft

- Every scene has depth: a background with a gradient, soft bokeh or a floor, a subject, and light.
- Change scale or angle from one scene to the next (wide, then close), and keep the light coming from the same side.
- Hand over between scenes: a push into the phone becomes the app, the route's pin becomes the parcel, a word's underline becomes the next line.
- The end card holds the logo, the product's name and the call to action; one light sweep crosses the logo, then everything holds.

## Checks at every preview

- Readable at phone size, inside the safe areas, in the brand's colours.
- Every move eases, no element appears or disappears without motion, and the hero moment holds.
- Motion blur on every move, glow only where light belongs, sound on the beats.
