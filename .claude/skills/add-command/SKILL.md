---
name: add-command
description: Add or change a FossCut editing command, query or MCP tool end to end (model, core command, safety contract, CLI, MCP, schemas, tests, docs). Use for any new capability that changes or reads project state, for example clip.split, text.add, effect.set_param, export.start, media.transcript.
---

# Add a command end to end

Governing decisions: [ADR-0002](../../../docs/adr/0002-agent-first-command-api.md) (one API, three front ends), [ADR-0006](../../../docs/adr/0006-agent-safety-contract.md) (safety contract), [ADR-0007](../../../docs/adr/0007-timeline-projection-and-patches.md) (projection and patches), [ADR-0005](../../../docs/adr/0005-time-model.md) (time).

## Before coding

- Name it `<area>.<verb>` (`clip.split`, `track.add`, `caption.style`). Check `capabilities` output or the registry in `fosscut-core` for an existing command that should be extended instead.
- Decide: write or read? destructive (needs `allow_destructive`)? long-running (returns a job)? Which entities does it touch, and what does undo restore?
- If it needs new stored data, run the `change-project-format` skill first.

## Steps

1. **Model** (`fosscut-model`): add or reuse the types the command operates on. Derive `serde` and `schemars`. Time fields use `fosscut-time` types.
2. **Request and response types** (`fosscut-core`): plain data structs with doc comments on every field (they become the JSON Schema agents read). Time inputs accept the explicit-unit forms (frames, seconds, timecode, flicks); bare numbers are rejected.
3. **Command implementation:** implement the command trait so the framework provides `expected_revision`, `dry_run`, `request_id`, batching, undo and operation-log entries. The implementation describes the change and validation only. Return typed errors with stable codes `fosscut.<area>.<reason>` and add new codes to the error catalogue with a `hint`.
4. **Guards:** if the command can remove primary-track content, overwrite, delete media or overwrite files, require `allow_destructive: true`.
5. **Register** the command in the capability registry so `capabilities` lists it with its schemas.
6. **Patch support:** if agents would express this edit declaratively, add the patch operation in the timeline projection layer and compile it to this command (no second execution path).
7. **CLI** (`fosscut-cli`): add the subcommand mirroring the command name (`fosscut clip split --clip clip_7k2m --at 118f --expected-revision 42 --json`). Human-readable output by default, the exact response JSON with `--json`.
8. **MCP** (`fosscut-mcp`): reachable through the generic command tool automatically once registered. Add a dedicated tool only if the operation is core to most agent sessions (ADR-0002 keeps the tool count small); write the tool description for an agent reader: what it does, when to use it, required fields, common errors.
9. **Tests:**
   - unit tests for validation and edge cases (zero-length, out-of-range times, locked tracks);
   - the generated contract tests must pass (stale revision, dry run, duplicate `request_id`, guard, undo/redo);
   - property tests where invariants exist (split then join restores the clip);
   - snapshot tests of the CLI `--json` output and the MCP schema;
   - an agent-level scripted test if the command is part of a skill workflow.
10. **Regenerate** schemas with `cargo xtask schemas` (implement that xtask task if it does not exist yet) and review the diff.
11. **Docs:** command reference entry, error codes, and any user-facing skill in `skills/` that should use it.
12. Run the `pre-pr-check` skill.

## Done when

The command works identically from a Rust test, the CLI with `--json`, and an MCP client; it appears in `capabilities`; all contract tests pass; schemas and snapshots are updated and explained in the PR.
