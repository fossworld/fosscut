# ADR-0042: Ship Three.js scene templates as web compositions

- **Status:** Proposed
- **Date:** 2026-11-02
- **Deciders:** maintainers
- **Related:** [ADR-0014](0014-licensing.md), [ADR-0015](0015-clean-room-policy.md), [ADR-0016](0016-testing-strategy.md), [ADR-0028](0028-a-built-in-library-made-by-fosscut.md), [ADR-0032](0032-keep-proxies-remuxes-and-rendered-frames-as-caches.md), [ADR-0037](0037-place-layers-in-3d-with-a-camera-and-depth-of-field.md), [ADR-0039](0039-render-web-compositions-in-a-headless-browser.md), [ADR-0040](0040-render-3d-objects-through-the-camera.md)

## Context

Ads of the class agents are asked for show a realistic phone with a live app rising and turning, a dark tilted city map with a glowing route drawing on, a parcel dropping onto a floor, an extruded logo catching a light sweep, and products on podiums, all with bloom, depth of field and motion blur. FossCut's own 3D layers and objects (ADR-0037, ADR-0040) place and light parts, but a designed scene with its camera moves is code, and agents write Three.js well. Web compositions (ADR-0039) already render any page frame by frame on FossCut's clock. Three.js is MIT (ADR-0014). Measured on an M3 Pro with Chrome 156, debug build, 1080x1920 and eight sub-frames per frame: SwiftShader takes 1.9 to 5.5 s per frame, Metal 0.34 to 0.63 s.

## Decision

1. **Vendored Three.js.** Version 0.185.1 from npm (integrity recorded), the newest release at least two weeks old with minified builds: `three.module.min.js`, `three.core.min.js`, `LICENSE` and only the add-ons the templates use (`CSS3DRenderer`, `SVGLoader`, `GLTFLoader` with its two utilities, `RoundedBoxGeometry`), about 1 MB, in `fosscut-core/presets/library/vendor/three/` with a README of sources and hashes. `fosscut-core` embeds them and writes them into each composition's folder; pages import them through an inline import map with relative paths.
2. **FossCut's scene kit** (`presets/library/scene-kit/stage.js`, copied as `kit/stage.js`) renders a scene for one time `t` and keeps nothing between frames:
   - **Motion blur:** `motion_blur.samples` sub-frames spread over `shutter` of a frame, averaged in linear light; the one nearest `t` comes last, and the scene ends posed at `t`.
   - **Depth of field:** a gather blur from the depth buffer with a thin-lens circle of confusion around a focus point the template moves with its subject; sharper samples in front do not spread.
   - **Bloom:** bright parts over a threshold, blurred through a chain of half-size levels.
   - **Output:** tone mapping (ACES, AgX or neutral), sRGB encoding and premultiplied alpha, so transparent backgrounds stay transparent and glow over them keeps its light.
   - Lighting comes from studios built in code and turned into environment maps (no downloaded HDRIs); randomness from seeded generators.
