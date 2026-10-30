# ADR-0025: Nest timelines in clips for compound clips

- **Status:** Proposed
- **Date:** 2026-10-30
- **Deciders:** maintainers
- **Related:** [ADR-0004](0004-open-project-format.md), [ADR-0008](0008-render-and-audio-engine.md), [ADR-0023](0023-clip-speed-as-a-time-map.md), [ADR-0024](0024-link-clips-in-groups-that-edit-together.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (subprojects, multiple timelines entries)

## Context

Editors let people turn a stretch of an edit into one clip and use it like footage: a lower third built from a title, a shape and a sound, an intro reused in several videos, a multi-layer montage moved as a block. The research shows a media-bin section for these "subprojects", separate from footage, beside multiple timelines in one project. It was inspected, not exercised.

FossCut projects already hold several timelines (`timeline.duplicate`), and a group (ADR-0024) keeps clips together, but a group is not a clip: it cannot be trimmed, faded, sped up or graded as one picture, and it cannot appear twice. A compound clip must keep the format's rules (ADR-0004: typed references, one owner per object, no copies of media), render through the same planner for preview and export (ADR-0008), and stay readable for agents.

## Decision

1. **A clip can show a timeline.** Clip content gains `{"kind": "timeline", "timeline": "timeline_…", "source_in"}`: the clip shows and plays another timeline of the same project from `source_in`, for its duration, on a video track. Speed (ADR-0023), transforms, effects, masks, keyframes and transitions apply to it as to a video clip.
2. **Nested timelines are ordinary timelines.** They are timeline documents in the manifest with `role: "compound"`, so `timeline.read`, every editing command and `export` work on them by id. The desktop lists them apart from the project's main timelines.
3. **No cycles, little depth.** A timeline may not show itself through any chain of clips, and chains are at most eight deep; validation reports `fosscut.model.nesting`. A compound timeline's source range is its duration: clips cannot read past its end.
4. **Picture and sound.** The planner renders a nested timeline at its own canvas size into a texture and places it like a picture of that size, fitted into the parent canvas. The mixer mixes the nested timeline at the parent's sample rate and applies the clip's volume, fades and pan. Nested timelines may use a different canvas, but not a different frame rate.
5. **Commands.** `clip.compound` moves chosen clips of one timeline into a new compound timeline, keeping their tracks' order and their timing, and puts one compound clip in their place on the lowest track. `clip.decompose` puts a compound clip's contents back in its place, and removes the compound timeline when nothing else shows it. `clip.add` takes a `timeline` instead of an `asset` to place an existing compound timeline.
6. **Agents read the nesting.** `timeline.read` shows a compound clip's timeline id and name, and `project.info` lists each timeline's role, so an agent can open and edit the inside.

## Consequences

- Compound clips reuse every clip feature (speed, effects, masks, transitions) and every timeline feature (editing, export) instead of adding parallel ones.
- Rendering nests: a frame may plan and composite timelines inside timelines, which costs a texture per level and must stay within the eight-level limit.
- Editing inside a compound timeline changes every clip that shows it, which is the point, and the undo log records it as an edit of that timeline.
- Different frame rates inside one project stay out of scope.

## Alternatives considered

- **Compound clips as stored copies of their clips:** simple to render, but every use is a separate copy, so a change does not propagate, and copies multiply ids and media references.
- **Groups with a shared transform:** lighter, but a group has no single picture for effects, masks and speed, and cannot be placed twice.
- **A separate project file per subproject:** cleaner isolation, but it breaks the single-bundle rule (ADR-0004) and complicates relinking and undo.

## Validation

- Model tests: a compound clip round trips, and validation reports cycles, chains deeper than eight, and reading past the nested timeline's end.
- Contract tests: `clip.compound` and `clip.decompose` round trip the timeline exactly, and undo restores both.
- Render and audio tests: a compound clip's frame matches rendering its timeline and placing the image, and its sound matches the nested mix.
