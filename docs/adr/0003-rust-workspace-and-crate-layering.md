# ADR-0003: Rust workspace and crate layering

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0008](0008-render-and-audio-engine.md), [ADR-0010](0010-desktop-shell-ui-and-web-path.md), private research: `docs/capcut-rea/ARCHITECTURE.md`

## Context

The owner chose Rust. Mature editors separate the document model, the editing commands, the media engine, and the GPU and ML backends. FossCut needs that separation so the same engine runs without a UI (CLI, MCP server, CI render tests, later a server or WebAssembly build) as well as inside the desktop app.

## Decision

A single Cargo workspace with these crates. Dependencies point only downward in the table.

| Crate | Responsibility | Must not depend on |
|---|---|---|
| `fosscut-time` | Time types: flicks, rational rates, timecode parsing and formatting ([ADR-0005](0005-time-model.md)) | anything else in the workspace |
| `fosscut-model` | Project document types, serialization, JSON Schema generation, validation, migrations ([ADR-0004](0004-open-project-format.md)) | media, render, UI |
| `fosscut-core` | Commands and queries, transactions, undo/redo, operation log, revisions, idempotency, capability registry, timeline projection and patches ([ADR-0006](0006-agent-safety-contract.md), [ADR-0007](0007-timeline-projection-and-patches.md)) | render, media backends, UI |
| `fosscut-media` | Probing, decoding, encoding, muxing behind backend traits; FFmpeg and platform encoders on native ([ADR-0009](0009-media-io-and-codecs.md)) | render, UI |
| `fosscut-render` | Processing graph, compositor, effects, color pipeline on `wgpu` ([ADR-0008](0008-render-and-audio-engine.md), [ADR-0011](0011-effects-and-animatable-properties.md)) | UI |
| `fosscut-audio` | Audio graph, mixing, resampling, loudness, playback device | render, UI |
| `fosscut-analysis` | Optional local analysis: speech with word timing, voice activity, beats, shots, faces and saliency ([ADR-0012](0012-ai-capabilities-bring-your-own-brain.md)) | render, UI |
| `fosscut-engine` | Session host: opens projects, runs commands, drives preview and export jobs, serves frames, hosts the local IPC endpoint | UI |
| `fosscut-mcp` | MCP server adapter over `fosscut-engine` (official Rust MCP SDK, `rmcp`) | UI |
| `fosscut-cli` | The `fosscut` binary: CLI commands, `fosscut mcp`, `fosscut skills` | UI |
| `apps/desktop` | Desktop shell hosting the engine and the UI ([ADR-0010](0010-desktop-shell-ui-and-web-path.md)) | |

Repository layout:

```text
crates/      Rust library and binary crates listed above
xtask/       repository automation run as `cargo xtask <task>`: schema generation, golden updates, fixtures, checks
apps/        desktop shell (and later web host)
ui/          shared TypeScript front end for desktop and web
skills/      agent-facing editing skills shipped to users (ADR-0013)
schemas/     generated JSON Schemas for the project format and commands
fixtures/    reproducible media generators and small test projects
docs/        research, ADRs, user and developer documentation
.claude/     development skills for AI coding agents
```

Rules:

- Edition 2024, stable toolchain pinned in `rust-toolchain.toml`, one MSRV for the workspace.
- Library crates use `thiserror` for typed errors; binaries may use `anyhow`.
- No `unsafe` outside `fosscut-media` and `fosscut-render` backend modules, each `unsafe` block justified in a comment.
- `fosscut-time`, `fosscut-model` and `fosscut-core` never touch the file system, the clock, threads or the environment directly: documents are read and written through the `ProjectStore` trait (a directory on desktop, browser storage or a server on the web), and timestamps come from the caller. This keeps the web build possible ([ADR-0010](0010-desktop-shell-ui-and-web-path.md)).
- Heavy or optional dependencies (FFmpeg, ONNX Runtime, speech models) sit behind Cargo features so `fosscut-model` and `fosscut-core` build for `wasm32` with no native libraries.
- Public types in `fosscut-model` and `fosscut-core` derive `serde` and `schemars` so schemas are generated, never hand-written.

## Consequences

- The model and command layers are testable without a GPU or FFmpeg, which keeps most agent-written code fast to verify.
- A WebAssembly build of model, core and render is structurally possible from day one.
- More crates mean more boundaries to maintain; the dependency rules are enforced by a CI check on the workspace graph.

## Alternatives considered

- **One large crate:** faster to start, but agents would couple UI, model and backends, and the web build would be blocked.
- **C++ engine with Rust bindings:** reuses existing editors' code but loses Rust's safety and the owner's language choice.
- **Embedding an existing open source engine (for example MLT):** GPL licensing and a different data model; contradicts [ADR-0004](0004-open-project-format.md) and [ADR-0014](0014-licensing.md).

## Validation

`cargo build -p fosscut-model -p fosscut-core --target wasm32-unknown-unknown` succeeds in CI once those crates exist, and a dependency-direction check passes.
