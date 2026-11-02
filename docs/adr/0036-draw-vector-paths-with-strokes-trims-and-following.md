# ADR-0036: Draw vector paths with strokes, trims and path following

- **Status:** Proposed
- **Date:** 2026-11-01
- **Deciders:** maintainers
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0004](0004-open-project-format.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0011](0011-effects-and-animatable-properties.md), [ADR-0014](0014-licensing.md), [ADR-0015](0015-clean-room-policy.md), [ADR-0025](0025-nest-timelines-in-clips-for-compound-clips.md), [ADR-0028](0028-a-built-in-library-made-by-fosscut.md)

## Context

Motion-graphics ads draw lines: a logo's arrow draws itself on, a delivery route draws along streets with a dot riding its end, an underline swooshes in under a title. Shape clips (ADR-0028) are closed, filled outlines from the mask geometry with an inner outline; they have no open paths, no strokes centred on a path, no way to show part of an outline, and nothing can move along them. Logos arrive as SVG files, which FossCut cannot import. Design tools solve this with strokes (width, caps, joins, dashes), trim paths (start, end and offset as fractions of the path's length) and motion along a path; these are public, well documented ideas (SVG 2 stroking, arc-length parametrization of Bezier curves). Agents need all of it as plain data with stable property paths and one consistent unit.

## Decision

1. **Shapes become vector paths** (`fosscut_model::ShapeContent`, format 0.16.0):
   - `closed` (true when omitted) leaves a `path` shape open; `contours` add more subpaths (holes, separate parts) in the same -1 to 1 box; `fill_rule` is `even_odd` (the default, as shapes filled before) or `nonzero`.
   - `fill` may be `null` for no fill; omitted it stays white. An open path's fill closes it with a straight edge, as in SVG.
   - `stroke` is centred on the outline: `color` and `gradient` as a fill's, `width`, `cap` (`butt`, `round`, `square`), `join` (`miter`, `round`, `bevel`), `miter_limit`, `dashes` and `dash_offset`.
   - `trim_start`, `trim_end` (0 to 1) and `trim_offset` (wrapping) keep part of the outline's whole length, contours counted one after another. Trims cut the stroke, and the fill of an open path.
2. **One unit along strokes.** Stroke width, dashes and dash offset are fractions of the canvas height, like text sizes; the clip's scale scales them. Trims and follow progress are fractions of the outline's length, measured in canvas pixels of the shape's box before its transform, so they mean the same at every output size.
3. **Every outline is a Bezier path.** `ShapeContent::outline` builds rectangles (rounded by cubic quarter circles), ellipses, stars, hearts and paths with `kurbo`; rectangles start at the top-left end of their top edge, ellipses at the top, stars at their top point, hearts at their dip, all clockwise. `trim_path`, `point_along` and `parse_path_data` live in `fosscut-model`, so planning, commands and rendering share them.
4. **Animatable properties** join the closed vocabulary (ADR-0011 item 3): `shape.trim_start`, `shape.trim_end`, `shape.trim_offset` and `shape.stroke_width` for shape clips (`Applies::Shape`), and `follow.progress` for following clips (`Applies::Follower`). The animation presets `draw_on` (in) and `draw_off` (out) narrow the trims through `AnimationDelta::draw`.
5. **Path following.** A clip's `follow` is `{clip, progress, orient, offset}`: a shape clip on the same timeline that follows nothing itself. The planner places the follower's centre at `progress` along the shape's outline (wrapping on closed paths), mapped through the shape clip's keyframed placement and animations at that moment (flips, scale, stretch, rotation around its anchor; turns in depth are not followed), plus `offset`; `orient` adds the path's direction to its rotation. `clip.follow` sets, changes or clears it with the safety contract; `fosscut.follow.invalid` names what cannot be followed. Removing a path clip releases its followers; `timeline.duplicate` and `clips.paste` point copies at the copied paths.
6. **Rendering.** Shapes that use none of these features draw exactly as before. Others are rasterized on the CPU with `tiny-skia` into antialiased coverage (fills by their rule, strokes with caps, joins and dashes) and painted in linear light like every shape, with the raster widened by the stroke's reach.
7. **SVG drawings** import as assets of kind `vector` (`.svg`, parsed with `usvg`), referenced by path hints like other media, with their size in `info.video` and `info.vector {view_box, paths}`. Image clips of them render with `resvg` at the size they show on the output, so they stay sharp at any resolution; frame providers hold the parsed drawing (`FrameProvider::vector`) instead of a decoder. Text and embedded raster images in SVG files are not drawn.
8. **Exploding drawings.** `vector.explode` turns a vector asset into a compound timeline (ADR-0025) on the drawing's proportions fitted to the parent canvas, with one shape clip per visible path on its own track named after the element's id, keeping its fill, stroke, rule, group opacity and placement; gradients keep their first and last stops and direction. The command takes the paths as `parts`; front ends read them from the file (`fosscut_engine::vector::with_parts`, called by `Projects::execute`), so agents give only the asset.
9. **Reach.** The CLI has `fosscut shape add|set` (SVG path data in canvas pixels, `--stroke`, `--cap`, `--join`, `--dash`, `--trim`), `fosscut clip follow` (with `--travel 0..1`) and `fosscut vector explode`; MCP agents use `edit` with `shape.add` (`d`), `shape.set`, `clip.follow`, `vector.explode` and `keyframe.set`, and `media_import` for SVG files. No MCP tool is added. `capabilities` lists the commands, the properties, the presets and a `vector` section with the units.

## Consequences

- Line art, routes, draw-on logos and moving markers are ordinary, editable clips with keyframes, readable by agents.
- Three permissively licensed libraries join the build (`kurbo`, `tiny-skia`, `usvg` and `resvg`, MIT, Apache-2.0 and BSD-3-Clause), without their font and image features.
- Two rasterizers draw shapes: the mask geometry for plain shapes and `tiny-skia` for vector ones, so their edges are antialiased slightly differently.
- `fill` can now be `null`, which every reader of shapes must handle; a merge patch's `null` for `fill` means no fill.
- Exploded drawings approximate gradients and patterns; the drawing as an image stays exact.

## Alternatives considered

- **Strokes as an effect pass on the GPU.** Rejected: caps, joins and dashes are geometry, exact and testable on the CPU, and shapes already rasterize there.
- **Storing SVG paths in the media table so `vector.explode` needs no file.** Rejected: large drawings would bloat `media.json` and every `media.list` an agent reads.
- **Rasterizing SVG files once at import.** Rejected: they would blur when scaled up, which is why vectors are used.
- **Following the trimmed part of a path.** Rejected: a dot riding a drawing line is the same progress keyed on both, and following the whole path keeps progress independent of trims.

## Validation

- **Model tests:** outlines measure as their shapes, trims keep and wrap the right parts, points carry the path's direction, SVG path data fits its box, followers sit on paths through transforms and anchors, and validation refuses broken strokes, trims and follows.
- **Core tests:** contract samples for `clip.follow` and `vector.explode`; path data, trims and draw animations, follows and their refusals, copies of followers, and exploded drawings with undo.
- **Render tests:** golden frames of round caps, dashes, gradient strokes, a 30% to 70% trim, followers on a drawing route, and an SVG drawing at two output sizes with a hard-edge check; plain shapes keep their goldens.
- **Engine and CLI tests:** SVG import, drawings and their exploded shapes rendering alike, and the whole flow from the command line.
- **Before Accepted:** a person reviews draw-on and follow ads made by an agent, and logos from common design tools explode faithfully.
