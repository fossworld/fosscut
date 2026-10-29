# ADR-0021: Store captions as pages of timed words on caption tracks, animated by declarative animators

- **Status:** Proposed
- **Date:** 2026-10-29
- **Related:** [ADR-0004](0004-open-project-format.md), [ADR-0005](0005-time-model.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0008](0008-render-and-audio-engine.md), [ADR-0011](0011-effects-and-animatable-properties.md), [ADR-0013](0013-agent-facing-editing-skills.md), [ADR-0020](0020-speech-recognition-with-whisper-cpp.md), private research: `docs/capcut-audit/feature-inventory.json` (CC-119, CC-122, CC-124, CC-174), `docs/capcut-rea/RESOURCES.md`, `docs/capcut-rea/DRAFT-FORMAT.md`

## Context

Word-timed captions are the first viral skill of milestone 6. Transcripts with a start and end for every word now exist (ADR-0020), but the project format cannot hold them. A text clip has one string and one style, with no timing inside the clip (ADR-0011, implementation notes).

The forces:

- **What users expect (research, behaviour only):**
  - Captions are shown a few words at a time. Each page holds a line or two, and every word is timed.
  - Animation follows the words: the spoken word lights up or pops, keywords stand out, words fill like karaoke.
  - Subtitle files import, and export as SRT.
  - In the editor studied, generating captions, finding filler words and exporting caption files are marked as paid features.
- **Format rules:** caption text follows ADR-0004 rule 8. It is structured and stored once, never as JSON inside a string.
- **What agents need:**
  - Small units they can read and edit.
  - Deterministic paging.
  - Restyling without rewriting every caption.
  - A way to place transcript words on the timeline without converting times by hand.
- **Readability:** vertical platforms draw their own buttons and text over the bottom and right of the frame, so captions need a safe area per canvas shape.
- **Layering:** the core is pure and cannot read the analysis cache (ADR-0003). Edits must still go through core commands with the safety contract (ADR-0006).

## Decision

1. **Format** (format version 0.2.0; readers accept 0.1.0 files unchanged, which have no captions):
   - **Caption tracks.** The track kind `caption` holds only caption clips and owns the captions' look in `caption_style`. Text tracks keep titles.
   - **Caption clips.** The clip kind `caption` is one page: `lines`, each with `words`.
   - **Words.** Each word has `text`, a `start` and an `end` in flicks relative to the clip's start, and an optional `emphasis` flag. Word times are stored exactly. They never extend beyond the clip.
   - **Clip edges** snap to frames like any clip (ADR-0005 rule 4).
   - **One source of truth.** The page's text is its words joined by spaces; it is not stored a second time.

   ```json
   { "id": "track_0005", "kind": "caption",
     "caption_style": { "weight": "black", "size": 0.055, "uppercase": true, "placement": "bottom",
       "stroke": { "enabled": true, "color": "#000000", "width": 0.12 },
       "animators": [ { "type": "word.highlight", "version": 1, "enabled": true, "params": { "color": "#FFE14D" } } ] },
     "clips": [ { "id": "clip_0012", "kind": "caption", "start": 0, "duration": 635040000,
       "lines": [ { "words": [ { "text": "This", "start": 0, "end": 141120000 },
                               { "text": "changes", "start": 141120000, "end": 423360000, "emphasis": true } ] } ] } ] }
   ```

2. **Caption style**, stored sparsely on the track:
   - **Font:** family, weight (`regular`, `bold`, `extra_bold`, `black`), italic and size (a fraction of canvas height).
   - **Text:** colour, alignment and `uppercase`.
   - **Decoration:** an outline (`stroke`: colour and width as a fraction of the font size) and a background box (`box`: colour, padding and corner radius), each with an explicit `enabled`.
   - **Position:** `placement` (`top`, `middle` or `bottom`) inside the safe area. A clip's transform offsets it.
   - **Animators** (item 3).
   - **Fonts:** only bundled fonts, as for all text. Inter's ExtraBold and Black weights from the same OFL release join the bundle.
3. **Animators** are registered types (`{type, version, enabled, params}`), listed with parameter schemas in `capabilities` (ADR-0011 item 1). They are evaluated per frame from word times alone:
   - **Page:** `page.fade` (in and out), `page.pop` (scale in).
   - **Words:**
     - `word.reveal` shows each word from its start.
     - `word.highlight` gives the spoken word a colour, an optional box and a scale.
     - `word.pop` scales each word up and back as it starts.
     - `word.karaoke` fills each word left to right from its start to its end.
     - `word.emphasis` styles words marked `emphasis`.
   - **The spoken word** is the last word that has started, until it ends. In gaps shorter than 0.3 s, it stays until the next word starts.
   - **Stable layout.** Layout is computed once per page at rest, so animation never reflows text.
   - **Later:** selectors by character and line (ADR-0011 item 5).
4. **Presets are data** (ADR-0011 item 7): built-in JSON documents in `fosscut-core`. The first four are `bold-pop`, `highlight`, `karaoke` and `boxed`, each written for FossCut. Applying a preset writes its values into the track, so a project never depends on a preset file. `capabilities` lists the presets.
5. **Safe areas** depend on the canvas shape:
   - taller than wide: 6% margins left and right, 10% at the top, 22% at the bottom;
   - otherwise: 5% on every side.

   Lines wrap inside the safe width. The renderer never moves captions outside the safe area unless a transform does.
