# ADR-0008: Processing graph with separate preview and export graphs on wgpu

- **Status:** Proposed
- **Date:** 2026-10-28
- **Related:** [ADR-0003](0003-rust-workspace-and-crate-layering.md), [ADR-0009](0009-media-io-and-codecs.md), [ADR-0011](0011-effects-and-animatable-properties.md), [ADR-0016](0016-testing-strategy.md), private research: `docs/capcut-rea/ARCHITECTURE.md`

## Context

An editor renders the same timeline for consumers with different needs: interactive preview (audio clock as master, frame dropping, reduced resolution, cached and proxy media, scrubbing), export (deterministic, every frame, full resolution, hardware encoding with fallback, parallel segments), plus frame grabs, audio, recording and analysis. Professional editors share processing steps between these paths and differ only in how they drive them. Output quality depends on correct GPU color handling: transfer conversion, tone mapping and scopes.

Round 1's roadmap states that preview and export must agree as a correctness contract. FossCut needs the desktop now and the browser later.

## Decision

1. **GPU API: `wgpu`** (Metal on macOS, Vulkan or DX12 on Windows and Linux, WebGPU in browsers). Shaders are written in WGSL; `naga` handles translation and validation. No OpenGL path.
2. **Evaluation model.** Rendering a frame at time *t* is a pure function of (timeline document, media frames, *t*, output settings). The compositor builds a per-frame render plan (layers in track order, each with source frame, transform, masks, effects, blend mode, opacity) and executes it on the GPU. No hidden state between frames except caches keyed by their inputs.
3. **Shared processing nodes, separate graph builders.**
   - *Preview graph:* audio clock is the master; video frames may be dropped; renders at preview resolution; uses proxies and cached pre-rendered segments when available; supports scrubbing and seek flushing.
   - *Export graph:* every frame in order at full resolution; never drops frames; may encode independent segments in parallel and concatenate; may remux untouched segments without re-encoding when settings allow.
   - *Frame-grab graph:* single frames or contact sheets for thumbnails and for agents ([ADR-0002](0002-agent-first-command-api.md)).
4. **Color pipeline.** Linear-light compositing in a working space (Rec.709 linear for SDR projects, Rec.2020 linear for HDR); explicit input transforms (YUV to RGB with correct matrix and range, transfer function decode) and output transforms (encode transfer, tone mapping when HDR goes to SDR). Scopes (waveform, parade, vectorscope) are GPU passes. The adjustment order of operations follows the reference documented in research, adapted and reimplemented.
5. **Audio engine (`fosscut-audio`).** Separate audio graph at 48 kHz float internally: per-clip gain, fades, pan, speed with pitch preservation, effects, per-track and master buses, EBU R128 loudness measurement and normalization, playback through `cpal`. Audio drives the preview clock.
6. **Caching.** Decoded frame cache, pre-rendered segment cache and proxy files live in the OS cache directory keyed by content hashes of their inputs, so edits invalidate exactly what they touch.
7. **Determinism.** Export is deterministic for a given machine, settings and codec. Golden-frame tests compare within documented tolerances across GPUs ([ADR-0016](0016-testing-strategy.md)).

## Implementation notes

Milestone 2 built the compositor in `fosscut-render` on wgpu 30:

- **Plans:** `plan_frame(project, timeline, time)` is pure and lists the visible layers bottom to top with source frame, display rotation, transform, opacity, mask and LUT; text clips become text layers (ADR-0011). Masks and LUTs live in the plan only; storing effects in projects is separate work under ADR-0011.
- **Passes:** per layer, Y'CbCr planes are uploaded and converted at source size to linear light in `rgba16float` with the stream's matrix and range, mipmapped in linear light, then composited over a 4x multisampled linear canvas with premultiplied alpha; the canvas is encoded with the output transfer function into the target.
- **Color:** sources are decoded with the inverse of their transfer function (BT.709 or sRGB), so untouched pixels round-trip exactly; LUTs apply to the BT.709-encoded signal. PQ and HLG sources fail with `fosscut.render.unsupported_color` until the HDR pipeline exists. Chroma is upsampled bilinearly with centre siting.
- **Graphs:** `ExportGraph` (every frame, full resolution, readback) and `PreviewGraph` (on demand, fitted to a preview size, into any texture view such as a window surface) share the plan and the renderer.
- **Tests:** color bars against hand-computed values, layering, masks, LUTs, rotation, aliasing-free downscaling, golden frames compared by PSNR (`cargo xtask goldens`), and preview/export parity above 40 dB at three timestamps. CI runs them on Mesa lavapipe (Linux), WARP (Windows) and Metal (macOS) with `FOSSCUT_REQUIRE_GPU=1`.
- **Audio mix** (milestone 3): `fosscut_audio::mix` renders any span of a timeline as interleaved 48 kHz stereo `f32`: audio clips, and the embedded audio of video clips, on unmuted tracks, each scaled by its clip's `volume`, summed and clamped to full scale. Positions are exact to the sample, since 48 kHz divides a second of flicks. Export mixes one frame's span at a time. Fades, pan, buses, loudness and playback come later.
- **Preview playback** (milestone 5): the desktop preview graph is `fosscut_engine::preview`.
  - **Frames:** `Previewer` renders frames on demand through the export's plan, renderer and media handling.
  - **Sound:** `Sound` mixes spans with the export's mixer.
  - **Audio clock:** the app mixes about 250 ms ahead into a queue that a `cpal` output stream drains in 32-bit float at the device's rate. The count of samples the device has played is the clock the picture follows. Silence played while the queue runs dry does not count, so the picture never runs ahead of the sound.
  - **No device:** without an audio output, or one that plays no float format, playback keeps time by the wall clock.
  - **Frame cache** (item 6, first part): the previewer keeps the most recently decoded frames, up to 256 MiB, dropping the least recently used first. A frame answers every time it covers, so scrubbing back over frames already shown skips the decoder's seek to the previous keyframe (about 220 ms on long-GOP sources).
  - **Not yet:** frame dropping under load, and proxies for media never shown yet.

## Consequences

- One shader language and one GPU API across desktop and web.
- Preview and export share the frame evaluation function, which makes parity testable.
- `wgpu` lags native APIs in some advanced features (for example zero-copy import of platform video frames); platform-specific interop modules may be needed for performance and live behind traits.
- Writing our own compositor and color pipeline is significant work; it is the core of the product's quality.

## Alternatives considered

- **GStreamer or MLT as the engine:** mature, but GPL or LGPL C code, different data model, hard to compile to the web, and weaker GPU compositing control.
- **Native Metal/Vulkan/DX12 backends:** maximum performance, triple the work; `wgpu` can be bypassed later for hot paths if measurements demand it.
- **OpenGL:** deprecated on macOS and absent from modern web targets.

## Validation

Spike: decode a 4K H.264 clip, apply transform, a mask and a LUT, and preview at 1080p and 60 fps on an M1-class Mac and a mid-range Windows GPU; export the same timeline and compare frames at three timestamps against the preview path within tolerance.

**Results on macOS (Apple M3 Pro, 2026-10-28):** the 4K source with transform, feathered mask and LUT rendered to 1080p in 1.9 ms median per frame, and playback presented at the display's 120 Hz; VideoToolbox decoded the source at 112 frames per second including the copy to system memory. Preview and export renders matched at three timestamps at 47.6 to 60.9 dB (`crates/fosscut-render/tests/parity.rs`). Random access is the weak point: seeking costs about 220 ms with this long-GOP source, so preview needs proxies and a decoded-frame cache (item 6). Windows has not been measured yet. Details: [milestone 2 spike results](../development/milestone-2-spike-results.md).
