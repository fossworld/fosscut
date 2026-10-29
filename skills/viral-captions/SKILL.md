---
name: viral-captions
description: Add word-timed, animated captions to a video with FossCut, transcribed on this machine. Use when the user asks for captions or subtitles, word-by-word, highlighted or karaoke captions, captions for a short or a vertical video, or to import or export an SRT file.
---

# Viral captions

Follow the safe editing loop of the `fosscut-basics` skill (read, plan with a dry run, apply with `expected_revision` and a `request_id`, check, export). This skill adds the caption steps.

## What a good result looks like

- Every spoken word appears, at the moment it is spoken, a few words at a time.
- Captions stay inside the safe area: FossCut keeps them clear of the margins where platforms draw their buttons (22% at the bottom of tall videos), unless you move them.
- The look suits the video, and keywords stand out without every word shouting.
- The user has seen a frame of the captions before you export.

## Steps

1. **Read.** `timeline_read` to find the clips with speech and the canvas shape (tall for shorts, wide otherwise).
2. **Transcribe.** `media_analyze` with `analyzers: ["transcript"]` for each asset with speech. Use `job_status` while it runs.
   - `fosscut.model.not_installed`: list the models with `models`, tell the user each size and license, and install one with `confirm: true` only after they agree.
   - Suggest whisper-base.en for English and whisper-small or whisper-large-v3-turbo for other languages. The first run of a large model is slow.
   - If the user names the language, pass `options: {"transcript": {"language": "de"}}`. Use the same transcript options when you generate.
3. **Choose a look.** `capabilities` lists the presets under `captions`:
   - `bold-pop`: heavy capitals that pop; punchy shorts.
   - `highlight`: the spoken word turns yellow; talking heads and tutorials.
   - `karaoke`: words fill as they are spoken; lyrics and paced narration.
   - `boxed`: words on a dark box; busy or bright footage.

   Ask the user if they have not said. Keep the default bottom placement unless something important sits there (a lower third, a product); then use `style: {"placement": "middle"}` or "top".
4. **Generate.** `captions` with `action: "generate"`, the preset and `expected_revision`, first with `dry_run: true`. Then apply with a `request_id`.
   - It captions the clips of the first video or audio track with sound. For another track, pass `source_track`.
   - Pages hold up to 4 words and 2 lines by default. For fast punchy captions use `paging: {"max_words": 3, "max_lines": 1}`.
   - Generating again on a track that has captions fails with `fosscut.clip.overlap`. Replacing them needs `replace: true` and `allow_destructive`: ask first.
5. **Polish.** `timeline_read` with `detail: "full"` shows every caption word with its timeline time.
   - Fix misheard words: `edit` with `captions.edit`, new `text` for the page, which keeps its timing.
   - Mark 1 to 3 keywords per sentence: `captions.edit` with `emphasis` word indexes. Presets style them through their `word.emphasis` animator.
   - Change the look with `captions.style`: a preset and/or a style patch such as `{"size": 0.07}`. Ask `capabilities` with `names: ["captions.style"]` for the fields.
   - Low-confidence words in the transcript (probability under 0.5) are worth reading back to the user.
6. **Check.** `frame_grab` in the middle of a few spoken words: the spoken word's highlight, the caption inside the frame, nothing important covered. A `contact_sheet` shows the pacing. Show the user.
7. **Export.** `export` as usual. If they want a subtitle file too, `captions` with `action: "export"` and a `.srt` file; its reply shows the first cues so you can check the text. Overwriting needs `allow_destructive`.

## Subtitle files

- **Import.** `captions` with `action: "import"` and a `.srt` file adds one caption page per cue, keeping its lines. Words get times spread over the cue, so highlight and karaoke follow the cue roughly. If the media has speech, generating from a transcript is more precise.
- **Export.** Writes one cue per page, times to the millisecond.

## When something fails

- `fosscut.captions.no_transcript`: transcribe the listed assets first, with the same options.
- `fosscut.revision.conflict`: read again and re-plan.
- Captions in the wrong place after a reframe: change `placement` rather than moving each clip.
