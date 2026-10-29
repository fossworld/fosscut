# ADR-0019: Run media analysis locally in fosscut-analysis, cached by content

- **Status:** Proposed
- **Date:** 2026-10-29
- **Related:** [ADR-0003](0003-rust-workspace-and-crate-layering.md), [ADR-0004](0004-open-project-format.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), [ADR-0014](0014-licensing.md), private research: `docs/capcut-audit/feature-inventory.json` (CC-084, CC-119, CC-121), `docs/capcut-rea/RESOURCES.md`, `docs/capcut-rea/AGENT-SUBSYSTEM.md`

## Context

Milestone 6 adds the first "viral" skills: word-timed captions, talking-head cleanup and vertical reframe (ADR-0013). Each needs facts about the footage first: speech with word timings, pauses, loudness and shot boundaries. ADR-0012 already decides the direction:

- analysis runs locally in `fosscut-analysis`;
- models download only on explicit request, each with a manifest;
- results are cached data keyed by asset content and model version, exposed to agents as queries.

ADR-0004 item 9 keeps such caches out of the project bundle.

What is not yet decided is how analysis is built, cached and reached. Four things push on that:

- **Time.** Analysis is slow next to editing commands: decoding an hour of video to find cuts takes minutes. It cannot run inside the pure core (ADR-0003: no file or clock access), and agents must not repeat it for every question.
- **The engine already has the patterns:** project hosting, background jobs with progress and cancellation (`fosscut_engine::jobs`), and the per-user cache directory (`fosscut_engine::cache_dir`).
- **Research, behaviour only.** In the editor studied, automatic captions, filler-word detection and auto reframe are paid features; its large-scale speech recognition runs in the cloud, while scene, face and saliency analysis runs on the device. Talking-head cleanup in its agent removes fillers, pauses and repetitions.
- **Opportunity.** Free, local analysis that an agent can query is FossCut's opening.

## Decision

1. **Crate.** `fosscut-analysis` holds the analyzers. It sits beside `fosscut-media`, `fosscut-render` and `fosscut-audio`, below `fosscut-engine`, and is `#![forbid(unsafe_code)]`.
   - Analyzers are pure functions over decoded audio samples or video frames, so they are unit-tested without FFmpeg.
   - A thin adapter decodes media with `fosscut-media` and feeds them.
2. **Analyzers** have a stable name, a version and typed options with defaults:
   - **Versions.** The version is an integer bumped whenever results for the same input can change.
   - **Results** are `serde` and `schemars` types.
   - **Times** are in the asset's own time, from its start, given as flicks with decimal seconds alongside.
   - **First set:**
     - `loudness`: ITU-R BS.1770-4 integrated loudness in LUFS (K-weighting, 400 ms blocks with 75% overlap, absolute gate at -70 LUFS, relative gate at -10 LU), plus sample peak in dBFS.
     - `silences`: ranges whose 50 ms RMS stays below a threshold (default -40 dBFS) for at least a minimum duration (default 0.3 s).
     - `scenes`: shot cuts. A 64-bin luma histogram is taken of every frame on a decimated grid. A cut is where the normalised L1 distance between neighbouring frames reaches a threshold (default 0.35) and the last cut is at least a minimum shot length (default 0.5 s) away. Each cut gives its frame, time and score.
   - Later analyzers (speech in a separate ADR, faces) follow the same contract.
3. **Cache.** Results are stored at `cache_dir()/analysis/<sample BLAKE3>-<size>/<analyzer>@<version>-<options hash>.json`:
   - The key is the content identity recorded at import (`media.json`), computed from the file when missing.
   - Each entry repeats the analyzer, version, options and model, if any.
   - Unreadable entries, or entries from another version, are recomputed.
   - Nothing is written into the bundle. Deleting the cache only costs time.
4. **Reaching analysis.**
   - **Engine:** `fosscut_engine::analysis::analyze(project, bundle, asset, analyzers, options)` returns cached results at once. Otherwise it runs a background job of kind `analysis` through `Jobs`, which grows from export-only to several job kinds.
   - **CLI:** `fosscut media analyze <asset> [analyzer ...] [--refresh]` waits, with progress on stderr.
   - **MCP:** `media_analyze` returns cached results in the reply, or a job id whose `job_status` carries the results. It is one new tool, keeping ADR-0002's tool budget.
   - **Discovery:** `capabilities` lists the analyzers with their options.
   - **Errors:** `fosscut.analysis.unknown_analyzer`, `fosscut.analysis.no_audio`, `fosscut.analysis.no_video`, plus the existing asset and missing-media codes.
5. **Models**, for analyzers that need one (speech first):
   - Manifests are compiled into the binary: name, task, version, license, size, SHA-256, download URL and languages.
   - A model installs only through an explicit call: `fosscut models install <name>` with confirmation, or the MCP `models` tool with `confirm: true`. Either way, size and license are reported first.
   - Downloads go to a temporary file in `cache_dir()/models/`, are checked against the SHA-256 and only then renamed into place. Installed models work offline.
   - `capabilities` reports which are installed.
   - Errors: `fosscut.model.not_installed`, `fosscut.model.checksum_mismatch`, `fosscut.model.download_failed`.
   - Only licenses that allow commercial use are listed (ADR-0014).
6. **Agents decide.** Analysis reports facts; it never edits. Removing fillers, choosing a reframe or placing captions is the agent's call, made with editing commands and its own checks (ADR-0012 item 1). Vertical reframe in this milestone is agent-directed: scene cuts plus frame grabs and contact sheets, with no face or saliency model yet.

## Consequences

- Agents build edits on measured facts: pauses from silences and word gaps, cuts from scene boundaries, loudness for normalisation later.
- Repeated questions are instant, and the cache survives across projects that use the same file.
- Analysis grows by adding analyzers, without new tools or front-end code.
- Versioning is a discipline: an analyzer change that alters results without a version bump serves stale answers. Tests pin results on generated fixtures to catch this.
- The cache can grow large for long footage. A size report and cleanup command will be needed.
- Jobs are no longer export-only, so their status carries either an export report or analysis results.

## Alternatives considered

- **Store analysis in the project bundle:** travels with the project, but breaks ADR-0004 item 9 (machine-generated, regenerable data) and bloats version control.
- **Compute on every request:** simple, but too slow for agents that ask several questions about the same footage.
- **One MCP tool per analyzer:** clearer names, but grows the tool list with every analyzer, against ADR-0002 item 5.
- **Machine-learned shot detection now:** better on gradual transitions, but needs a model download for a task that histogram differences handle well on hard cuts. It can be added later as another analyzer version.
- **Simple RMS for loudness:** easier, but platforms normalise to LUFS, and BS.1770 is public and small to implement.

## Validation

- Generated fixtures with known answers:
  - a 997 Hz sine at -20 dBFS in mono reads -23.0 LUFS (BS.1770 calibration: 0 dBFS reads -3.01 LUFS), within 0.1 LU;
  - tones with silent gaps give silences at the inserted ranges, within one 50 ms window;
  - colour sources cut together give scene cuts at exactly the joined frames.
- Cache hits return identical results without decoding, and option or version changes miss the cache.
- An agent answers "where are the pauses and cuts in this clip?" through `media_analyze` alone.
