# ADR-0002: One typed command API with GUI, CLI and MCP front ends

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0003](0003-rust-workspace-and-crate-layering.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0007](0007-timeline-projection-and-patches.md), private research: `docs/capcut-rea/AGENT-SUBSYSTEM.md`, `docs/capcut-rea/ARCHITECTURE.md`

## Context

FossCut's defining goal is that people create high-quality, viral videos by talking to an AI agent (Claude Code, Codex, Cursor or any MCP client), while a human can open the same project in a desktop app and keep editing.

CapCut already lets its own built-in assistant edit projects, which shows the approach works when every editing action is available as a command rather than only through the UI. That access is reserved for CapCut's own cloud-connected assistant: external agents have no supported way in, so community tools generate CapCut project files directly and still need CapCut to render them.

## Decision

1. **The command API is the product core.** Every state change to a project is a typed command (`clip.split`, `clip.move`, `text.add`, `effect.set_param`, `export.start`, …) with a JSON-serializable request and response, defined once in Rust (`fosscut-core`). Every read is a typed query.
2. **Three thin front ends call the same API:**
   - the **GUI** (desktop now, web later, [ADR-0010](0010-desktop-shell-ui-and-web-path.md)),
   - the **CLI** `fosscut` with `--json` output for scripts and agents with shell access,
   - an **MCP server** `fosscut mcp` (stdio, and streamable HTTP for remote or web use) for any MCP client.
   No front end may change a project except through commands. No capability exists only in the GUI.
3. **Session host.** A project is edited by exactly one engine session at a time. The desktop app hosts sessions; when `fosscut` (CLI or MCP) targets a project that is open in the app, it connects to the app's session over a local socket, so the user watches the agent's edits live and can undo them. When no app is running, the CLI and MCP server host a headless session in-process.
4. **Agent eyes are part of the API**, not extras: frame grabs at a time, contact sheets, waveform and loudness reads, transcripts with word timings, beat and scene lists, and an export verification report (actual codec, duration, frame count). Agents must be able to check their own work.
5. **MCP surface is coarse, CLI surface is fine.** The CLI mirrors the full command tree. The MCP server exposes a small set of tools (on the order of 15 to 25: open and inspect project, read timeline, plan and apply changes, run a named command, import and analyze media, frame grab, contact sheet, export, job status, undo, capabilities) because large tool lists degrade agent performance. Command schemas are discoverable through a `capabilities` tool and MCP resources.
6. **Capability discovery.** A single `capabilities` query returns the schema version, every command with its JSON Schema, effect types, available codecs on this machine, and installed analysis models.

## Implementation notes

- **Session host** (milestone 4): `fosscut_engine::project` finds, creates, opens and locks project bundles, and `fosscut_engine::import` probes media into `media.add` arguments, so the CLI and the MCP server host projects the same way. Without a live host, each write takes the bundle's writer lock for the length of one request and reopens the project from disk, so several front ends can work on one project in turn.
- **Live sessions** (milestone 5): [ADR-0022](0022-live-sessions-over-a-local-socket.md) decides how a host (the desktop app, or `fosscut host`) keeps a project open as its only writer and how the CLI and MCP reach it. `fosscut_engine::project::Projects` routes every call by the bundle's writer lock, so the existing front ends went live without other changes.
- **Agent eyes, first step:** `fosscut_engine::frame` renders one frame of a timeline, or a contact sheet of frames spread evenly over it, through the same plan and renderer as an export, so an agent checks what it will export. The CLI writes them as PNG files (`fosscut frame`, `fosscut contact-sheet`), reporting every time in frames, timecode, seconds and flicks.
- **MCP server** (milestone 4): the `fosscut-mcp` crate, on the official Rust SDK `rmcp`, serves 17 tools on stdio as `fosscut mcp` (18 since `media_analyze`, ADR-0019, 19 since `models`, ADR-0020, and 20 since `captions`, ADR-0021):
  - projects: `project_new`, `project_open`, `project_info`;
  - reads: `timeline_read`, `media_list`, `history`, and `query` for any other query;
  - writes: `media_import`, `edit` (any registered command, as one atomic batch with `expected_revision`, `dry_run`, `request_id`, `allow_destructive` and `reason`), `undo`, `redo`;
  - agent eyes: `frame_grab` and `contact_sheet`, which return PNG images;
  - export: `export` (a dry run, or a background job), `job_status` (which can wait up to 60 s) and `job_cancel`;
  - and `capabilities`.

  The server remembers an open project (from `--project`, the single bundle in its working directory, or the last `project_open` or `project_new`); any tool can name another. Reads carry read-only annotations. FossCut errors are tool results with `isError` set and the typed `{code, message, hint, details}`; protocol errors are only for malformed calls. Writes are logged as made by `agent:<client name>`. The server's instructions describe the safe editing loop. Streamable HTTP waits for the local authentication design in the Consequences below.
- **Capabilities:** beside the core's commands and queries, `capabilities` lists the engine's operations (`export.start`, `frame.grab`, `frame.contact_sheet`) with the command line that runs each, and the export presets. At full detail, the error list includes the engine's codes (export, time range, jobs). Every schema at once is about 67 KB, more than agent clients show inline, so `capabilities` also takes `names` and returns only those commands and queries with their schemas (three commands come to about 20 KB): the first real-agent runs had to learn arguments from dry-run errors without it.
- **Timeline editing commands:** `clips.copy` is a query that returns clips as stored, and `clips.paste` places such documents at a time with new ids, in the same project or another with the same media: a clipboard that agents and the desktop share. `clip.copy_attributes` copies placement, crop, opacity, blend, sound settings, effects, masks or animations between clips. `timeline.split` splits every clip under a time, `clip.replace` swaps a clip's media, `clip.align` lines a picture up with the canvas, and `clip.trim` with `ripple` deletes the start or end of a clip and closes the gap.

## Consequences

- Agent support is never an afterthought: a feature is not done until it is reachable from CLI and MCP and covered by an agent-level test.
- The GUI becomes a client of the same API, which keeps undo, history and collaboration consistent.
- Designing every feature as a serializable command costs more up front than direct UI mutation.
- A local IPC protocol between front ends and the app's session host must be designed and secured (local socket, per-session token, no network listener by default).

## Alternatives considered

- **GUI first, scripting API later:** the path most editors took; agent support stays partial forever. Rejected.
- **Agents write project files directly** (the community approach for CapCut): no validation, no live preview, concurrent writes corrupt state. Rejected; file editing stays possible for offline tools but is not the agent path.
- **Expose every command as its own MCP tool:** simple but floods agent context and hurts tool selection. Rejected in favour of coarse tools plus a generic `command` tool with discoverable schemas.

## Validation

- A spike where Claude Code, through `fosscut mcp`, reproduces the round 1 baseline (import the test pattern, add a title, split, export 1080p30, verify metadata) without any GUI involvement.
- The same edit sequence run while the desktop app has the project open appears live in the app and undoes cleanly.
