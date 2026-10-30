# ADR-0029: Cut multicam angles on compound clips

- **Status:** Proposed
- **Date:** 2026-10-30
- **Deciders:** maintainers
- **Related:** [ADR-0025](0025-nest-timelines-in-clips-for-compound-clips.md), [ADR-0019](0019-local-analysis-pipeline-and-cache.md), [ADR-0004](0004-open-project-format.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (create multicamera clip, split scenes)

## Context

Interviews, podcasts and events are often filmed by two or more cameras at once. Editors let people group those angles into one multicamera clip, line them up by their sound, and cut between angles while one good sound track plays throughout. The research shows the entry point, disabled with a single source; synchronization and editing were not exercised.

FossCut already nests timelines in clips (ADR-0025): a compound clip shows another timeline, with speed, effects and transitions applying to it as a whole, and splitting it keeps both parts showing the same timeline. A multicam clip is that with one more choice: which angle shows.

## Decision

1. **The angles live in a compound timeline.** `multicam.create` makes a timeline with `role: "compound"`. Each angle is a video clip on a track of its own, placed at its offset in the shared scene (an angle that started later sits later), with the first angle on top. Clips start on frames while measured offsets fall between them, so an angle starts on the frame at or after its offset and skips the rest of the offset at the start of its media: angles line up to the flick, not to the nearest frame. Only the chosen sound angle's track is heard; the others are muted.
2. **A clip chooses its angle.** A compound clip may carry `angle`, a video track of its timeline. The planner then shows that track's picture alone, with the timeline's tracks of other kinds as usual. Validation reports an angle that is not such a track (`fosscut.model.angle`). Without `angle`, a compound clip shows every track, as before.
3. **Cutting is splitting.** `multicam.switch` splits the multicam clip at a time and gives the part after the cut the new angle, by number or track id; at a part's start it changes that part. Every part keeps playing the same sound and the same scene time, so cuts stay in sync.
4. **Offsets are measured, then given.** `multicam.create` takes the offsets explicitly. The engine measures them from the sound (`fosscut_analysis::sync`: envelope cross-correlation, then refinement to the sample) and front ends pass them, so the command stays deterministic and testable.
5. **Splitting at scene changes.** `fosscut_engine::scenes` maps the shot cuts measured in a clip's media to the timeline through its speed and returns `clip.split` calls, latest first, so the clip keeps its id as the first shot.

## Consequences

- Multicam reuses compound clips: trimming, speed, effects, transitions and export work on it unchanged, and an agent can read or edit the angles' timeline directly.
- A cut is a split, so a long multicam edit has many clip parts. That is the timeline people see, and undo works per switch.
- Angles must share the project's frame rate, as compound clips do; angles of other rates need converting first.

## Alternatives considered

- **A dedicated multicam clip kind with a cut list.** Rejected: it would duplicate the compound clip's rendering, sound and editing, and a cut list hides the edit from the timeline people see.
- **Syncing inside the command.** Rejected: measuring sound needs decoding, which the core does not do; the engine measures and the command applies.

## Validation

- **Core tests:** angles line up by their offsets with the chosen angle heard, switching splits and sets angles, and errors are typed.
- **Planner test:** an angle shows only its track.
- **Sync tests:** offsets are recovered within 2 ms from shifted noisy recordings.
- **Before Accepted:** a real-agent check builds a two-camera interview from synced angles and exports it.
