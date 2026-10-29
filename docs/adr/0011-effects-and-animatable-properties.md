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

- **Text, first step** (milestone 3): text clips render as layers in `fosscut-render`. `cosmic-text` shapes and wraps the text with only the bundled Inter fonts (Regular, Bold, Italic, Bold Italic; SIL Open Font License, provenance in `crates/fosscut-render/assets/fonts/inter/README.md`), never system fonts, so text renders identically everywhere. Size is a fraction of the canvas height, lines wrap at 90% of the canvas width and align within the text block, and colors are sRGB hex decoded to linear light. Text is rasterized on the CPU at the output resolution (sharp in preview and export) and composited as a premultiplied layer with the clip's transform, opacity, mask and LUT. Placement centres the exact layout box rather than the pixel-cropped raster, so text keeps its position at every output size; styled runs, per-word timing and animators come later.

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
