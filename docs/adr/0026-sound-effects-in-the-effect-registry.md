# ADR-0026: Apply sound effects from the effect registry in the mixer

- **Status:** Proposed
- **Date:** 2026-10-30
- **Deciders:** maintainers
- **Related:** [ADR-0008](0008-render-and-audio-engine.md), [ADR-0011](0011-effects-and-animatable-properties.md), [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), [ADR-0019](0019-local-analysis-pipeline-and-cache.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (audio panel: normalize loudness, enhance voice, reduce noise, isolate voice)

## Context

People cleaning up talking-head footage and podcasts expect a few sound tools beside volume and fades: reduce steady noise (hiss, hum, room tone), make a voice clearer, keep or remove the voice in a song, and bring clips to a common loudness. The research lists all four in the audio panel, most marked as paid extras; they were inspected, not run.

FossCut already has a registry of effect types with parameter schemas (ADR-0011 item 1) that `effect.add`, `effect.set`, `effect.copy`, `capabilities` and the inspector understand, and one mixer that preview and export share (ADR-0008). Picture effects run per frame on the GPU and may be keyframed; sound has state across samples (noise estimates, filter memory, compressor envelopes), and preview mixes in short blocks from wherever playback starts. No language model runs in the core (ADR-0012); classical signal processing covers these tools well, with known limits.

## Decision

1. **One registry.** Sound effects are effect types like picture effects, named `audio.*` (or `voice.*` for the voice filters of item 8), with the category `audio`. `capabilities` lists them with their parameters, and `effect.add`, `effect.set`, `effect.remove`, `effect.move` and `effect.copy` handle them unchanged. Clips keep picture and sound effects in their one `effects` list; the planner skips sound effects and the mixer skips picture effects.
2. **The mixer applies them.** For each clip, the mixer runs its enabled sound effects in order on the clip's sound after speed (ADR-0023) and before volume, pan, channels, fades and transitions. Implementations live in `fosscut-audio` (`effects.rs`), are deterministic, and use no model.
3. **Where they go.** Sound effects go on clips with sound: audio clips, video clips whose asset has audio, and compound clips. Picture effects go on clips with a picture. Commands refuse the other combinations with `fosscut.effect.wrong_target`, and validation reports `fosscut.model.effect_target`.
4. **Constant parameters.** Sound effect parameters do not take keyframes (`animatable: false`) in this version, so a clip's sound is processed with one setting from start to end.
5. **Same sound in every block.** State starts fresh at the clip's start. To mix a range, the mixer feeds the effects up to `WARM_UP_SECONDS` (2 s) of the clip before the range and their look-ahead after it (silence past the clip's end), with sample positions counted from the clip's start so analysis windows line up. Preview and export then agree within 1e-3 of full scale, exactly at the start of a clip.
6. **First types.**
   - **`audio.denoise` (amount):** suppresses steady background noise. It uses a short-time Fourier transform, a minimum-statistics noise estimate (Martin, 2001) and a decision-directed Wiener gain (Ephraim and Malah, 1984), with a floor down to -24 dB. Sounds held for longer than about 1.4 s count as noise, so the tool suits speech, not sustained music.
   - **`audio.enhance_voice` (amount):** applies a rumble high-pass, a cut of muddy low mids and a presence lift (RBJ cookbook biquads). A compressor with makeup gain then evens out the level.
   - **`audio.vocals` (mode isolate or remove):** the classical centre-channel method. Mid and side are separated, and the band-limited mid keeps a voice mixed in the middle of a stereo track. It does nothing on mono, and it is not source separation; a separation model may replace it later under ADR-0012.
   - **`audio.limiter` (gain, ceiling):** a look-ahead peak limiter. It raises the level by a gain (0 to 24 dB) and holds every sample peak at or under a ceiling (-12 to 0 dBFS). The gain each sample needs is held at its minimum over the next 5 ms, averaged over 5 ms and released over 80 ms, so no peak passes.
