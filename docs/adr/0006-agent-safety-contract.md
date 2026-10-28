# ADR-0006: Agent safety contract for every write

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0007](0007-timeline-projection-and-patches.md), private research: `docs/capcut-rea/AGENT-SUBSYSTEM.md`

## Context

Autonomous agents make mistakes, retry after timeouts, work from stale reads, and sometimes act in parallel with a human. Robust agent tooling guards against this with revision checks on every write, dry runs, idempotency keys, typed error codes, capability discovery, readiness states for assets, non-destructive default placement, and refusals to delete primary footage unless explicitly allowed. FossCut adopts such a contract for all front ends, humans included.

## Decision

1. **Revisions.** The project has one revision number (in `project.json`) that increments on every committed transaction, including undo and redo. Per-timeline revisions may be added later if concurrent editing of different timelines needs them. Every write accepts `expected_revision`. A mismatch fails with `fosscut.revision.conflict`, returns the current revision and a summary of what changed, and changes nothing. CLI and MCP writes from agents must pass it; the GUI passes it implicitly.
2. **Dry run and plan.** Every write supports `dry_run: true`, which validates and returns the predicted diff without committing. A dry run is never treated as a commit and returns no new revision.
3. **Idempotency.** Writes accept `request_id`. Repeating the same id with the same arguments returns the original result without executing again; the same id with different arguments fails with `fosscut.request.reused`. Ids are kept for the session and recorded in the operation log.
4. **Transactions.** A `batch` of commands applies atomically: all or nothing, one revision step, one undo step.
5. **Typed errors.** Errors carry a stable `code` (`fosscut.<area>.<reason>`), a human message, a machine `hint` for recovery, and structured `details`. The catalogue of codes is part of the public API and documented.
6. **Destructive operations need explicit permission.** Removing clips from the primary video track, overwrite placement, deleting media from the project, replacing transcribed caption text, and overwriting existing export files fail with `fosscut.guard.destructive` unless the request includes `allow_destructive: true`. Default placement is insert or append, never overwrite.
7. **Asset readiness.** Assets have states `probing`, `ready`, `missing`, `failed`, and generated assets also `generating`. Only `ready` assets can be placed.
8. **Long jobs.** Export, proxy generation and analysis return a job id immediately, with progress, cancellation and a final result object. Jobs are idempotent by `request_id` too.
9. **Operation log.** Every committed transaction appends a record to `history/ops.jsonl`: revision before and after, commands and arguments, actor (`human`, or `agent:<client name>`), optional reason text supplied by the agent, timestamp. Undo and redo operate on these transactions. The GUI can show and revert agent changes as a group.
10. **No ambient authority.** Commands only touch files the project references or paths the user granted (import sources, export destinations). The MCP server never exposes arbitrary file reads or shell execution.

## Consequences

- Agents can retry safely and recover from conflicts by re-reading and re-planning.
- Users can audit and revert exactly what an agent did.
- Every command implementation must handle dry run, revisions and idempotency; the command framework in `fosscut-core` provides these generically so individual commands only describe their effect.

## Alternatives considered

- **Last write wins:** silent data loss when a human and an agent edit together.
- **Locking the project while an agent works:** blocks the user and leaves stale locks after crashes.
- **Trusting agents not to delete:** contradicted by experience; guards cost little.

## Validation

Contract tests run every command through: stale revision, dry run, duplicate request id, destructive guard, and undo. A command cannot be registered without passing them.
