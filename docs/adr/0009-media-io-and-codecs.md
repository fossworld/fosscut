# ADR-0009: FFmpeg under LGPL plus platform encoders, behind backend traits

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0008](0008-render-and-audio-engine.md), [ADR-0010](0010-desktop-shell-ui-and-web-path.md), [ADR-0014](0014-licensing.md), private research: `docs/capcut-rea/ARCHITECTURE.md`, `docs/capcut-audit/README.md`

## Context

Shipping FFmpeg under the LGPL rules out its best software H.264 and HEVC encoders (x264 and x265 are GPL), so permissively licensed editors rely on platform hardware encoders for those formats. In round 1, CapCut's export dialog said H.264 but the file was HEVC, a mismatch the user could not see. Short-form creators need H.264 and HEVC MP4, ProRes for pro handoff, alpha video for overlays, GIF, and audio-only exports. A future web build cannot ship FFmpeg's native libraries and must use the browser's WebCodecs.

## Decision

1. **Backend traits** in `fosscut-media`: `Demuxer`, `Decoder`, `Encoder`, `Muxer`, `Prober`, each with capability reporting. The engine depends only on the traits.
2. **Native backend:** FFmpeg (current stable major) linked **dynamically**, built or obtained **without GPL or nonfree components** for official FossCut distributions, through maintained Rust bindings. Hardware encoders first: VideoToolbox (macOS), Media Foundation, NVENC, AMF or QSV (Windows), VA-API, NVENC (Linux). dav1d for AV1 decode.
3. **Software H.264/HEVC encoding is not bundled** (x264 and x265 are GPL). When no hardware encoder is available, FossCut offers AV1 (SVT-AV1 or rav1e, permissive licenses) or lets the user point FossCut at their own FFmpeg build. This trade-off is surfaced, not hidden.
4. **Web backend (later):** WebCodecs for decode and encode, with Rust demuxers and muxers for MP4 and WebM compiled to WebAssembly.
5. **Export settings are validated before rendering.** The requested container, codec, profile, pixel format, frame rate, resolution, bitrate and color metadata are checked against the probed capabilities of this machine. If something is unsupported, the export fails with a typed error that lists alternatives; FossCut never silently substitutes a codec.
6. **Every export is verified.** After writing, FossCut probes the output and returns a report: container brand, codecs, profile, resolution, frame rate, frame count, duration, audio layout, color tags, file size. Agents receive this report and can compare it to the request.
7. **Proxies and intermediates** use a fixed, hardware-friendly profile and are stored in the cache directory.
8. **Probing** records stream timebases, rotation, color primaries, transfer, matrix and range, variable frame rate, and alpha, because the time model and color pipeline depend on them.

## Implementation notes

