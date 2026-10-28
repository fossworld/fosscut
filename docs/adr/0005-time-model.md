# ADR-0005: Integer flicks for timeline time, exact rationals for rates

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0004](0004-open-project-format.md), [ADR-0007](0007-timeline-projection-and-patches.md), private research: `docs/capcut-rea/DRAFT-FORMAT.md`

## Context

Editors commonly store time as integer microseconds or floating-point seconds, with a floating-point frame rate. Neither is exact: a cut at 3 seconds and 28 frames at 30 fps is 3.9333… seconds, which microseconds can only approximate, so frame boundaries drift and the rounding error moves into the neighbouring clip. Floating-point seconds (as in OpenTimelineIO rates) accumulate rounding errors under repeated edits, which agents perform often. Units must also be unambiguous at the API boundary, because agents mix frames, seconds and timecode.

## Decision

1. **Timeline positions and durations are `i64` flicks.** One flick is 1/705,600,000 of a second. Every common video rate (23.976, 24, 25, 29.97, 30, 48, 50, 59.94, 60, 90, 100, 120 fps, including the NTSC 1000/1001 rates) and every common audio rate (8 to 192 kHz, including 44.1 kHz) divides it exactly. The range covers about 400 years.
2. **Rates are exact rationals** (`30000/1001`, never `29.97`).
3. **Source media timestamps keep their native stream timebase** in the media layer. Conversion to flicks happens at one boundary function with round-to-nearest and is tested for every timebase seen in fixtures (including the 1/15360 and 1/90000 timebases common in MP4 and MPEG-TS, which flicks do not divide).
4. **Edits snap to frames of the timeline rate by default.** Sub-frame positions are allowed only for audio clips and are flagged in the projection.
5. **Units are always explicit at the API boundary.** Commands accept `{"frames": 118}`, `{"seconds": "3.9333"}`, `{"timecode": "00:00:03:28"}` or `{"flicks": 2775360000}`. Bare numbers are rejected with an error that lists the accepted forms. Responses return frames, seconds and timecode together so agents never convert.
6. **`fosscut-time` owns all arithmetic**: checked operations, frame snapping, timecode parsing (non-drop and drop-frame), and formatting. No other crate does time math on raw integers.

## Consequences

- Splits, trims and ripples are exact; repeated agent edits never drift.
- Project files contain large integers; projections and responses show frames and timecode for readability.
- Media with exotic timebases needs the documented rounding rule and tests.

## Alternatives considered

- **Microseconds:** not exact for 30 fps or NTSC rates.
- **Floating-point seconds or OTIO `RationalTime` with float rate:** accumulates error.
- **Per-timeline frame counts only:** breaks when clips of different rates or audio samples meet; mixed-rate timelines are common in short-form content.
- **Arbitrary rationals everywhere:** exact but slower and awkward to compare and store; flicks give a fixed common denominator.

## Validation

Property tests: for every supported rate, `frames → flicks → frames` is the identity; split followed by join restores the original clip exactly; the round 1 split at 00:00:03:28 lands exactly on frame 118.
