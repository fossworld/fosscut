# ADR-0012: Bring your own brain; local analysis, optional generation providers

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0013](0013-agent-facing-editing-skills.md), [ADR-0017](0017-provenance.md), private research: `docs/capcut-rea/RESOURCES.md`, `docs/capcut-rea/AGENT-SUBSYSTEM.md`

## Context

In CapCut, the assistant and the generation features (image, video, voices, avatars) are cloud services gated by subscriptions, daily allowances and credits, while many analysis features (cutouts, tracking, scene and beat detection) work on the device. Round 1 found the resulting access rules hard to predict.

FossCut's users arrive with their own agent (Claude Code, Codex, Cursor), which already has a capable model and the user's account.

## Decision

1. **Reasoning belongs to the user's agent.** FossCut's core has no dependency on any language model or vendor account. The agent plans; FossCut executes and reports.
2. **Analysis primitives run locally** in `fosscut-analysis`, behind a feature flag and installed on demand:
   - speech recognition with word-level timestamps and language detection;
   - voice activity, pauses and filler-word detection (from the transcript);
   - beat and tempo tracking, loudness and silence detection;
   - shot and scene boundary detection;
   - face and salient-object detection for auto reframe, plus tracking;
   - subject segmentation (background removal) as a later addition.
   Models run through ONNX Runtime or a dedicated runtime where clearly better (for example a `whisper.cpp`-based recognizer), on CPU or GPU. Each model is described by a manifest: name, version, task, license, size, SHA-256, source URL. Models download only after the user or agent explicitly requests them; nothing is bundled by default.
3. **Analysis results are cached data**, keyed by asset content and model version, and exposed through queries (`media.transcript`, `media.beats`, `media.scenes`, `media.faces`) so agents and skills build edits on facts rather than guesses.
4. **Generation is optional and provider-based.** Image, video, voice and music generation are accessed through a provider interface with user-supplied credentials. Before any paid or remote call, FossCut returns an estimate (provider, model, units, approximate cost) and requires explicit confirmation (`confirm_cost: true`). Results become assets with provenance (provider, model, prompt, time) recorded in the project ([ADR-0017](0017-provenance.md)).
5. **MCP sampling as an option.** Where FossCut itself needs a language model (for example to title chapters inside a long job), it may request it from the connected client through MCP sampling, so it uses the user's agent and account instead of its own.
6. **Offline behaviour is explicit.** Every AI feature declares whether it is local, needs a model download, or needs a remote provider, and the `capabilities` query reports what is available right now.

## Consequences

- FossCut works fully offline for editing and analysis once models are installed, and costs nothing unless the user chooses a paid provider.
- No account system, credit ledger or server is required to ship.
- Quality of local models varies by language and hardware; the analysis layer must report confidence and let skills degrade gracefully.
- Model licenses must be checked one by one; some strong models are non-commercial and cannot be offered by default ([ADR-0014](0014-licensing.md)).

## Alternatives considered

- **Built-in assistant with a bundled or hosted model:** duplicates what the user's agent already does and creates cost and privacy obligations. Could exist later as an optional client of the same API.
- **Cloud-only analysis:** simpler to ship, but costly, slow for long footage and bad for privacy.
- **Bundling all models in the installer:** large downloads and license risk.

## Validation

On a fixture with speech, pauses, filler words and music: transcript word timings within 100 ms of reference for English, beat times within 50 ms, shot boundaries at known cuts, all offline on an M1-class Mac within real time.
