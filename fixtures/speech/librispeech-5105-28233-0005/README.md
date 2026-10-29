# LibriSpeech 5105-28233-0005

An 11.4 second English read-speech recording with a word alignment, used to test speech recognition and word timings (ADR-0020, Validation).

| | |
|---|---|
| Recording | LibriSpeech `test-clean` utterance 5105-28233-0005, read by a LibriVox volunteer |
| Corpus | LibriSpeech ASR corpus by Vassil Panayotov, Guoguo Chen, Daniel Povey and Sanjeev Khudanpur ([openslr.org/12](https://www.openslr.org/12/)) |
| Alignment | Montreal Forced Aligner word alignments shared by Loren Lugosch, as published in the Hugging Face dataset `gilkeyio/librispeech-alignments` by Kim Gilkey, revision `0daa1eb43dda38ee6ce752e785555380e5628f5c`, split `test_clean`, row 9 |
| License | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| Added | 2026-10-29 |

| File | Contents | SHA-256 |
|---|---|---|
| `audio.flac` | The recording as the dataset distributes it: FLAC, 16 kHz, mono, unmodified | `84e9fa1c16d810fc4270e87bfaae86eb8fa4b4013307be94d15916ed468b7400` |
| `alignment.json` | The utterance id, its transcript and its 37 words with start and end times in seconds | `458e69936a236f85bc69ed08c86469113ac33dbdf03c83bdb6db7368fa00e0eb` |

Changes: `alignment.json` keeps only the id, transcript and word timings from the dataset row and is reformatted as JSON. The audio is unchanged.

Transcript: "sometimes he would wander on foot upon the sandy shore and sometimes he would enjoy a ride along the summit of the cliff altogether being in no hurry at all to bring his task to an end".
