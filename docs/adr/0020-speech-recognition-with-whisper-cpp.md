# ADR-0020: Recognize speech locally with whisper.cpp, with word timings

- **Status:** Proposed
- **Date:** 2026-10-29
- **Related:** [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), [ADR-0014](0014-licensing.md), [ADR-0016](0016-testing-strategy.md), [ADR-0019](0019-local-analysis-pipeline-and-cache.md), private research: `docs/capcut-rea/RESOURCES.md`

## Context

Word-timed captions and talking-head cleanup, the first viral skills of milestone 6, start from a transcript whose words carry their own times. ADR-0012 asks for local speech recognition with word-level timestamps and language detection, within 100 ms of a reference for English and faster than real time on an M1-class Mac. ADR-0019 decides how analyzers, their cache and model installation work; this record decides the recognizer.

The forces:

- **Word timing quality decides caption quality.** Whisper models recognise well across languages, but their native timestamps are segment-level. Word times come either from token timestamps or from aligning tokens with the model's cross-attention (dynamic time warping, DTW).
- **whisper.cpp** (MIT) runs Whisper on CPU and on Apple GPUs (Metal), and implements token timestamps and DTW alignment for every Whisper model size. **`whisper-rs`** (Unlicense, a public-domain dedication) builds it from source with CMake and exposes segments, tokens, token times, DTW times and the detected language.
- **The pure-Rust alternative** (`candle`'s Whisper, MIT or Apache-2.0) avoids the C++ build but has no word alignment; we would have to build DTW ourselves.
- **Models.** Whisper weights are MIT. Converted `ggml` files for every size are published with SHA-256 sums in the whisper.cpp model repository on Hugging Face.
- **Research, behaviour only.** In the editor studied, large-scale speech recognition for captions runs in the cloud and filler-word detection is a paid option. A local, free recognizer is FossCut's opening.
- **Disfluencies.** Whisper tends to write clean text and drop "um" and "uh", which cleanup needs to see.

## Decision

1. **Runtime.** Speech runs on whisper.cpp through `whisper-rs`:
   - **Feature flag:** it sits behind the `speech` feature of `fosscut-analysis`, on in the CLI and MCP builds, so the crate still builds without a C++ toolchain when it is off.
   - **Backends:** Metal on macOS and the CPU elsewhere. CUDA and Vulkan come later behind further features.
   - **Licensing:** the Unlicense joins the dependency allow list (ADR-0014), as a public-domain dedication like the already allowed CC0-1.0.
2. **Models** are `ggml` files from `huggingface.co/ggerganov/whisper.cpp`, all MIT, listed in the manifests of ADR-0019 item 5 with their sizes and SHA-256:

   | Name | File | Size | Languages |
   |---|---|---|---|
   | `whisper-tiny.en` | `ggml-tiny.en.bin` | 77.7 MB | English |
   | `whisper-base.en` | `ggml-base.en.bin` | 148 MB | English |
   | `whisper-small` | `ggml-small.bin` | 488 MB | 99 languages |
   | `whisper-large-v3-turbo` | `ggml-large-v3-turbo.bin` | 1.62 GB | 99 languages |

   - **Downloading:** with the system `curl` (shipped with macOS, Linux distributions and Windows 10 and later), so FossCut carries no TLS stack of its own.
   - **Offline:** `fosscut models install <name> --from <file>` verifies and installs a file obtained some other way.
3. **The `transcript` analyzer** reads the audio as 16 kHz mono and returns:
   - the language: the option `language`, English for English-only models (which cannot detect one), otherwise detected;
   - the model used;
   - segments with their text, times (from their first and last words) and no-speech probability;
   - words with their text, times and probability.

   Words are assembled from tokens: a token that begins with a space starts a word, and punctuation stays attached. Times are in the asset's own time, like every analyzer, and come from DTW alignment with the model's preset:
   - **Token times.** whisper.cpp stamps each token with the moment the alignment moves on to the next token, so a token ends at its own stamp and starts at the stamp of the token before it. Both are moved 80 ms earlier, the lag measured against reference alignments (Validation). Tokens without a stamp keep whisper's token timestamps.
   - **Quiet edges.** Alignment smears words over the pauses around them, so each word's start and end move inward past quiet 10 ms windows: below both 10 dB over the background (the 10th percentile level) and 30 dB under the loud parts (the 95th percentile). Every word keeps at least 30 ms.
4. **Model choice.** The option `model` defaults to the best installed model that fits the language, in the order large-v3-turbo, small, base.en, tiny.en (`.en` models only for English). With none installed, the analyzer fails with `fosscut.model.not_installed`, whose hint names each model with its size.
5. **Disfluencies.** Each transcript reports:
   - **filler candidates:** words matching a per-language list, for English "um", "umm", "uh", "uhm", "erm", "er", "ah", "hm", "hmm" and "mm", ignoring case and punctuation;
   - **pauses:** gaps between consecutive words of at least `min_pause_ms`, default 300.

   Both are reported and never removed; cleanup is the agent's decision (ADR-0019 item 6). Unless `keep_fillers` is false, recognition starts from a short prompt written with disfluencies, so the model keeps writing them.
6. **Jobs.** Long media runs as an analysis job (ADR-0019 item 4), with progress from whisper.cpp's callback and cancellation through its abort callback. whisper.cpp's own logging is switched off; failures surface as `fosscut.analysis.speech_failed`.

## Consequences

- Captions and cleanup get word times and filler candidates on the user's machine, offline once a model is installed and free of charge.
- Building FossCut needs CMake and a C++ compiler, which GitHub's runners include. The first build takes longer.
- The 80 ms lead and the quiet thresholds are calibrated on two recordings. Retuning them on a larger evaluation set changes results, so it bumps the analyzer's version.
- A model must be installed before the first transcript. Agents learn this from the error and its hint, and ask the user, because a download is involved.
- Accuracy and speed depend on the model: tiny.en is quick but makes more mistakes. Skills should prefer a larger model when the user agrees to the download.
- Whisper can still miss fillers or invent words in silence. Pauses are cross-checked against the `silences` analyzer, and skills check frames and listen points before cutting.

## Alternatives considered

- **`candle` Whisper (pure Rust):** no C++ build and licenses already allowed, but no word alignment; building DTW ourselves risks the 100 ms target and delays captions.
- **ONNX Runtime models (for example via `sherpa-onnx`):** a broad model choice, but a native runtime download and a second model format, with word timing that varies by model.
- **A built-in HTTP and TLS stack for downloads:** self-contained, but the usual root-certificate crate's license is outside the allow list, and the stack adds a native build. `curl` is everywhere FossCut runs.
- **Cloud recognition:** better on hard audio, but ADR-0012 keeps analysis local, private and free.

## Validation

- **Fixture:** LibriSpeech test-clean utterance `5105-28233-0005` (11.4 s, 37 words; CC BY 4.0) with its Montreal Forced Aligner word alignment, committed with a license record.
- **Accuracy:** with `whisper-tiny.en`, at least 90% of the reference words are recognised in order, their start times have a median error of at most 100 ms, and at least 90% of the words start within 300 ms. The reading's two pauses (3.45 to 3.82 s and 6.94 to 7.63 s) are found.
- **Measured** with `whisper-tiny.en` on an Apple M3 Pro:
  - on the fixture, all 37 words, a median start error of 60 ms, 95% within 300 ms and 73% within 100 ms; both pauses found;
  - on a held-out recording of 24 synthesized words joined by known gaps, a median start error of 7 ms and none above 300 ms;
  - taking DTW stamps as token starts instead, as first implemented, put words 340 ms late on the fixture and about 500 ms late on the held-out recording.
- **Speed:** transcription runs faster than real time on the CI runners and on an M1-class Mac with `whisper-base.en`.
- **Cache:** a second request for the same file and options comes from the cache.
