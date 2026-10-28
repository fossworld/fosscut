<!-- markdownlint-disable-file MD041 -->

## Summary

What changes and why, in a few sentences.

Closes #

## Architecture

- ADRs this implements or relies on:
- [ ] No architectural change, or a new or superseding ADR is included

## Checklist

- [ ] Title follows conventional commits (`feat(core): …`, `fix(render): …`)
- [ ] Every commit is signed off (`git commit -s`)
- [ ] Tests added or updated at the right level (ADR-0016); each behaviour change has a test that fails without it
- [ ] No test was weakened, skipped or deleted, and the coverage minimums were not lowered
- [ ] New capabilities are reachable from CLI and MCP and listed in `capabilities` (ADR-0002)
- [ ] Writes follow the agent safety contract: revisions, dry run, idempotency, guards (ADR-0006)
- [ ] Schemas, snapshots and docs regenerated or updated where public behaviour changed
- [ ] User-facing skills in `skills/` updated if the API they use changed (ADR-0013)
- [ ] No material copied from CapCut or other products; new assets have a license record (ADR-0015, ADR-0014)

## AI assistance

- [ ] No AI tools were used
- [ ] AI tools were used: name the tool(s) and what they did. I have reviewed and understand every line of this change.

## Verification

Commands run and their results, frame grabs or export reports where relevant.
