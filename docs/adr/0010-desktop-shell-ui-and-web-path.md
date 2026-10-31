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
  - **Inspector:** shows the selected clip at the playhead: position, scale (uniform, or width and height), rotation, opacity, crop, volume, pan and fades, plus mirror, flip, blend mode, channels and whether the clip is active (V). A diamond beside each animatable value adds or removes its keyframe at the playhead. Values with keyframes are edited by a keyframe at the playhead, others by `clip.set`. Buttons step to the previous and next keyframe and set the easing of those at the playhead. The UI evaluates keyframes itself, mirroring `fosscut-model`. The timeline draws keyframes as diamonds, fades as ramps, and deactivated clips dimmed.
  - **Effects panel:** the selected clip's effects in order, each with its parameters built from the `capabilities` registry. Numbers are sliders with keyframe diamonds, pairs two sliders, LUTs a list of the project's LUT assets, and curves an editor (click to add a point, drag to move, double-click to remove). Effects can be bypassed, reordered and removed. Below are Add an effect, Apply a look, Auto color and Match color to; the transport bar adds an adjustment layer at the playhead. Auto color goes through the app's `color_suggest` command, which measures the clip and returns an `effect.add` the window applies like any edit.
  - **Masks:** the inspector lists the selected clip's masks, each with its shape, combine mode, invert, and fields with keyframe diamonds; text masks take their text. An SVG overlay on the preview, placed by mirroring the renderer's fit and transform, outlines the masks. While a mask tool is on it draws: the pen adds a point per click (Done or Enter to finish), and the brush paints strokes of a chosen size, or erases them. `timeline.read` reports each picture's displayed size for this placement.
  - **Motion:** the inspector's Motion section picks the transition into the next clip (offered only when one starts at the cut) with its duration and direction, and the clip's entrance, exit and loop animations with their durations. The timeline draws each transition as a bow tie across its cut, as wide as it lasts.
  - **Speed:** for video and audio clips, the inspector's Speed section sets a rate or a curve preset, reverse, pitch and smooth slow motion with `clip.speed` (rippling what follows), and Freeze frame holds the picture at the playhead with `clip.freeze`. Timeline labels show the speed, such as `1.5x` or `curve, reversed` (ADR-0023).
  - **Timeline editing:** Mod+C, Mod+X and Mod+V copy, cut and paste clips through `clips.copy` and `clips.paste` at the playhead, and Mod+Alt+C and Mod+Alt+V copy and paste attributes. Q and W delete the selected clip's part before or after the playhead and close the gap. Mod+Shift+B splits every track, M adds or removes a marker at the playhead (markers show as flags on the ruler), and `[` and `]` jump between markers. Shift-click selects several clips. The transport bar holds the linkage and magnet switches (P toggles the magnet), and the inspector aligns a clip with the canvas's edges or centre (ADR-0024).
  - **Editor tools:**
    - **Tools:** A and B switch between selecting and the blade, which splits a clip where it is clicked.
    - **Snapping:** N toggles snapping of dragged edges to other clips' edges, the playhead and markers, within eight pixels.
    - **Skimming:** S toggles skimming (the preview axis): hovering over the timeline shows that frame without moving the playhead, and a line marks it.
    - **Shuttle:** J, K and L shuttle backwards, stop and forwards, doubling the speed with each press up to 8x. The preview's render thread paces speeds other than 1x by the wall clock, without sound, and stops backwards playback on the first frame.
    - **Zoom:** Mod+= and Mod+- zoom the timeline, and Shift+Z fits the edit.
    - **Full screen:** Mod+Shift+F (or the header button) shows the preview full screen above the transport bar, through the app's `window_fullscreen` command; Escape leaves.
    - **Rulers and guides:** Mod+Shift+R shows rulers in canvas pixels along the preview. Dragging from a ruler adds a guide, dragging a guide moves it, and dropping it on its ruler or double-clicking removes it. Guides belong to the window, not the project.
  - **Media bin and timelines:** the side panel's Media section searches, filters by kind and folder and sorts the project's assets. Each asset can be added at the playhead, renamed (double-click), shown in the file manager (`media_reveal`) and removed; a missing file is marked and relinked through a file dialog (`media_relink`). Chosen media can be moved to a folder or start a new timeline. Tabs over the timeline switch between the project's timelines (the one shown is the primary one), rename them, add and remove them.
  - **Project browser and trash:** with no project open, the window shows the recent projects as cards with a frame of each (rendered once per revision into the cache folder), their length and size, and a search. Moving a project to the trash moves its whole bundle into the app's data folder (never the open project, nor one another program holds open); the trash restores it to where it was, or beside it when that place is taken, and deletes it for good only after asking.
  - **Compound clips:** Mod+Alt+G turns the selection into a compound clip; the inspector opens or decomposes it, and the media panel lists compound timelines (ADR-0025).
  - **Sound:** playback follows the sound (ADR-0008, implementation notes).
