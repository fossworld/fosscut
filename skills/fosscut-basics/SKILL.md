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
| Word-timed captions, SRT files, lyrics, bilingual captions (the `viral-captions` skill has the flow) | `captions`; `edit` with `captions.edit` and `translation` for a second language | `fosscut captions generate`, `fosscut captions import <file.srt>`, `fosscut captions export out.srt --subtitles both` |
| Cut pauses or retakes from every track at once (`talking-head-cleanup` skill) | `edit` with `timeline.remove_ranges` | `fosscut timeline remove-ranges 3.4s..4.1s` |
| A vertical copy whose clips follow their subject shot by shot (`vertical-reframe` skill) | `auto_edit` with action `vertical`, after `media_analyze` with `["pose"]` (or `["matte"]`) and `["scenes"]`; action `reframe` with a `region` for anything else | `fosscut timeline vertical`, `fosscut clip reframe --clip <id> --region 0.3,0.5,0.2,0.4` |
| The best parts of a long video as vertical shorts (`long-to-shorts` skill) | `auto_edit` with action `short_candidates` (after a transcript), then `make_short` with a candidate's `from` and `to` | `fosscut shorts candidates --clip <id>`, `fosscut shorts make --clip <id> --from 12.4s --to 41s` |
| Cut to the music: markers on beats, cuts moved onto them (`beat-sync` skill) | `auto_edit` with action `beat_markers` (`music`, `every`) or `beat_cuts` (`track`, `music`) | `fosscut beats markers --music <id> --every 4`, `fosscut beats cut --track <id> --music <id>` |
| Keep the camera on a moving subject, zoomed in | `auto_edit` with action `camera_track` (`zoom` 1 to 4) | `fosscut clip camera-track --clip <id> --zoom 1.5` |
| Placement, crop, mirror, opacity, blend mode, volume, fades, pan, deactivate a clip | `edit` with `clip.set` (paths in `capabilities` under `properties`) | `fosscut clip set --clip <id> --blend screen --fade-in 0.5s --set crop.left=0.1` |
| Animate any of those over time (zoom in, fade a title, duck music) | `edit` with `keyframe.set` at two or more times, with an easing | `fosscut keyframe set --clip <id> --property opacity --at 1s --value 0` |
| Color and look: exposure, white balance, curves, HSL, wheels, LUTs, vignette, blur, sharpen, grain (types and parameters in `capabilities` under `effects`) | `edit` with `effect.add`, `effect.set`; `look.apply` for a ready look; `adjustment.add` to grade everything below | `fosscut effect add --clip <id> --type color.adjust --param exposure=0.5`, `fosscut look apply teal_orange --clip <id>` |
| Correct a clip's color, or match it to another | `color` with action `auto` or `match` | `fosscut color auto --clip <id>`, `fosscut color match --clip <id> --reference <id>` |
| Show part of a picture: shapes, a pen path, text or brush strokes, feathered, combined, animated; remove a green screen | `edit` with `mask.add`, `mask.set`; `effect.add` with `key.chroma` | `fosscut mask add --clip <id> --shape ellipse --set feather=0.3`, `fosscut effect add --clip <id> --type key.chroma` |
| Transitions between touching clips: dissolve, dip to a color, wipe, slide, push, zoom, blur, iris, clock, spin | `edit` with `transition.set`, `transition.remove` (types in `capabilities` under `transitions`) | `fosscut transition set --clip <id> --type push --duration 0.4s --param direction=up` |
| Animate a clip in, out or in a loop (fade, slides, zooms, pop, drop; pulse, swing, float, shake); glitch, VHS, RGB split, pixelate, mirror, shake, flash and zoom effects | `edit` with `animation.set` (presets in `capabilities` under `animations`); `effect.add` for the effects | `fosscut animation set --clip <id> --in pop:0.4s --combo pulse`, `fosscut effect add --clip <id> --type stylize.glitch` |
| Speed: faster, slower, a speed curve or preset (montage, hero, bullet, jump_cut, flash_in, flash_out), reverse, smooth slow motion (`interpolate` `blend`, or `flow` for in-between frames drawn from the motion), a freeze frame | `edit` with `clip.speed` (the clip keeps its media, so its length changes; `ripple: true` moves what follows) and `clip.freeze` | `fosscut clip speed --clip <id> --rate 0.25 --flow --ripple`, `fosscut clip freeze --clip <id> --at 3s --duration 1s` |
| Motion blur, video noise reduction, flicker removal (they look at neighbouring frames) | `edit` with `effect.add` of `motion.blur`, `detail.denoise` or `color.deflicker` | `fosscut effect add --clip <id> --type detail.denoise --param strength=0.6` |
| Copy and paste clips, copy attributes, split every track, replace media, align on the canvas, delete a clip's start or end and close the gap | `query` with `clips.copy`, then `edit` with `clips.paste`; `clip.copy_attributes`, `timeline.split`, `clip.replace`, `clip.align`, `clip.trim` with `ripple` | `fosscut clips copy --clip <id> --json`, `fosscut timeline split --at 4s`, `fosscut clip align --clip <id> --horizontal left` |
| Keep clips together: groups, extracted sound; markers at beats and cues | `edit` with `clip.group`, `clip.ungroup`, `clip.extract_audio`, `clip.restore_audio` (moves, splits, trims and removals follow a clip's group unless `linked: false`); `marker.add` | `fosscut clip extract-audio --clip <id>`, `fosscut clip group --clip a --clip b`, `fosscut marker add --at 2s --name drop` |
| Organize media: rename, folders, find moved files, swap media everywhere; several timelines in one project | `edit` with `media.rename`, `media.move`, `media.folder`, `media.replace`, `media.remove`; `media_import` with `relink` for a file that moved (`media_list` marks `missing`); `timeline.add` (from assets), `timeline.set_primary`, `timeline.rename`, `timeline.remove` | `fosscut media relink asset_0001 moved/shot.mp4`, `fosscut timeline add --asset asset_0001 --primary` |
| Compound clips: turn clips into one clip (trim, speed, effects and transitions then apply to all of it), reuse it, edit inside it, break it apart | `edit` with `clip.compound`, `clip.decompose`, `clip.add` with `compound`; `timeline_read` with the compound clip's `timeline` to see inside | `fosscut clip compound --clip a --clip b --name Intro`, `fosscut clip add --compound <timeline>` |
| Titles: a preset (outline, shadow, boxed, highlight, headline, spaced, neon, glow, retro, comic), outline, box, shadow, glow, letter and line spacing, case, underline, parts in another colour or size (spans), a typewriter or word-by-word reveal; restyle later, save a style as a preset, add a title template (lower-third, headline, chapter, quote, callout, end-card) | `edit` with `text.add` (`preset`, `style`, `spans`, `reveal`), `text.set`, `text.preset_save`, `text.template` (presets and templates in `capabilities` under `text`) | `fosscut text add "Big news" --preset headline --reveal word --reveal-motion rise`, `fosscut text template lower-third --line "Sam Lee" --line Editor` |
| Fonts beyond the bundled Inter | `media_import` a `.ttf` or `.otf` file the user gives you, then name a family from its `families` in `style.font` | `fosscut media import Brand.otf`, `fosscut text add Hello --font "Brand Sans"` |
| Clean up sound: reduce background noise, make a voice clearer, keep or remove the voice of a stereo song, limit peaks | `edit` with `effect.add` and `audio.denoise`, `audio.enhance_voice`, `audio.vocals` or `audio.limiter` (on clips with sound; `capabilities` with the type's name lists its parameters) | `fosscut effect add --clip <id> --type audio.denoise --param amount=0.7` |
| Change a voice: deeper, higher, robot, phone call, radio, megaphone, broken mic, whisper, a hall or a cave (`capabilities` lists the voice presets; single `voice.*` effects too) | `edit` with `voice.apply` (`clip`, `preset`, `replace`) or `effect.add` of a `voice.*` type | `fosscut audio voice robot --clip <id>` |
| Even out loudness between clips (to -14 LUFS for short-form, -16 for podcasts); it adds an `audio.limiter` where peaks leave too little room | `audio_normalize` | `fosscut audio normalize --clip <id> --clip <id> --target -14` |
| Shapes, stickers (badges, buttons, arrows, speech bubbles, check marks), solid or gradient backgrounds | `edit` with `shape.add` (an `item` from `capabilities` under `library.visuals`, or a `shape` with `fill`, `outline` and `label`), `shape.set` | `fosscut library add badge-new --at 2s`, `fosscut library list --kind sticker` |
| A whole edit from a project template (vertical promo, quote card, photo montage, product reveal, talking-head intro) with your shots and words | `edit` with `template.apply` (`media`, `lines`) | `fosscut library add vertical-promo --media asset_0001 --line "Big news"` |
| Sound effects (whoosh, pop, ding, riser, impact and more) and music loops, made by FossCut and free to use commercially | `library` with action `list` (kind `sound` or `music`), then action `add` with `name` and `start` | `fosscut library add whoosh --at 1.5s` |
| Fill the bars around a 16:9 shot in a vertical video with its own picture blurred, or a colour | `edit` with `clip.canvas` (`clips`, `fill` of kind `blur` or `color`) | `fosscut clip canvas --clip <id> --blur` |
| The user's defaults (loudness target, image length, frame rate of new projects), to follow in your requests | `query` with name `preferences.read` | `fosscut preferences` |
| Split a clip at its scene changes, so each shot is a clip | `auto_edit` with action `split_scenes` | `fosscut clip split-scenes --clip <id>` |
| Steady shaky footage (keys the clip's position, rotation and scale, with a zoom up to 1.5) | `auto_edit` with action `stabilize` and `strength` 0 to 1 | `fosscut clip stabilize --clip <id> --strength 0.5` |
| Make a mask, title or sticker follow something that moves: find its centre and size with `frame_grab`, then track it | `auto_edit` with action `track`, `region`, `at` and `mask` (a mask of the clip) or `follow` (another clip) | `fosscut clip track --clip <id> --region 0.5,0.4,0.2,0.3 --follow <title>` |
| Several cameras of one scene: line them up by their sound in one multicam clip, then cut between angles | `auto_edit` with action `multicam` (`angles`, `sound`); `edit` with `multicam.switch` (`clip`, `at`, `angle` from 1) | `fosscut multicam create <asset> <asset>`, `fosscut multicam switch --clip <id> --at 12s --angle 2` |
| Record a voiceover, a camera or the screen, only when the user asks (it uses their devices) | `record` with action `devices`, then `start` with `what` and `confirm: true`; `job_status`, `job_cancel` to stop (the recording is kept); `media_import` the job's `recording.output` | `fosscut record microphone --duration 30s --at 2s`, `fosscut record --list` |
| Remove or replace a background, or keep only the person: measure the subject first, then key it (needs local models; ask the user before installing them with `models`) | `media_analyze` with `analyzers: ["matte"]` (options `matte.model` `person`, fast, or `subject`, slow), then `edit` with `effect.add` of `key.subject` (`invert`, `background` color or blur) | `fosscut media analyze <asset> matte`, `fosscut effect add --clip <id> --type key.subject` |
| Effects that follow a person's body: outline, glow, shadow, sparkles | `media_analyze` with `analyzers: ["pose"]`, then `effect.add` of `body.outline`, `body.glow`, `body.shadow` or `body.sparkle` | `fosscut media analyze <asset> pose`, `fosscut effect add --clip <id> --type body.glow` |
| A voiceover from text, in an English voice | `record` with action `voices`, then action `speak` (`text`, `voice`, `at`) | `fosscut speak "Welcome back" --voice af_heart --at 2s` |
| Translate captions between English and Hungarian, or dub them in English | `captions` with action `translate` (`to`, `mode` bilingual or replace) or `dub` (`voice`, `translation`); you can also translate pages yourself with `captions.edit` | `fosscut captions translate --to hu`, `fosscut captions dub --voice bm_george` |
| Look at the result | `frame_grab`, `contact_sheet` | `fosscut frame <time> --out still.png`, `fosscut contact-sheet --out sheet.png` |
| Export | `export`, then `job_status` | `fosscut export <file>` |
| Export choices: resolution (720p to 8K), frame rate, bit rate, a part of the timeline, a cover image, subtitles beside the file; ProRes profiles, GIF, WAV, FLAC or MP3 (`capabilities` lists the presets); rights notes on the media used | `export` with `resolution`, `fps`, `bitrate`, `range`, `cover`, `subtitles`, `preset` | `fosscut export clip.gif --range 2s..6s`, `fosscut export Promo.mp4 --resolution 720p --fps 30 --cover 1s --subtitles srt` |

The commands `edit` runs include `clip.add`, `clip.split`, `clip.move`, `clip.trim` (move a clip's start or end edge), `clip.remove`, `text.add`, `text.set` and `track.add`. Before using one the first time, ask `capabilities` for it by name to get its arguments; do not guess field names. `detail: "full"` returns every schema at once and is too large to read comfortably.

## The safe editing loop

1. **Open.** `project_open` with the user's `.fosscut` bundle, or `project_new` (1920x1080 at 30 fps by default; 1080x1920 for vertical video). Media must be imported with `media_import` before clips can use it. When the result's `session.live` is true, the user has the project open in FossCut and watches your edits arrive; they may edit too, so expect revision conflicts.
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
| `fosscut.session.unavailable` | The app holding the project closed or stopped answering. If `details.outcome` is `unknown`, read `history` to see whether the change landed before trying again. |
| `fosscut.guard.destructive` | Ask the user; retry with `allow_destructive` only if they agree. |
| `fosscut.clip.overlap` | The place is taken. Choose a free time, or add a track with `track.add`. |
| `fosscut.time.out_of_range` | The time is past the end of the timeline; check `duration` in `timeline_read`. |
| `fosscut.export.codec_unavailable` | Offer the codecs in `details.available`; do not pick one silently. |
| `fosscut.export.missing_media` | Tell the user which file is missing (`details.tried`) and ask them to restore it. |
| `fosscut.media.probe_unavailable` | FFmpeg's `ffprobe` is not installed; ask the user to install FFmpeg. |
| `fosscut.project.busy` | Another program is writing the project; wait a few seconds and retry. |

If a call times out, retry it with the same `request_id`: FossCut returns the first result instead of applying the change twice.
