# CLAUDE.md

Claude Code follows the shared agent instructions:

@AGENTS.md

## Claude Code specifics

- Development skills in `.claude/skills/` load automatically; use the one matching the task (table in AGENTS.md).
- For work that may need an ADR, plan first and draft the ADR before writing code.
- Shared permissions are in `.claude/settings.json`; personal overrides go in `.claude/settings.local.json`, which is git-ignored.
