---
name: talking-head-cleanup
description: Tighten talking-head footage with FossCut by cutting filler words, long pauses and retakes found in a local transcript, keeping picture, sound and captions in sync. Use when the user asks to clean up a talk, interview, vlog or podcast video, remove ums, uhs, pauses or dead air, cut retakes, or make the edit snappier.
---

# Talking-head cleanup

Follow the safe editing loop of the `fosscut-basics` skill (read, plan with a dry run, apply with `expected_revision` and a `request_id`, check, export). This skill adds the cleanup steps.

## What a good result looks like

- **Fillers:** "um", "uh" and similar are gone.
- **Pauses:** shortened to a natural breath of about a quarter of a second, not removed outright, so the speech still sounds human.
- **Retakes:** false starts and repeated sentences keep only the attempt the speaker finished.
- **Meaning:** nothing the speaker meant to say is lost.
- **Captions:** they still match the speech. FossCut cuts caption tracks with the picture.
- **Approval:** the user has seen the list of cuts before you apply them.

## Steps

1. **Read.** `timeline_read` with `detail: "full"` to find the clips with speech: their `start`, `source_in` and asset.
2. **Transcribe.** `media_analyze` with `analyzers: ["transcript"]` for each asset with speech, and `job_status` while it runs.
   - If no speech model is installed, ask the user before installing one with `models`.
   - The transcript lists `words`, `fillers` and `pauses` in the asset's own time.
3. **Map to the timeline.** A word at asset time `t` in a clip appears at `start + (t - source_in)`, if `source_in <= t < source_in + duration`. Cut ranges are always given in timeline time.
4. **Plan the cuts.**
   - **Fillers:** cut from the filler's start to the start of the next word, so the hesitation after it goes too.
   - **Pauses:** for each pause longer than 0.6 s, cut its middle and keep about 0.12 s at each end: from `start + 0.12` to `end - 0.12`.
   - **Retakes:** read the transcript text. Where the speaker restarts a sentence or repeats a phrase, cut the abandoned attempt, from its first word's start to the start of the attempt that is kept.
   - Never cut inside a word, and keep at least 2 frames of the material around each cut.
   - Fillers are listed for English; for other languages, find them in the words yourself.
5. **Show the plan.** List each cut with its timecode, the words it removes, and the total time saved. Ask before applying.
6. **Dry run, then apply.**
   - `edit` with `timeline.remove_ranges`, every range in one command, and `dry_run: true`. Check `removed` and `duration_after`.
   - Then apply with `expected_revision`, a `request_id` and `allow_destructive: true`. Cutting the main footage needs the user's approval; give it only after they have agreed to the plan.
   - The whole cleanup is one undo step.
7. **Smooth the jump cuts** (optional, ask first). Alternate a slight punch-in on every other segment so the cuts read as intentional: `clip.set` with `{"transform.scale": 1.1}` on the 2nd, 4th and later segments of the speaker's clip. `timeline_read` lists the segments after the cut.
8. **Check.**
   - `timeline_read`: the new duration.
   - `frame_grab` just before and after the first few cuts: the speaker should not jump in an odd way.
   - `contact_sheet` for the whole edit.
   - If there are captions, a frame at a spoken word shows them in sync.
9. **Export** as usual, and report the time saved and what was cut.

## When something fails

- `fosscut.guard.destructive`: the cut touches the main video track; get the user's approval and pass `allow_destructive: true`.
- `fosscut.revision.conflict`: read again, re-map the times and re-plan.
- Only some tracks should be cut (music that must not jump): pass `tracks` with the tracks to cut. Tracks left out keep their timing.
