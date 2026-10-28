# ADR-0018: AI-assisted development workflow

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0001](0001-record-architecture-decisions.md), [ADR-0015](0015-clean-room-policy.md), [ADR-0016](0016-testing-strategy.md), [AGENTS.md](../../AGENTS.md), [CONTRIBUTING.md](../../CONTRIBUTING.md)

## Context

FossCut will be built mostly by AI coding agents directed by maintainers and contributors ("vibecoded"). Without structure this produces inconsistent architecture, unreviewed large diffs, missing tests, hallucinated APIs and license contamination. Agents work best with explicit written conventions, small tasks with clear acceptance checks, and fast automated feedback.

## Decision

1. **`AGENTS.md` at the root is the single source of instructions for coding agents** (read natively by Codex, Cursor and others). `CLAUDE.md` imports it so Claude Code reads the same rules. It covers: architecture in brief with ADR links, repository map, commands to build and test, coding conventions, the definition of done, clean-room rules, and what never to do.
2. **Development skills in `.claude/skills/`** encode recurring procedures: writing an ADR, adding a command end to end, adding an effect, changing the project format, adding golden-frame tests, running the pre-PR check, clean-room review, and consulting the CapCut research. Skills reference ADRs instead of repeating them.
3. **Work in small, verifiable units.** One issue, one pull request, one concern. Issues state acceptance criteria that an agent can check. Architectural changes start with an ADR pull request.
4. **Definition of done** for code changes: builds, formatted, lint-clean with warnings denied, tests added or updated at the right level ([ADR-0016](0016-testing-strategy.md)), public API changes reflected in schemas and snapshots, CLI and MCP reachability for new capabilities ([ADR-0002](0002-agent-first-command-api.md)), docs updated, CI green.
5. **Humans own merges.** Every pull request needs approval from a maintainer; agents do not merge. Pull requests disclose AI assistance in the template.
6. **Conventional commit messages and pull request titles** (`feat(core): …`, `fix(render): …`) with DCO sign-off; release notes are generated from them.
7. **Continuous integration is the gate:** formatting, lints, tests on three operating systems, dependency license and advisory checks, documentation link checks and ADR index consistency.
8. **No secrets in the repository or in agent context.** Provider keys for AI features are read from the user's environment or OS keychain, never committed, never logged.

## Consequences

- Any agent session can become productive quickly and produce changes that fit the architecture.
- Process overhead per change is higher than ad hoc prompting, but review stays tractable.
- `AGENTS.md` and the skills must evolve with the code; stale instructions are treated as bugs.

## Alternatives considered

- **Per-tool instruction files only** (`CLAUDE.md`, `.cursorrules`, separate copies): drift between tools. One shared `AGENTS.md` avoids it.
- **Letting agents merge with passing CI:** too risky for a project whose output is user media and whose license must stay clean.

## Validation

New contributors (human or agent) complete a first issue using only `AGENTS.md`, the skills and the ADRs, without maintainer clarification, in a trial run before the first public call for contributors.
