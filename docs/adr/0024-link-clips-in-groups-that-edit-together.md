# ADR-0024: Link clips in groups that edit together

- **Status:** Proposed
- **Date:** 2026-10-30
- **Deciders:** maintainers
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0004](0004-open-project-format.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0007](0007-timeline-projection-and-patches.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (linkage, ungroup, extract and restore audio entries)

## Context

Editors let people tie clips together so that one edit moves all of them: a video and the sound taken out of it, a title and the picture it labels, a few clips arranged as a unit. The research lists a linkage switch with configurable defaults, an ungroup command and an extract and restore audio pair as separate controls. They were inspected, not exercised.

FossCut clips are independent today: `clip.move`, `clip.trim`, `clip.split` and `clip.remove` touch one clip. Extracting sound would leave a video clip and an audio clip that drift apart at the first move. An agent that edits a timeline someone arranged by hand needs to see which clips belong together and to keep them together, or to break a link on purpose. Links must be stored in the project (they outlive a session), be sparse, and keep the format's rules (ADR-0004): typed ownership, short stable ids, no machine data.

## Decision

1. **Groups are an id on clips.** A clip may carry `group: "group_…"` (a new id kind). Clips with the same group form it; there is no group document. A group has at least two clips on one timeline; validation reports `fosscut.model.group` otherwise. Commands that would leave a group with one clip remove the id from that clip.
2. **Making and breaking groups.** `clip.group` puts two or more clips of one timeline into a new group, taking them out of any group they were in. `clip.ungroup` takes clips out of their groups.
3. **Edits follow the group.** `clip.move`, `clip.remove` and `clip.split` act on every clip of the group unless the call passes `linked: false`. A move shifts every member by the same time; only the clip named changes track. A split cuts every member that spans the time, and the right parts form a new group. `clip.trim` moves the same edge of the members whose edge coincides with the named clip's. Every member must pass the usual checks (overlaps, locked tracks, media range), or the command fails as a whole. Results list each clip they changed.
4. **Extracted sound is a group.** `clip.extract_audio` creates an audio clip with the video clip's asset, source range, speed, volume and audio settings, on the first audio track free over its span (or a new one), and groups it with the video clip. The video clip records `audio.detached: true` and is no longer heard. `clip.restore_audio` removes the audio clips of its group that play the same asset and clears `detached`; if they were deleted, it only clears the flag.
5. **Agents and the app see groups.** `timeline.read` shows each clip's `group`, and `detached` with the clip's audio settings. The desktop's linkage switch decides whether its edits pass `linked: true`; it defaults to on.

## Consequences

- An agent sees every link in the projection and keeps it by default, so hand-made arrangements survive its edits.
- Extracted sound edits independently (its own volume, effects later, trims with `linked: false`) yet stays in sync when the picture moves.
- Commands that touch groups do more work and must check every member; their results grow to list them.
- One clip belongs to at most one group, so nested groups are not possible. Compound clips (a sub-timeline as one clip) are a separate, later decision.

## Alternatives considered

- **A group document listing clip ids:** explicit, but the ids go stale when clips are split or removed, and every command must update two places. An id on the clip cannot dangle.
- **Links only for extracted audio:** smaller, but the same mechanism serves any arrangement, and two concepts would confuse agents.
- **Edits ignore groups unless asked (`linked: true` to follow):** safer for an agent that skips the projection, but it breaks what the person arranged on the first edit. Defaulting to the link, with the result listing what moved, makes the effect visible.
- **Muting the video clip on extraction (`volume: 0`):** loses the volume the person set and cannot be told apart from a deliberately silent clip.

## Implementation notes

- **Model** (format 0.8.0): `Clip::group` holds the group id, `ClipAudio::detached` marks extracted sound, and validation reports `fosscut.model.group` and `fosscut.model.detached`. Commands that can leave a group with one clip (`clip.remove`, `clip.ungroup`, `clip.split`, `clips.paste`, `clip.restore_audio`) tidy it away.
- **Linked edits:** `clip.move` takes every member out of its track before placing any, so members on one track never collide with each other's old places. `clip.trim` passes `ripple` on to the members it trims.
- **Copies:** `timeline.duplicate` and `clips.paste` give the copies' groups new ids, so a group never spans timelines and pasted clips form groups of their own.
- **Sound:** the mixer skips a video clip whose sound is detached. `clip.extract_audio` copies the clip's volume, fades, pan, channels, speed and their keyframes to the new audio clip, and joins the video clip's group when it already has one.
- **Desktop:** the transport bar has the linkage switch (edits pass `linked: false` when it is off) and the magnet switch, which makes deleting from the main track ripple. Shift-click adds clips to the selection; Mod+G groups them and Mod+Shift+G ungroups. The inspector extracts and restores sound, and the timeline marks grouped clips with a bar.

## Validation

- Contract tests: moving, splitting, trimming and removing a member edits the group, `linked: false` edits one clip, and a member that cannot move fails the whole command.
- Extracting and restoring sound round trips: the mix is the same before extraction, after it, and after restoring.
- Validation reports one-clip groups and groups across timelines.
