# ADR-0038: Emit particles deterministically from their birth index and seed

- **Status:** Proposed
- **Date:** 2026-11-01
- **Deciders:** maintainers
- **Related:** [ADR-0004](0004-open-project-format.md), [ADR-0005](0005-time-model.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0008](0008-render-and-audio-engine.md), [ADR-0011](0011-effects-and-animatable-properties.md), [ADR-0016](0016-testing-strategy.md), [ADR-0028](0028-a-built-in-library-made-by-fosscut.md), [ADR-0032](0032-keep-proxies-remuxes-and-rendered-frames-as-caches.md), [ADR-0035](0035-render-temporal-effects-from-neighbouring-frames.md)

## Context

Motion-graphics ads lean on particles: sparks gathering into a logo, glowing embers, dust in a beam of light, snow, confetti, bokeh, light streaks and a burst behind a product as it appears. FossCut has no particles.

Particle systems in other tools usually simulate step by step from the first frame, so a frame depends on the frames before it. That breaks what FossCut promises: preview scrubbing, frame grabs, caches keyed by plan (ADR-0032) and exports must give the same picture for the same time, on any machine, rendered in any order. Golden frames also run on other GPU adapters in CI, so randomness must not come from floating-point sine hashes that differ between drivers.

## Decision

1. **A particle clip.** `ClipContent::Particles` (`{"kind": "particles", "particles": {...}}`, format 0.16.0, additive) sits on a video track. Its picture covers the canvas and takes the clip's transform, opacity, blend mode, masks, effects, keyframes and animations like any picture. The emitter lives under its own key because a clip's content is flattened into the clip object and the emitter's `speed`, `opacity` and `blend` would otherwise meet the clip's own fields.
2. **The emitter** (`fosscut_model::ParticleContent`, sparse with documented defaults):
   - **Where:** `emitter` with `shape` (`point`, `line`, `circle`, `rectangle`; a shape clip's path can be added later as another shape), `position` and `size` as canvas fractions like `transform.position`, `angle`, and `edge` (the outline only).
   - **When:** `rate` per second (0 to 2000), `bursts` (`at` in emission time, `count` 1 to 5000, at most 64), `prewarm` (up to 30 seconds of emission before the clip opens), and `offset`, where in its emission the clip starts.
   - **How it moves:** `life`, `speed` (canvas heights per second) and their variances, `direction` (degrees clockwise from rightwards; -90 is up), `spread`, `aim` (`direction`, `outward`, `inward`, or `converge`: an eased path from birth to the emitter's centre), `gravity`, `wind`, `drag`, `turbulence` with `turbulence_scale`, `rotation` and `spin` with variances.
   - **How it looks:** `size` with variance, `size_over_life` and `opacity_over_life` (1 to 8 evenly spread multipliers), `opacity`, `colors` (1 to 8, through life or picked at random by `color_by`), `blend` between particles (`add` or `normal`), `sprite` (`dot`, `spark` streak stretched along the velocity, `star`, `ring`, `square` with `aspect` and tumbling `flip`, or `image`: an image asset whose brightness is its coverage, tinted by the particle's colour), `softness`, and `seed`.
   - **Units:** distances, sizes and speeds are in canvas heights, so motion looks alike on every canvas. Times that are timeline time (`bursts.at`, `prewarm`, `offset`) are flicks (ADR-0005); lifetimes and rates are simulation parameters in seconds.
3. **Animatable settings** join the closed vocabulary (ADR-0011 item 3) with `Applies::Particles`: `particles.rate`, `particles.speed`, `particles.direction`, `particles.spread`, `particles.gravity`, `particles.wind`, `particles.turbulence`, `particles.size`, `particles.opacity` and the pair `particles.emitter.position`. A particle reads them when it is born and keeps them for life, so a moving emitter leaves a trail; the rate's curve decides when particles are born. The clip's own `opacity` and transform fade or move the whole picture.
4. **Determinism** (`fosscut_render::particles`, all on the CPU in `f64`):
   - **Births:** continuous particles are numbered from the start of emission. The count born by emission time `t` is the integral of the rate: closed form for a constant rate, otherwise the midpoint rule on a grid of 32 steps between keyframes, which depends only on the keyframes and never on the time drawn. Particle `n` is born where the count reaches `n` plus a random fraction. Burst particles are numbered per burst.
   - **Randomness:** every choice is `splitmix64` of the seed, the stream (continuous or burst number), the particle's number and the choice. Turbulence is two octaves of value noise whose lattice values are one integer mix each.
   - **State at `t`:** closed form from the birth: constant acceleration with exponential drag (terminal velocity under gravity), or the eased converge path; turbulence adds the field's difference between birth and now, sampled where the particle would be, so it starts at nothing, stays bounded and moves nearby particles alike. Sparks measure their velocity over 1/240 s for their direction and length.
   - **Splits and trims** move `offset` by the same amount through `Clip::retime_part`, and keep bursts and keyframes in emission time, so both parts continue the same particles.
   - **Limits:** at most 20 000 particles are drawn at once (the newest when more live), and with motion blur at most 200 000 sprites in all (fewer shutter samples beyond that).
5. **Rendering** (`gpu/particles.rs`, `shaders/particles.wgsl`). The planner gives the layer `LayerSource::Particles` with the content, the clip's `particles.*` keyframes, the time from the clip's start, the frame length and an image sprite's frame. The renderer draws one instanced quad per particle into a transparent `rgba16float` texture of the output size, in premultiplied linear light, with analytic shapes softened by at least one pixel; particles smaller than 0.75 pixels in radius keep their light by losing opacity, so they never flicker. `add` sums light while coverage combines as with normal layers; `normal` covers. The texture is mipmapped and composited as the layer's picture. A `motion.blur` effect on the clip also opens a shutter over the particles' own motion: each is drawn at evenly spread moments with its opacity shared. The plan's cache key encodes the emitter, so cached frames follow every change.
6. **Presets** (ADR-0028), FossCut's own numbers in `fosscut-core/presets/library/particles.json`: `sparks`, `embers`, `dust`, `snow`, `confetti`, `bokeh`, `light-streaks`, `burst`, `spark-gather` (converging into the centre for a logo reveal) and `sparkle`. They are listed by `capabilities` under `library.particles` and by `fosscut_engine::library` with kind `particles`.
7. **Commands and reach.** `particles.add` (from a preset or the defaults, with a merge patch `set`, `bursts` and `prewarm` in any time unit, a `seed`; on a track of particle clips above the video tracks unless a track is given; 5 seconds by default) and `particles.set` (a preset, a merge patch, bursts, prewarm, seed; settings with keyframes are refused with `fosscut.clip.property_animated`). Errors are `fosscut.particles.invalid`, `fosscut.library.unknown_item` and `fosscut.asset.not_found`. `capabilities` lists the commands, the properties and, under `particles`, the emitter's choices, limits and schema. The CLI has `fosscut particles presets|add|set` (`--burst COUNT@TIME`, `--prewarm`, `--seed`, `--set` JSON) and `fosscut library add <preset>`; MCP reaches them through `edit` and the `library` tool, with no new tool. Image sprites count as uses of their asset for `media.remove`, follow `media.replace`, and are checked by exports and reported in their rights notes.

## Consequences

- Any frame of a particle clip renders on its own, so scrubbing, frame grabs, caches and exports agree, and particles are safe in compound clips and transitions.
- Agents can name every setting, start from presets, animate the emitter and keep edits undoable.
- Closed-form motion rules out collisions, attraction to arbitrary points, and particles that react to the scene or to each other; turbulence is a displacement, not a force, so it wobbles particles rather than steering them far. Particles do not emit particles.
- Simulation runs on the CPU every frame. In a debug build on an Apple M3 Pro under heavy load, presets take 0.02 to 1.3 ms and add about 1.5 to 3 ms to a 1080p frame; 20 000 plain dots take 18 ms and 20 000 turbulent sparks 66 ms to simulate. Optimized builds are several times faster.
- Image sprites use brightness as coverage, so artwork made as light on black works best.

## Alternatives considered

- **Step-by-step simulation with cached state.** Rejected: a frame would depend on what was rendered before, caches would hold simulation state, and seeking far would mean simulating everything before it.
- **Simulation on the GPU in a compute shader.** Deferred: the CPU closed form is fast enough for the limits above, and keeps results identical across adapters; the instance format allows a compute pass later.
- **A CPU raster like shapes and text.** Rejected: uploading a canvas-sized float image every frame costs more than instanced quads, and image sprites already live on the GPU.
- **Sine-based hashes in shaders.** Rejected: they differ between drivers and break goldens on other adapters.

## Validation

- **Model:** sparse round trips beside the clip's own fields, every range check names its field, and every animatable path reads and writes.
- **Determinism:** unit tests show steady births, prewarm, gravity, drag and converge in closed form, keyframed rates, split parts equal to the whole, shutter samples, the particle limit, and a frame independent of earlier ones; GPU tests render a frame alone and after others byte for byte, on a fresh renderer too; the CLI grabs the same frame in two processes byte for byte.
- **Rendering:** GPU tests check placement, every sprite's shape, add against normal, motion blur and image sprites; reviewed golden frames show sparks, embers, confetti, bokeh, a burst and gathering sparks.
- **Commands:** contract samples for both commands, preset validity, typed errors, keyframes, splits and trims, media remove and replace, and a CLI end-to-end test.
- **Before Accepted:** a person reviews the presets on vertical and horizontal canvases, and an optimized build is measured at 1080p.
