# ADR-0033: Run local vision and voice models with ONNX Runtime

- **Status:** Proposed
- **Date:** 2026-10-31
- **Deciders:** maintainers
- **Related:** [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), [ADR-0014](0014-licensing.md), [ADR-0019](0019-local-analysis-pipeline-and-cache.md), [ADR-0020](0020-speech-recognition-with-whisper-cpp.md), [ADR-0021](0021-captions-as-timed-word-clips.md), [ADR-0011](0011-effects-and-animatable-properties.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (automatic background removal, body effects, text to speech, video translator)

## Context

Four editing features need trained models:

- **Automatic background removal:** a subject separated from its background in every frame.
- **Body effects:** looks that follow a person, such as outlines and glows.
- **Text to speech:** a voiceover from written text.
- **Video translator:** speech in one language as captions in another, and a voice in the target language.

The research shows all four behind paid plans in the reference editor. FossCut runs every model locally and installs it only when asked (ADR-0012, ADR-0020). Weights must be under licenses the project allows (ADR-0014).

The maintainers surveyed candidates on 2026-10-31 and chose these:

- **BiRefNet_lite** (MIT) for subject mattes.
- **MediaPipe Pose Landmarker** (Apache-2.0) for body keypoints and a person mask.
- **Kokoro-82M** (Apache-2.0) for English voices. Pronunciation comes from CMUdict (BSD-2-Clause), mapped by FossCut to Kokoro's phonemes, with FossCut's own rules for words it lacks. `misaki-rs` was dropped: it depends on a WTFPL crate, and its dictionaries were corrected with GPL eSpeak output.
- **OPUS-MT** for English and Hungarian (Apache-2.0).

These were excluded:

- **For their licences:** RobustVideoMatting (GPL-3.0), BRIA RMBG (non-commercial), Ultralytics YOLO (AGPL), NLLB, SeamlessM4T and F5-TTS (non-commercial), and Piper's Hungarian voices, which were fine-tuned from research-only data.
- **No Hungarian voice:** no small, cleanly licensed model for Hungarian text to speech exists, so Hungarian is translation only.

## Decision

1. **One inference runtime.** Models in ONNX form run through the `ort` crate (MIT or Apache-2.0) with its dynamic loading. FossCut ships or installs Microsoft's ONNX Runtime library (MIT) the way it links FFmpeg dynamically (ADR-0009).
   - **Where the library is found:** at `FOSSCUT_ORT_DYLIB`, then beside the executable, then in the models folder. `fosscut models install onnxruntime` installs it there from Microsoft's release, verified by SHA-256.
   - **Hardware:** sessions run on the CPU by default, which is deterministic; CoreML on macOS and DirectML on Windows are opt-in, falling back to the CPU when a model does not compile for them.
   - **Where the code lives:** `fosscut-analysis`, behind a default feature `onnx`. A missing runtime is the typed error `fosscut.model.runtime_missing`, never a crash.
2. **Models are manifests of files** (extending ADR-0020 item 2). Each has a name, a task, a license with its attribution, and files with address, size and SHA-256. `fosscut models` and the MCP `models` tool list and install them only after confirmation. Nothing downloads on its own.
3. **Background removal is a cached analysis, then an effect.**
   - **Measuring:** the analyzer `matte` runs BiRefNet_lite on a clip's frames. It smooths the alpha mattes over time and caches them by content, model and options (ADR-0019), like the other analyzers.
   - **Applying:** the effect `key.subject` (with `feather`, `threshold`, `invert` to remove the subject instead, and `background` for what shows behind it) reads the cached matte as the clip's alpha. The renderer never runs a model.
   - **Missing mattes:** an export of a clip whose matte is missing fails before rendering with `fosscut.analysis.missing`, which names the analysis to run.
4. **Body effects follow cached pose and person masks.**
   - **Measuring:** the analyzer `pose` runs the Pose Landmarker and caches 33 keypoints per frame, with visibility, plus its person mask.
   - **Applying:** the effects `body.outline`, `body.glow` and `body.shadow` draw from the cached person mask, and `body.sparkle` places particles at keypoints. They are ordinary registered effects with parameter schemas (ADR-0011), rendered on the GPU from the cached data.
5. **Text to speech writes an audio asset.**
   - **Engine:** `fosscut_engine::voice` synthesizes text with a chosen Kokoro voice into a 24 kHz WAV in the bundle's `speech/` folder. The file is named by text, voice and model, so the same request reuses it. The engine then imports it and places it like a library sound.
   - **Determinism:** the same text and voice give the same file.
   - **Languages:** English only. Other languages report `fosscut.voice.language_unavailable`.
6. **Translation fills caption translations.** `captions translate` translates a caption track's pages with an installed OPUS-MT pair into each page's `translation` (ADR-0021 bilingual captions), or replaces the words.
   - **Timing:** translated pages keep their page timing, with word timings spread across each page.
   - **Dubbing:** for English targets, a translated track can be dubbed with text to speech, one clip per page, placed at the page's start.
   - **Agents:** they may translate captions themselves instead, with `captions.edit` (ADR-0012).
7. **No model in the core.** All of this sits in `fosscut-analysis` and `fosscut-engine`. Commands stay deterministic and only apply results, and the core builds for WebAssembly unchanged.

## Implementation notes

- **Runtime:** ONNX Runtime 1.31.1 from Microsoft's releases, and 1.23.2 on Intel Macs, the last release with a build for them. It loads through `ort` 2.0.0-rc.15 with `load-dynamic`, which accepts 1.22 and later, and ONNX Runtime's telemetry is switched off.
- **Hardware:** CoreML and WebGPU did not run these models correctly or faster: the vision models fail to compile, and the voice and translation models compile and then fail while running. Microsoft's Windows build has no DirectML. Inference therefore runs on the CPU by default, and accelerators are opt-in for a model checked on them.
- **Speed:** BiRefNet_lite takes 9 to 14 s per 1024 by 1024 frame on an Apple M3 Pro CPU.
- **Pose:** Google publishes Pose Landmarker only as TFLite, so FossCut converts it with the scripts in `scripts/models/` and hosts the result on the `models-v1` release.
- **Translation:** English to Hungarian uses the older `opus-mt-en-hu` (Apache-2.0), because `opus-mt-tc-big-en-hu` has no ONNX export.
- **Mattes:** the default matte is the pose model's person mask (`model: person`, about 0.2 s a frame in a debug build). BiRefNet (`model: subject`) accepts only 1024 by 1024 input, so long clips use `every` to measure every Nth frame and follow the subject between them. Mattes and landmarks are smoothed with a bilateral filter in time, and a person seen in one frame only is dropped.
- **Pronunciation:** CMUdict (BSD-2-Clause, installed with the voice model) is mapped to Kokoro's phonemes by FossCut's own table, with rules for British voices and for words, numbers and abbreviations it lacks. `misaki-rs` was dropped: it depends on a WTFPL crate, and its dictionaries were corrected with GPL eSpeak output.
- **Captions of spoken text:** each speech file has its text beside it (`speech/<name>.txt`). Captions generated from such a clip keep the recognizer's timing but spell the words as written, so "FossCut" is never captioned as "faucet".
- **Translation in order:** sentences are translated one segment at a time by default. Batching is about twice as fast, but the 8-bit models quantize a whole batch together, which changed 3 of 10 test translations.

## Consequences

- One runtime serves vision, voice and translation, with hardware acceleration and no Python.
- Exports stay deterministic and fast, because models run once per clip as cached analyses, not while rendering.
- **Shipping:** releases ship or offer one more native library (about 10 MB on macOS and Linux, 75 MB on Windows), and the models need checksums kept current.
- **Attribution:** every model carries its credit line, shown wherever models are listed and in the export's rights notes.
- **Gaps:** Hungarian has no voice yet, and mattes are per frame with smoothing rather than a recurrent video model, so fast motion may flicker slightly.

## Alternatives considered

- **candle (pure Rust):** it needs no native library, but covers fewer of these models; it can be revisited for translation.
- **Running models while rendering:** exports would be slower and depend on hardware, and the previews and exports of one frame could differ.
- **Cloud services:** these are against ADR-0012's local-first rule, and would cost money and privacy.

## Validation

- **Tests with the installed models:**
  - **Mattes:** they separate a person from a generated background.
  - **Pose:** keypoints land on a figure.
  - **Voice:** spoken text is recognised again by whisper as the same words.
  - **Translation:** English to Hungarian and back keeps the meaning of short sentences (checked for known words).
- **Before Accepted:** a person reviews mattes and body effects on real footage on macOS and Windows.
