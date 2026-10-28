---
name: write-adr
description: Draft a FossCut architecture decision record. Use when a change affects crate boundaries, the project format, the time model, the command/CLI/MCP contract, rendering or media backends, UI technology, licensing, AI providers, or supersedes an existing ADR.
---

# Write an ADR

Process rules are in [ADR-0001](../../../docs/adr/0001-record-architecture-decisions.md). This skill is the procedure.

## Steps

1. **Check existing decisions.** Read `docs/adr/README.md` and every ADR your topic touches. If an accepted ADR already decides it, either follow it or write a superseding ADR; never edit an accepted decision in place.
2. **Gather evidence.** Cite measurements, spikes, upstream documentation, or the private research (`docs/capcut-audit/`, `docs/capcut-rea/`, git-ignored and present only in maintainers' checkouts). Refer to research as plain paths, never Markdown links, and summarize only what the decision needs. Separate facts from inferences.
3. **Create the file.** Next free number, kebab-case title:

   ```bash
   ls docs/adr | grep -E '^[0-9]{4}-' | tail -1
   cp docs/adr/template.md docs/adr/NNNN-short-title.md
   ```

4. **Write it** following the template:
   - Title in the imperative or as the chosen option ("Integer flicks for timeline time").
   - Status `Proposed`, today's date, related ADRs and research links.
   - Context: the forces and constraints, with evidence.
   - Decision: precise enough that an agent can implement it without asking. Use numbered rules, names of crates, types, commands, error codes.
   - Consequences: positive and negative, and what must now be maintained.
   - Alternatives: each serious option with the reason it lost.
   - Validation: the spike, benchmark or test that moves it to Accepted.
5. **Update the index** table and reading order in `docs/adr/README.md`.
6. **Update dependents.** If the ADR supersedes another, set the old one's status to `Superseded by ADR-NNNN` (the only edit allowed on an accepted ADR) and link both ways. Update `AGENTS.md` if a rule there changes.
7. **Verify:**

   ```bash
   python3 scripts/check-adr-index.py
   npx markdownlint-cli2 docs/adr/*.md
   ```

8. **Open a PR** titled `docs(adr): NNNN short title`, containing only the ADR and index changes. Implementation follows in later PRs.

## Quality bar

- One decision per ADR. Split if you find yourself writing "and also".
- No hedging in the Decision section; uncertainty goes into Validation.
- Keep it under about 120 lines; link out for detail.
