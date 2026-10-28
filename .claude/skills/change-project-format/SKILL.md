---
name: change-project-format
description: Change FossCut's stored project format safely, including schema version bump, migration, JSON Schema regeneration, fixtures and round-trip tests. Use for any change to types that are serialized into Name.fosscut bundles (project.json, media.json, timelines, operation log).
---

# Change the project format

Governing decisions: [ADR-0004](../../../docs/adr/0004-open-project-format.md) (format rules), [ADR-0005](../../../docs/adr/0005-time-model.md) (time), [ADR-0011](../../../docs/adr/0011-effects-and-animatable-properties.md) (effects). Lessons from other editors' formats are in the private research (`docs/capcut-rea/DRAFT-FORMAT.md`), when present in your checkout.

## Decide the kind of change

| Change | Version bump | Migration |
|---|---|---|
| New optional field with a default that preserves old behaviour | minor | none (default fills it) |
| New effect type or parameter | minor | none, but older readers must reject it clearly |
| Renamed, moved, re-typed or removed field; changed meaning or units | major | required |
| Structural change (ownership, ids, file layout) | major | required, and an ADR (use `write-adr`) |

## Checklist against ADR-0004

- Typed, nested ownership; no untyped id lists.
- No JSON inside strings; no duplicated sources of truth.
- Defaults omitted on write and documented in the schema.
- Explicit `enabled` for any effect-like object.
- Times in flicks via `fosscut-time`.
- Nothing machine-specific, absolute self-paths or telemetry.
- Short stable ids with a kind prefix for new object kinds.

## Steps

1. Change the types in `fosscut-model` with doc comments on every new field.
2. Bump `format_version` according to the table.
3. For major changes, add a migration step `vN -> vN+1` as a pure function on the JSON value, plus a test with a before and after fixture. Migrations are never edited after release; fix forward with a new step.
4. Make older-reader behaviour explicit: newer files must fail with an error that names the unsupported version and features.
5. Regenerate schemas with `cargo xtask schemas` (implement the task if missing) and review the diff in `schemas/`.
6. Update fixture projects in `fixtures/` through their generator, not by hand.
7. Tests: round-trip (load, save, load is identical), migration from every supported older version, property tests for invariants, snapshot of the timeline projection if it changes.
8. If the change affects the agent-facing projection or patch operations, update [ADR-0007](../../../docs/adr/0007-timeline-projection-and-patches.md)-related docs and skills in `skills/`.
9. Run the `pre-pr-check` skill. Title the PR `feat(model): …` or `feat(model)!: …` for breaking changes.
