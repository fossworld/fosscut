---
name: fosscut-basics
description: Edit video projects with FossCut safely through its MCP tools or the fosscut CLI, from opening a project and importing media to cutting, titling, checking frames and exporting a verified file. Use for any request to edit, cut, title, check or export a video with FossCut, and before any other FossCut skill.
---

# FossCut basics

A good result is the edit the user asked for, checked with your own eyes (frame grabs) and delivered as a file whose export report matches the request. Every change is one undoable step, nothing is removed or overwritten without the user's consent, and the user hears exactly what changed.

## Tools

FossCut's MCP server (`fosscut mcp`) offers these tools; every one also exists on the command line with `--json`.

| Need | MCP tool | CLI |
|---|---|---|
| What FossCut can do | `capabilities` | `fosscut capabilities` |
| The arguments of the commands you will use | `capabilities` with `names: ["clip.add", "text.add"]` | `fosscut capabilities clip.add text.add` |
| Create or open a project | `project_new`, `project_open` | `fosscut project new <path>`, `-p <path>` on every command |
| Read the timeline | `timeline_read` | `fosscut timeline read --json` |
| Import media | `media_import` | `fosscut media import <file>` |
| Change the project | `edit` | `fosscut exec '<request JSON>'` or `fosscut clip ...`, `fosscut text ...` |
| Undo, redo, history | `undo`, `redo`, `history` | `fosscut undo`, `fosscut redo`, `fosscut history` |
| Facts about media: loudness, pauses, shot cuts | `media_analyze` | `fosscut media analyze <asset>` |
| What is said, with a time for every word, fillers and pauses | `media_analyze` with `analyzers: ["transcript"]` | `fosscut media analyze <asset> transcript` |
| Speech models (installing downloads; ask the user first) | `models` | `fosscut models list`, `fosscut models install <name> --yes` |
| Word-timed captions, SRT files (the `viral-captions` skill has the flow) | `captions` | `fosscut captions generate`, `fosscut captions import <file.srt>` |
| Look at the result | `frame_grab`, `contact_sheet` | `fosscut frame <time> --out still.png`, `fosscut contact-sheet --out sheet.png` |
| Export | `export`, then `job_status` | `fosscut export <file>` |

The commands `edit` runs include `clip.add`, `clip.split`, `clip.move`, `clip.remove`, `text.add` and `track.add`. Before using one the first time, ask `capabilities` for it by name to get its arguments; do not guess field names. `detail: "full"` returns every schema at once and is too large to read comfortably.

## The safe editing loop

1. **Open.** `project_open` with the user's `.fosscut` bundle, or `project_new` (1920x1080 at 30 fps by default; 1080x1920 for vertical video). Media must be imported with `media_import` before clips can use it.
2. **Read.** `timeline_read` before every change. Note its `revision` and the ids of the tracks and clips you will touch (`clip_0003`, `track_0002`). Look up the arguments of the commands you plan to use with `capabilities` and `names`.
3. **Plan.** Turn the request into commands. For anything beyond one or two simple additions, run `edit` with `dry_run: true` and check the change summary. When a change moves or cuts much of the edit, tell the user the plan before applying it.
4. **Apply.** `edit` with `expected_revision` set to the revision you read, a `request_id` that names this change (reuse it only to retry the same change after a timeout), and a short `reason`. Batch the commands of one logical change into one `edit`, so it is one undo step.
5. **Check.** `frame_grab` at the moments that matter (a title, both sides of a cut), and `contact_sheet` for the whole timeline. Look at the images; if something is wrong, fix it or `undo` before going on.
6. **Export.** `export` with the file name (and a `preset` when the user named a codec), plus `expected_revision`, so the file matches what you checked. It returns a job: call `job_status` with `wait_seconds: 60` until `state` is `succeeded`. Then compare the report with the request.
7. **Report.** Tell the user what changed (and that `undo` reverses it), what you checked, and the export report: codec, size, frame rate, frame count, duration and audio.

## Times

Every time needs a unit: `{"frames": 45}`, `{"seconds": "1.5"}`, `{"timecode": "00:00:01:15"}`, `{"flicks": n}`, or the short forms `"45f"` and `"1.5s"`. Bare numbers are refused. Results report each time in frames, timecode, seconds and flicks; prefer frames for exact cuts.

## Permission

- Never set `allow_destructive` on your own. Removing clips from the main video track, overwriting a file and similar changes need the user's yes for that change.
- Exports never substitute a codec. If the requested one is unavailable, say so and offer the codecs in the error's `details.available`.
- Do not tell the user a change happened until the tool result confirms it.

## When something fails

Errors arrive as `{"error": {"code", "message", "hint", "details"}}`. Follow the hint; the common cases:

| Code | What to do |
|---|---|
| `fosscut.revision.conflict` | Someone else changed the project. Read the timeline again, re-plan, and apply with the new revision. Never just resend. |
| `fosscut.guard.destructive` | Ask the user; retry with `allow_destructive` only if they agree. |
| `fosscut.clip.overlap` | The place is taken. Choose a free time, or add a track with `track.add`. |
| `fosscut.time.out_of_range` | The time is past the end of the timeline; check `duration` in `timeline_read`. |
| `fosscut.export.codec_unavailable` | Offer the codecs in `details.available`; do not pick one silently. |
| `fosscut.export.missing_media` | Tell the user which file is missing (`details.tried`) and ask them to restore it. |
| `fosscut.media.probe_unavailable` | FFmpeg's `ffprobe` is not installed; ask the user to install FFmpeg. |
| `fosscut.project.busy` | Another program is writing the project; wait a few seconds and retry. |

If a call times out, retry it with the same `request_id`: FossCut returns the first result instead of applying the change twice.
