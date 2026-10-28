# Milestone 2 spike results: preview path and compositor

Measured on 2026-10-28 on one machine. The validations in [ADR-0008](../adr/0008-render-and-audio-engine.md) and [ADR-0010](../adr/0010-desktop-shell-ui-and-web-path.md) also ask for a mid-range Windows GPU; that measurement is still open.

## Outcome

- **Preview path: A, native surface.** wgpu renders into the window behind a transparent webview. It meets the scrubbing targets: 57.5 new frames per second while scrubbing forward, with 12 ms median and 20.5 ms p95 from scrub input to presented frame. Frame transport (B) works on macOS, but it reaches only 29 frames per second while scrubbing, because every frame needs a render, readback, transfer and draw before the next one can start. On Windows, published WebView2 transfer rates of 125 to 156 MB/s would put the transfer of one 1080p RGBA frame alone above 50 ms.
- **Compositor (ADR-0008): the target is met on this machine.** A 4K H.264 source with transform, feathered mask and 33³ LUT renders to 1080p in 1.9 ms median per frame. Playback presents at the display's 120 Hz. Preview/export parity is covered by `crates/fosscut-render/tests/parity.rs`: 47.6 to 60.9 dB at three timestamps.
- **Seeking is decode-bound in both paths.** Random seeks and backward scrubbing take about 220 ms, far above the 50 ms target. The cause is decoding from the previous keyframe of a long-GOP 4K source, not the preview path. The fix belongs to media handling: proxies, a decoded-frame cache, or software decoding for random access.

## Setup

| | |
|---|---|
| Machine | Apple M3 Pro (11-core CPU), 120 Hz ProMotion display, scale factor 2 |
| GPU backend | Metal, through wgpu 30.0.1 |
| Shell | Tauri 2.12.5, system WKWebView |
| Decoder | FFmpeg 8.1.1 through `fosscut-media`, VideoToolbox with software fallback |
| Source | `cargo xtask fixtures` `test-pattern-4k.mp4`: 3840x2160, 30 fps, H.264 (libx264 veryfast, CRF 20), keyframe every 30 frames, 10 s |
| Scene | One clip, scale 0.92, rotation 2°, feathered ellipse mask, 33³ warm LUT, rendered by `fosscut-render` at 1920x1080 |
| UI | 960x540 CSS pixel preview (1920x1080 physical), scrub bar, 2D-canvas timeline with 2,000 clips on 8 tracks |

The spike is `spikes/preview/`, a throwaway Tauri app outside the workspace. With `FOSSCUT_SPIKE_BENCH=1` it runs a scripted benchmark and writes JSON to `spikes/preview/results/`; `python3 spikes/preview/summarize.py` computes the medians below. Mode A ran four times (a fourth run checked an outlier) and mode B three times; tables show medians. Latencies are measured in the webview from the input to one display refresh after the frame is presented (A) or drawn (B).

## Preview path results (medians)

| Metric | A: native surface | B: frame transport | Target |
|---|---|---|---|
| Playback, frames presented per second | 116.1 | 89.8 | 60 |
| Playback, render per frame (ms, p50) | 1.9 | 5.5 (with readback) | |
| Forward scrub, new frames per second | **57.5** | 29.0 | 60 |
| Forward scrub, latency p50 (ms) | **12** | 30 | under 50 |
| Forward scrub, latency p95 (ms) | **20.5** | 41 | under 50 |
| Backward scrub, new frames per second | 5.5 | 5.0 | 60 |
| Backward scrub, latency p50 (ms) | 198 | 260 | under 50 |
| Random seek, latency p50 (ms) | 219.5 | 224 | under 50 |
| Random seek, latency p95 (ms) | 384.5 | 390 | under 50 |
| Timeline (2,000 clips) scroll, frames per second | 60.3 | 60.3 | 60 |
| Timeline scroll, frame time p95 (ms) | 17.5 | 18 | |
| Timeline scroll during playback, frames per second | 60.3 | 60.3 | 60 |

Notes:

- The webview's `requestAnimationFrame` runs at 60 Hz on this 120 Hz display, so the UI sends at most 60 scrub targets per second. Mode A's 57.5 new frames per second is about 96% of that. Mode A presents playback at 120 Hz because the surface follows the display, not the webview.
- In mode B, transferring an 8.3 MB RGBA frame through the `frame://` URI scheme took 2.2 ms median (733 MB/s), and drawing it with `putImageData` took 1 ms. B's limit is the serial round trip, not bandwidth, on macOS.
- One of the four mode A runs scrubbed at 0.3 frames per second while its other measurements were normal; a fourth run to check it was normal again (57.8 frames per second, 11 ms median). See [Open questions](#open-questions).
- The 2,000-clip timeline draws only visible clips (about 60 per frame) and holds 60 fps in both modes, including during playback.

## Decode budget

Measured by `spikes/preview/src/bin/decode_bench.rs` on the same source:

| | VideoToolbox | Software (FFmpeg threads) |
|---|---|---|
| Sequential decode, frames per second | 112.5 | 470.2 |
| Sequential decode, ms per frame p50 (including copy to system memory) | 8.1 | 1.5 |
| Random seek, ms p50 / p95 | 243 / 327 | 58 / 85 |
| One frame backward, ms p50 / p95 | 211 / 343 | 58 / 84 |

The test pattern is far easier to decode than camera footage, so these numbers overstate real throughput. VideoToolbox decodes one frame at a time, and every seek restarts it at a keyframe; software decoding parallelizes across cores. Each VideoToolbox frame is copied from the GPU-backed surface to system memory and uploaded again, which zero-copy import would avoid.

## Decisions and follow-ups

1. **ADR-0010:** the preview path is A (native surface), recorded in the ADR. The UI talks to an abstract preview surface as decided, so the web build can use a WebGPU canvas.
2. **ADR-0008:** the validation passes on Apple silicon; implementation notes and these results are linked from the ADR.
3. **Seeking needs media work before the desktop app** (milestone 5): proxies with short GOPs (ADR-0009, item 7), a decoded-frame cache around the playhead, and choosing software decoding for random access when it is faster.
4. **Mode A needs platform care:** the preview hole must follow layout and scale changes; input over the preview arrives in the webview and is forwarded; presenting must not block when the window is hidden. Linux WebKitGTK has known flicker with this layering (tauri-apps/tauri issue 9220).
5. **Zero-copy VideoToolbox import** is not needed for 4K30 single-layer preview (decode 8 ms plus render 2 ms per frame), but becomes worthwhile for multi-layer or 60 fps 4K timelines.

## Open questions

- **Windows:** both validations ask for a mid-range Windows GPU with WebView2. Run `spikes/preview` there with `FOSSCUT_SPIKE_BENCH=1` for mode A (and mode B from the spike pull request's first commit if it is still useful), and add the numbers here.
- **Occasional stall while scrubbing in mode A:** one of four runs presented almost no frames while scrubbing, and it did not reproduce. Hidden or occluded windows make Metal's drawable acquisition wait, which matches the symptom but is not confirmed; the desktop app should skip presenting while the window is not visible and must be tested for it.
- **Real footage:** repeat the decode budget with phone and camera files (high bitrate, HEVC, 10-bit HDR), which the HDR pipeline will need anyway.
