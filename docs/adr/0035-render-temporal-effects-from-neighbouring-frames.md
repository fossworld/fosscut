# ADR-0035: Render temporal effects from neighbouring frames

- **Status:** Proposed
- **Date:** 2026-10-31
- **Deciders:** maintainers
- **Related:** [ADR-0008](0008-render-and-audio-engine.md), [ADR-0011](0011-effects-and-animatable-properties.md), [ADR-0023](0023-clip-speed-as-a-time-map.md), [ADR-0004](0004-open-project-format.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (optical flow, motion blur, reduce image noise, remove flickers)

## Context

Four picture effects need more than the frame being drawn:

- **Optical-flow slow motion:** smooth slow motion invents in-between frames from the motion between source frames.
- **Motion blur:** blurs along that motion.
- **Noise reduction:** averages a pixel with its matches in the neighbouring frames.
- **Flicker removal:** evens out brightness between frames.

The research shows each as an entry behind a paid plan. FossCut renders preview and export from one frame evaluation (ADR-0008). Smooth slow motion already mixes the next source frame into the current one (`Interpolate::Blend`, ADR-0023 item 8).

## Decision

1. **Layers may carry neighbouring frames.** The planner gives a layer that needs them its previous and next source frames. The frame sources of the engine supply them for export, frame grabs and the preview, decoding forward without seeking back for each frame. Preview and export of a frame stay identical.
2. **Optical flow in the renderer.** Dense optical flow between two source frames is estimated coarse to fine, classically and deterministically. Only the renderer uses it, and it is never stored in the project.
3. **Slow motion by flow.** `Interpolate::Flow` (format 0.15.0) warps both neighbouring frames toward the in-between time, blends them, and handles occlusions by forward-backward consistency.
4. **Effects in the registry** (ADR-0011):
   - **`motion.blur`:** blurs along the optical flow and along the clip's own animated movement.
   - **`detail.denoise`:** edge-preserving spatial smoothing, and optionally motion-compensated averaging with the neighbouring frames.
   - **`color.deflicker`:** scales a frame's brightness toward its neighbours' smoothed level.
5. **Determinism.** Any randomness uses integer hashes, and results do not depend on the GPU adapter beyond the golden tolerance (ADR-0016).

## Consequences

- Smooth slow motion, motion blur, denoising and deflickering render the same in preview and export, with no stored side data to keep in sync.
- These layers cost more to render: up to three source frames and a flow estimate each.
- Format 0.15.0: older builds cannot open projects that use `Interpolate::Flow`.

## Alternatives considered

- **Flow from a model** (for example RIFE): better on hard motion, but a download, a runtime and a license check (ADR-0033) for one effect. Classical flow first.
- **Precomputed flow as a cached analysis:** this is faster to render, but adds a cache that must follow every trim and speed change. It can come later if speed requires it.

## Validation

- **Golden frames:** for each effect.
- **Flow interpolation:** it is measurably closer to a rendered true midpoint than blending (PSNR).
- **Noise:** variance drops while edges stay sharp.
- **Flicker:** brightness variation drops by 80% while fades are kept.
- **Parity:** preview and export render the same frames.
- **Before Accepted:** a person reviews slow motion and denoising on real footage.

## Implementation notes

- **Planner:** a layer carries `flow`, its `neighbours` (the source frames before and after, nearest first, with the source frames per timeline frame) and its `shutter` placements (the animated transform at `samples` moments while the shutter is open). The frame cache key covers them (`CACHE_KEY_VERSION` 2).
- **Frame sources:** `DecoderFrames` keeps the frames it decoded most recently, up to 256 MiB, so neighbours are decoded once and never by seeking back. The preview keeps none there, because it has its own frame cache.
- **Optical flow** (`fosscut_render::gpu::flow`, `flow.wgsl`): coarse-to-fine pyramidal Lucas-Kanade with robust weights and colour-adaptive windows, then a joint bilateral smoothing pass, at most 960 px wide at the finest level. Flows are kept between frames keyed by a fingerprint of their pixels and always estimated earlier frame first, so a frame renders the same whatever came before it.
- **Effects:**
  - **`motion.blur`:** `amount` is the shutter as a fraction of a frame and `samples` the placements. It gathers along the fastest nearby motion (McGuire et al. 2012), and a moving layer is drawn at each placement and averaged.
  - **`detail.denoise`:** `strength`, and `temporal` for motion-compensated averaging with consistency and colour rejection before a 7x7 bilateral filter.
  - **`color.deflicker`:** `strength` and `window` (2 to 16 frames): the mean luminance over a box window of source frames sets a gain.
- **Measured:** flow slow motion reaches 39.2 dB against the true middle frame, where blending reaches 30.1 dB; denoise lowers noise variance by 76% (spatial) and 92% (temporal) with edges unchanged; a ±5% flicker on a fade is removed entirely, the fade kept within 1% a frame; preview, frame grabs and the export graph give identical temporal frames.
- **Limits:** adjustment layers and compound clips get no neighbours (denoise is spatial only there, deflicker and the flow part of motion blur do nothing); transition sides are drawn without their shutter streak; kept flows use about 66 MB of GPU memory.
- **Front ends:** `clip.speed` takes `interpolate: "flow"` (`fosscut clip speed --flow`), and the effects come from the registry through `effect.add`.
