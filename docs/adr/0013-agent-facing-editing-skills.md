# ADR-0013: Ship editing know-how as open agent skills

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), [ADR-0016](0016-testing-strategy.md), private research: `docs/capcut-rea/AGENT-SUBSYSTEM.md`

## Context

Tools alone do not make an agent a good editor. CapCut's assistant offers guided workflows such as highlight extraction, edit plans and voice-over scripts, talking-head cleanup (fillers, pauses, repetition), music alignment and long-to-short condensation, but they are closed and tied to its service. The `SKILL.md` format (YAML frontmatter with `name` and `description`, Markdown body, optional scripts and references) is supported by Claude Code and a growing set of agents.

## Decision

1. **FossCut ships user-facing editing skills** in `skills/`, one folder per skill with a `SKILL.md`, using only the public CLI/MCP API. These are distinct from the development skills in `.claude/skills/` ([ADR-0018](0018-ai-assisted-development-workflow.md)).
2. **First catalogue**, in priority order:
   1. `fosscut-basics`: how to inspect a project, read the timeline, plan and apply, verify with frame grabs and export reports, respect guards.
   2. `talking-head-cleanup`: transcript-driven removal of filler words, long pauses and retakes, with jump-cut smoothing.
   3. `viral-captions`: word-timed animated captions, keyword emphasis, safe areas for 9:16 platforms.
   4. `vertical-reframe`: 16:9 to 9:16 or 1:1 with subject tracking and per-shot framing.
   5. `long-to-shorts`: find highlights with lead-in and payoff, cut to target durations, add hook text.
   6. `beat-sync`: cut and transition on beats, music ducking under speech, fade-out at the end.
   7. `b-roll-and-overlays`: place inserts, titles, lower thirds and stickers on topic changes.
   8. `export-for-platforms`: presets and checks for short-form platforms and YouTube, with verification.
3. **Distribution.** `fosscut skills install --target <claude|codex|cursor|dir>` copies skills to the agent's skill directory; the MCP server also exposes them as MCP prompts and resources for clients without skill support.
4. **Every skill has an evaluation**: a fixture project or media, a scripted run, and checks on the resulting timeline and export (duration bounds, no remaining fillers, captions within safe area, cuts on beats). Skills change only with passing evaluations.
5. **Skills never encode private knowledge or copied text** from CapCut or other products ([ADR-0015](0015-clean-room-policy.md)); they are written from editing practice and FossCut's own API.

## Implementation notes

- **`fosscut-basics`** (milestone 4) is the first skill: the safe editing loop (open, read, plan with a dry run, apply with `expected_revision` and a `request_id`, check with frame grabs and contact sheets, export as a job and check the report), permissions, times, and what to do for each common error code. Every step names the MCP tool and the CLI command.
- **Milestone 6 skills:** `viral-captions`, `talking-head-cleanup` and `vertical-reframe`, each with an evaluation on committed or generated media and a real-agent run recorded in its pull request. Evaluations that transcribe declare `needs: ["speech"]`, and fixtures can be copied from the repository.
- **Evaluations** are `eval/eval.json`: generated fixtures, then scripted tool calls with expected values, expected errors, image sizes and saved values. `crates/fosscut-cli/tests/skills.rs` replays every skill's evaluation through `fosscut mcp` in CI and checks each `SKILL.md` names real tools. A real-agent run is recorded in the pull request that changes a skill.
- **Distribution:** skills are built into the binary (`crates/fosscut-mcp/src/skills.rs`; a test keeps it equal to `skills/`). `fosscut skills install --target claude` writes `.claude/skills/`, `--target codex` writes `.agents/skills/` (Codex's folder), `--user` uses the home folder, and `--dir` any folder. Replacing a copy with local changes needs `--allow-destructive`. Cursor has no skill folder FossCut writes yet. The MCP server lists every skill as a prompt and as a `fosscut://skills/<name>` resource.
- **`motion-graphics`** turns a brief into a storyboard of scenes built as compound clips from the motion-design primitives of ADR-0036 to ADR-0039 and the library's device frames, UI kit and compositions (ADR-0028), with worked patterns in `references/`; its evaluation builds a logo scene with a draw-on, a glow, a text reveal, particles and a camera push, and exports it.

## Consequences

- Users get high-quality results from natural-language requests without prompt engineering.
- Skills become a contribution area for editors and creators who do not write Rust.
- Skills depend on API stability; breaking API changes must update skills in the same pull request.

## Alternatives considered

- **Hard-code workflows as commands** (for example `auto_shorts`): opaque, inflexible, and duplicates what agents do well with good instructions.
- **Rely on users' prompts only:** inconsistent quality and repeated failure modes.

## Validation

Each first-catalogue skill passes its evaluation with at least two different agents (for example Claude Code and Codex) on the fixture corpus.
