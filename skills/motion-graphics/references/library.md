# FossCut's motion-graphics library

Everything here is drawn by FossCut and free to use, also commercially. List it with the `library` tool (action `list`, `kind` sticker, title, device, drawing, composition, particles) or `fosscut library list --kind device`; `capabilities` lists the same under `library`. Add an item with the `library` tool (action `add`, `name`, and `timeline` to put it in a scene's compound timeline) or `fosscut library add <name> --timeline <id>`.

## The grid

UI kit sizes are in grid units of 1/120 of the canvas height, the same across and down, so a part keeps its proportions on every canvas: one unit is 9 px on 1920x1080, 13 px on the 720x1560 phone screen and 16 px on 1080x1920. Change a part's size with `transform.scale`, its colour with `shape.set` (`{"fill": {"color": "#00A86B"}}`, `{"label": {"text": "Buy now"}}`, or the label's `style.color`).

The palette: ink `#151A23`, grey text `#7B8494`, lines `#DCE1E8`, page `#F4F6FA`, accent blue `#3D6BF5`, success green `#19B46B`, orange-red `#FF5A3C`, gold `#FFB72B`, dark surface `#1C2230`.

## Stickers (shape clips)

| Name | Size (units) | What it is |
|---|---|---|
| `ui-search-bar` | 48 x 7 | White pill with a light edge, a magnifier and a grey "Search" placeholder |
| `ui-caret` | 0.5 x 4 | The blinking insertion point: opacity keyframes held at 1 and 0 every half second |
| `ui-button-primary` | 24 x 7 | Accent pill, bold white "Add to cart" |
| `ui-button-secondary` | 24 x 7 | White pill with an accent edge and label |
| `ui-card` | 22 x 30 | White product card with soft corners |
| `ui-panel` | 52 x 18 | Wide white card with "Arriving today" over "Out for delivery" at its upper left |
| `ui-image-slot` | 18 x 14 | Peach gradient tile standing in for a product photo |
| `ui-price-tag` | 12 x 5 | Orange-red tag with a hole and a bold price |
| `ui-toast` | 40 x 8 | Dark pill, white message, green check mark drawn by the stroke |
| `ui-check-badge` | 5 x 5 | Green disc with a white check drawn by the stroke |
| `ui-notification` | 52 x 12 | White banner with a bold title over a grey line |
| `ui-app-icon` | 7 x 7 | Accent rounded square with a white shopping bag drawn by the stroke |
| `ui-tracker` | 44 x 3 | Four grey step dots joined by a line |
| `ui-tracker-progress` | 44 x 3 | The same steps in accent blue, up to the third |
| `ui-rating` | 16 x 3 | Five gold stars; `crop.right` 0.2 leaves four |
| `ui-map-pin` | 6 x 8 | Orange-red pin with a hole, its tip at the bottom centre |
| `ui-location-dot` | 3 x 3 | Accent dot with a white ring, for a courier riding a route |
| `swoosh-underline` | 40 x 5 | Brush-like underline, orange to yellow, round ends |
| `swoosh-circle` | 30 x 12 | Hand-drawn loop around a word |

Backgrounds: `ui-screen-light` (the page colour) and `ui-screen-dark` (for dark screens and night maps).

**Marks drawn by the stroke.** In `ui-toast`, `ui-check-badge`, `ui-app-icon` and `ui-search-bar` the filled shape comes first and the mark (check, bag, magnifier) is the last part of the outline; the shape's `trim_start` hides the stroke on the rest, and its description names that number. `animation.set` with `{"in": {"preset": "draw_on", "duration": "0.4s", "delay": "0.2s"}}` draws only the mark, after the toast has arrived.

**The tracker.** `ui-tracker-progress` ends its steps at `shape.trim_end` 0.06, 0.37, 0.69 and 1. Lay it exactly over `ui-tracker` (same position) and keyframe `shape.trim_end` from one step to the next, or draw it on with the preset.

## Title templates for app text and type

- `ui-search-text`: a query typed letter by letter, sized for the search bar; put it over the bar's placeholder (hide the placeholder with `shape.set` `{"label": null}`) and a `ui-caret` just after its last letter.
- `ui-status-text`: a bold status over a grey detail line, rising in line by line, for a `ui-panel`.
- `kinetic-type`: big bold capitals zooming in word by word, the second line orange.

Text is centred where it is placed, so line a left-aligned title up with `frame_grab` and nudge `transform.position`.

## Device frames (drawings with open screens)

| Name | Drawing | Screen (x, y, width, height in the drawing) | Screen canvas |
|---|---|---|---|
| `phone` | 412 x 832 | 26, 26, 360 x 780, radius 46 | 720x1560 |
| `tablet` | 1112 x 856 | 44, 44, 1024 x 768, radius 22 | 1440x1080 |
| `laptop` | 1600 x 864 | 176, 52, 1248 x 702, radius 8 | 1920x1080 |
| `browser` | 1296 x 824 | 8, 96, 1280 x 720, bottom radius 16 | 1920x1080 |

Adding a device imports its SVG into the bundle's `library/` folder and places it as an image clip that fits the canvas. The result's `screen` gives, for that timeline, the screen's `position` and `size` as fractions of the canvas, and the `scale` that makes a picture of the screen canvas (a compound timeline or composition of that size) fill it exactly. The screen is open, so the content goes on a track below the device, with `clip.set` `{"transform.position": position, "transform.scale": scale}`. Put both in one scene timeline and animate the scene's compound clip to move the phone with its screen. `vector.explode` turns a device into editable shapes (to recolour its frame).

## Drawings

`street-map`: a dark 1920x1080 city map on a 240-pixel street grid with blocks, a park, a river and a diagonal avenue. Streets run along x = 120, 360, 600, 840, 1080, 1320, 1560, 1800 and y = 90, 330, 570, 810, 1050 (main avenues at x = 360, 1080, 1800 and y = 330, 810). On a 1920x1080 scene timeline the map fills the canvas, so path data in those pixels follows its streets, for example `M 360 1000 L 360 810 L 840 810 L 840 330 L 1560 330 L 1560 200`.

## Web compositions

`shop-app` (720x1560, 30 fps, 9 s): a generic shop app. At 0.6 s its search bar types the first line with a caret; from about 2.3 s four product cards rise in one after another; at 4.3 s the first is tapped; at 4.6 s its page slides in with name, price, rating and a button; at 6.3 s the button is pressed; at 6.7 s a toast with a check confirms it. Its five `lines` are the query, the product's name, its price, the button and the toast. Different lines make a separate copy, so several versions can live in one project. It renders in a headless browser (Google Chrome, Chromium, Edge or Brave, or the `chrome-headless-shell` model); a clip can start part way in with `source_in`, or be sped up with `clip.speed`.