3. **Parameters.** A library composition declares typed `params` in `compositions.json`: `color`, `text`, `number` (range, whole), `bool`, `choice`, `points`, `group` (fields) and `slot` with the media it `accepts` (`image`, `video`, `svg`, `model`, `composition`). A composition with `kit: "three"` also has the scene parameters of `presets/library/scene-params.json` (`transparent`, `background`, `exposure`, `tone_mapping`, `bloom`, `depth_of_field`, `motion_blur`, and `width`, `height`, `duration`, `fps`, which rewrite its meta tag), with its own `defaults`. `fosscut-core` checks given values and fills defaults; anything else is `fosscut.library.invalid_params`. Pages carry the same defaults between `// fosscut:defaults` markers, checked by a test, so they render with none.
4. **Adding.** `library` add (MCP `library` tool, `fosscut library add`, `fosscut_engine::library::media_args_with`) takes `params` (an object) and `media` (asset ids for the slots `params` leaves out, in order). The engine writes the page, the kit, `params.json` with every value, and the slots' files under `media/`: images, videos, SVGs and `.glb` files by copy, `.gltf` models and web composition assets with their folders, and a library composition named in a composition slot written with its defaults. A slot's value in `params.json` is `null` or `{src, kind, name}` plus, for compositions, `width`, `height`, `fps` and `duration`.
5. **Identity.** The folder is `library/<name>/` while that is free or holds the same files, else `library/<name>-<first 8 hex of the content hash>/`; it is written beside and moved into place whole. The asset's identity is the folder's (ADR-0039), so other parameters make another asset and the same ones find the existing asset; a folder an asset uses is never rewritten.
6. **GPU mode.** A page with `<meta name="fosscut:renderer" content="gpu">` is drawn on macOS by a browser of its own started with `--use-angle=metal --ignore-gpu-blocklist` instead of SwiftShader (`fosscut_engine::web::WebRenderer`). Elsewhere it draws in software. `FOSSCUT_WEB_RENDERER=software|gpu` overrides every page. The frame cache key (ADR-0032) includes `web-software` or `web-gpu`. `capabilities` reports the mode under `web.renderer`. Software stays the default, and tests and goldens draw with SwiftShader.
7. **Templates**, FossCut's own and generic: `route-map-3d`, `parcel-drop-3d`, `phone-3d`, `logo-extrude-3d` and `product-podium-3d`, each a folder with `index.html` and `scene.js`, driven only by `window.fosscut.seek(t)`. The phone shows HTML through `CSS3DRenderer`. A composition slot is an iframe in a shadow root, moved to its own time with `__fosscut.show`. Pictures and videos sit in the same layer, kept out of reach of FossCut's clock so they follow their own.

## Consequences

- Agents get ad-grade 3D scenes by name and parameters, editable as ordinary clips, and can write their own scenes with the same kit.
- Each added variant copies about 1 MB of Three.js into the bundle.
- SwiftShader is slow for 3D: the GPU mode is what makes previews and exports practical, and its frames depend on the machine's GPU.
- The phone's HTML screen is composited by the browser over the WebGL canvas: it is not blurred by depth of field or motion blur, and does not bloom.
- A Three.js or Chrome update can shift pixels: the scene goldens are reviewed again when either changes.

## Alternatives considered

- **Three.js's EffectComposer passes** (`UnrealBloomPass`, `BokehPass`): `BokehPass` writes opaque alpha and the bloom's additive alpha is not premultiplied, so transparency breaks; the kit's passes are short and keep it.
- **Blender:** later, as an optional renderer of the user's own files; too heavy to ship as the default.
- **Native scene graphs in `fosscut-render`:** a renderer, loaders and authoring format of our own for what Three.js already does.
- **HTML drawn into a WebGL texture:** stable Chrome has no such API without flags; CSS 3D keeps the screen live and sharp.
- **Loading Three.js from a CDN or an external import map:** compositions render offline (ADR-0039).

## Validation

- **Core:** parameter types, ranges, groups, points, slots and canvas values; the kit's files; the pages' defaults agree with the declared ones.
- **Engine:** folders, `params.json`, slot copies and identities by parameters; wrong media and unknown assets refused; GPU mode switches and the cache key.
- **Browser** (`tests/scenes.rs`, SwiftShader):
  - Every template gives byte-identical frames alone and after others, and an opaque background, or a clear one when `transparent`.
  - Bloom, depth of field and motion blur change measured luminance and detail.
  - The phone shows the shop-app screen.
  - The GPU draws within 26 dB of SwiftShader.
  - One golden per template at 270x480 (32 dB).
- **CLI:** `library add` with `--params` and an image slot, then a frame grab.
- **Before Accepted:** a maintainer reviews the templates in a 1080x1920 ad, and checks the GPU mode and frame times on CI's macOS runners and on Linux and Windows.
