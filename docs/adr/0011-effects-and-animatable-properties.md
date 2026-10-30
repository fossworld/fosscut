# ADR-0011: Built-in effects with declarative parameters and a closed property vocabulary

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0004](0004-open-project-format.md), [ADR-0008](0008-render-and-audio-engine.md), [ADR-0015](0015-clean-room-policy.md), private research: `docs/capcut-rea/RESOURCES.md`, `docs/capcut-rea/DRAFT-FORMAT.md`

## Context

In CapCut, effects, filters, transitions and text templates arrive as downloadable catalog items, so a project renders only where those items are available. Animation in professional editors works on a fixed set of animatable properties, and animated captions need per-word timing and range-based animators. For agents, the key need is that every effect has a documented type, a parameter schema with ranges and units, and a stable path for keyframes, so an agent can set and animate parameters without guessing.

## Decision

1. **Effect types are registered in code** with a manifest: `type` (namespaced, for example `color.hsl`, `mask.ellipse`, `blur.gaussian`, `transition.crossfade`), `version`, inputs (one for filters, two for transitions), parameters (name, type, default, range, unit, animatable flag, description), and WGSL passes. The registry is exported through `capabilities` so agents discover effects and their schemas at runtime.
2. **Effect instances** are stored as `{id, type, version, enabled, params, keyframes}` inside the owning clip, track or adjustment layer ([ADR-0004](0004-open-project-format.md)). Unknown types or newer versions fail validation with a clear error.
3. **Closed animatable property vocabulary.** Every animatable value has a path: `transform.position`, `transform.scale`, `transform.rotation`, `opacity`, `volume`, `effects.<id>.<param>`, `text.runs.<n>.color`, and so on. Keyframes are `{time, value, interpolation}` with interpolation `hold`, `linear`, or `bezier` with explicit control points. The list of paths is generated from the model and published with the schema.
4. **Initial built-in set** (reimplemented from public formulas): transform and crop; opacity and 11 blend modes (normal, multiply, screen, overlay, darken, lighten, color dodge, color burn, hard light, soft light, linear burn); masks (rectangle, ellipse, linear, mirror, polygon/pen, text) with feather, expansion, round corners, invert, multiple masks per clip; chroma key; color (exposure, temperature, tint, contrast, saturation, highlights, shadows, whites, blacks, curves, HSL in eight bands, color wheels, LUT `.cube` import, vignette, sharpen, grain); blur; speed ramps; transitions (crossfade, dip, push, slide, wipe, zoom); adjustment layers.
5. **Text and captions** get a dedicated engine: styled runs, per-word timing from transcripts, and animators with range selectors (by character, word, line), expressions limited to a safe declarative set, and presets. This is the backbone of "viral" captions.
6. **Third-party effects later, sandboxed:** user effect packages may contain only a manifest and WGSL (no scripts) so they are portable and safe. OpenFX plug-in hosting for native desktop plug-ins is a later, optional feature behind a feature flag.
7. **Presets are data.** Looks, text styles and animation presets are JSON documents over the same parameter vocabulary, shipped with clear licenses and provenance.

## Implementation notes

- **Text, first step** (milestone 3): text clips render as layers in `fosscut-render`. `cosmic-text` shapes and wraps the text with only the bundled Inter fonts (Regular, Bold, Italic, Bold Italic; SIL Open Font License, provenance in `crates/fosscut-render/assets/fonts/inter/README.md`), never system fonts, so text renders identically everywhere. Size is a fraction of the canvas height, lines wrap at 90% of the canvas width and align within the text block, and colors are sRGB hex decoded to linear light. Text is rasterized on the CPU at the output resolution (sharp in preview and export) and composited as a premultiplied layer with the clip's transform, opacity, mask and LUT. Placement centres the exact layout box rather than the pixel-cropped raster, so text keeps its position at every output size; styled runs come later.
- **Captions** (milestone 6): per-word timing, word animators, caption presets and safe areas are decided in [ADR-0021](0021-captions-as-timed-word-clips.md).
- **Clip properties and keyframes** (format 0.3.0, item 3):
  - **Vocabulary:** `fosscut_model::clip_properties()` is the closed list, published by `capabilities` as `properties` with each path's kind, range, choices and whether it takes keyframes. Paths: `transform.position`, `transform.scale`, `transform.stretch` (per-axis factors on top of the uniform scale), `transform.rotation`, `transform.flip_horizontal`, `transform.flip_vertical`, `crop.left`, `crop.right`, `crop.top`, `crop.bottom`, `opacity`, `blend`, `enabled`, `volume`, `audio.pan`, `audio.fade_in`, `audio.fade_out`, `audio.channels`.
  - **Storage:** a clip's `keyframes` map holds a list per animatable path, sorted by time, one keyframe per time. Times are offsets from the clip's start, so keyframes move with the clip. Splits, trims and range cuts keep every keyframe and only shift times, so the curve stays exact on both sides of a cut.
  - **Interpolation:** `hold`, `linear` (the default, omitted) or `{"bezier": [x1, y1, x2, y2]}` as in CSS. Commands also take the names `ease_in`, `ease_out` and `ease_in_out`. Values hold before the first keyframe and after the last.
  - **Commands:** `clip.set` sets plain values by path. It refuses an animated property with `fosscut.clip.property_animated`. `keyframe.set` adds or replaces a keyframe; without a value it keeps the current animated value. `keyframe.remove` removes one keyframe, or all of them (keeping the value at a given time).
  - **Crop:** removes edges without moving what remains. Edges are measured on the picture before flips.
  - **Blend modes:** the eleven of item 4 plus add, difference and exclusion. Every mode but normal uses the separable formulas of the W3C Compositing and Blending specification on sRGB-encoded values, then decodes back to linear light. Runs of normal layers still draw in one multisampled pass; a blended layer is drawn alone and combined with the canvas in a full-screen pass. Tests check every mode against the formulas, with a reviewed golden frame.
  - **Deactivated clips:** a clip with `enabled: false` keeps its place but is neither drawn nor heard.
