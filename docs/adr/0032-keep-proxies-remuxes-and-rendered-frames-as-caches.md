# ADR-0032: Keep proxies, remuxes and rendered frames as caches

- **Status:** Proposed
- **Date:** 2026-10-30
- **Deciders:** maintainers
- **Related:** [ADR-0008](0008-render-and-audio-engine.md), [ADR-0009](0009-media-io-and-codecs.md), [ADR-0019](0019-local-analysis-pipeline-and-cache.md), [ADR-0031](0031-keep-user-preferences-outside-projects.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (proxy media, render cache, automatic remux, performance)

## Context

Editors make large or awkward media fast to edit in three ways. Proxies are smaller copies that the preview plays. Remuxed copies put the streams of a hard-to-seek file into a container that seeks well. A render cache keeps frames that were already rendered. The research shows a proxy toggle with a location and sizes, a render cache with a location, a 10 GB clearing threshold and automatic rendering, and automatic remuxing to MOV with its own cache.

FossCut already caches analysis results by content outside the project (ADR-0019). Preview and export share one frame evaluation (ADR-0008), and exports are verified against what was asked for (ADR-0009).

## Decision

1. **Caches by content, outside the project.** Each cache lives in its own folder of FossCut's cache folder (`proxies/`, `remux/`, `frames/`). Entries are keyed by the asset's content identity, or for frames by the rendered frame's content, plus a format version. They never change a project, and deleting them loses nothing.
2. **Proxies.**
   - **What a proxy is:** a ProRes Proxy copy of a video asset at half or a quarter of its size, without sound. ProRes Proxy is chosen because FFmpeg's own `prores_ks` encodes it everywhere and every frame decodes on its own, which suits scrubbing.
   - **When they are made:** by background jobs (`fosscut media proxy`, and the desktop app). With `performance.proxies` set to `auto`, a job is queued for assets longer than 1920 pixels on either side; with `on`, for every video asset.
   - **Who uses them:** only the desktop preview, and only when the preference allows. Sound, exports, frame grabs and contact sheets always read the original. Placement works in fractions of the picture, so a proxy shows the same framing.
3. **Remuxed copies.**
   - **Which files:** files in containers that seek slowly or inexactly (MPEG transport streams such as `.ts`, `.mts` and `.m2ts`, MPEG program streams, AVI and FLV), when `performance.auto_remux` is on.
   - **How:** a job copies their streams without re-encoding into QuickTime in `remux/`, once per content identity.
   - **Who uses them:** the preview decodes from the copy. Exports keep reading the original, so the remux never affects what is delivered.
4. **Rendered frames.**
   - **What is kept:** frames rendered for the desktop preview, keyed by a stable hash of the frame's render plan and the output size. Each layer's source, time, placement, effects with their parameters, masks and transitions go into the key, so an edit that changes a frame changes its key.
   - **Who uses them:** showing a cached frame skips rendering, and automatic rendering fills the cache around the playhead while the app is idle.
   - **Eviction:** least recently used frames go first once `performance.render_cache_gb` is exceeded.
5. **Policy and control.**
   - **Policy:** `cache.max_age_days` and `cache.limit_gb` (ADR-0031) govern every cache, analysis included. Entries unused for longer, or the oldest beyond the limit, are removed when a host starts and after jobs finish.
   - **Control:** `fosscut cache info` reports each cache's size, `fosscut cache clear [proxies|remux|frames|analysis]` empties them, and the desktop Settings window shows and clears them.
6. **Hardware acceleration.** `performance.hardware_decoding` becomes the decoders' `HardwareDecoding` option. `performance.hardware_encoding` set to false leaves the hardware encoders out of the candidates. A codec with no software encoder then becomes unavailable and reports so; it is never replaced.

## Consequences

- Large and awkward media edit smoothly, while exports, frame grabs and agents' checks keep seeing the originals exactly.
- The cache folder can grow; the policy keeps it bounded, and clearing is always safe.
- Render plans need a stable hash that covers everything that affects pixels. A missed input would show a stale frame in the preview only, never in an export. The render cache's version is bumped whenever rendering changes.

## Alternatives considered

- **Proxies for exports too:** faster exports, at the price of quality and of exports that no longer match what was asked for.
- **Remuxing in place, or into the bundle:** this would change the user's media or grow the project with copies that are only a speed aid.
- **Caching rendered frames by timeline revision:** any edit would drop every frame, even those it does not touch.

## Validation

- **Engine tests:**
  - Proxies are made once per content and used by the preview only.
  - Remuxed copies decode the same frames as the original.
  - Frame cache keys change with any change to a frame, and not otherwise.
  - Eviction keeps the limit, and the cache policy removes old entries.
- **Before Accepted:** a person edits 4K footage with proxies, and a transport-stream file with remuxing, on a laptop.