- **Redesign** (the P2 batch):
  - **Look:** design tokens in `ui/src/theme/tokens.css` (an orange accent scale, neutrals, a colour per clip kind, spacing, radii, type, shadows and motion that respects reduced motion), dark by default with a light theme or the system's, chosen per window. Text pairs meet WCAG AA. Icons are Lucide (ISC). The page stays transparent where the native preview draws, so nothing between the root and the preview may create a stacking context.
  - **Layout:** a top bar (app menu, save state, agents, canvas shape, panel toggles, Export with progress), a resource browser of ten categories (media as thumbnails that run through frames on hover and drag onto the timeline; audio, text, stickers, effects, transitions, captions, looks, templates and voice), the player with zoom, safe zones, rulers and full screen, an inspector with tabs by clip kind, and a timeline with track headers, frame thumbnails, waveforms, transition badges and markers. Rarely used dialogs load when first opened, so no script chunk exceeds 500 KB.
  - **App commands:** `media_thumbnails` (cached frames as PNG), `media_waveform` (cached peaks), `media_import` (picked or dropped files as one step; a dropped bundle opens), and the measured edits of ADR-0034 (`clip_reframe`, `clip_camera_track`, `timeline_vertical`, `beats_markers`, `beats_cut`, `shorts_candidates`, `shorts_make`), which apply the returned calls like any edit. A missing analysis offers to run it, and to install models first when it needs them.
  - **P2 in the inspector:** slow motion by repeated frames, blending or optical flow; reframe and camera tracking in Tools; a Shorts tab that lists scored moments with their reasons and builds one; voice presets, beat markers and cutting to the beat for clips with sound; vertical versions from the timeline's menu.
  - **Not yet:** track mute, hide and lock show but cannot change, because no `track.set` command exists.
- **Releases:** debug builds load the interface from the Vite server (`devUrl`); release builds (`cargo tauri build`, in the release workflow) embed `ui/dist`. No crate feature may enable `tauri/custom-protocol`, because CI builds with `--all-features` and that feature needs the built interface. The release workflow bundles a dmg (signed ad hoc) with FFmpeg's libraries in `Contents/Frameworks` and `ffprobe` in `Contents/MacOS`, an NSIS installer with the DLLs and `ffprobe.exe` beside the executable, and a deb with both in `/usr/lib/fosscut`, all LGPL builds (ADR-0009).

## Validation

Preview spike, on macOS (Apple silicon) and Windows: 1080p preview at 60 fps while scrubbing, under 50 ms from seek to displayed frame, UI timeline with 2,000 clips scrolling at 60 fps. The option that meets the targets with less platform code wins; if neither does in a webview, revisit the UI framework.

**Results on macOS (Apple M3 Pro, 2026-10-28):** with the native surface, forward scrubbing showed 57.5 new frames per second at 12 ms median and 20.5 ms p95 latency, and the 2,000-clip timeline scrolled at 60 fps, also during playback. Random seeks and backward scrubbing took about 220 ms in both options; that is decoding from the previous keyframe of a long-GOP 4K source, which proxies and frame caching address (ADR-0008, ADR-0009). Windows has not been measured yet. Details: [milestone 2 spike results](../development/milestone-2-spike-results.md).
