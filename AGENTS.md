# AGENTS.md

Instructions for AI coding agents (Claude Code, Codex, Cursor and others) working on FossCut. Humans should read it too. If anything here conflicts with an accepted ADR, the ADR wins and this file needs fixing.

## What FossCut is

An open source video editor written in Rust, built so that people create high-quality short-form and long-form videos by talking to an AI agent, and can keep editing the same project in a desktop app (a web version may follow). The editor exposes one typed command API through three front ends: a GUI, a CLI with `--json` output, and an MCP server ([ADR-0002](docs/adr/0002-agent-first-command-api.md)).

**Status:** milestone 1 (foundations) exists: `fosscut-time`, `fosscut-model`, `fosscut-core` and the `fosscut` CLI. Milestone 2 (render and preview spikes) is done on macOS: `fosscut-media` decodes video through FFmpeg 8 with VideoToolbox, `fosscut-render` composites frames on wgpu with a linear-light color pipeline, and the preview spike in `spikes/preview` chose rendering into the window behind the webview ([results](docs/development/milestone-2-spike-results.md)). Milestone 3 (first render loop) is done: `fosscut-media` encodes, muxes and probes outputs, `fosscut-render` draws text layers with the bundled Inter font, `fosscut-audio` mixes timeline audio, and `fosscut-engine` exports with verification through `fosscut export`. Milestone 4 (MCP server and `fosscut-basics` skill) is done: `fosscut-engine` hosts projects for every front end, renders frame grabs and contact sheets (`fosscut frame`, `fosscut contact-sheet`) and runs exports as background jobs, `fosscut-mcp` serves coarse tools to any MCP client as `fosscut mcp`, and `skills/fosscut-basics` teaches agents the safe editing loop, with an evaluation replayed in CI and `fosscut skills install` to add it to an agent. Milestone 6 (analysis and the first viral skills) is done: `fosscut-analysis` measures loudness, silences and shot cuts, cached by content outside the project ([ADR-0019](docs/adr/0019-local-analysis-pipeline-and-cache.md)), and transcribes speech with word timings, filler candidates and pauses through whisper.cpp ([ADR-0020](docs/adr/0020-speech-recognition-with-whisper-cpp.md)) once a model is installed with `fosscut models install` or the MCP `models` tool, all reachable as `fosscut media analyze` and the MCP `media_analyze` tool. Captions are done: caption tracks hold pages of timed words animated by presets ([ADR-0021](docs/adr/0021-captions-as-timed-word-clips.md)), generated from transcripts or imported and exported as SRT through `fosscut captions` and the MCP `captions` tool, with the `viral-captions` skill. `timeline.remove_ranges` cuts time ranges out of every track with ripple (the `talking-head-cleanup` skill), and `timeline.duplicate` with `clip.set` makes vertical versions framed shot by shot (the `vertical-reframe` skill). Milestone 5 (desktop app) is done: live sessions ([ADR-0022](docs/adr/0022-live-sessions-over-a-local-socket.md)) let a host hold a project open as its only writer while the CLI and MCP reach it over a local socket; `fosscut_engine::project::Projects` routes every call by the bundle's `.writer.lock`. The desktop app (`apps/desktop`, Tauri, with the React UI in `ui/`) hosts the project it opens, so agents' edits appear in it live; it previews with wgpu behind the webview (`fosscut_engine::preview`), and shows a canvas timeline, the agents connected and the history by who made each change. Playback follows the sound; clips move and trim by dragging (`clip.trim` moves an edge), an inspector sets placement, opacity and volume, and split, delete, undo and redo work from the keyboard. Editing depth is under way (format 0.3.0): clips have a closed property vocabulary (ADR-0011 item 3) with crop, mirror, non-uniform scale, 14 blend modes, fades, pan, fill channel and deactivation, and `keyframe.set` animates any numeric property with hold, linear or Bezier easing. Clips carry effects from a registry published by `capabilities` (format 0.4.0): color adjustments, curves, HSL, wheels, LUTs from `.cube` assets, vignette, blur, sharpen and grain, rendered by `fosscut-render`'s effect passes; adjustment layers grade everything below them, built-in looks are presets of those effects, and `fosscut_engine::color` corrects or matches a clip's color from measured frames. Masks (format 0.5.0: shapes, pen paths, text and brush strokes, combined and keyframed) are rasterized by `fosscut_render::mask`, and `key.chroma` removes a green or blue screen. Transitions over cuts and clip animations (format 0.6.0) evaluate in `fosscut_render::plan`, which renders both sides of a transition and combines them in one pass, with stylizing and moving effects such as glitch, VHS and shake. Clip speed (format 0.7.0, [ADR-0023](docs/adr/0023-clip-speed-as-a-time-map.md)) maps timeline time to source time for frames, sound (resampled or WSOLA-stretched in `fosscut-audio`), splits and captions, with `clip.speed` and `clip.freeze`. Timelines have markers, and clips link in groups that move, split, trim and are removed together, including sound extracted from a video clip (format 0.8.0, [ADR-0024](docs/adr/0024-link-clips-in-groups-that-edit-together.md)); `clips.copy` and `clips.paste` make a clipboard of clip documents. The desktop adds editor tools (blade, snapping, skimming, shuttle through the preview's `Job::Shuttle`, full screen, rulers and guides). The media bin has folders, renaming, relinking of moved files and replacement (format 0.9.0), projects hold several timelines (`timeline.add`, `timeline.set_primary`), and the desktop shows a media panel, timeline tabs, and with no project open a project browser with a trash. Compound clips show other timelines of the project (format 0.10.0, [ADR-0025](docs/adr/0025-nest-timelines-in-clips-for-compound-clips.md)), rendered by nested plans in `fosscut-render` and mixed through `fosscut-audio`. Text (format 0.11.0, [ADR-0027](docs/adr/0027-styled-text-with-project-fonts.md)) has outlines, boxes, shadows, glows, spacing, case, spans and reveals composed by `fosscut_render::text`, project fonts imported as font assets, saved text presets and title templates (`text.set`, `text.preset_save`, `text.template`). Sound effects (`audio.denoise`, `audio.enhance_voice`, `audio.vocals`, [ADR-0026](docs/adr/0026-sound-effects-in-the-effect-registry.md)) come from the effect registry and run in `fosscut_audio::effects`; `fosscut_engine::loudness` levels clips (`fosscut audio normalize`, the MCP `audio_normalize` tool); caption pages take translations for bilingual captions, and caption generation can skip fillers and caption lyrics. A built-in library made by FossCut ([ADR-0028](docs/adr/0028-a-built-in-library-made-by-fosscut.md), format 0.12.0) adds shape clips for shapes, stickers and backgrounds (`shape.add`, `shape.set`, drawn by `fosscut_render::shape`), project templates (`template.apply`), and sound effects and music loops synthesized by `fosscut_audio::synth`, all listed by `fosscut_engine::library` (`fosscut library`, the MCP `library` tool). Exports take a resolution, frame rate, bit rates and a range, write cover images and subtitles beside the file, report rights notes, and add ProRes profiles, GIF, WAV, FLAC and MP3. Multicam clips (format 0.13.0, [ADR-0029](docs/adr/0029-cut-multicam-angles-on-compound-clips.md)) are compound clips that show one `angle` of a timeline holding each camera on its own track, lined up by their sound (`fosscut_engine::motion::sync_offsets`) and cut with `multicam.switch`; `fosscut_engine::scenes` splits clips at shot cuts. Stabilization and region tracking are measured by `fosscut-analysis` (`motion`, `track`) and land as keyframes through `keyframe.set_all` ([ADR-0030](docs/adr/0030-apply-measured-motion-as-keyframes.md)), reachable as `fosscut clip split-scenes|stabilize|track`, `fosscut multicam` and the MCP `auto_edit` tool. `fosscut_media::capture` records microphones, cameras and screens through FFmpeg input devices, and `fosscut_engine::record` runs recordings into the bundle's `recordings/` folder. User preferences live outside projects in `preferences.json` ([ADR-0031](docs/adr/0031-keep-user-preferences-outside-projects.md), `fosscut_engine::preferences`, `fosscut preferences`): front ends fill requests from them, while commands never read them. Proxies, remuxed copies and rendered preview frames are caches by content that only the preview uses ([ADR-0032](docs/adr/0032-keep-proxies-remuxes-and-rendered-frames-as-caches.md), `fosscut_engine::{proxy, remux, frame_cache, cache}`, `fosscut media proxy|remux`, `fosscut cache`), and clips take a canvas fill (format 0.14.0, `clip.canvas`). Local models run through ONNX Runtime ([ADR-0033](docs/adr/0033-run-local-vision-and-voice-models-with-onnx-runtime.md), `fosscut_analysis::onnx`, installed only on request with `fosscut models`): the `pose` and `matte` analyzers feed the `key.subject` and `body.*` effects, `fosscut_engine::voice` speaks text with Kokoro and CMUdict pronunciation (`fosscut speak`, the MCP `record` tool), and `fosscut_engine::captions` translates captions between English and Hungarian with OPUS-MT and dubs them (`fosscut captions translate|dub`). `spikes/` holds throwaway prototypes outside the workspace and its gates. All ADRs are *Proposed* until accepted by a maintainer.

## Read before you work

1. [docs/adr/README.md](docs/adr/README.md): the decisions and the suggested milestones. Read the ADRs that touch your task in full.
2. The development skill for your task in `.claude/skills/` (list below).
3. Competitor research is private ([ADR-0015](docs/adr/0015-clean-room-policy.md)). Maintainers keep it in the git-ignored folders `docs/capcut-audit/` (what CapCut does, feature ids `CC-001` to `CC-202`) and `docs/capcut-rea/` (how it is built). If they exist in your checkout, consult them only when they inform behaviour, through the `capcut-research` skill; if they do not, work from the ADRs, which summarize what matters. Never copy from the research into code, and never commit or link to it.

## Repository map

| Path | Contents |
|---|---|
| `crates/` | Rust crates, layered as in [ADR-0003](docs/adr/0003-rust-workspace-and-crate-layering.md): `fosscut-time` → `fosscut-model` → `fosscut-core` → `fosscut-media`, `fosscut-render`, `fosscut-audio`, `fosscut-analysis` → `fosscut-engine` → `fosscut-mcp`, `fosscut-cli` |
| `xtask/` | Repository automation (`cargo xtask <task>`): schemas, goldens, fixtures, checks |
| `apps/desktop/` | Tauri desktop shell ([ADR-0010](docs/adr/0010-desktop-shell-ui-and-web-path.md)) |
| `ui/` | Shared TypeScript and React front end |
| `skills/` | User-facing agent skills for editing ([ADR-0013](docs/adr/0013-agent-facing-editing-skills.md)) |
| `schemas/` | Generated JSON Schemas; never edit by hand |
| `fixtures/` | Generators for test media and small test projects |
| `docs/adr/` | Architecture decision records |
| `docs/capcut-audit/`, `docs/capcut-rea/` | Private competitor research, git-ignored; present only in maintainers' checkouts |
| `.claude/skills/` | Development skills for agents working on this repository |
| `.github/` | CI, release and dependency workflows, issue and PR templates |

Directories that do not exist yet are created by the first task that needs them, following ADR-0003.

## Architecture rules

- **Every state change is a command** in `fosscut-core`; every read is a query. GUI, CLI and MCP never mutate a project any other way. A capability is not done until it is reachable from CLI and MCP and listed by `capabilities` (ADR-0002).
- **Every write honours the safety contract:** `expected_revision`, `dry_run`, `request_id` idempotency, atomic batches, typed `fosscut.<area>.<reason>` errors, destructive guards, operation log (ADR-0006). Use the command framework; do not reimplement these per command.
- **Time is `i64` flicks**; rates are exact rationals; all time math goes through `fosscut-time`; API inputs require explicit units (ADR-0005).
- **Project format rules:** versioned schema with migrations, typed nested ownership, one immutable asset table, sparse documents, short stable ids, semantic effect parameters with explicit `enabled`, no machine-specific or telemetry data (ADR-0004).
- **Crate dependencies point downward only.** `fosscut-time`, `fosscut-model` and `fosscut-core` must build for `wasm32-unknown-unknown` with no native libraries.
- **Rendering uses `wgpu` and WGSL.** No OpenGL. Preview and export share the frame evaluation function (ADR-0008).
- **Media I/O goes through backend traits;** FFmpeg is linked dynamically and must not pull in GPL or nonfree components. Never substitute a codec silently; validate settings and verify exports (ADR-0009).
- **Effects are registered types with parameter schemas;** animatable properties come from a closed, generated vocabulary (ADR-0011).
- **No language model inside the core.** The user's agent reasons; FossCut executes. Remote AI providers need explicit cost confirmation (ADR-0012).

## Commands

Run the checks that apply to what you changed. CI runs all of them.

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace --all-features
cargo test --workspace --all-features --doc
cargo xtask coverage
cargo deny check
cargo xtask schemas --check
python3 scripts/check-adr-index.py
npx markdownlint-cli2
```

`cargo xtask coverage` needs `cargo-llvm-cov` and the `llvm-tools` component; it fails below 90% line coverage for the workspace or 85% for any crate, and writes `target/coverage/lcov.info`. Tests that need FFmpeg skip locally when it is missing but fail in CI, which sets `FOSSCUT_REQUIRE_FFMPEG=1`; set it yourself when you have FFmpeg installed. Building `fosscut-media` needs FFmpeg 8 development libraries; CONTRIBUTING.md lists the variables per platform. GPU tests in `fosscut-render` skip without an adapter locally and fail in CI, which sets `FOSSCUT_REQUIRE_GPU=1`. The default `speech` feature of `fosscut-analysis` builds whisper.cpp, which needs CMake and a C++ compiler. Speech tests need `whisper-tiny.en` in `FOSSCUT_MODELS_DIR` (`fosscut models install whisper-tiny.en --yes` puts it in the cache folder; point the variable there); they skip without it locally and fail in CI, which sets `FOSSCUT_REQUIRE_SPEECH=1`. `cargo xtask goldens [--update]` runs or rewrites the golden frames; follow the `golden-frame-tests` skill.

After changing public model or command types, regenerate the JSON Schemas with `cargo xtask schemas` and commit `schemas/`. `cargo xtask fixtures` writes test media to `fixtures/generated/` (needs FFmpeg). Snapshot tests use `insta`; record new snapshots with `INSTA_UPDATE=always cargo test -p <crate>` and review the `.snap` diff.

To try the editor from the command line (ffprobe is needed for `media import`, a wgpu adapter for `export`):

```bash
cargo run -q -p fosscut-cli -- project new Demo.fosscut
cargo run -q -p fosscut-cli -- -p Demo.fosscut media import fixtures/generated/test-pattern.mp4
cargo run -q -p fosscut-cli -- -p Demo.fosscut clip add --asset asset_0001
cargo run -q -p fosscut-cli -- -p Demo.fosscut timeline read --json
cargo run -q -p fosscut-cli -- -p Demo.fosscut frame 1s --out still.png --width 640
cargo run -q -p fosscut-cli -- -p Demo.fosscut export Demo.mp4
cargo run -q -p fosscut-cli -- -p Demo.fosscut mcp    # MCP on stdio, for an agent
```

For the UI, from `ui/`: `pnpm install`, then `pnpm run lint`, `pnpm run typecheck`, `pnpm run test`. `ui/package.json` declares `packageManager` and `engines.node`, which CI reads. The desktop app runs with `pnpm --dir ui dev` and then `cargo run -p fosscut-desktop -- <project>`; UI code reaches the engine only through `ui/src/engine` (ESLint enforces it), and in a plain browser `pnpm dev` shows a sample project on a stand-in engine.

## Conventions

- **Rust:** edition 2024, toolchain from `rust-toolchain.toml`. Library crates return typed errors (`thiserror`); binaries may use `anyhow`. No `unwrap`/`expect` in library code outside tests unless an invariant is documented next to it. No `unsafe` outside backend modules of `fosscut-media` and `fosscut-render`: the workspace denies `unsafe_code`, other crates `#![forbid(unsafe_code)]`, and a backend module opts in with `#![allow(unsafe_code)]` while each block carries a `// SAFETY:` comment. Public items have doc comments. Public model and command types derive `serde` and `schemars`.
- **Tests:** follow [ADR-0016](docs/adr/0016-testing-strategy.md). Tests are the main safeguard in an AI-written codebase: every behaviour change and bug fix comes with a test that fails without it, and assertions check results, not just that code runs. New commands get contract tests; new effects get golden frames; format changes get round-trip and migration tests; API output changes update snapshots deliberately; CLI behaviour gets end-to-end tests. A pull request cannot merge while any CI job fails (the required `CI result` check) or while coverage is below the minimums.
- **Fixtures:** generate media with scripts (FFmpeg `lavfi` generators); commit only small files with a license record.
- **Commits and PRs:** conventional titles (`feat(core): …`), `git commit -s` sign-off, one concern per PR, fill in the PR template including AI-assistance disclosure.
- **Docs:** update the relevant ADR, schema docs or skill in the same PR as the behaviour change.

## Development skills

| Skill | Use it when |
|---|---|
| `pre-pr-check` | Before opening or updating any pull request |
| `write-adr` | A change affects architecture, formats, public contracts, licensing or AI providers |
| `add-command` | Adding or changing an editing command, query or MCP tool |
| `add-effect` | Adding or changing an effect, transition, mask or adjustment |
| `change-project-format` | Any change to stored project data or its schema |
| `golden-frame-tests` | Adding, updating or debugging render tests |
| `write-editing-skill` | Writing or changing a user-facing skill in `skills/` |
| `clean-room-check` | Reviewing a change for material copied from other products |
| `capcut-research` | Checking how CapCut behaves or is built before designing a feature |

## Never

- Copy code, shaders, scripts, prompts, skill texts, fonts, models, effect packages, icons or media from CapCut or any other product.
- Commit anything under `docs/capcut-audit/` or `docs/capcut-rea/`, or add Markdown links to them; refer to research as plain paths only.
- Add GPL, AGPL or non-commercial dependencies, models or assets.
- Edit generated files in `schemas/` by hand, or update golden images without reviewing the image diff.
- Weaken, skip, ignore or delete tests, add retries, or lower the coverage minimums to make CI pass. If a test is wrong, fix it and explain why in the pull request.
- Change an accepted ADR's decision in place; write a superseding ADR.
- Commit secrets, API keys, personal media or personal data; read provider keys only from the environment or OS keychain.
- Merge pull requests or push to `main`.
