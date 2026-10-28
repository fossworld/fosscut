# ADR-0001: Record architecture decisions

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR index](README.md), [ADR-0018](0018-ai-assisted-development-workflow.md)

## Context

FossCut will be developed largely by AI coding agents (Claude Code, Codex, Cursor) supervised by human maintainers. Agents start every session without memory of earlier discussions. Decisions that live only in chat history or in a maintainer's head are re-litigated, contradicted or silently broken. The project also has two rounds of CapCut research whose conclusions need to be turned into binding choices.

## Decision

1. Architecture decisions are recorded as numbered Markdown files in `docs/adr/`, named `NNNN-short-title.md`, using [template.md](template.md).
2. An ADR is required for any change to: crate boundaries, the project file format, the time model, the public command/CLI/MCP contract, the rendering or media backends, the UI technology, licensing, or the AI provider model.
3. Status moves `Proposed` → `Accepted` only after a maintainer approves it in a pull request. An accepted ADR is never edited in substance; it is superseded by a new ADR that links back.
4. `docs/adr/README.md` keeps the index current. Agents read the index before architectural work (see [AGENTS.md](../../AGENTS.md)).
5. Code comments and pull requests reference ADRs by number (`ADR-0005`) when they implement or rely on a decision.

## Consequences

- Agents and new contributors can load the project's reasoning in minutes.
- Writing an ADR adds friction to big changes. That friction is intended.
- The index must be maintained; CI checks that every ADR file is listed.

## Alternatives considered

- **Design docs in a wiki:** invisible to agents working in the repository and not versioned with the code.
- **Decisions in issues only:** hard to find, no single current state.

## Validation

The first implementation pull requests cite ADRs, and reviewers can point to an ADR when rejecting an off-architecture change.
