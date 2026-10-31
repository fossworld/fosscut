# ADR-0004: Open, versioned, agent-friendly project format

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0005](0005-time-model.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0011](0011-effects-and-animatable-properties.md), private research: `docs/capcut-rea/DRAFT-FORMAT.md`

## Context

Project files of existing editors, CapCut's included, are often hard to use outside the application that wrote them. Recurring problems: time stored in units that cannot represent frame boundaries exactly, untyped cross-references, the same data stored twice in different shapes, many default-valued records per clip, effects that only render with the vendor's downloaded packages, absolute paths that go stale, a single backup generation, and machine identifiers embedded in the document. Agents need small, unambiguous, diffable documents; users need projects that survive moving machines.

## Decision

A FossCut project is a directory bundle `Name.fosscut/`:

```text
Name.fosscut/
  project.json          manifest: format version, project id, name, settings, timeline list, primary timeline
  media.json            asset table: one record per source file
  timelines/<id>.json   one document per timeline
  history/ops.jsonl     append-only operation log (ADR-0006)
```

Rules:

1. **Versioned schema.** `project.json` carries `format_version` (semantic version). Every change to the format ships a migration in `fosscut-model` and a regenerated JSON Schema in `schemas/`. Older readers refuse newer files with a clear error that names the features they cannot represent.
2. **Typed, nested ownership.** A clip owns its effects, masks, keyframes and text runs inline. References across documents are typed (`{"asset": "asset_3f9k"}`), never untyped id lists.
3. **One immutable asset table.** Each asset records a content identity (size plus BLAKE3 hash), relative and absolute path hints, and probed stream metadata. Clips reference assets; splitting a clip never copies an asset record.
4. **Sparse documents.** Default values are omitted when writing and documented in the schema. A plain clip is a few lines, not dozens of fields.
5. **Short, stable, readable ids.** Every object has an id of the form `<kind>_<4 to 8 base32 chars>` (`clip_7k2m`, `track_v1a`), unique within the project and never reused.
6. **Semantic effect parameters.** Effects are stored as `{type, version, enabled, params}` with a closed parameter vocabulary per type (ADR-0011), never as a path to a package cache.
7. **Explicit `enabled` on every effect, mask and adjustment**, separate from its parameters.
8. **Structured rich text.** Text content is a string plus style runs over character ranges; no JSON inside strings, one source of truth for style.
9. **Nothing machine-specific or telemetric** in the bundle: no device ids, no usage analytics, no absolute location of the bundle itself. Caches (thumbnails, waveforms, proxies, analysis results) live in the OS cache directory keyed by asset content identity and can always be regenerated.
10. **Atomic saves.** Write to a temporary file, fsync, rename. Undo history is the operation log, not backup copies.
11. **Timelines are independent documents** listed in the manifest; nested timelines (compound clips) are referenced by timeline id.
12. **Interchange.** Import and export OpenTimelineIO for professional tools. Importing projects from other editors, CapCut included, for migration may be added later as separate crates ([ADR-0015](0015-clean-room-policy.md)).

Illustrative clip (not normative; the generated schema is):

```json
{
  "id": "clip_7k2m",
  "kind": "video",
  "asset": "asset_3f9k",
  "source_in": 0,
  "start": 0,
  "duration": 2775360000,
  "transform": { "scale": 1.2 },
  "effects": [
    { "id": "fx_c1", "type": "mask.ellipse", "version": 1, "enabled": true,
      "params": { "center": [0.5, 0.5], "size": [0.28, 0.5], "feather": 0.05 } }
  ]
}
```

## Consequences

- Projects are portable, reviewable in git, and small enough to fit in an agent's context.
- JSON Schema lets agents and external tools validate edits before applying them.
- Every format change costs a migration and tests; this is deliberate.
- Content hashing of large media costs time; identity starts with size plus a sampled hash and completes the full hash in the background.

## Alternatives considered

- **Single JSON file:** simpler, but large diffs and merge conflicts across timelines.
- **SQLite project database:** good for huge projects, opaque to agents and git; may be reconsidered as a cache, not as the document.
- **Adopting OpenTimelineIO as the native format:** excellent for interchange but lacks effects, text styling, masks and keyframes at the level FossCut needs, and uses floating-point rates.
- **Adopting an existing proprietary editor's schema:** ties FossCut to a format it does not control and inherits the problems listed in the context.

## Implementation notes

- **Markers** (format 0.8.0): a timeline's `markers` hold `{id, at, name, color}` in time order, omitted when empty, with `marker_…` ids. `marker.add`, `marker.set` and `marker.remove` edit them, `timeline.read` lists them, and `timeline.duplicate` copies them with new ids. Selectors by marker (ADR-0007, item 4) can build on them.
- **Clip groups and detached sound** (format 0.8.0) are decided in [ADR-0024](0024-link-clips-in-groups-that-edit-together.md).
- **Media bin** (format 0.9.0): an asset's `folder` and the media document's sorted `folders` organize the bin; `name`, `folder` and the path hints are organization, while the content identity and the probed information never change (item 3). `media.rename`, `media.move` and `media.folder` organize; `media.relink` records a moved file's new place only when its identity matches; `media.replace` points every clip at another asset; `media.remove` refuses an asset in use unless told to remove its clips. Front ends mark assets whose files are missing (`fosscut_engine::import::mark_missing`).
- **Timelines:** `timeline.add` creates one, empty or from chosen media placed one after another with the first video's canvas and rate; `timeline.rename`, `timeline.set_primary` and `timeline.remove` (a project keeps at least one) complete the set. A removed timeline's document is deleted from the bundle (`ProjectStore::remove`).

- **Text, fonts and sound** (format 0.11.0):
  - **Text clips:** a text clip's style gains underline, case, letter and line spacing, an outline, a box, a shadow and a glow; clips gain spans and a reveal, as decided in [ADR-0027](0027-styled-text-with-project-fonts.md).
  - **Project additions:** `project.json` holds the project's saved `text_presets`, and fonts import as `font` assets with their families.
  - **Sound and captions:** sound effects share the clips' `effects` list ([ADR-0026](0026-sound-effects-in-the-effect-registry.md)), and caption pages gain a `translation`.

- **Shape clips** (format 0.12.0): shapes, stickers and backgrounds are clips of kind `shape`, as decided in [ADR-0028](0028-a-built-in-library-made-by-fosscut.md). Library sounds are WAV files written into the bundle's `library/` folder and imported as ordinary audio assets.

- **Multicam angles** (format 0.13.0): a compound clip may carry `angle`, the video track of its timeline whose picture shows, as decided in [ADR-0029](0029-cut-multicam-angles-on-compound-clips.md). Stabilization and tracking add no stored data of their own: they write ordinary keyframes ([ADR-0030](0030-apply-measured-motion-as-keyframes.md)). Recordings are files in the bundle's `recordings/` folder, imported as ordinary assets.

- **Canvas fills** (format 0.14.0): a video or image clip may carry `canvas`, what fills the canvas around its picture: `{"kind": "color", "color"}`, or `{"kind": "blur", "amount"}`, its own picture scaled to cover and blurred (`clip.canvas`). User preferences never enter a project; they live in a file of their own ([ADR-0031](0031-keep-user-preferences-outside-projects.md)).

## Validation

- Round-trip property tests: load, save, load yields identical documents.
- The round 1 baseline project (two clips after a split, a title, a disabled mask) serializes to under 2 KB.
- A project moved to another directory or machine relinks its media through hash and relative paths.
