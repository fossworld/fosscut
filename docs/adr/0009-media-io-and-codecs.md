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
- **Decoding** (milestone 2): `fosscut-media` exposes `VideoDecoder` with exact flick timestamps, `frame_at` seeking that returns the frame displayed at a time, color tags with documented defaults for untagged streams, rotation from the display matrix, and VideoToolbox hardware decoding with software fallback. Frames are copied to system memory as NV12 or YUV 4:2:0; formats such as 4:2:2 and 10-bit are converted to 8-bit NV12 for now. Zero-copy import of VideoToolbox surfaces into the renderer is measured in the milestone 2 preview spike.
- **CI** links the LGPL shared builds from BtbN/FFmpeg-Builds on Linux and Windows and Homebrew's `ffmpeg@8` on macOS (`.github/actions/setup-ffmpeg`).

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
