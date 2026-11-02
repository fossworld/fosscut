# ADR-0041: Keep graphics colours at their code values

- **Status:** Proposed
- **Date:** 2026-11-02
- **Deciders:** maintainers
- **Related:** [ADR-0008](0008-render-and-audio-engine.md), [ADR-0009](0009-media-io-and-codecs.md), [ADR-0011](0011-effects-and-animatable-properties.md), [ADR-0027](0027-styled-text-with-project-fonts.md), [ADR-0036](0036-draw-vector-paths-with-strokes-trims-and-following.md), [ADR-0038](0038-emit-particles-deterministically.md), [ADR-0039](0039-render-web-compositions-in-a-headless-browser.md), [ADR-0040](0040-render-3d-objects-through-the-camera.md)

## Context

FossCut composites in linear Rec.709 light and encodes every output it writes today (exports, frame grabs, contact sheets, the preview) with the BT.709 transfer function (ADR-0008, item 4). Sources are decoded with the inverse of their tagged transfer, so footage tagged BT.709, and untagged pictures, which default to BT.709, come out with the code values they went in with.

Colours written as hex did not. Text, shapes, strokes, captions, particles, 3D materials and light tints decoded `#RRGGBB` with the sRGB curve, as a web page would, and the export then encoded the result with BT.709. The two curves differ most in the shadows: `#2E3B57` (46, 59, 87), a typical dark brand colour, came out as (30, 43, 72), and `#808080` as 115. Web composition frames were tagged sRGB on purpose to match those hex colours (ADR-0039, item 3), and so were PNGs that carry an `sRGB` chunk, while the same PNG without the chunk kept its values. An agent following a brand guide, or comparing a frame grab with a design, saw every dark colour shift.

Designers, brand guides, CSS and design tools give colours as code values, and players show video code values on the display much as browsers show a page, without converting BT.709 into sRGB. People expect a colour they typed to be the colour in the file. Camera footage is different: its transfer says how scene light was encoded, and compositing it in linear light needs that transfer's inverse.

## Decision

1. **Graphics are display-referred.** These are decoded to linear light with the inverse of the output transfer function, so their code values come out of an export unchanged (within rounding), while blending, opacity, masks and effects still work in linear light:
   - every colour given as hex: text and its outline, box, shadow and glow; caption styles and animators; shape fills, gradients, outlines and strokes; canvas fills; particle colours; 3D object materials, emissive colours and lights; light effect tints; the `dip` transition's colour; `key.subject` fills; the `key.chroma` key colour;
   - SVG drawings, and the pictures inside 3D model files;
   - web composition frames, and any source tagged with the sRGB transfer (PNGs with an `sRGB` chunk, screen recordings).
2. **One helper.** `fosscut_render::color` holds `GRAPHICS_TRANSFER` (BT.709, since every output is SDR BT.709 today), `graphics_to_linear`, `graphics_from_linear` and `hex_to_linear`. No renderer decodes a colour another way. Shaders receive colours as linear light; an effect that works on sRGB-encoded values (ADR-0011: `key.chroma`) receives its colour converted from that linear value into its working space on the CPU.
3. **Sources.** `TransferFunction::for_source` maps `Transfer::Srgb` to `GRAPHICS_TRANSFER`. Tags stay as stored and probed (ADR-0009, item 8): web frames remain tagged sRGB, since Chrome draws them in its sRGB profile, and so do pictures with an `sRGB` chunk; the mapping belongs to the colour pipeline. Footage tagged BT.709 or linear keeps converting by its tag, and PQ and HLG keep failing with `fosscut.render.unsupported_color` until the HDR pipeline exists.
4. **Working spaces do not change.** Grading, blend modes, glow thresholds and colour measurement keep working on sRGB-encoded values of linear light (ADR-0011). Only how code values become linear light changes.
5. **Storage of model pictures.** Pictures inside 3D model files stay 8-bit `Rgba8UnormSrgb` textures with mipmaps made in linear light; their code values are mapped through `graphics_to_linear` into that storage at upload, which rounds by at most one code value.

## Consequences

- `#2E3B57` stays (46, 59, 87) in exports, frame grabs and the preview; a CSS colour, the same hex in a shape and a PNG pixel agree, with or without an `sRGB` chunk.
- Existing projects render their dark and middle colours lighter than before, as their hex values say; reviewed golden frames record the change.
- A hex colour means a code value in the output, not a colorimetric sRGB colour. A player that shows video through a different display transform than it shows web pages shows FossCut's graphics the way it shows any video.
- HDR outputs (PQ or HLG) cannot use the inverse of their transfer for graphics: PQ's inverse turns code values into absolute luminance. They will need a reference white for graphics (for example the 203 cd/m2 of ITU-R BT.2408) with graphics decoded by the SDR curve below it, so `GRAPHICS_TRANSFER` becomes a function of the output. A future SDR output with another transfer (for example sRGB-encoded image sequences) sets the graphics transfer to its own, so code values still survive.
- A camera that tagged its video sRGB would be treated as graphics; none is known to.

## Alternatives considered

- **Keep decoding hex as sRGB:** right only if players converted BT.709 video to sRGB for display, which they do not; dark brand colours keep shifting.
- **Encode exports with sRGB:** footage would no longer round-trip, and players ignore or mishandle sRGB tags on video.
- **Retag sRGB frames as BT.709 in `fosscut-media` and the web decoder:** reaches the same pixels for pictures, but makes probing report tags the files do not carry, and still leaves hex colours to each renderer. The pipeline is the one place that knows the output.
- **Convert colours when they are written:** stored values would no longer be what people typed, and every front end would have to know the rule.

## Validation

- **Export round trip:** a shape filled `#2E3B57` and text in `#FF7A45` keep their code values within 1 after FFmpeg decodes the exported file.
- **Pictures:** a PNG with an `sRGB` chunk keeps its values through an export; a web composition's CSS colour equals the same hex in a shape, and both equal the hex.
- **Unit tests** of the helpers, and every changed golden frame reviewed against its predecessor.
- **Before Accepted:** a maintainer compares an export in QuickTime, a browser and on a phone with the same hex in a design tool.