- **Bindings:** `rsmpeg` (MIT) over `rusty_ffmpeg`, linked dynamically against FFmpeg 8. `ffmpeg-next` was not chosen because its WTFPL license is outside the dependency policy in `deny.toml`.
- **Decoding** (milestone 2): `fosscut-media` exposes `VideoDecoder` with exact flick timestamps, `frame_at` seeking that returns the frame displayed at a time, color tags with documented defaults for untagged streams, rotation from the display matrix, and VideoToolbox hardware decoding with software fallback, also partway through a stream: a decoder in `Auto` mode whose hardware decode fails (for example when the system has no decode sessions left) reopens the file in software and carries on from the same frame, while `Require` reports the failure. Pictures stored as R'G'B', such as PNG, are converted with BT.601 in limited range and tagged so. Frames are copied to system memory as NV12 or YUV 4:2:0; formats such as 4:2:2 and 10-bit are converted to 8-bit NV12 for now. Zero-copy import of VideoToolbox surfaces into the renderer is measured in the milestone 2 preview spike.
- **CI** links the LGPL shared builds from BtbN/FFmpeg-Builds on Linux and Windows and Homebrew's `ffmpeg@8` on macOS (`.github/actions/setup-ffmpeg`).
- **Encoding and muxing** (milestone 3): `FfmpegWriter` writes MP4, QuickTime and WebM with exact frame timestamps (frame `n` at `n / rate`, including 30000/1001), BT.709 limited-range tags, video from Y'CbCr frames or from the renderer's RGBA (converted with BT.709 coefficients), and AAC or Opus audio. Encoders are opened by name in preference order: H.264 and HEVC from VideoToolbox, NVENC, Quick Sync, AMF or Media Foundation only (VideoToolbox with `allow_sw=0`); ProRes 422 from `prores_ks`; AV1 from SVT-AV1, libaom or rav1e; VP9 from libvpx. `libx264` and `libx265` are never used, even when the linked FFmpeg contains them.
- **Capabilities:** `video_encoder(codec)` and `audio_encoder(codec)` open each candidate at a small size the first time a codec is asked for and remember the answer, so a compiled-in but unusable encoder (a hardware encoder in a virtual machine) is not offered, and an H.264 export never starts the AV1 encoders. `encoder_capabilities()` lists them all. A request for a codec without an encoder fails with `fosscut.media.encoder_unavailable` listing the codecs that work; impossible combinations fail with `fosscut.media.unsupported`.
- **Verification:** `probe_output` reports container, brand, codecs, profile, size, exact rate, frame count (counted when the container does not record it), stream durations, pixel format, color tags, audio format and file size.
- **Audio decoding:** `FfmpegAudioDecoder::read(start, duration)` returns exactly `duration` of interleaved `f32` at the requested rate and channel count, resampled with swresample and placed to the sample; time outside the stream is silence. A mono source plays at full level on both channels of a stereo output; swresample's default upmix would lower it by 3 dB.
- **Export** (milestone 3): `fosscut-engine` offers the presets `mp4-h264`, `mp4-hevc`, `mp4-av1`, `mov-prores`, `webm-vp9` and `m4a-aac`, which take size and rate from the timeline. Settings are checked before anything renders: a preset whose codec this machine cannot encode fails with `fosscut.export.codec_unavailable` and lists the codecs that work; it is never replaced by another codec. `fosscut export` without `--preset` picks by extension (`.mov` ProRes, `.webm` VP9, `.m4a` AAC) and otherwise the most compatible video preset available (H.264, then HEVC, then AV1), naming the codec in its report. After writing, the file is probed and compared with the request: container and brand, video codec, size, rate, frame count, duration, and audio codec, rate and channels. Any difference fails with `fosscut.export.verification_failed`, carrying the probe report.
- **More formats** (CC-163 to CC-172):
  - **ProRes:** the Proxy, LT, Standard, HQ and 4444 profiles of `prores_ks`, with 4444 carrying alpha from the RGBA input.
  - **GIF:** palettes from a deterministic median-cut quantizer with Bayer dithering, looping forever.
  - **Audio only:** 16 and 24-bit PCM in WAV, FLAC, and MP3 through `libmp3lame` (LGPL, optional: without it the `mp3` preset is unavailable and exports refuse it, never substituting).
  - **Presets:** `mov-prores-proxy`, `mov-prores-lt`, `mov-prores-hq`, `mov-prores-4444`, `gif`, `wav`, `wav-24`, `flac` and `mp3` join the earlier ones.
  - **Settings:** exports take a resolution (the shorter side, 480p to 8K), a frame rate, video and audio bit rates and a range of the timeline. They can also write a PNG cover image and SRT or WebVTT subtitles beside the file, and report where each used asset comes from (ADR-0028).
  - **Bit rates:** a chosen video bit rate replaces the AV1 and VP9 encoders' constant-quality settings. Windows' Media Foundation H.264 encoder has a quality floor: on hard content it overshoots low targets (about 445 kb/s for 100 kb/s at 320x180 noise), in its default constant bit rate mode.
- **Recording** (CC-027): `fosscut_media::capture` lists microphones, cameras and screens through FFmpeg's input devices (`avfoundation` on macOS; `dshow` and `gdigrab` on Windows; `v4l2`, `pulse` or `alsa`, and `x11grab` on Linux) and records one or two of them through the same encoders, settings checks and probing as exports. Pictures are scaled to fit the requested size and timed to its constant rate. The file is written under a temporary name and moved into place only when complete, so a failed recording leaves nothing behind. FFmpeg's `lavfi` input works like a device, which is how recording is tested without hardware.
- **Encoder delay and seeking:** each audio read decodes from at least 100 ms before its start (longer when the container asks, as Matroska and Ogg do for Opus), and reads near the start begin with the stream's first packet. The first packet after a seek states how many samples to drop: the encoder delay at the start of the stream, none elsewhere. Lossy sources (AAC in MP4 and QuickTime, MP3, Opus) then decode on the same samples as a decode from the beginning. Before this, AAC sources played about 10 ms late and MP3 lost 630 samples after a seek. WebM keeps a 24-sample offset after mid-stream seeks, because it stores milliseconds.
- **SVT-AV1 banner:** SVT-AV1 prints its configuration to stderr when it opens; set `SVT_LOG=1` to keep only its errors.
- **Rounded timestamps:** containers with coarse time bases (WebM stores milliseconds) round frame timestamps, so frame lookup allows half a tick of slack when the frame duration is not a whole number of ticks.

## Consequences

- Licensing stays compatible with a permissive project license ([ADR-0014](0014-licensing.md)).
- Machines without hardware H.264/HEVC encoders get a clear message and AV1 or bring-your-own-FFmpeg options instead of a broken export.
- Packaging FFmpeg shared libraries for three platforms is release work that CI must automate.

## Alternatives considered

- **GStreamer:** strong pipeline model, heavier packaging, fewer Rust-native media pieces than FFmpeg for probing and formats.
- **Pure Rust media stack only:** promising crates exist for some containers and codecs, but coverage of real-world camera and phone files is not there yet.
- **Bundling a GPL FFmpeg with x264:** best software encoding quality, but forces the whole distribution under GPL.

## Validation

Export matrix test in CI: H.264, HEVC, ProRes 422, HEVC with alpha (macOS), AV1, GIF and audio-only, each verified by probing. The round 1 regression (requested H.264, got HEVC) is a permanent test.
