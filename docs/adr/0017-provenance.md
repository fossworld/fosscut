# ADR-0017: Transparent provenance, no hidden watermarks

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0006](0006-agent-safety-contract.md), [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), private research: `docs/capcut-rea/ARCHITECTURE.md`, `docs/capcut-rea/RESOURCES.md`

## Context

Some editors embed invisible watermarks or identifiers in exports, and users are not always told which exports carry which marks. Platforms increasingly ask creators to disclose AI-generated content, and C2PA is the open standard for this.

## Decision

1. **No hidden watermarks.** FossCut never embeds invisible identifiers or tracking marks in user exports.
2. **Optional C2PA Content Credentials** at export, using the open `c2pa` Rust crate, signed with a key the user controls. Off by default; the export dialog and API make the choice explicit.
3. **AI disclosure is recorded faithfully.** Assets produced by generation providers (ADR-0012) carry provenance in the project (provider, model, prompt, time). When C2PA is enabled, the manifest lists them as ingredients with the appropriate digital source type, so platforms can label AI content correctly.
4. **The operation log** ([ADR-0006](0006-agent-safety-contract.md)) records which edits were made by agents. It stays inside the project and is not embedded in exports unless the user opts in.

## Consequences

- Users can trust that FossCut exports contain only what they chose.
- Creators who need AI disclosure for platforms get a standards-based way to provide it.
- Key management for signing is the user's responsibility; FossCut provides clear setup guidance.

## Alternatives considered

- **Mandatory invisible watermarking:** contrary to user trust and unnecessary for an editor without a cloud service.
- **Server-side signing:** requires FossCut to operate infrastructure and identity.

## Validation

An export with C2PA enabled validates with standard C2PA verification tools; an export without it contains no FossCut-added metadata beyond standard container tags.
