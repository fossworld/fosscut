---
name: clean-room-check
description: Review a FossCut change for material copied or derived from CapCut or other proprietary products, and for unlicensed assets. Use before submitting changes to effects, captions, presets, project format, skills, prompts or assets, and when reviewing others' pull requests.
---

# Clean-room check

Governing decisions: [ADR-0015](../../../docs/adr/0015-clean-room-policy.md) (clean room), [ADR-0014](../../../docs/adr/0014-licensing.md) (licenses).

## What is allowed

- Behaviour and structure described in the private research (`docs/capcut-audit/`, `docs/capcut-rea/`, git-ignored): what a feature does, its parameters, its order of operations, its user-facing results. Summaries may appear in public docs; the research files themselves and links to them may not.
- Public specifications, standards, papers and formulas.
- FossCut's own experiments and measurements.

## What must never appear

- Code, pseudocode, shaders, scripts, prompts, UI strings or comments from CapCut or other proprietary products.
- Their models, effect or template packages, LUT files, fonts, icons, sounds, stock media or screenshots used as product assets.
- Identifiers that only make sense as copies: vendor catalog ids, internal type, module, file and package names, obfuscated kernel names. Maintainers keep the list of known vendor identifiers in the private research (`docs/capcut-rea/CLEAN-ROOM-MARKERS.txt`, one case-insensitive pattern per line).

## Procedure

1. List added files and binary assets:

   ```bash
   git diff --stat origin/main...HEAD
   git diff --name-only --diff-filter=A origin/main...HEAD
   ```

2. Search the diff for vendor markers:

   ```bash
   git diff origin/main...HEAD | grep -niE 'capcut|jianying|bytedance' || true
   ```

   If the private marker list exists in your checkout, also run:

   ```bash
   git diff origin/main...HEAD | grep -niEf docs/capcut-rea/CLEAN-ROOM-MARKERS.txt || true
   ```

   Plain-path research references and summaries in ADRs are fine; hits in code, shaders, skills or assets need an explanation or removal.

3. Confirm nothing from the private research is staged and no Markdown link points into it:

   ```bash
   git ls-files docs/capcut-audit docs/capcut-rea
   git diff origin/main...HEAD | grep -nE '\]\([^)]*capcut-(audit|rea)/' || true
   ```

   Both must print nothing.

4. For every new binary or data asset (images, fonts, LUTs, audio, video, models), confirm a license record exists (source, license, author) and that the license is on the allowed list in ADR-0014.
5. For shaders and algorithms, confirm the PR cites the public source of the formula.
6. For skills and prompts in `skills/`, confirm the text was written for FossCut and does not paraphrase vendor skill texts closely.
7. For decompiler-derived knowledge (if REA with a decompiler was ever used), confirm the implementer did not read the decompiled output and worked from a behavioural specification.

## Outcome

Report "clean" with what you checked, or list each concern with file and line and the fix required. When in doubt, remove the material and reimplement from first principles.