7. **Loudness normalization is not an effect.** `fosscut_engine::loudness::normalize` measures each chosen clip as it plays, alone, at volume 1 and without its own limiter, with the ITU-R BS.1770 meter (ADR-0019).
   - **Enough room:** when the clip's peaks leave room under the ceiling (default -1 dBFS), it sets the volume that reaches the target (default -14 LUFS) and removes a limiter left by an earlier normalization.
   - **Too little room:** it adds or adjusts the clip's `audio.limiter`, with the gain measured again, up to four times, until the clip reaches the target within 0.1 LU or the gain range ends.
   - **Without a limiter:** with `limit` off, it only sets volumes and reports the clips the ceiling held back.
   - **Applying:** it returns `clip.set` and effect commands, which the front ends (`fosscut audio normalize`, the MCP `audio_normalize` tool, the desktop) apply as one undoable change.
8. **Voice filters and presets.** The `voice.*` types change a voice's character: `voice.pitch` (semitones, formants kept or not), `voice.robot`, `voice.telephone`, `voice.megaphone`, `voice.echo`, `voice.reverb`, `voice.chorus`, `voice.broken` and `voice.whisper`. Their specs live in `fosscut_model::voice_filters` and their processing in `fosscut_audio::voice`, written from published methods (WSOLA, cepstral envelopes, RBJ filters, a feedback delay network).
   - **Presets:** named sets of voice filters (deep, chipmunk, robot, phone call, radio, megaphone, cave and others) are data, listed by `capabilities` under `voice_presets`. The command `voice.apply` copies a preset's effects into a clip, which owns them; with `replace` it swaps only the clip's `voice.*` effects. An unknown name fails with `fosscut.voice.preset_unknown`.
   - **Warm-up per chain:** item 5's warm-up is the sum of what each effect of a clip remembers, capped at 2 s, so a chain of short filters reads less before a range.

## Consequences

- Agents and people find sound tools where they find effects, with schemas, copy and undo, and no second effect system to maintain.
- Every mixed block reads up to two extra seconds of a clip with sound effects. That is cheap for decoding and adds a fixed delay to effects-heavy preview starts.
- Without keyframes, a fade of the denoise amount needs two clips. Allowing keyframes later means processing in segments with crossfades, and changing `animatable` requires a new effect version.
- The vocal tool's limits must be stated wherever it is offered, so people do not expect separation quality.

## Alternatives considered

- **A separate list of audio effects on clips.** Rejected: it doubles every effect command and the inspector's effect panel for no gain, since the registry already describes parameters.
- **Process sound effects offline into rendered files.** Rejected: it makes edits slow, duplicates media against ADR-0004, and goes stale on every trim.
- **A model for denoise and voice isolation now.** Rejected for this step: it needs a download with licence review and an ADR-0012 cost path; the classical tools ship today and keep the same effect names for an upgrade.

## Validation

- Unit tests in `fosscut-audio` measure what each effect does:
  - **Denoise:** noise lowered by 23.6 dB while the tone changes by less than 0.05 dB.
  - **Enhance voice:** rumble cut by 9.7 dB and presence lifted by 6.6 dB.
  - **Vocals:** a centred voice cancels exactly.
  - **Streaming:** windows cut from a clip match the whole clip within 1e-5.
- Mixer tests check that mixing in blocks equals mixing at once within 1e-3, and that disabled effects change nothing.
- Engine tests check that normalized clips play within 0.3 LU of the target and that the ceiling holds peaks. On recorded speech, whose peaks leave no room at -14 LUFS, the limiter brings it to the target with peaks under -0.95 dBFS.
- **Voice filters:** unit tests measure each one: pitch lands within 0.12% of its target, robot sidebands at -6.02 dB, the telephone band and echo repeats where set, a reverb's decay growing with its room, and every filter and preset equal in blocks of 512 to 4800 samples within 1e-3, with peaks no higher than the input's beyond 0.07 dB.
- A real-agent check found the first version, which only set volumes, left dynamic speech at -22 LUFS when -16 was asked: the limiter came from that finding.
- Before Accepted:
  - **Listening test:** on recorded speech with room noise, the denoised result sounds better than the original.
  - **Speed:** effects-heavy preview starts stay under 150 ms on the reference laptop.
