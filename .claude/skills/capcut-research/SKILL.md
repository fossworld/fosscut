---
name: capcut-research
description: Consult FossCut's private CapCut research (round 1 hands-on audit and round 2 REA static analysis) to learn how a feature behaves or is built before designing it, or extend the research with REA. Use when a task mentions CapCut parity, a CC-xxx feature id, or asks what users expect from a feature.
---

# Use the CapCut research

The research tells you what users expect and how a mature editor structures a feature. It is never a source of code or assets, and it is **private** ([ADR-0015](../../../docs/adr/0015-clean-room-policy.md)): the folders `docs/capcut-audit/` and `docs/capcut-rea/` are git-ignored and exist only in maintainers' checkouts.

## First, check it is there

```bash
ls docs/capcut-audit docs/capcut-rea
```

If the folders are missing, say so, and work from the ADRs, which summarize the findings each decision needs. Do not try to recreate or download the research.

## Where to look

| Question | Source (plain paths; never link to them from public files) |
|---|---|
| Does CapCut have feature X, is it free, what controls does it show? | `docs/capcut-audit/FEATURE-MATRIX.md` (ids `CC-001` to `CC-202`), `docs/capcut-audit/feature-inventory.json`, screenshots and `.txt` accessibility transcripts in `docs/capcut-audit/screenshots/` |
| What was actually tested and what happened? | `docs/capcut-audit/TEST-LOG.md` |
| How does CapCut's AI agent edit projects? | `docs/capcut-rea/AGENT-SUBSYSTEM.md` |
| How is the engine built (preview, export, GPU, codecs)? | `docs/capcut-rea/ARCHITECTURE.md` |
| How does CapCut store projects, time, masks, text? | `docs/capcut-rea/DRAFT-FORMAT.md` |
| Effects structure, adjustment order, blend modes, ML models, hidden features? | `docs/capcut-rea/RESOURCES.md` |

Search quickly:

```bash
grep -n -i "<feature words>" docs/capcut-audit/FEATURE-MATRIX.md docs/capcut-rea/*.md
python3 -c "import json;[print(r['id'],r['feature'],'|',r['access_label'],'|',r['behavior']) for r in json.load(open('docs/capcut-audit/feature-inventory.json'))['features'] if '<word>' in json.dumps(r).lower()]"
```

Each record in `features` has `id`, `category`, `feature`, `access`, `access_label`, `test`, `evidence`, `behavior` and `fosscut_priority`.

## How to use what you find

- Summarize expected behaviour in your own words in the issue, ADR or design notes, at the level the decision needs. Refer to the source as a plain path such as `docs/capcut-rea/DRAFT-FORMAT.md`, never as a Markdown link (CI's link check would fail on the public repository).
- Never quote screenshots, transcripts or personal details from the evidence in public files.
- Note the research limits: CapCut 9.6.0 on macOS, signed out, 28 October 2026; many features were inspected, not exercised.
- Design FossCut's version from its ADRs; CapCut's internals are a reference, not a template (its project format in particular is a list of things to do differently).

## Extending the research with REA

REA ([morluto/rea](https://github.com/morluto/rea)) needs Node.js `^22.23.4`, `^24.22.1` or `>=26.12.1`. Install it outside the repository, run read-only commands against the installed app, and write new findings to `docs/capcut-rea/` (private) with evidence and limitations. `docs/capcut-rea/REA-LOG.md` lists the commands that worked and their quirks. Rules:

- Inspect only; do not modify or launch the app unless the task explicitly scopes runtime observation.
- Open only synthetic test projects, never personal ones; redact device identifiers and local paths.
- Keep REA evidence bundles out of the repository (large, contain local paths).
- Decompiled output (with a Ghidra, Hopper or IDA provider) may only be turned into a behavioural specification; whoever reads it does not implement the matching code.
