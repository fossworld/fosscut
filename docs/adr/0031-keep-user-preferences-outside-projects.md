# ADR-0031: Keep user preferences outside projects

- **Status:** Proposed
- **Date:** 2026-10-30
- **Deciders:** maintainers
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0004](0004-open-project-format.md) (no machine-specific data in projects), [ADR-0010](0010-desktop-shell-ui-and-web-path.md), [ADR-0019](0019-local-analysis-pipeline-and-cache.md), [ADR-0032](0032-keep-proxies-remuxes-and-rendered-frames-as-caches.md), private research: `docs/capcut-audit/FEATURE-MATRIX.md` (settings: locations, cache, editing and timeline defaults, performance, language, updates, notifications, shortcuts)

## Context

Editors keep settings that belong to the person and the machine rather than to a project. The research shows several groups of them:

- **Places:** where projects, recordings and downloads go.
- **Cache policy:** how long caches are kept, and when they are cleared.
- **Defaults:** editing defaults such as frame steps, image length and loudness target, and timeline defaults such as audio while scrubbing, linked editing, the frame rate of new projects and the time display.
- **Performance:** hardware encoding and decoding, proxies, a render cache, automatic remuxing and optimization.
- **The application:** an export sound, the language, the update policy, notifications, and custom keyboard shortcuts.

FossCut has none of these yet. Projects must not hold machine-specific data (ADR-0004). Commands must stay deterministic, so an agent and a person get the same result from the same request (ADR-0002).

## Decision

1. **One preferences file per user**, `preferences.json`, in FossCut's configuration folder:
   - `FOSSCUT_CONFIG_DIR` when set;
   - otherwise `~/Library/Application Support/FossCut` on macOS, `%APPDATA%\FossCut` on Windows, and `$XDG_CONFIG_HOME/fosscut` (or `~/.config/fosscut`) elsewhere.

   The file holds `version: 1` and only the values that differ from the defaults. Unknown fields are ignored on reading, so an older FossCut reads a newer file. It is written atomically. `fosscut_engine::preferences` owns the types, defaults, validation and paths.
2. **Groups and defaults.**
   - **`locations`:** `projects` (the Movies folder or home), `recordings` (inside the project) and `exports` (beside the project).
   - **`cache`:** `max_age_days` 30, with 0 keeping caches forever, and `limit_gb` 20.
   - **`editing`:** `frame_step` 10, `nudge_pixels` 10, `image_duration` 5 s and `loudness_target` -14 LUFS.
   - **`timeline`:**
     - `scrub_audio` true and `linked` true;
     - `frame_rate` 30, the rate of new projects;
     - `time_display` timecode, frames or seconds.
   - **`export`:** `sound` true and `sound_name` `ding`, from the library.
   - **`performance`:**
     - `hardware_decoding` auto or off, and `hardware_encoding` true;
     - `proxies` off, auto or on, with `proxy_size` half or quarter;
     - `render_cache_gb` 10, `auto_render` true and `auto_remux` true.
   - **`language`:** a language code, `en` by default.
   - **`updates`:** off, notify or check, with notify as the default.
   - **`notifications`:** true.
   - **`shortcuts`:** a `set` name and `overrides` from action to keys.
3. **Front ends apply preferences as explicit arguments.** A command never reads preferences. The CLI and the desktop app fill a request from them: a new project's rate, an image clip's length, the normalize target. A request therefore means the same thing on every machine, and the operation log shows the values used.
4. **The engine applies performance and cache preferences.** These change speed and disk use, never results. They cover hardware decoding and encoding, proxies, remuxed copies, the render cache and cache cleanup (ADR-0032). Exports and verification frame grabs read original media. Turning hardware encoding off never substitutes a codec: a codec without a software encoder becomes unavailable, with the usual typed error (ADR-0009).
5. **Who may change them.**
   - **People:** they change preferences in the desktop app's Settings window and with `fosscut preferences get|set|reset|path`. Values are validated (`fosscut.preferences.invalid`).
   - **Agents:** they read them through the MCP `query` tool (`preferences.read`), so they can follow the person's defaults. Preferences belong to the person, so MCP offers no way to change them.
6. **Updates and notifications stay local.** Checking for updates asks the GitHub releases API for the latest version only when `updates` is not off, sends nothing about the user, and only reports. Installing stays the person's action. Notifications use the operating system's notifications when allowed.

## Consequences

- Defaults are visible, editable and the same in the CLI and the app, while projects stay free of machine data.
- Agents' requests do not change meaning with someone's settings, because front ends write the values into each request.
- A new preference needs a default, validation, a Settings control and documentation here.

## Alternatives considered

- **Preferences inside the project bundle:** this would mix machine-specific paths and performance choices into shared projects (ADR-0004 item 7).
- **Commands that read preferences:** the same request would mean different things on different machines and in tests.
- **Agents changing preferences:** they belong to the person; an agent that needs another value passes it in its request.

## Validation

- **Engine tests:** read, write, defaults, validation, ignoring unknown fields, and atomic writes.
- **CLI end to end:** `fosscut preferences` changes a value, and a new project takes the preferred frame rate.
- **Desktop tests:** the Settings window and the shortcut editor.
- **Before Accepted:** a person uses the Settings window on macOS and Windows.
