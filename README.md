# FossCut

An open source video editor, written in Rust, that you can drive with natural language through any AI agent.

Ask Claude Code, Codex, Cursor or any MCP client to "turn this interview into three 45-second vertical clips with animated captions and beat-synced cuts", watch the edit appear in the desktop app, and keep refining it by hand or by conversation. FossCut exposes one typed editing API through three front ends: a desktop app (a web version may follow), a CLI with JSON output, and an MCP server. Every agent edit is validated, reviewable and undoable.

**Status:** early development. Milestone 1 is in place: the open project format, frame-exact time, the command layer with its agent safety contract, and the `fosscut` CLI. Milestone 2 is done on macOS: frame-exact video decoding (`fosscut-media`), a GPU compositor with a linear-light color pipeline (`fosscut-render`), and a preview spike that chose rendering into the window behind the webview ([results](docs/development/milestone-2-spike-results.md)). Export, the MCP server and the desktop app come next ([milestones](docs/adr/README.md#suggested-first-milestones)). Contributions are welcome now, when they shape the project most.

## Try it

You need [Rust](https://rustup.rs) and, for importing media, FFmpeg's `ffprobe`.

```bash
cargo install --path crates/fosscut-cli
```

```bash
fosscut project new Demo.fosscut
```

```bash
fosscut -p Demo.fosscut media import my-clip.mp4
```

```bash
fosscut -p Demo.fosscut clip add --asset asset_0001
```

```bash
fosscut -p Demo.fosscut text add "Wait for it" --start 0f --duration 2s
```

```bash
fosscut -p Demo.fosscut clip split --clip clip_0003 --at 00:00:03:28
```

```bash
fosscut -p Demo.fosscut timeline read
```

Every command accepts `--json` for agents, `--dry-run` to preview, `--expected-revision` to guard against stale edits and `--request-id` for safe retries; `fosscut undo` and `fosscut redo` work across sessions. `fosscut capabilities` lists everything the editor can do.

## Principles

- **Agent-first, human-friendly.** No capability exists only in the GUI. Agents get the same power as people, plus eyes: frame grabs, transcripts, beats, export reports ([ADR-0002](docs/adr/0002-agent-first-command-api.md)).
- **Safe autonomous editing.** Revision checks, dry runs and plan/apply, idempotent requests, guarded destructive operations and a full operation log ([ADR-0006](docs/adr/0006-agent-safety-contract.md), [ADR-0007](docs/adr/0007-timeline-projection-and-patches.md)).
- **Open, portable projects.** A versioned, documented format with frame-exact time ([ADR-0004](docs/adr/0004-open-project-format.md), [ADR-0005](docs/adr/0005-time-model.md)).
- **Bring your own brain.** No mandatory account or model. Your agent reasons; FossCut edits, analyses locally and calls paid AI services only when you say so ([ADR-0012](docs/adr/0012-ai-capabilities-bring-your-own-brain.md)).
- **Honest exports.** What you request is what you get, verified; no hidden watermarks ([ADR-0009](docs/adr/0009-media-io-and-codecs.md), [ADR-0017](docs/adr/0017-provenance.md)).

## Documentation

| | |
|---|---|
| [Architecture decision records](docs/adr/README.md) | 18 proposed decisions and suggested first milestones |
| [AGENTS.md](AGENTS.md) | Instructions for AI coding agents working on this repository |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to contribute, DCO sign-off, workflow, releases |
| [GitHub setup](docs/development/github-setup.md) | Repository settings and branch protection for maintainers |
| [Milestone 2 spike results](docs/development/milestone-2-spike-results.md) | Preview path and compositor measurements behind ADR-0008 and ADR-0010 |

The architecture builds on two rounds of private research into CapCut 9.6.0 (a hands-on audit and a static analysis with [REA](https://github.com/morluto/rea)). The research is not part of this repository; the ADRs summarize what matters for each decision. FossCut contains no CapCut code or assets ([clean-room policy](docs/adr/0015-clean-room-policy.md)).

## Repository layout

```text
crates/       Rust crates: time, model, core, media, render, audio, analysis, engine, mcp, cli
apps/         desktop shell
ui/           shared TypeScript front end
skills/       editing skills for AI agents
docs/         research, ADRs, development docs
.claude/      development skills for AI coding agents
.github/      CI, release and dependency workflows
```

Folders appear as the implementation reaches them ([ADR-0003](docs/adr/0003-rust-workspace-and-crate-layering.md)).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the [Code of Conduct](CODE_OF_CONDUCT.md). Report security issues privately as described in [SECURITY.md](SECURITY.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option ([ADR-0014](docs/adr/0014-licensing.md)). Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in FossCut, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
