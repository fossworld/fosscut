# ADR-0023: Change clip speed with a time map from timeline time to source time

- **Status:** Proposed
- **Date:** 2026-10-30
- **Deciders:** maintainers
- **Related:** [ADR-0005](0005-time-model.md), [ADR-0008](0008-render-and-audio-engine.md), [ADR-0011](0011-effects-and-animatable-properties.md), [ADR-0006](0006-agent-safety-contract.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (speed, reverse and freeze entries), `docs/capcut-rea/DRAFT-FORMAT.md`

## Context

Editors let people play a clip faster or slower, by a constant factor or along a curve, backwards, or frozen on one frame. Smooth slow motion fills the frames between source frames, and audio either keeps its pitch or changes it with the speed. The research shows these as separate controls: a constant multiplier that changes the clip's length and offers pitch preservation, curve presets with a custom curve, reverse and freeze actions, and a display of the speed curve on the timeline.

FossCut's clips so far play their source one to one: the source time is the clip's `source_in` plus the time since the clip's start. Speed changes that mapping. It must stay exact in flicks (ADR-0005), keep preview and export identical (ADR-0008), and leave keyframes, effects and masks meaningful. Their times are offsets on the timeline from the clip's start (ADR-0011), and must not be bent by speed.

## Decision

1. **A clip's speed is a time map.** Video and audio clips get an optional `speed` object. The map takes a clip offset `t` (timeline time since the clip's start, from 0 to the clip's duration `D`) to a source time `s(t)`. Without `speed`, `s(t) = source_in + t`, as today.
2. **Constant speed.** `speed.rate` is a factor from 0.05 to 100: `s(t) = source_in + rate * t`. The clip uses `S = rate * D` of its source, which must lie within the media (`fosscut.clip.source_range`).
3. **Speed curves.** `speed.curve` is a list of points `{at, rate}`, where `at` runs from 0 to 1 over the clip's duration and `rate` from 0.05 to 100. Between points the rate changes linearly, and before the first point and after the last it holds. The source used is the integral, `S = D * ∫ rate`, computed in closed form per segment and rounded to the nearest flick. `rate` and `curve` exclude each other. Presets (`montage`, `hero`, `bullet`, `jump_cut`, `flash_in`, `flash_out`) are curves written for FossCut, which the command copies into the clip, as with looks.
4. **Reverse.** `speed.reverse: true` plays the same source range backwards: `s'(t) = source_in + S - (s(t) - source_in)`.
5. **Freeze.** `speed.rate: 0` holds the frame at `source_in` for the clip's duration (`S = 0`). `clip.freeze` makes one: it splits a clip at a time and inserts a frozen clip of a given length there, rippling what follows.
6. **Changing speed changes length.** `clip.speed` keeps the source range a clip uses and sets `D = S / mean rate`, so faster clips get shorter and slower ones longer, and the duration is snapped to frames. A clip that would grow into the next one fails with `fosscut.clip.overlap` unless `ripple: true` moves the later clips of that track. Trimming a clip with speed moves its edges in timeline time; the source moves by the map.
7. **Keyframes, effects and masks keep timeline time.** Their offsets stay offsets from the clip's start on the timeline, so an animation keeps its timing whatever the speed. Splitting a sped-up clip splits its map: each part keeps the curve's shape over its own part, and the right part's `source_in` is `s(split offset)`.
8. **Frames.** The planner asks for the source frame at `s(t)`. With `speed.interpolate: "blend"`, a sample between source frames mixes the two nearest frames by the fractional position: smooth slow motion. Motion-compensated interpolation may come later as another value of `interpolate`, without changing the format.
9. **Sound.** With `speed.pitch: "preserve"` (the default), audio is time-stretched with WSOLA (waveform-similarity overlap-add) at the map's rate, block by block. With `"shift"` it is resampled, so pitch follows speed. Reversed clips play their audio backwards, and frozen clips are silent. Captions do not take speed.
10. **Agents read the map.** `timeline.read` shows a clip's speed, the source range it uses and, for curves, the rate at the clip's start and end. `capabilities` lists the presets.

## Consequences

- One mapping serves video frames, audio, the timeline display and validation, so preview, export and the agent view stay consistent.
- Keyframes and effects keep their timing through speed changes, which matches what people expect when they time a title to the edit, not to the source.
- WSOLA and frame blending are new code paths with their own tests; motion-compensated slow motion stays future work.
- Source ranges now depend on speed, so every place that checks or moves `source_in` (trim, split, range removal, relink) must go through the map.

## Alternatives considered

- **Speed as an animatable property (`speed` keyframes):** would reuse the keyframe system, but a keyframed rate changes the source a clip consumes, which ripples into duration and validation and makes trims ambiguous. A dedicated map keeps those rules in one place.
- **Retiming as an effect:** effects change pixels at a time; speed changes which time is shown and must also drive audio, so it belongs to the clip.
- **Keeping duration fixed when speed changes (source range grows or shrinks):** simpler for overlaps, but it surprises people. A faster clip that keeps its length runs out of media, and editors commonly shorten it instead.
- **Storing a precomputed frame list:** exact but large, and not editable as a curve.

## Validation

- Exact tests of the map: constant, curved, reversed and frozen clips give the expected source time at chosen offsets, the integral matches a numeric reference, and splits keep both parts on the same curve.
- Export tests: a clip at 2x is half as long, with the expected frame count. A reversed clip shows its last frame first. A frozen clip shows one frame and is silent.
- Audio tests: a tone keeps its frequency at 2x with `preserve` and doubles with `shift`.
- A golden frame of blended slow motion.
