# ADR-0030: Apply measured motion as keyframes

- **Status:** Proposed
- **Date:** 2026-10-30
- **Deciders:** maintainers
- **Related:** [ADR-0011](0011-effects-and-animatable-properties.md) (item 3, keyframes), [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), [ADR-0019](0019-local-analysis-pipeline-and-cache.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (stabilization, mask tracking, text motion tracking)

## Context

Editors steady shaky footage and make masks, titles and stickers follow a moving subject. The research shows stabilization with strength settings (export blocked behind a paid plan), mask tracking with a direction choice, and motion tracking for text; tracking was not run.

The work behind them is measurement: how the camera moved, or where a region went, frame by frame. FossCut already measures media locally and deterministically (ADR-0019), with no models in the core (ADR-0012). It already animates any numeric clip and mask property with keyframes that people and agents can read and change (ADR-0011 item 3).

## Decision

1. **Measure in `fosscut-analysis`.**
   - **`motion`:** estimates the camera's motion between frames by block matching on a downscaled pyramid, with a robust similarity fit. It accumulates a camera path and smooths it with a Gaussian scaled by strength into corrections, plus the one zoom that hides the moving edges.
   - **`track`:** follows a region by normalized cross-correlation template matching with slow template updates, and reports when it loses it.

   Both are classical and deterministic.
2. **Apply as keyframes.** The engine turns measurements into keyframes of existing properties, one per frame of the clip, through `keyframe.set_all`. That command replaces a property's keyframes in one undoable step.
   - **Stabilization:** sets the clip's `transform.position`, `transform.rotation` and `transform.scale`.
   - **Tracking:** sets a mask's `center` (mask tracking), or another clip's `transform.position`, so a title or sticker follows the subject (motion tracking).
3. **Nothing new to render.** Preview and export already evaluate keyframes, so stabilized and tracked clips render identically everywhere, and people can adjust or remove the motion like any animation.

## Consequences

- One mechanism serves stabilization, mask tracking and motion tracking, and the result is visible, editable data rather than a hidden effect.
- Clips gain many keyframes (one per frame per property). Storage and `timeline.read` grow with clip length; readers should use `detail` levels.
- Changing a stabilized clip's trim keeps the keyframes where they were in the clip; stabilizing again replaces them.
- Rolling shutter and lens distortion are not corrected; strong motion may need more zoom than the 1.5 cap allows.

## Alternatives considered

- **A stabilize effect that warps on the GPU from a stored path.** Deferred: it would need its own storage and a warp pass, and hide the motion from agents. Keyframes can later feed such an effect if needed.
- **Feature tracking with a model.** Rejected for now (ADR-0012); template matching covers the common cases of a face, product or object moving across the frame.

## Validation

- **Analysis tests:** synthetic frames with known motion are recovered within a third of a pixel. Shaky paths lose at least 80% of their jitter while pans are kept, and moving squares are tracked within 1.5 pixels and reported lost when covered.
- **Engine tests:** stabilizing and tracking generated media produce keyframes that cancel the motion and follow the region.
- **Before Accepted:** a person reviews stabilization on handheld footage, and a real-agent check makes a title follow a moving subject.
