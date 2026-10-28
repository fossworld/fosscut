# Contributing to FossCut

Thank you for helping build an open source video editor that people and AI agents can use together. FossCut is at an early stage: the research and architecture decisions exist, the code is starting. This is the best time to shape it.

## Ground rules

- Follow the [Code of Conduct](CODE_OF_CONDUCT.md).
- Read the [architecture decision records](docs/adr/README.md) before changing anything structural. Architecture changes start with an ADR pull request ([ADR-0001](docs/adr/0001-record-architecture-decisions.md)).
- Respect the [clean-room policy](docs/adr/0015-clean-room-policy.md): never add code, shaders, scripts, prompts, fonts, models, effect packages or other material taken from CapCut or any other product.
- Contributions are licensed under `MIT OR Apache-2.0` ([ADR-0014](docs/adr/0014-licensing.md)) and must be signed off under the [Developer Certificate of Origin](#developer-certificate-of-origin).

## Ways to contribute

- **Engine and app code** in Rust and TypeScript.
- **Agent skills** in `skills/`: editing know-how written as `SKILL.md` files, with evaluations ([ADR-0013](docs/adr/0013-agent-facing-editing-skills.md)). No Rust needed.
- **Effects, presets and text styles** reimplemented from public formulas, with licenses for every asset.
- **Test media and fixtures** you have the rights to share, with a license record.
- **Bug reports, design feedback and documentation.**

## Development setup

| Tool | Purpose |
|---|---|
| [rustup](https://rustup.rs) | Installs the toolchain pinned in `rust-toolchain.toml` automatically |
| `cargo install cargo-nextest cargo-deny` | Test runner and dependency policy checks used by CI |
| Node.js and pnpm | The shared UI in `ui/` (once it exists) |
| Python 3 | Repository check scripts |
| FFmpeg (shared libraries, LGPL build) | Media I/O once `fosscut-media` exists ([ADR-0009](docs/adr/0009-media-io-and-codecs.md)) |

Local checks that mirror CI:

```bash
cargo fmt --all
```

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

```bash
cargo nextest run --workspace --all-features --no-tests=pass
```

```bash
cargo deny check
```

```bash
python3 scripts/check-adr-index.py
```

```bash
npx markdownlint-cli2
```

Rust commands apply once the workspace has its first crate; the documentation checks apply now.

## Workflow

1. **Start from an issue.** Use the "Implementation task" template for well-scoped work with acceptance criteria. Comment that you are taking it.
2. **Branch, keep it small.** One concern per pull request. Large features are split into reviewable steps.
3. **Write tests at the right level** ([ADR-0016](docs/adr/0016-testing-strategy.md)): unit and property tests, command contract tests, snapshots, golden frames, export verification.
4. **Keep agents first-class.** A new capability is reachable from the CLI and MCP, appears in `capabilities`, and follows the [agent safety contract](docs/adr/0006-agent-safety-contract.md).
5. **Use a conventional pull request title**, for example `feat(core): add clip split command` or `fix(render): correct mask feather`. Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `revert`. Pull requests are squash-merged with this title, and release notes are generated from it.
6. **Sign off every commit** (`git commit -s`).
7. **CI must be green** and a maintainer must approve. Agents never merge.

## Working with AI coding agents

FossCut is built mostly with AI assistance ([ADR-0018](docs/adr/0018-ai-assisted-development-workflow.md)).

- [AGENTS.md](AGENTS.md) is the instruction file for every agent; `CLAUDE.md` points Claude Code to it. Development skills live in `.claude/skills/`.
- You are responsible for everything you submit, whoever typed it. Read and understand the diff before opening a pull request.
- Say in the pull request template whether and how AI tools were used.
- Never paste secrets, private media or personal data into an agent session that works on this repository.

## Developer Certificate of Origin

By signing off a commit you certify the [Developer Certificate of Origin 1.1](https://developercertificate.org): that you wrote the contribution or otherwise have the right to submit it under the project's license. Add the sign-off with `git commit -s`, which appends a line like:

```text
Signed-off-by: Your Name <you@example.com>
```

The name and email must match the commit author. To sign off commits you already made on a branch:

```bash
git rebase --signoff origin/main
```

A pull request check verifies the sign-off on every non-bot commit.

## Releases

Maintainers create a tag `vMAJOR.MINOR.PATCH` on `main`. The release workflow builds the `fosscut` CLI for Linux, macOS and Windows once the CLI crate exists, generates notes from conventional commits with [git-cliff](cliff.toml), and opens a draft GitHub release for review. Desktop installers join the same workflow when the desktop app exists ([ADR-0010](docs/adr/0010-desktop-shell-ui-and-web-path.md)).
