# FossCut

An open source video editor, written in Rust, that you can drive with natural language through any AI agent.

Ask Claude Code, Codex, Cursor or any MCP client to "turn this interview into three 45-second vertical clips with animated captions and beat-synced cuts", watch the edit appear in the desktop app, and keep refining it by hand or by conversation. FossCut exposes one typed editing API through three front ends: a desktop app (a web version may follow), a CLI with JSON output, and an MCP server. Every agent edit is validated, reviewable and undoable.

**Status:** early development. Milestone 1 is in place: the open project format, frame-exact time, the command layer with its agent safety contract, and the `fosscut` CLI. Milestone 2 is done on macOS: frame-exact video decoding (`fosscut-media`), a GPU compositor with a linear-light color pipeline (`fosscut-render`), and a preview spike that chose rendering into the window behind the webview ([results](docs/development/milestone-2-spike-results.md)). Milestone 3 renders a timeline to a video file: titles, the clips' audio, hardware H.264 or HEVC where the machine has it, ProRes, AV1 and VP9, each export verified by probing the written file. Milestone 4 adds the MCP server and the `fosscut-basics` skill, so agents can edit, check their work with frame grabs and contact sheets, and export. Milestone 6 adds local analysis (loudness, pauses, shot cuts and speech with word timings), word-timed, animated captions with SRT import and export, talking-head cleanup (cutting pauses, fillers and retakes across every track at once) and vertical reframing. Milestone 5, the desktop app, is done: the app opens a project and shows a GPU preview, a timeline and who is editing; when an agent edits the same project through `fosscut mcp`, its changes appear in the app as they land, and you can undo them there. Editing depth is under way: crop, mirror, blend modes, audio fades and keyframed animation of placement, opacity and volume are in, and so are color effects (adjustments, curves, HSL, color wheels, LUTs, vignette, blur, sharpen, grain), adjustment layers, ready-made looks, and automatic color correction and matching ([milestones](docs/adr/README.md#suggested-first-milestones)). Contributions are welcome now, when they shape the project most.

## Try it

You need [Rust](https://rustup.rs), FFmpeg 8 with its shared libraries (set up as in [FFmpeg for development](CONTRIBUTING.md#ffmpeg-for-development)) and its `ffprobe` tool, and a GPU or a software renderer such as Mesa lavapipe.

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

```bash
fosscut -p Demo.fosscut contact-sheet --out Demo-sheet.png
```

```bash
fosscut -p Demo.fosscut export Demo.mp4
```

Every command accepts `--json` for agents, `--dry-run` to preview, `--expected-revision` to guard against stale edits and `--request-id` for safe retries; `fosscut undo` and `fosscut redo` work across sessions. `fosscut export` renders, encodes and then probes the file to check it matches what you asked for; `fosscut export --presets` lists the formats this machine can write. `fosscut frame 2s --out still.png` and `fosscut contact-sheet` render what the export will show, so you or your agent can check an edit first. `fosscut media analyze asset_0001` measures loudness, pauses and shot cuts on your machine and caches them; after `fosscut models install whisper-base.en --yes` (148 MB), `fosscut media analyze asset_0001 transcript` writes down the speech with a time for every word, filler words such as "um" and the pauses between words. `fosscut captions generate --preset highlight` turns that into animated captions, a few words at a time with the spoken word highlighted, and `fosscut captions export subs.srt` writes them as a subtitle file. `fosscut clip trim --clip <id> --start 1s` moves a clip's edge, `fosscut timeline remove-ranges 3.4s..4.1s --allow-destructive` cuts a pause out of every track and closes the gap, and `fosscut timeline duplicate --width 1080 --height 1920 --fill` starts a vertical version that `fosscut clip set --clip <id> --position 0.4,0` frames shot by shot. `fosscut capabilities` lists everything the editor can do, and `fosscut capabilities clip.split` shows a command's arguments. `fosscut host --events` keeps the project open as a live session until Ctrl-C: edits from other terminals and agents then run there in order and print as they happen, the way the desktop app will show them.

### The desktop app

The app is early: it opens and creates projects, plays them with sound, and shows the timeline, the agents connected and every change by who made it. Drag clips to move them and their edges to trim them, set placement, opacity and volume in the inspector, and split, delete, undo and redo. With [Node.js](https://nodejs.org) 24 and pnpm, run the interface's development server, then the app with the project to open:

```bash
pnpm --dir ui install
```

```bash
pnpm --dir ui dev
```

```bash
cargo run -p fosscut-desktop -- Demo.fosscut
```

While it is open, `fosscut` and `fosscut mcp` edit that project through the app, so changes from the command line or an agent appear in the window as they land.

## Use it from an agent

`fosscut mcp` serves FossCut to any MCP client over stdin and stdout. Start your agent in the folder with your media (or with a `.fosscut` project, which then opens automatically) and add the server:

| Agent | Setup |
|---|---|
| Claude Code | `claude mcp add fosscut -- fosscut mcp` |
| Codex | In `~/.codex/config.toml`: `[mcp_servers.fosscut]` with `command = "fosscut"` and `args = ["mcp"]` |
| Cursor | In `.cursor/mcp.json`: `{"mcpServers": {"fosscut": {"command": "fosscut", "args": ["mcp"]}}}` |

Then ask in plain language, for example "import interview.mp4, cut it at 1:12, add the title 'Episode 4' for three seconds and export it as MP4". The agent gets 20 tools: projects, timeline reads, media import and analysis (loudness, pauses, shot cuts, speech with word timings), speech model installs that it asks you about first, word-timed captions from transcripts or SRT files, an `edit` tool that runs any command from `capabilities` as one undoable step, undo and redo, frame grabs and contact sheets to check its work, and exports that run as background jobs and are verified when they finish. Its changes are recorded as made by `agent:<name>` in the project history, and you can undo them from the CLI. When the project is open live (`fosscut host`, and soon the desktop app), the agent's edits go to that session, so you see them as they land.

Agents edit better with FossCut's skills: `fosscut-basics` teaches the safe editing loop, `viral-captions` word-timed captions, `talking-head-cleanup` cutting pauses, fillers and retakes, and `vertical-reframe` 9:16 versions that keep each shot's subject in frame. `fosscut skills install --target claude` (or `--target codex`) adds them to the project in the current folder, and `--user` adds them for all your projects. MCP clients without skill support find the same text as the server's prompts.

## Principles

- **Agent-first, human-friendly.** No capability exists only in the GUI. Agents get the same power as people, plus eyes: frame grabs, transcripts, beats, export reports ([ADR-0002](docs/adr/0002-agent-first-command-api.md)).
- **Safe autonomous editing.** Revision checks, dry runs and plan/apply, idempotent requests, guarded destructive operations and a full operation log ([ADR-0006](docs/adr/0006-agent-safety-contract.md), [ADR-0007](docs/adr/0007-timeline-projection-and-patches.md)).
- **Open, portable projects.** A versioned, documented format with frame-exact time ([ADR-0004](docs/adr/0004-open-project-format.md), [ADR-0005](docs/adr/0005-time-model.md)).
- **Bring your own brain.** No mandatory account or model. Your agent reasons; FossCut edits, analyses locally and calls paid AI services only when you say so ([ADR-0012](docs/adr/0012-ai-capabilities-bring-your-own-brain.md)).
- **Honest exports.** What you request is what you get, verified; no hidden watermarks ([ADR-0009](docs/adr/0009-media-io-and-codecs.md), [ADR-0017](docs/adr/0017-provenance.md)).

## Documentation

| | |
|---|---|
| [Architecture decision records](docs/adr/README.md) | 21 proposed decisions and suggested first milestones |
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
