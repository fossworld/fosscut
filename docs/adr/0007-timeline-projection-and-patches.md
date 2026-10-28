# ADR-0007: Compact timeline projection with plan and apply

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0006](0006-agent-safety-contract.md), private research: `docs/capcut-rea/AGENT-SUBSYSTEM.md`

## Context

Fine-grained commands are ideal for precise edits ("split clip_7k2m at frame 118"). Restructuring work ("cut this 20-minute interview into a 45-second vertical short with captions") is easier for a language model to express as a target state. A good agent interface offers both: precise commands, and a compact document view that the agent edits by selector or patch, previews as a plan, and then applies, with revision checks and protection against accidentally removing primary footage.

The full project document is too verbose for an agent's context on long timelines, and raw JSON Patch with array indexes is brittle when anything moves.

## Decision

1. **Timeline projection.** A query `timeline.read` returns a compact, agent-oriented JSON view of a timeline: tracks, clips with ids, positions in frames and timecode, source ranges, text content, enabled effects with non-default parameters, markers, and the current revision. Options select a time range, tracks, and detail level (`outline`, `normal`, `full`). The projection is a read model; it is not the storage format.
2. **Id-addressed patches.** Agents submit a patch: a list of operations addressed by stable ids and selectors, never by array index.

   ```json
   {
     "base_revision": 42,
     "ops": [
       { "op": "split", "clip": "clip_7k2m", "at": { "frames": 118 } },
       { "op": "set", "target": "clip_7k2m", "path": "transform.scale", "value": 1.2 },
       { "op": "remove", "select": { "track": "track_v1a", "range": { "timecode": ["00:00:10:00", "00:00:12:00"] } }, "ripple": true },
       { "op": "add", "track": "track_t1", "clip": { "kind": "text", "text": "Wait for it", "start": { "seconds": "1.5" }, "duration": { "seconds": "2" } } }
     ]
   }
   ```

3. **Plan, then apply.** `timeline.plan(patch)` validates, resolves selectors, expands to concrete commands, checks guards (ADR-0006) and returns the command list, a human-readable diff summary, warnings, and the would-be revision. `timeline.apply(patch)` performs the same steps and commits them as one transaction. Applying requires `base_revision` to match.
4. **Selectors** are a small, documented JSON language: by id, kind, track, time range, text match, marker, or effect type, combinable with `and`. No embedded scripting.
5. **Patches compile to ordinary commands.** There is one execution path; the patch layer adds no capabilities that commands lack.

## Consequences

- Agents can choose precise commands or declarative restructuring, and both get the same validation, guards, undo and logging.
- The projection must stay stable and documented, because skills and agent prompts will depend on it; changes are versioned with the API.
- Selectors add parsing and resolution code that needs thorough tests.

## Alternatives considered

- **XML projection:** workable, but JSON matches the rest of the API and MCP payloads.
- **RFC 6902 JSON Patch on the stored document:** index-based paths break when clips move; it would also couple agents to the storage format.
- **A text DSL or embedded scripting language for agents:** compact, but harder to validate and a larger security surface. May be revisited for a read-only summary format.

## Validation

- An agent-level test converts the round 1 eight-second clip into a 9:16 version with a caption and a split using one plan and one apply, and the result matches a golden projection.
- Patches built against a stale revision fail without side effects.
