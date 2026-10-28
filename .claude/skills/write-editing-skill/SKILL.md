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
3. **Follow the safe editing loop** from `fosscut-basics`: read the timeline, gather facts (transcript, beats, scenes, faces), plan with `timeline.plan`, show the user the plan summary for large changes, apply with `base_revision`, verify with frame grabs or a contact sheet, export and check the export report.
4. **Respect guards.** Ask the user before destructive operations; never pass `allow_destructive` without their consent.
5. **Handle failure modes explicitly:** missing analysis models (offer to install), low transcript confidence, no beats detected, revision conflicts (re-read and re-plan).
6. **Keep it short.** Put long style guides and presets in `references/` and tell the agent when to read them.
7. **Write for any agent.** No vendor-specific tool names; FossCut's MCP tools and CLI only.

## Evaluation

Every skill needs `eval/` with:

- a fixture specification (generated or licensed media; never personal or copyrighted footage),
- a scripted run that an evaluation harness can execute through MCP deterministically,
- checks on the resulting timeline projection and export report.

Run the deterministic evaluation in CI. Before changing a skill's behaviour, also run it with at least one real agent (Claude Code or Codex) and record the outcome in the PR.

## Clean room

Write from editing practice and FossCut's API. Do not paraphrase CapCut's skill descriptions or any vendor prompts; run the `clean-room-check` skill before submitting.
