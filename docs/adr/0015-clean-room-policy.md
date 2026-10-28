# ADR-0015: Clean-room policy for competitor research

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0014](0014-licensing.md), private research: `docs/capcut-audit/README.md`, `docs/capcut-rea/README.md`

## Context

FossCut studies CapCut through hands-on audits and static inspection with REA. The research documents structure and behaviour, but the inspected application contains copyrighted code and assets. AI coding agents may also be tempted to reproduce text they have seen. Copying any of it would expose the project legally and taint its license. The research evidence also contains personal data from the maintainer's machine (project thumbnails, local paths), and publishing detailed reverse-engineering notes adds risk without helping users.

## Decision

1. **Allowed sources of knowledge:** observed behaviour (what the app does, with which parameters and results), public specifications and standards, published formulas, the private research reports, and FossCut's own experiments.
2. **Never in the repository or in releases:** CapCut code or pseudocode, shaders, scripts, prompts, models, effect or template packages, fonts, icons, sounds, stock media, or screenshots of third-party copyrighted assets used as product content.
3. **Research stays private.** It lives in the git-ignored folders `docs/capcut-audit/` and `docs/capcut-rea/` on maintainers' machines, or in a separate private repository when it must be shared. It is never committed to the public repository, and CI fails if it is. Public documents (ADRs, issues, pull requests, skills) may summarize findings at the level a decision needs and refer to the research by plain path, never by link, and never paste vendor material.
4. **Decompilation boundary.** If deeper reverse engineering (for example decompiled functions through REA with Ghidra) is ever needed, the person or agent session that reads decompiled output writes only a behavioural specification (inputs, outputs, formulas in their own words, test vectors); a different person or session implements from that specification.
5. **Interoperability is legitimate.** Importing CapCut projects or matching its export behaviour for user migration is allowed, implemented from the documented format and behaviour, in a separate crate.
6. **Agents are instructed** in `AGENTS.md` and the `clean-room-check` development skill to refuse copying vendor material and to flag suspected contamination in review.
7. **Personal data in research** (project thumbnails, local paths, account details) is redacted before research is shared with anyone, including collaborators through a private repository.

## Consequences

- Contributors can use the research freely for design while the code base stays clean.
- Some exact visual matches (a particular effect look) will take longer, because they must be rebuilt from first principles.

## Alternatives considered

- **No research at all:** slower design, more guesswork about what users expect.
- **Unrestricted use of inspected assets:** legally unacceptable for an open source project.

## Validation

Pull request template confirms compliance; reviewers and the `clean-room-check` skill look for vendor identifiers, copied comments and suspicious binary assets; CI rejects any tracked file under the research folders, and the link check rejects links into them.