- **Effects** (format 0.4.0, items 1, 2, 4 and 7):
  - **Registry:** `fosscut_model::effect_types()` lists each type with its version, title, category and parameters. A parameter has a kind (number, pair, bool, choice, curve or asset), a default, a range, a unit and whether it is animatable. `capabilities` publishes it as `effects`. The first types are `color.adjust`, `color.curves`, `color.hsl` (eight bands), `color.wheels` (lift, gamma, gain, offset), `color.lut`, `color.vignette`, `blur.gaussian`, `detail.sharpen` and `stylize.grain`.
  - **Instances:** a clip's `effects` list holds `{id, type, version, enabled, params, keyframes}` in the order they apply. `params` keeps only values that differ from the defaults. Unknown types, newer versions, unknown parameters and out-of-range values fail validation. Effect ids (`effect_…`) are unique in a project, so splits, cuts and copies give the copies new ids.
  - **Commands:** `effect.add`, `effect.set` (values, resets, bypass), `effect.remove`, `effect.move` and `effect.copy` (apply one clip's effects to others). Numeric parameters animate with `keyframe.set` as `effects.<id>.<param>`.
  - **Adjustment layers:** an `adjustment` clip on a video track has effects but no picture. `adjustment.add` puts it on an adjustment track above every video track, so titles and captions above stay as they are. While it lasts, its effects apply to the canvas composed below it, mixed back by its opacity.
  - **LUTs:** `.cube` files import as `lut` assets (`media import`, `media_import`) with their table size checked; `color.lut` names one, with an intensity and skin-tone protection. Exports check that the files exist before rendering.
  - **Looks** (item 7): built-in looks such as `warm`, `teal_orange`, `vintage` and `noir` are JSON documents of ordinary effects with chosen parameters, written for FossCut and published in `capabilities`. `look.apply` copies them into a clip, so a project never depends on the look afterwards.
  - **Rendering:** effects run on each layer's texture at its own size before placement, one full-screen pass per effect (`shaders/effects.wgsl`), then the result is mipmapped for compositing. Exposure, white balance and vignetting multiply linear light; tone, curves, HSL, wheels and grain work on sRGB-encoded values; LUTs on the BT.709 signal, as before. Curves interpolate with monotone cubics (Fritsch and Carlson), so they never overshoot. Blurs are separable Gaussians that sample a mip level needing at most 24 taps. Sharpening adds the difference from a blur. Grain is seeded by the frame, so a frame renders the same every time. Every effect leaves the picture unchanged at its defaults, which the tests check.
  - **Automatic color** (ADR-0012: measured locally, no model): `fosscut_engine::color` renders seven frames of a clip without effects and measures their mean color, median, 1st and 99th luma percentiles and colorfulness. `color.auto` adds a `color.adjust` first in the clip's effects: a partial gray-world white balance, median brightness towards middle gray, and levels stretched only when the picture has a real tonal range. `color.match` adds one that brings the channel balance, brightness, spread and colorfulness close to a reference clip. Both are engine operations (`fosscut color auto|match`, the MCP `color` tool, the desktop's Auto color and Match color) that end in an ordinary `effect.add`, so they are undoable.

## Consequences

- Agents can enumerate effects and parameters, and invalid values are rejected before rendering.
- Projects remain renderable on any machine, because effects are code plus parameters, not downloaded binaries.
- A large built-in set is work; it is ordered by the roadmap and by what skills need first (captions, transforms, color, transitions, speed).

## Alternatives considered

- **Downloadable vendor packages with embedded scripts:** flexible but opaque, insecure for third-party content, and ties projects to a cache.
- **Open-ended keyframable paths:** easier to extend but impossible for agents to discover or validate.
- **A general node-graph compositor exposed to users:** powerful, but a poor fit for short-form editing and for agents; internal render plans can still be graphs.

## Validation

Golden-frame tests for every effect at default and extreme parameters; a schema test proving every animatable path in the registry resolves on a sample project.
