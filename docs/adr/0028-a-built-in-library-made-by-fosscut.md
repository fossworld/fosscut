# ADR-0028: Make the built-in library from FossCut's own data and code

- **Status:** Proposed
- **Date:** 2026-10-30
- **Deciders:** maintainers
- **Related:** [ADR-0004](0004-open-project-format.md), [ADR-0011](0011-effects-and-animatable-properties.md) (item 4, mask shapes), [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), [ADR-0014](0014-licensing.md), [ADR-0015](0015-clean-room-policy.md), [ADR-0027](0027-styled-text-with-project-fonts.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (stickers, shapes, templates, stock media, music, sound effects, commercial filter)

## Context

Short-form editors come with a library people reach for constantly:

- **Graphics:** shapes and stickers (badges, arrows, buttons, speech bubbles) and backgrounds.
- **Templates:** whole-project templates to fill with your own shots.
- **Sound:** sound effects (whooshes, pops, dings, risers) and music.
- **Licensing:** a switch that shows only what may be used commercially.

The research shows these as large downloadable catalogues with mixed entitlements and licences that were not audited.

FossCut cannot ship or download such catalogues: ADR-0015 forbids copying them, ADR-0014 allows only assets with known permissive licences, and fetching assets from a service would need accounts, network access and licence tracking that do not fit a local-first editor. Agents also need items they can name and reason about, not opaque packages.

## Decision

1. **Everything is made by FossCut.**
   - **Data and code:** the library consists of data files written for FossCut in `fosscut-core/presets/` (shapes, stickers, backgrounds, project templates, title templates, text and caption styles, looks) and code that synthesizes sounds (`fosscut_audio::synth`).
   - **Licence:** every item is free to use, also commercially, under the project's licence, so the commercial filter (CC-147) is always satisfied.
   - **No downloads:** nothing is downloaded.
2. **Shape clips.** A new clip kind `shape` (format 0.12.0) draws a shape on a video track:
   - **Outline:** a rectangle with rounding, an ellipse, a star, a heart, or a pen path, using the mask geometry of ADR-0011 item 4.
   - **Size:** fractions of the canvas.
   - **Paint:** a fill colour, or a linear or radial gradient to a second colour.
   - **Edge:** an inner outline.
   - **Label:** an optional text label styled like a text clip.

   `fosscut_render::shape` composes it on the CPU at the output resolution. Backgrounds are rectangles that cover the canvas; stickers are shapes with labels. They take transforms, effects, masks, keyframes and animations like any clip.
3. **Commands.**
   - **`shape.add`:** draws a shape from fields or from a library item (`item`), resized for the canvas from a 16:9 design. It puts backgrounds on a track below the footage and other shapes on a track above it.
   - **`shape.set`:** changes a shape by a JSON merge patch.
   - **`template.apply`:** builds a new timeline from a project template, with the caller's media in its slots and lines in its titles. It uses `text.template`, `transition.set` and `shape.add`, so everything it makes stays editable.
4. **Sounds become media.** A sound or music loop is rendered deterministically at 48 kHz and written once into the bundle's `library/` folder as a WAV file. It is then imported with `media.add` like any audio. A project therefore plays the same everywhere and keeps working if FossCut's synthesizer changes later. Exports note library media as free to use.
5. **One catalogue.** `fosscut_engine::library` lists every kind, with names, descriptions, tags, lengths and tempos, and searches it. `capabilities` lists the visual items and templates; `fosscut library` and the MCP `library` tool list everything and add any item with the right command.
6. **Rights notes instead of a rights check.** Exports report each used asset's origin, `library` (free to use) or `yours` (check you may publish it), and fonts you imported. FossCut has no rights database and sends nothing anywhere to check (CC-172).

## Consequences

- People and agents get a usable library without accounts, downloads or licence worries, and every item is readable data an agent can change.
- The library is smaller and plainer than a commercial catalogue: no stock footage, licensed music or photographic stickers. Growing it means writing more data and synthesis code, reviewed like code.
- Generated WAV files add a few megabytes to bundles that use music loops.
- Shape clips are a new format element that every front end must show and every planner path must handle.

## Alternatives considered

- **Download packs from an online catalogue.** Rejected: it needs a service, accounts, network access and licence auditing, against ADR-0012 and ADR-0015.
- **Ship recorded sounds and music.** Deferred: recordings need clear licences and add weight; synthesized sounds are deterministic and free of claims. Recorded packs can be added later, each with its licence record.
- **SVG stickers rendered by a vector library.** Deferred: the mask geometry already covers badges, arrows, bubbles and bursts. A vector renderer such as `resvg` can come with a later decision if richer artwork is needed.

## Validation

- **Core tests:** every built-in item validates on a default project, names are unique, and templates reference existing titles, items and transitions. Shapes draw on the right tracks, change by patches, and refuse bad input with typed errors, and templates fill slots and titles.
- **Render tests:** fills, gradients (linear and radial), outlines and labels are covered, with a golden frame of stickers and a background.
- **Sound tests:** every sound renders deterministically, without clipping, with loops that join seamlessly.
- **Before Accepted:**
  - **Library review:** a person reviews the library on vertical and horizontal canvases.
  - **Real-agent check:** a promo is built from a template with library sounds.
