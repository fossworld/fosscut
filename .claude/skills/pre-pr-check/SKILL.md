---
name: pre-pr-check
description: Run FossCut's local quality gate before opening or updating a pull request. Use whenever a change is ready for review, after rebasing, or when CI failed and you need to reproduce it locally.
---

# Pre-PR check

Run the same checks CI runs, fix what fails, and report the results in the pull request's Verification section.

## 1. Find what changed

```bash
git status --short
git diff --stat origin/main...HEAD
```

Decide which groups below apply. Documentation-only changes still run the docs group.

## 2. Docs and ADRs (always)

```bash
python3 scripts/check-adr-index.py
npx markdownlint-cli2
```

If `lychee` is installed, also run `lychee --offline --include-fragments './**/*.md'` to catch broken relative links and anchors.

## 3. Rust (when `Cargo.toml` exists)

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
FOSSCUT_REQUIRE_FFMPEG=1 cargo nextest run --workspace --all-features
cargo test --workspace --all-features --doc
cargo xtask coverage
cargo doc --workspace --no-deps --all-features
cargo deny check
```

For changes in `fosscut-time`, `fosscut-model` or `fosscut-core`, also build the portable crates for the web target:

```bash
cargo build -p fosscut-time -p fosscut-model -p fosscut-core --target wasm32-unknown-unknown
```

Run only the crates that exist. Install missing tools with `cargo install cargo-nextest cargo-llvm-cov` and `rustup component add llvm-tools`. Drop `FOSSCUT_REQUIRE_FFMPEG=1` only if FFmpeg is not installed, and say in the PR that the end-to-end tests were skipped locally.

`cargo xtask coverage` prints line coverage per crate and fails below the minimums in `xtask/src/main.rs`. If your change lowers coverage, add tests for the new code; to find uncovered lines, run `cargo llvm-cov nextest --workspace --exclude xtask --open`. Report the coverage table in the PR's Verification section.

## 4. Generated artefacts

If public types, commands or the project format changed, regenerate and review:

- JSON Schemas in `schemas/` (`cargo xtask schemas`),
- snapshot tests (`cargo insta review` when `insta` snapshots changed),
- golden frames only through the `golden-frame-tests` skill.

A diff in `schemas/` or snapshots must be explained in the PR description.

## 5. UI (when `ui/package.json` exists)

From `ui/`:

```bash
pnpm install --frozen-lockfile
pnpm run lint
pnpm run typecheck
pnpm run test
```

## 6. Pull request hygiene

- Title follows conventional commits; every commit is signed off (`git log --format='%h %s%n%b' origin/main..HEAD | grep -c Signed-off-by`).
- One concern per PR; split if the diff mixes refactoring with features.
- PR template filled in, including ADR references and AI-assistance disclosure.
- Run the `clean-room-check` skill if the change touches effects, formats, captions, presets or anything inspired by the research.

## Rules

- Never silence a failing check to get green: no `#[allow]` without a justification comment, no skipped or `#[ignore]`d tests, no retries, no lowered coverage minimums.
- Every behaviour change and bug fix has a test that fails without it. Check this by reverting the fix locally or reasoning it through, and say how in the PR.
- A PR cannot merge while the `CI result` check fails; it covers every test, lint, coverage and policy job.
- If a check fails for reasons unrelated to your change, say so in the PR and open an issue instead of fixing it silently in the same PR.
