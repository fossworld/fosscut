---
name: long-to-shorts
description: Find the parts of a long FossCut video (a podcast, interview, talk or stream) that stand alone, and make vertical shorts of them with captions and a title, framed on the speaker. Use when the user asks to turn a long video into shorts, clips, Reels or TikToks, to find highlights or the best moments, or to repurpose a video for social media.
---

# Long video to shorts

Follow the safe editing loop of the `fosscut-basics` skill (read, plan with a dry run, apply with `expected_revision` and a `request_id`, check, export). This skill adds the steps that find and build shorts.

## What a good result looks like

- Each short is a new vertical timeline (1080x1920) of one stretch of whole sentences: it starts on a hook and ends on a finished thought, between 15 and 60 seconds unless the user wants otherwise.
- The speaker stays in frame shot by shot, and every spoken word is captioned inside the safe area.
- The source timeline is unchanged, and the user chose which candidates became shorts.

## Steps

1. **Read** the timeline with `timeline_read` and pick the long clip with speech.
2. **Measure** its asset with `media_analyze`: `analyzers: ["transcript", "levels", "scenes"]`. Only the transcript is required; the loudness over time (`levels`) and the shot cuts (`scenes`) improve the scores, and `shorts.missing` names any that were not measured. For framing on a person also add `"pose"` (it needs the `mediapipe-pose-full` and `onnxruntime` models). Follow running jobs with `job_status`. Missing models: tell the user each size and license from `models`, and install only with their yes.
3. **Find candidates.** `auto_edit` with action `short_candidates`, the `clip`, and `min_seconds`, `max_seconds` and `count` when the user named lengths. Each candidate has its `text`, `score` and `reasons` (speech pace, pauses, loudness peaks, shot cuts, whether it starts and ends on sentence boundaries), and its span in the media and on the timeline.
4. **Choose with the user.** Show the top candidates: the opening words, the length and why each scored. Read the text yourself as well: prefer one that opens with a question, a strong claim or a surprise, and skip one that depends on what came before it. Let the user pick, or pick the best two or three when they asked you to.
5. **Build each short.** `auto_edit` with action `make_short`, the `clip`, `from` and `to` set to the candidate's `span.start` and `span.end` (media time), a `name`, `caption_preset` (`bold-pop` suits most shorts) and a `title` of a few words that names the hook. It frames the speaker shot by shot when the pose analysis exists (`subject: "person"`), or keeps the picture centred with `subject: "centre"`. Run it with `dry_run: true` first when you build several.
6. **Check** each short with `contact_sheet` and a `frame_grab` at its first spoken word: the title and captions read clearly and nobody is cut off. Adjust captions with the `viral-captions` skill, and the title with `text.set`.
7. **Export** each short with `export` and its `timeline`, and report its file, length and the line it opens with.

## When something fails

- `fosscut.analysis.missing`: run the analysis the message names, then retry.
- `fosscut.shorts.no_candidates`: no stretch of whole sentences fits the lengths. Widen `min_seconds` and `max_seconds`, or check the transcript has speech.
- A short that starts mid-thought: move `from` back to the previous sentence's start from the transcript's words.
- `fosscut.reframe.no_subject`: the short stays centred and says why; frame it by region with the `vertical-reframe` skill.
- `fosscut.revision.conflict`: read again and re-plan.