6. **Core commands** (ADR-0002, ADR-0006):
   - **`captions.add`** takes timed words in timeline time, with explicit units, plus an optional preset or style. It pages them into clips on a caption track, creating one when none is named. Paging is deterministic:
     - A new page starts after sentence-ending punctuation, after a gap of `max_gap_ms` (default 600), or when the next word would exceed `max_words` (default 4) or `max_lines` (default 2) lines of `max_chars_per_line` (default 18 on tall canvases, 32 otherwise).
     - A page lasts from its first word to its last word's end plus `linger_ms` (default 300), and never past the next page.
     - It never overwrites. `replace: true` clears the track first and needs `allow_destructive`.
   - **`captions.edit`** replaces a page's words. Given new `text` instead of timed words, the page's time is shared across the new words by character count. It can also set `emphasis` by word index.
   - **`captions.style`** applies a preset and then a style patch to a caption track.
   - **Existing clip commands:** `clip.split`, `clip.move`, `clip.remove` and trims work on caption clips. A split gives each word to the side that holds most of it. Ripple edits (milestone 6 cleanup) keep words aligned with their media.
7. **Engine operations**, exposed by the CLI (`fosscut captions generate|import|export`) and one new MCP tool, `captions`:
   - **`generate`** reads the cached transcript of each clip on a source track (the timeline's first video or audio track by default). It maps every word inside the clip's used source range to timeline time, then runs `captions.add` with the caller's `expected_revision`, `dry_run` and `request_id`. With no cached transcript it fails with `fosscut.captions.no_transcript`, and the hint is to run `media_analyze` with `transcript`.
   - **`import`** reads SRT (UTF-8, LF or CRLF): one cue becomes one page, and word times are spread over the cue by character count.
   - **`export`** writes SRT with one cue per page, times rounded to the millisecond and lines kept. A file exported from millisecond-aligned times imports back unchanged.
8. **Errors:** `fosscut.captions.invalid_words` (unordered, overlapping or empty), `fosscut.captions.unknown_preset`, `fosscut.captions.no_transcript`, `fosscut.captions.invalid_srt` (with the line number) and the existing `fosscut.track.kind_mismatch` for caption clips on other tracks.
9. **Skill:** `skills/viral-captions` (ADR-0013) runs the flow, with an evaluation. The flow:
   1. transcript;
   2. generate with a preset suited to the canvas;
   3. mark keywords with `captions.edit`;
   4. check frames at spoken words;
   5. export.

## Consequences

- **For agents:** captions become a few commands, and the captions stay editable afterwards: word by word, page by page, or the whole look at once.
- **For users:** caption generation, styling, SRT import and export run locally and free.
- **Format:** the project format gains its first new kinds since 0.1.0, so the version moves to 0.2.0. Older FossCut builds refuse 0.2.0 files with a clear error.
- **New maintenance:** animators need golden frames and a frame evaluation shared by preview and export (ADR-0008). Every new animator adds to that set.
- **Imported timing:** SRT cues carry no word times. Words in imported captions get estimated times, which `generate` can replace once a transcript exists.
- **Fonts:** the bundled font set grows by two files.

## Alternatives considered

- **Captions as ordinary text clips with style runs:** no new kinds, but word timing and animators would sit on every title. Restyling would also rewrite each clip.
- **One clip holding the whole transcript, paged when rendering:** compact, but agents could not edit or trim a page, and clip commands and ripple edits would have to reach inside it.
- **Style on every clip instead of the track:** each page fully self-contained, but restyling touches every clip and projects grow large. A track-level style, plus `emphasis` on words, covers the observed uses.
- **Free-form expressions for animation:** flexible, but agents cannot validate or discover them (ADR-0011 item 3).
- **Paging in the agent:** no core logic, but every agent would page differently, and long transcripts cost many tokens.

## Implementation notes

- **Paging at frame edges.** A page ends on the frame of the next page's first word. When two words meet inside one frame, the earlier word's end is cut to the frame, so every word start stays exact; highlights begin there.
- **Explicit breaks.** `captions.add` words take `new_page` and `new_line`. SRT import uses them, with limits high enough that only the cues' own breaks apply. Cue edges are snapped to frames before paging, so an exported file imports back to the same frames.
- **Captions follow the edit.** A page never lingers past the end of the timeline's other tracks, so captions do not lengthen an export; a real-agent run found the last page adding black frames before this rule.
- **Line length.** By default, as many characters as fit the safe width at the track's size, from an average glyph width (wider for capitals and the black weight), at most 18 on tall canvases and 32 otherwise.
- **No captions to export** is `fosscut.captions.none`. Exports return a preview of the first cues, so an agent without file access can check them. With no speech model, `generate` reports `fosscut.captions.no_transcript` too, since no transcript can exist.
- **Spacing.** Outlines widen the space between words by 1.5 times their width. Presets keep lasting scale changes off emphasised words; only brief pops grow words into the gaps.

## Validation

- **Paging property tests:**
  - every word appears exactly once, in order, inside its page;
  - pages never overlap and respect the limits;
  - the same input always gives the same pages.
- **Golden frames:**
  - each animator at fixed times;
  - both canvas shapes with every placement, inside the safe areas.
- **Clip edits:** splitting, trimming and moving a caption clip keep every word, and times stay relative to the clip.
- **Migration:** 0.1.0 projects load and save as 0.2.0 unchanged apart from the version, and round-trip tests cover caption tracks.
- **SRT:** round trip exact for millisecond-aligned times; malformed files fail with a line number.
- **Real agent:** on the speech fixture, the `viral-captions` evaluation covers every transcript word with a caption, keeps the captions inside the 9:16 safe area, shows the highlight on the spoken word in a frame grab, and exports a verified file.
