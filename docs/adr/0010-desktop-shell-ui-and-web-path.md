# ADR-0010: Tauri desktop shell with a shared TypeScript UI, web build later

- **Status:** Proposed (the macOS preview spike is done; the Windows measurement is open)
- **Date:** 2026-10-28
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0003](0003-rust-workspace-and-crate-layering.md), [ADR-0008](0008-render-and-audio-engine.md), private research: `docs/capcut-rea/AGENT-SUBSYSTEM.md`, `docs/capcut-rea/RESOURCES.md`

## Context

The owner wants a desktop app first and possibly a web version later. Established editors build their core UI with native toolkits and increasingly use web technology for newer panels, with the native engine delivering preview frames to the web view without copying them through the UI layer. AI coding agents are most productive in TypeScript and React. A video editor's hardest UI parts are the timeline (thousands of elements, smooth zoom and scroll) and the preview (latency, frame rate).

## Decision

1. **Desktop shell: Tauri 2.** The Rust side hosts the engine session ([ADR-0002](0002-agent-first-command-api.md)); the UI runs in the system webview.
2. **UI: TypeScript, React and Vite in `ui/`**, shared between desktop and the future web build. The timeline is drawn on a canvas (2D canvas first, WebGPU if needed), not as DOM elements per clip. State comes from engine queries and events; the UI never mutates project state except through commands.
3. **Preview path: A, native surface** (chosen by the milestone 2 spike, [results](../development/milestone-2-spike-results.md)). The engine renders with `wgpu` into the window behind a transparent webview, and the UI leaves the preview area transparent. The alternative, *B, frame transport* (offscreen render, frames sent to a webview canvas through a custom protocol), reached only 29 frames per second while scrubbing on macOS and is expected to be slower on Windows. The UI talks to an abstract `PreviewSurface` so the web build can substitute a WebGPU canvas.
4. **One engine client.** UI code talks to the engine only through a single engine-client module in `ui/` (commands, queries, events, preview frames). On desktop it uses Tauri; on the web it will use an in-page WebAssembly engine or a WebSocket to a server. Components never call Tauri APIs directly, so the same UI runs in both.
5. **Agent-aware UI.** The app shows agent activity: connected clients, the operation log grouped by agent transaction, a diff view of `timeline.plan` results with accept and reject, and one-click undo of an agent's transaction.
6. **Web path (later, separate ADR when started):** the same `ui/`, the engine's model, core and render crates compiled to WebAssembly with `wgpu` on WebGPU and WebCodecs for media ([ADR-0009](0009-media-io-and-codecs.md)), projects stored locally (File System Access, OPFS) or on a server. Agents reach a web session through the MCP server's HTTP transport.

## Consequences

- One UI codebase for desktop and web; the large TypeScript ecosystem and strong agent fluency in React speed up development.
- System webviews differ (WebKit on macOS and Linux, WebView2 on Windows); the UI needs cross-webview testing, and some advanced web features may be unavailable on Linux WebKitGTK.
- The native-surface option depends on platform-specific window composition code.

## Alternatives considered

- **Qt with Rust bindings:** proven for editors, but heavy, LGPL obligations, weaker agent fluency, and no web reuse.
- **Pure Rust UI (egui, Iced, Slint):** single language and good GPU integration; egui even runs on the web. Professional editor UX (complex panels, text layout, accessibility, theming) is harder, and the ecosystem for timeline-grade widgets is thinner. Kept as fallback if the preview spike fails for webviews.
- **Electron:** consistent Chromium everywhere and good zero-copy precedent, but large bundles and memory, and a Node runtime the engine does not need.

## Implementation notes

- **First app** (milestone 5):
  - **Crates:** `apps/desktop` is a workspace member, a thin Tauri crate. The preview lives in `fosscut_engine::preview` (a `Previewer` that keeps media open between frames, rendering byte for byte what frame grabs render) and is tested without a window.
  - **Agents and the app:** the app hosts the project it opens (ADR-0022), so agents connected through `fosscut mcp` edit the session the window shows. Each commit reaches the preview and the UI as it lands.
- **Webview access:** `build.rs` lists the app's commands, and the window may call only those (`capabilities/default.json`). The open and save dialogs run in Rust, so the webview has no file-system or dialog permission.
- **Preview, path A as in the spike:**
  - **Rendering:** the engine renders into an offscreen texture sized to fit the hole the page reports, then draws it into the window surface, letterboxed.
  - **Pacing:** a render thread draws on demand and paces playback by the wall clock. Audio, and the audio clock, come next.
  - **Minimized window:** nothing is presented.
- **One engine client:**
  - **The boundary:** `ui/src/engine` is the only code that imports Tauri, and ESLint enforces it. The UI's state and edits sit in `Editor`.
    - **Revisions:** edits go against the revision on screen.
    - **Conflicts:** a conflict reloads and says so.
    - **Destructive guard:** it asks before retrying with consent.
  - **Tests:** Vitest covers `Editor` against a stand-in engine, along with timeline geometry, time math (including drop-frame timecode), keyboard shortcuts and history grouping.
- **Agent-aware UI** (item 5, first part):
  - **Who is here:** the connected clients, polled every two seconds.
  - **History:** grouped into runs by who made each change.
  - **Undo:** of the newest change, whoever made it.
  - **Later:** the diff view of `timeline.plan` results comes later.
- **Editing in the app** (milestone 5, second step):
  - **Moving and trimming:** dragging a clip moves it, onto another track of the same kind under the pointer, and dragging an edge trims it with the new `clip.trim` command. Both move in whole frames and send one command when released.
  - **Inspector:** shows the selected clip's position, scale, rotation, opacity and volume in percent and degrees, and sends `clip.set`.
  - **Sound:** playback follows the sound (ADR-0008, implementation notes).
- **Releases** are not set up yet. Debug builds load the interface from the Vite server (`devUrl`). No crate feature may enable `tauri/custom-protocol`, because CI builds with `--all-features` and that feature needs the built interface.

## Validation

Preview spike, on macOS (Apple silicon) and Windows: 1080p preview at 60 fps while scrubbing, under 50 ms from seek to displayed frame, UI timeline with 2,000 clips scrolling at 60 fps. The option that meets the targets with less platform code wins; if neither does in a webview, revisit the UI framework.

**Results on macOS (Apple M3 Pro, 2026-10-28):** with the native surface, forward scrubbing showed 57.5 new frames per second at 12 ms median and 20.5 ms p95 latency, and the 2,000-clip timeline scrolled at 60 fps, also during playback. Random seeks and backward scrubbing took about 220 ms in both options; that is decoding from the previous keyframe of a long-GOP 4K source, which proxies and frame caching address (ADR-0008, ADR-0009). Windows has not been measured yet. Details: [milestone 2 spike results](../development/milestone-2-spike-results.md).
