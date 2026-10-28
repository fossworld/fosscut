# ADR-0010: Tauri desktop shell with a shared TypeScript UI, web build later

- **Status:** Proposed (requires the preview spike below before acceptance)
- **Date:** 2026-10-28
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0003](0003-rust-workspace-and-crate-layering.md), [ADR-0008](0008-render-and-audio-engine.md), private research: `docs/capcut-rea/AGENT-SUBSYSTEM.md`, `docs/capcut-rea/RESOURCES.md`

## Context

The owner wants a desktop app first and possibly a web version later. Established editors build their core UI with native toolkits and increasingly use web technology for newer panels, with the native engine delivering preview frames to the web view without copying them through the UI layer. AI coding agents are most productive in TypeScript and React. A video editor's hardest UI parts are the timeline (thousands of elements, smooth zoom and scroll) and the preview (latency, frame rate).

## Decision

1. **Desktop shell: Tauri 2.** The Rust side hosts the engine session ([ADR-0002](0002-agent-first-command-api.md)); the UI runs in the system webview.
2. **UI: TypeScript, React and Vite in `ui/`**, shared between desktop and the future web build. The timeline is drawn on a canvas (2D canvas first, WebGPU if needed), not as DOM elements per clip. State comes from engine queries and events; the UI never mutates project state except through commands.
3. **Preview path:** decided by the spike below between:
   - *A. Native surface:* the engine renders with `wgpu` into a native view composited with the webview (preferred for zero-copy and latency);
   - *B. Frame transport:* the engine renders offscreen and transfers frames to a canvas in the webview through shared memory or a custom protocol.
   Whichever wins, the UI talks to an abstract `PreviewSurface` so the web build can substitute a WebGPU canvas.
4. **Agent-aware UI.** The app shows agent activity: connected clients, the operation log grouped by agent transaction, a diff view of `timeline.plan` results with accept and reject, and one-click undo of an agent's transaction.
5. **Web path (later, separate ADR when started):** the same `ui/`, the engine's model, core and render crates compiled to WebAssembly with `wgpu` on WebGPU and WebCodecs for media ([ADR-0009](0009-media-io-and-codecs.md)), projects stored locally (File System Access, OPFS) or on a server. Agents reach a web session through the MCP server's HTTP transport.

## Consequences

- One UI codebase for desktop and web; the large TypeScript ecosystem and strong agent fluency in React speed up development.
- System webviews differ (WebKit on macOS and Linux, WebView2 on Windows); the UI needs cross-webview testing, and some advanced web features may be unavailable on Linux WebKitGTK.
- The native-surface option depends on platform-specific window composition code.

## Alternatives considered

- **Qt with Rust bindings:** proven for editors, but heavy, LGPL obligations, weaker agent fluency, and no web reuse.
- **Pure Rust UI (egui, Iced, Slint):** single language and good GPU integration; egui even runs on the web. Professional editor UX (complex panels, text layout, accessibility, theming) is harder, and the ecosystem for timeline-grade widgets is thinner. Kept as fallback if the preview spike fails for webviews.
- **Electron:** consistent Chromium everywhere and good zero-copy precedent, but large bundles and memory, and a Node runtime the engine does not need.

## Validation

Preview spike, on macOS (Apple silicon) and Windows: 1080p preview at 60 fps while scrubbing, under 50 ms from seek to displayed frame, UI timeline with 2,000 clips scrolling at 60 fps. The option that meets the targets with less platform code wins; if neither does in a webview, revisit the UI framework.
