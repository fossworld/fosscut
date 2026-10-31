# ADR-0034: Plan shorts, reframes and beat edits from measurements

- **Status:** Proposed
- **Date:** 2026-10-31
- **Deciders:** maintainers
- **Related:** [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), [ADR-0019](0019-local-analysis-pipeline-and-cache.md), [ADR-0029](0029-cut-multicam-angles-on-compound-clips.md), [ADR-0030](0030-apply-measured-motion-as-keyframes.md), [ADR-0033](0033-run-local-vision-and-voice-models-with-onnx-runtime.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (long video to shorts, auto reframe, automatic vertical version, camera tracking, auto beat)

## Context

Short-form editing repeats a few jobs:

- **Shorts from long recordings:** cut a long recording into short clips that stand on their own.
- **Reframing:** frame a wide picture for a tall screen, following the subject.
- **Camera following:** keep the camera on a moving person.
- **Beat edits:** cut to the beat of the music.

The research shows all of these as one-click entries behind paid plans, none executed. FossCut already measures transcripts, pauses, loudness, shot cuts, poses, mattes, motion and tracked regions locally (ADR-0019, ADR-0030, ADR-0033). Choosing which part of a talk is interesting is reasoning, and that belongs to the user's agent (ADR-0012).

## Decision

1. **Measure classically, then propose calls.** New analysis and engine code measures and returns ordinary commands. The front ends apply them in one request, as `fosscut_engine::motion` and `scenes` do (ADR-0029, ADR-0030).
2. **Beats.** The analyzer `beats` finds an onset-strength envelope by spectral flux, the tempo by autocorrelation, and beat times by dynamic programming (Ellis 2007). Results are cached like the others.
   - **Markers:** `fosscut_engine::beats` places markers at every first, second or fourth beat of a music clip.
   - **Cutting:** it moves the cuts between a track's clips to the nearest beats within a tolerance, keeping the clips' order and never overlapping them.
3. **Subject framing.** `fosscut_analysis::framing` turns per-frame subject boxes into a smooth crop path for a target aspect. A box comes from the person in the `pose` analysis, the subject in the `matte`, or a region tracked by `fosscut_analysis::track`. The path has a dead zone, a pan speed limit and easing, headroom and zoom limits, and starts afresh at every shot cut.
   - **Auto reframe:** `fosscut_engine::reframe` keys a clip's `transform.position` and `transform.scale` through `keyframe.set_all` (ADR-0030), so the clip follows its subject on a canvas of another aspect.
   - **Vertical versions:** they are `timeline.duplicate` onto 1080x1920, with every video clip reframed.
   - **Camera tracking:** the same path applied on the clip's own canvas with a zoom.
4. **Shorts are proposals with reasons.** `fosscut_analysis::highlights` scores candidate segments of 15 to 60 s that start and end at sentence boundaries. The scores come from speech density, pauses, loudness, shot changes and how self-contained a segment is, and each candidate lists its numbers.
   - **Choosing:** the user's agent chooses or re-ranks the candidates; FossCut never decides what a segment means.
   - **Building:** `fosscut_engine::shorts` builds a chosen range as a new vertical timeline, reframed to its subject, optionally with word-timed captions from the transcript.

## Consequences

- Agents get one-call building blocks for the most common short-form jobs, and every result is visible, editable data: timelines, keyframes, markers and cuts.
- No model is added. Reframing needs a `pose` or `matte` analysis (ADR-0033), or a region the agent tracks.
- Highlight scores are signals, not judgments; the quality of a short depends on the agent's choice.

## Alternatives considered

- **A built-in model that picks highlights:** rejected by ADR-0012. The agent reads the transcript and the scores instead.
- **Reframing as a render-time effect:** it would hide the motion from agents and from the timeline. Keyframes stay editable (ADR-0030).

## Validation

- **Beats:** a synthetic click track at 120 bpm with jitter is recovered within 10 ms and 1 bpm.
- **Framing:** framing paths keep a moving box inside the crop without crossing shot cuts.
- **Commands:** the returned calls apply cleanly and give the expected keyframes, markers, cuts and timelines.
- **Before Accepted:** a person reviews a vertical version and a short made from a real talk.

## Implementation notes

- **Analyzers:** `beats` (tempo, beats with strength, downbeats when the accents show them) and `levels` (K-weighted loudness of one-second windows, for the highlight scores) join the others in `fosscut-analysis`, cached by content.
- **Front ends:**
  - **MCP:** the `auto_edit` tool gains the actions `reframe`, `camera_track`, `vertical`, `beat_markers`, `beat_cuts`, `short_candidates` and `make_short`, so the server keeps 25 tools.
  - **CLI:** `fosscut clip reframe`, `fosscut clip camera-track`, `fosscut timeline vertical`, `fosscut beats markers|cut` and `fosscut shorts candidates|make`.
  - **Revisions:** planners whose calls create a timeline (`vertical`, `make_short`) are applied at the revision they were planned at unless the caller names another, so the ids they report are the ones made.
- **Analyses are never run implicitly:** reframing, vertical versions and shorts read cached analyses and fail with `fosscut.analysis.missing`, naming the asset, analyzer and options to run, because pose and matte need models the user installs. Beats are measured on demand, since they need no model.
- **Errors:** `fosscut.beats.none` (no steady pulse), `fosscut.reframe.no_subject` (nobody found in a clip; vertical versions leave such clips centred and list them) and `fosscut.shorts.no_candidates`.
- **Skills:** `vertical-reframe` follows a subject automatically or by a region; `long-to-shorts` and `beat-sync` are new, each with an evaluation replayed in CI.
