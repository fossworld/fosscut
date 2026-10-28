---
name: golden-frame-tests
description: Create, update or debug FossCut golden-frame render tests and preview/export parity checks. Use when adding effects or compositing features, when a render test fails, or when goldens must be regenerated after an intended visual change.
---

# Golden-frame tests

Governing decision: [ADR-0016](../../../docs/adr/0016-testing-strategy.md), items 4 and 5.

## Writing a test

1. Build the timeline in code from generated fixtures (FFmpeg `lavfi` test patterns, solid colors, gradients, generated text). Never use downloaded or copyrighted media.
2. Choose timestamps that exercise the feature: the first frame, a keyframe, between keyframes, a transition midpoint, the last frame.
3. Render at a small resolution (for example 320x180) unless the feature depends on resolution.
4. Compare against the stored golden PNG with a perceptual metric and an explicit threshold (for example PSNR above 40 dB or SSIM above 0.99). Choose the loosest threshold that still catches a real regression, and comment why.
5. Add a parity check: the same frame from the preview evaluation path and from a decoded export must match within tolerance.

## Creating or updating goldens

Goldens change only when the visual change is intended.

1. Run the update task (`cargo xtask goldens --update <test filter>`; implement it in `xtask/` if missing). It writes new PNGs and a side-by-side diff image to a temporary folder.
2. Open and inspect every diff image. Describe in the PR what changed visually and why it is correct.
3. Keep PNGs small; if the golden set grows past a few megabytes, propose Git LFS in an issue.

## Debugging a failure

1. Look at the diff image first, not the metric.
2. Check whether the failure is GPU-specific (different adapter or driver in CI). If so, prefer fixing precision issues in the shader; add a per-backend golden only with a maintainer's agreement.
3. Check color handling: transfer function, matrix and range of the fixture, premultiplied alpha, working space.
4. Check time: does the frame index map to the expected flicks for the fixture's rate?

## Never

Raise a threshold, delete a golden or mark a test ignored just to make CI pass.
