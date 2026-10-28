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
