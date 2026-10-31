---
name: beat-sync
description: Cut a FossCut edit to its music: mark the beats and bars, and move the cuts between shots onto them without changing the edit's order or length. Use when the user asks to sync cuts to the beat, cut to the music, make a montage or music video, or mark the beats of a song.
---

# Beat sync

Follow the safe editing loop of the `fosscut-basics` skill (read, plan with a dry run, apply with `expected_revision` and a `request_id`, check, export). This skill adds the beat steps.

## What a good result looks like

- Every cut between shots lands on a beat, or on a bar for a calmer edit, and the user can see the beats as timeline markers.
- The shots keep their order, nothing overlaps, and no shot becomes a flash shorter than a quarter second.
- Cuts that could not move are named with the reason, so the user can decide.

## Steps

1. **Read** the timeline with `timeline_read` (`detail: "full"`): the music clip and the video track whose cuts should follow it. The music needs a few seconds with a clear pulse.
2. **Mark the beats.** `auto_edit` with action `beat_markers`, the `music` clip, and `every` (1 for every beat, 2 for every other, 4 for bars). The result has `beats.tempo_bpm`, `beats.beats_per_bar` and `beats.confidence`; below about 0.3 the pulse is weak, so tell the user. When the user knows the tempo, pass `bpm`.
3. **Plan the cuts.** `auto_edit` with action `beat_cuts`, the video `track`, the same `music` and `every`, and `dry_run: true`. Each cut moves to the nearest counted beat within `tolerance` (half the beat spacing by default) as a roll edit: the shot before ends there and the shot after starts there. Read `beats.snapped` (each cut's `from` and `to`) and `beats.unsnapped` (cuts that stay, with `reason`).
4. **Apply** the same call without `dry_run`, with `expected_revision` and a `request_id`. Tell the user how many cuts moved, and which stayed and why.
5. **Make it land.** For a stronger hit, add a short transition or an animation on the downbeats (`transition.set` with a 2 to 4 frame `push` or `zoom`), or a `library` sound such as a whoosh just before a big cut. Ask the user first.
6. **Check** with `contact_sheet`, and `frame_grab` on both sides of a few cuts. Then **export**.

## When something fails

- `fosscut.beats.none`: no steady beat was found (speech, ambient sound, rubato). Ask the user for the tempo and retry with `bpm`, or place cuts by hand at markers they add.
- A cut that needs more media than a shot has (`unsnapped`): trim the shot's other end with `clip.trim`, or accept the cut where it is.
- Cuts land a frame off the beat: the timeline frame rate rounds them to frames, which is expected.
- `fosscut.revision.conflict`: read again and re-plan.
