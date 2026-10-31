# Architecture decision records

Decisions that shape FossCut, written so humans and AI coding agents can follow them. The process is described in [ADR-0001](0001-record-architecture-decisions.md); new records start from [template.md](template.md).

All records are **Proposed** until a maintainer accepts them in a pull request. They draw on two rounds of private CapCut research, a hands-on audit and a static REA analysis, kept by maintainers in the git-ignored folders `docs/capcut-audit/` and `docs/capcut-rea/` ([ADR-0015](0015-clean-room-policy.md)). Research references in the records are plain paths into those folders and are not published.

| ADR | Decision | Status |
|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Proposed |
| [0002](0002-agent-first-command-api.md) | One typed command API with GUI, CLI and MCP front ends | Proposed |
| [0003](0003-rust-workspace-and-crate-layering.md) | Rust workspace and crate layering | Proposed |
| [0004](0004-open-project-format.md) | Open, versioned, agent-friendly project format | Proposed |
| [0005](0005-time-model.md) | Integer flicks for timeline time, exact rationals for rates | Proposed |
| [0006](0006-agent-safety-contract.md) | Agent safety contract for every write | Proposed |
| [0007](0007-timeline-projection-and-patches.md) | Compact timeline projection with plan and apply | Proposed |
| [0008](0008-render-and-audio-engine.md) | Processing graph with separate preview and export graphs on wgpu | Proposed |
| [0009](0009-media-io-and-codecs.md) | FFmpeg under LGPL plus platform encoders, behind backend traits | Proposed |
| [0010](0010-desktop-shell-ui-and-web-path.md) | Tauri desktop shell with a shared TypeScript UI, web build later | Proposed, spike required |
| [0011](0011-effects-and-animatable-properties.md) | Built-in effects with declarative parameters and a closed property vocabulary | Proposed |
| [0012](0012-ai-capabilities-bring-your-own-brain.md) | Bring your own brain; local analysis, optional generation providers | Proposed |
| [0013](0013-agent-facing-editing-skills.md) | Ship editing know-how as open agent skills | Proposed |
| [0014](0014-licensing.md) | Dual MIT or Apache-2.0 license, DCO, dependency license policy | Proposed, owner confirmation required |
| [0015](0015-clean-room-policy.md) | Clean-room policy for competitor research | Proposed |
| [0016](0016-testing-strategy.md) | Testing strategy: contracts, golden frames, export verification, agent evaluations | Proposed |
| [0017](0017-provenance.md) | Transparent provenance, no hidden watermarks | Proposed |
| [0018](0018-ai-assisted-development-workflow.md) | AI-assisted development workflow | Proposed |
| [0019](0019-local-analysis-pipeline-and-cache.md) | Run media analysis locally in fosscut-analysis, cached by content | Proposed |
| [0020](0020-speech-recognition-with-whisper-cpp.md) | Recognize speech locally with whisper.cpp, with word timings | Proposed |
| [0021](0021-captions-as-timed-word-clips.md) | Store captions as pages of timed words on caption tracks, animated by declarative animators | Proposed |
| [0022](0022-live-sessions-over-a-local-socket.md) | Host live sessions over a local socket with per-session keys | Proposed |
| [0023](0023-clip-speed-as-a-time-map.md) | Change clip speed with a time map from timeline time to source time | Proposed |
| [0024](0024-link-clips-in-groups-that-edit-together.md) | Link clips in groups that edit together | Proposed |
| [0025](0025-nest-timelines-in-clips-for-compound-clips.md) | Nest timelines in clips for compound clips | Proposed |
| [0026](0026-sound-effects-in-the-effect-registry.md) | Apply sound effects from the effect registry in the mixer | Proposed |
| [0027](0027-styled-text-with-project-fonts.md) | Compose styled text on the CPU with bundled and project fonts only | Proposed |
| [0028](0028-a-built-in-library-made-by-fosscut.md) | Make the built-in library from FossCut's own data and code | Proposed |
| [0029](0029-cut-multicam-angles-on-compound-clips.md) | Cut multicam angles on compound clips | Proposed |
| [0030](0030-apply-measured-motion-as-keyframes.md) | Apply measured motion as keyframes | Proposed |

## Reading order

- **Product shape:** 0002, 0006, 0007, 0013, 0022.
- **Data:** 0004, 0005, 0011, 0021, 0023, 0024, 0025, 0027, 0028, 0029.
- **Engine:** 0003, 0008, 0009, 0010, 0012, 0019, 0020, 0026, 0030.
- **Project rules:** 0001, 0014, 0015, 0016, 0017, 0018.

## Suggested first milestones

These follow from the records above and from the round 1 roadmap in the private research (`docs/capcut-audit/FOSSCUT-ROADMAP.md`):

1. **Foundations:** `fosscut-time`, `fosscut-model` and `fosscut-core` with the command framework and safety contract, CLI with `--json`, no rendering yet. Exit check: the round 1 edit sequence (import, title, split) runs as commands with undo, revisions and an operation log.
2. **Spikes:** preview path for ADR-0010 and the wgpu compositor for ADR-0008.
3. **First render loop:** decode, composite, export with verification. Exit check: the round 1 baseline export, verified by probing.
4. **MCP server and `fosscut-basics` skill.** Exit check: an agent reproduces the baseline through MCP alone.
5. **Desktop app** on the same engine, with live agent sessions.
6. **Analysis primitives and the first viral skills:** captions, talking-head cleanup, vertical reframe.
