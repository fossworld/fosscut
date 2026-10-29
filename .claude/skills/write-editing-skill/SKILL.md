---
name: write-editing-skill
description: Write or change a user-facing FossCut editing skill in skills/ (SKILL.md for AI agents that drive FossCut through MCP or the CLI), including its evaluation. Use for workflows like talking-head cleanup, viral captions, vertical reframe, long-to-shorts, beat sync and platform exports.
---

# Write a user-facing editing skill

Governing decision: [ADR-0013](../../../docs/adr/0013-agent-facing-editing-skills.md). These skills are shipped to end users and run inside their agents; they are different from the development skills in `.claude/skills/`.

## Structure

```text
skills/<skill-name>/
  SKILL.md          frontmatter (name, description) + instructions
  references/       optional detail loaded on demand (style guides, presets)
  eval/             evaluation: fixture spec, scripted steps, checks
```

- `name`: kebab-case, matches the folder.
- `description`: one or two sentences saying what the skill achieves and the user requests that should trigger it ("Use when the user asks to clean up a talking-head video, remove ums, pauses or retakes").

## Writing the instructions

1. **Start from the goal and the checks**, not from commands: what does a good result look like, measurably (target duration, no fillers left, captions inside the 9:16 safe area, cuts on beats)?
2. **Use only the public API** (MCP tools and CLI commands listed by `capabilities`). Reference commands by name; do not hard-code schemas that `capabilities` already provides.
3. **Follow the safe editing loop** from `fosscut-basics` (`skills/fosscut-basics/SKILL.md`): read the timeline, gather facts (transcript, beats, scenes, faces), plan with an `edit` dry run, show the user the plan summary for large changes, apply with `expected_revision` and a `request_id`, verify with `frame_grab` or `contact_sheet`, export and check the export report. When ADR-0007's patches exist, planning becomes `timeline.plan` and applying needs `base_revision`.
4. **Respect guards.** Ask the user before destructive operations; never pass `allow_destructive` without their consent.
5. **Handle failure modes explicitly:** missing analysis models (offer to install), low transcript confidence, no beats detected, revision conflicts (re-read and re-plan).
6. **Keep it short.** Put long style guides and presets in `references/` and tell the agent when to read them.
7. **Write for any agent.** No vendor-specific tool names; FossCut's MCP tools and CLI only.

## Evaluation

Every skill needs `eval/eval.json` with:

- `fixtures`: media generated with FFmpeg (`file`, and the `ffmpeg` arguments before it), never personal or copyrighted footage;
- `steps`: the tool calls an agent following the skill would make, each with `tool`, `args` and an `about` line, checked with:
  - `expect`: values at dotted paths of the result (`results.0.left.end.frames`), equal or matching `{"$len": n}`, `{"$min_len": n}` or `{"$one_of": [...]}`;
  - `expect_error`: the error code a step must fail with (`expect` then checks the error);
  - `expect_image`: the size of the PNG a frame or contact sheet returns;
  - `save`: values to reuse later as `"${name}"` (ids, revisions, job ids);
  - `repeat_while`: polling, for `job_status`.

`crates/fosscut-cli/tests/skills.rs` replays every evaluation through `fosscut mcp` in CI, and checks each `SKILL.md` (frontmatter name and description, tool names that exist). `skills/fosscut-basics/eval/eval.json` is the model to copy. Also add the skill to `SKILLS` in `crates/fosscut-mcp/src/skills.rs`, which builds it into `fosscut skills install` and the MCP prompts; a test fails until you do. Before changing a skill's behaviour, also run it with at least one real agent (Claude Code or Codex) and record the outcome in the PR.

## Clean room

Write from editing practice and FossCut's API. Do not paraphrase CapCut's skill descriptions or any vendor prompts; run the `clean-room-check` skill before submitting.
