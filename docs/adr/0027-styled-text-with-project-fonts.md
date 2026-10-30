# ADR-0027: Compose styled text on the CPU with bundled and project fonts only

- **Status:** Proposed
- **Date:** 2026-10-30
- **Deciders:** maintainers
- **Related:** [ADR-0004](0004-open-project-format.md), [ADR-0008](0008-render-and-audio-engine.md), [ADR-0011](0011-effects-and-animatable-properties.md) (item 5, the text engine), [ADR-0014](0014-licensing.md), [ADR-0021](0021-captions-as-timed-word-clips.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (typography, spacing, appearance, text presets, text effects and templates)

## Context

Titles are most of what short-form editors add after the cut. The research shows a text inspector with these controls:

- **Typography:** font, size, bold, italic, underline, letter case and colour.
- **Spacing and alignment:** letter and line spacing, alignment.
- **Appearance:** outline, background box, shadow and glow.
- **Reuse:** a "save preset" entry, a library of text effects (styled looks) and title templates (multi-line titles that animate in).

They were inspected, with most assets in downloadable catalogues.

FossCut's text layer had one style for the whole text, with the bundled Inter fonts only, so text renders the same on every machine (ADR-0011 item 5). Captions already compose outlines and boxes on the CPU at the output resolution (ADR-0021). People also bring their own brand fonts; installed system fonts would make a project look different on another computer, and FossCut cannot ship fonts it has no licence to.

## Decision

1. **Style fields.** A text clip's style has:
   - **Typography:** `font`, `size`, `color`, `bold`, `italic`, `underline`, `case` (`as_written`, `upper`, `lower`, `title`; the stored text keeps its letters) and `align`.
   - **Spacing:** `letter_spacing` (em) and `line_height` (multiple of the size).
   - **Appearance:** `stroke`, `box`, `shadow` (colour, offset, blur) and `glow` (colour, radius).

   Lengths are fractions of the font size, so a style looks the same at every size. Values have documented ranges; validation reports `fosscut.model.text_style`, and commands report `fosscut.text.invalid`.
2. **Spans.** `spans` restyle character ranges of the written text (colour, bold, italic, underline, font, size); later spans win where they overlap. Positions count Unicode scalar values and survive case changes, which may change the shown length.
3. **Reveals.** `reveal` shows the text one unit after another (`letter`, `word` or visual `line`) with a motion (`appear`, `fade`, `rise`) over a duration from the clip's start, and optionally leaves the same way before its end. A reveal is evaluated from the clip's time alone, like caption animators.
4. **CPU composition.** `fosscut-render` shapes text with `cosmic-text` and composes it at the output resolution, back to front: line boxes, shadow, glow, outline (exact distance transform), then fill. Each reveal look group is composed separately so units fade with their own outline and shadow. The raster's layout box keeps the text in place whatever the effects' margins. Preview and export share this path (ADR-0008).
5. **Fonts are bundled or project assets.** Text may name Inter (bundled, SIL Open Font License) or a family of a font asset of the project: a `.ttf`, `.otf` or `.ttc` file imported like media (asset kind `font`, with its families and face count recorded). Machine fonts are never used. The renderer loads a font asset's file when a text names its family, and validation refuses unknown families (`fosscut.model.unknown_font`, `fosscut.text.unknown_font`). The person who imports a font is responsible for its licence; FossCut ships only fonts allowed by ADR-0014.
6. **Presets and templates are data.** Built-in style presets and title templates are JSON documents written for FossCut in `fosscut-core`, listed by `capabilities`; using one copies its values into the clip. A project's own presets are named styles in `project.json` (`text_presets`), managed by `text.preset_save` and `text.preset_remove`.
7. **Commands.**
   - **`text.add`:** takes a preset, a style patch, spans and a reveal.
   - **`text.set`:** changes the text, preset, style patch, spans or reveal of an existing clip.
   - **`text.template`:** adds a template as one text clip whose lines are spans, placed and revealed as the template says.

## Consequences

- Rich titles stay one clip and one document entry that agents can read and patch, and render identically wherever the project and its font files go.
- Composition on the CPU costs a few milliseconds per changed title; rasters are cached by content, size and, during a reveal, the moment.
- Fonts travel with projects as assets, so relinking and missing-media handling apply to them too.
- Text effects that need per-glyph geometry (curved paths, 3D extrusion) need another decision; gradients and textures in letters are not covered yet.

## Alternatives considered

- **Use installed system fonts.** Rejected: the same project would render differently on another machine, and exports could change silently when a font is updated or missing.
- **Several text clips for multi-line titles.** Rejected: lines could not align to each other or move as one, and agents would have to keep them in sync.
- **GPU text rendering (signed distance fields).** Deferred: it would scale better for many animated glyphs, but the CPU path already matches captions and is exact at every size.

## Validation

- **Unit tests:** in `fosscut-render`, outlines, shadows, glows, boxes, spans, case, spacing, underlines, line height and reveals each change the composed pixels as expected. A renamed copy of Inter shows that a project font is used, not a fallback.
- **Golden frames:** `styled-text.png` and `bilingual-caption.png` pin the look.
- **Core tests:** cover presets, patches, fonts, saved presets and templates with typed errors.
- **CLI test:** runs an end-to-end session from styling to export.
- **Before Accepted:** a real-agent check builds a title sequence from templates and presets, and a person reviews the built-in presets and templates on vertical and horizontal canvases.
