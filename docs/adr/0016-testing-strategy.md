# ADR-0016: Testing strategy: contracts, golden frames, export verification, agent evaluations

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0005](0005-time-model.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0008](0008-render-and-audio-engine.md), [ADR-0013](0013-agent-facing-editing-skills.md), [ADR-0018](0018-ai-assisted-development-workflow.md), private research: `docs/capcut-audit/TEST-LOG.md`

## Context

Most code will be written by AI agents, so tests are the main defence against regressions and the main feedback signal for the agents themselves. A video editor fails in ways unit tests miss: off-by-one frames, color shifts, audio drift, preview that differs from export, exports whose codec differs from the request (as round 1 observed). Round 1 also showed the value of a reproducible synthetic fixture.

## Decision

1. **Unit and property tests** (`proptest`) for `fosscut-time`, `fosscut-model` and `fosscut-core`: time arithmetic, serialization round-trips, migrations, command invariants (split then join restores the clip, ripple keeps later clips contiguous).
2. **Command contract tests** (ADR-0006) generated for every registered command: stale revision, dry run, duplicate request id, destructive guard, undo and redo.
3. **Snapshot tests** (`insta`) for timeline projections, plans, error payloads, CLI JSON output and MCP tool schemas, so API changes are visible in review.
4. **Golden-frame render tests:** render specific frames of fixture timelines and compare to stored references with perceptual tolerances (PSNR or SSIM thresholds per test), on at least one GPU backend in CI and a software adapter where available. Goldens are small PNGs updated only with an explicit command and reviewer approval.
5. **Preview and export parity:** for each golden timeline, frames from the preview evaluation path and from the exported file match within tolerance at chosen timestamps.
6. **Export verification tests:** each export preset is rendered and probed; container, codecs, resolution, frame rate, frame count, duration and color tags must match the request. The round 1 baseline (8 s, 1080p30, title) and the H.264-request regression are permanent cases.
7. **Fixtures are generated, not downloaded:** `fixtures/` contains scripts (FFmpeg `lavfi` generators like round 1's test pattern, synthesized speech-like and music-like audio, generated subtitles) plus a small, licensed corpus for analysis features (speech with pauses and fillers, music with clear beats, a moving subject, a shaky shot, green screen). Every file has a license and source record.
8. **Fuzzing** (`cargo-fuzz`) for project parsing, patch and selector parsing, and media probing entry points; run nightly.
9. **Agent evaluations:** scripted MCP sessions on fixture projects verify skills and tool ergonomics deterministically in CI. Model-in-the-loop evaluations with real agents run on demand or nightly, because they cost money and are non-deterministic; their results are tracked over time rather than gating every pull request.
10. **CI matrix:** Linux, macOS and Windows for Rust tests; GPU-dependent tests marked and run where a GPU or a software adapter exists.
11. **Failing tests block merging.** Every CI job feeds one aggregate check, `CI result`, which branch protection on `main` requires together with the pull request checks ([GitHub setup](../development/github-setup.md#branch-protection-ruleset-on-main)). A failed or cancelled job fails it; only jobs for project parts that do not exist yet may be skipped. Tests never retry on failure (a flaky test is a bug), and CI installs every external tool the tests need, such as FFmpeg, and sets `FOSSCUT_REQUIRE_FFMPEG=1` so that tests depending on it fail instead of skipping. An ignored test must state its reason (`#[ignore = "…"]`, enforced by Clippy).
12. **Coverage minimums.** `cargo xtask coverage` measures line coverage with `cargo-llvm-cov` over all tests, including the end-to-end CLI tests, and fails below 90% for the workspace or below 85% for any crate in `crates/` (coverage is counted per source line from the LCOV report). The minimums live in `xtask/src/main.rs`; they are raised as coverage grows and are never lowered to make a change pass. Coverage shows which code runs, not whether its results are checked, so assertions are still reviewed; mutation testing (`cargo-mutants`) is the planned complement.

## Consequences

- Agents get fast, precise feedback and cannot quietly break frame accuracy or color.
- Golden images and fixtures need storage discipline (small sizes, Git LFS if they grow).
- GPU variance across vendors requires tolerances and occasional per-backend goldens.
- Required checks and coverage minimums add CI time and occasionally force tests for code that is hard to reach; that cost is accepted because agents otherwise ship untested paths.

## Alternatives considered

- **Manual QA before releases:** does not scale with agent-generated change volume.
- **Bit-exact frame comparison:** fails across GPUs and drivers; perceptual thresholds are the practical standard.

## Implementation notes

- **CI focus on macOS** (2026-11-01, the maintainer's decision while desktop work targets macOS): pull requests run the Rust tests on macOS only, with the lints, docs, UI, dependency and WebAssembly checks. The Linux and Windows test jobs and coverage (item 12) run nightly on `main` and on demand (`workflow_dispatch`), still feeding `CI result` there, and releases build macOS packages only. Items 10 to 12 apply in full again when the other platforms return.

## Validation

Every merged feature pull request adds or updates at least one test at the appropriate level. No pull request merges with a failing, cancelled or silently skipped test job, and coverage stays at or above the minimums in item 12.
