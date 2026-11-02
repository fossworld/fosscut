# ADR-0043: Run the user's own agent from the desktop

- **Status:** Proposed
- **Date:** 2026-11-02
- **Deciders:** maintainers
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0010](0010-desktop-shell-ui-and-web-path.md), [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), [ADR-0013](0013-agent-facing-editing-skills.md), [ADR-0022](0022-live-sessions-over-a-local-socket.md), [ADR-0031](0031-keep-user-preferences-outside-projects.md), [ADR-0039](0039-render-web-compositions-in-a-headless-browser.md)

## Context

People want to drop a raw talking-head video into FossCut and get a reel edited by AI without leaving the app. ADR-0012 keeps every language model out of the core: the user's agent plans and FossCut executes. Until now the app could only wait for an agent the user started in a terminal and pointed at `fosscut mcp`.

Agent command lines run headless. Claude Code takes a prompt with `-p`, MCP servers from a file with `--mcp-config`, an allow-list of tools, a turn cap (`--max-turns`, accepted though not in `--help`) and a spending cap (`--max-budget-usd`), and prints one JSON event per line with `--output-format stream-json`: `system` (model, session, MCP server status), `assistant` (text and tool calls), `user` (tool results) and a final `result` (turns, duration, cost, summary). A run from a terminal with only FossCut's tools edited a project live.

The constraints: agents are third-party products with their own licences and accounts, so FossCut must not install or bundle them, and it must never handle API keys. Apps started from the dock see a short `PATH`. The desktop release does not ship the `fosscut` CLI. Running the user's own Remotion (ADR-0039, item 6) set the pattern: find the user's program, show the exact command, run it only after the user agreed, and end its whole process group on cancel.

## Decision

1. **Only the user's own agent.** `fosscut_engine::agent::find` takes the `agent.command` preference (an absolute path, ADR-0031), else `FOSSCUT_AGENT`, else `claude` on the path, then in `~/.local/bin`, `~/.claude/local`, `/opt/homebrew/bin` and `/usr/local/bin` (`%APPDATA%\npm` on Windows). A configured program that is missing is reported, never replaced by another. None found is `fosscut.agent.not_found`; the window then hands off: the brief to copy and the line `claude mcp add fosscut -- <the app> mcp`. FossCut never installs, updates or bundles an agent. Claude Code comes first; another agent is another command built from the same request.
2. **No keys.** FossCut never asks for, reads, stores or passes credentials; the agent runs with its own login. The MCP server's environment carries only FossCut's variables (not `FOSSCUT_AGENT`) and the program and library search paths.
3. **Confirmation first.** `agent::plan` without `confirmed` fails with `fosscut.agent.confirmation_required`, whose details carry `agent`, the exact `command` and `notes`: that the agent is the user's own program running with their account and may cost money, that it reaches only FossCut's tools for this project, and its caps. The window shows the brief in full above the command. The working folder is derived from the bundle's path, so the command agreed to is the command that runs.
4. **The command.** In the working folder `agent/<first 16 hex of BLAKE3 of the bundle path>/` of FossCut's cache: `claude -p <brief> --mcp-config <folder>/mcp.json --strict-mcp-config --allowedTools mcp__fosscut__*,Skill --disallowedTools Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch --max-turns N [--max-budget-usd B] --output-format stream-json --verbose`. Each run writes the folder afresh: `mcp.json` and FossCut's built-in skills in `.claude/skills/`, where the agent finds project skills. The turn cap is 1 to 1000 (200 by default), the spending cap optional (more than 0, at most 1000 US dollars). The brief is not empty and at most 20000 characters; one starting with a dash is prefixed with "Brief:" and a space. The suggested brief (`agent::default_brief`) names the `talking-head-reel` skill, the project, the video and the canvas, and asks the agent not to export.
5. **One MCP server: the app itself.** `mcp.json` names only FossCut's server: the app's own executable in `mcp` mode (`fosscut-desktop mcp -p <bundle>`, which serves `fosscut_mcp::serve_stdio` on stdio and opens no window), with the `FOSSCUT_RUNTIME_DIR` of the app's workspace. Its edits reach the session the app hosts (ADR-0022) as `agent:<client name>` (`agent:claude-code`) and undo like any other.
6. **A background job.** `Jobs::start_agent` runs one agent per project at a time (another start while one runs is `fosscut.export.output_busy`). `fosscut_engine::external`, shared with Remotion, starts it under a shell that leads a process group and ends the group when its standard input closes, so the agent and the MCP server it started never outlive FossCut; cancelling ends the group, and the edits made so far stay. `JobStatus.agent` (an `AgentReport`) holds, while it runs and after it ended: turns of the cap, tool calls and errors, the last tool and message, the model, the session id, whether it reached FossCut's server, and the cost, duration, outcome and summary the agent reports; `progress` counts turns of the cap. A `success` result succeeds; `error_max_turns` and `error_max_budget_usd` fail with `fosscut.agent.limit_reached`; any other ending fails with `fosscut.agent.failed`.
7. **What is logged.** The job keeps the last 30 lines in short form: the agent's error output and non-JSON lines, its text cut to 400 characters, tool names, tool errors and the outcome. FossCut writes nothing else and sends nothing anywhere; the agent keeps its own session history as it always does.
8. **Reach: the desktop only.** Commands `agent_video_pick`, `agent_project`, `agent_find`, `agent_start`, `agent_status` and `agent_cancel`, used by the start screen's "AI edit from a video" card, a video dropped on the start screen, and the app menu. A video becomes a project beside it (a save dialog suggests `<video name>.fosscut`) at the canvas chosen (9:16 by default) and at the standard frame rate within 1% of the video's, with the video imported and placed: two of the user's undoable steps. Then come the brief and caps, the agreement, and a panel docked over the inspector with progress and Cancel while the edits land, then the summary. There is no MCP tool, since an agent does not start another with the user's account, and no CLI command, since a terminal user runs their agent directly. `capabilities` does not list it: it changes no project state itself.

## Consequences

- One drop gets an AI edit made with the user's own agent and account, live and undoable, with no keys in FossCut and nothing extra to install but the agent.
- FossCut depends on Claude Code's flags and event stream, including the undocumented `--max-turns`; a change there needs a change here. Tests run fake agents; a real run is manual.
- The agent's own configuration (model, hooks) still applies. FossCut denies the built-in tools that run commands, change files or reach the web, and leaves out other MCP servers, but cannot vouch for everything a user's setup adds.
- To maintain: the command, the event parser, the notes and the hand-off line.

## Alternatives considered

- **A built-in assistant on a hosted model** (ADR-0012's alternative): FossCut would carry cost, keys and privacy duties.
- **Bundling or installing Claude Code:** another company's licence, size and update cycle; refused, as for Remotion.
- **Calling a model API with the user's key:** FossCut would hold keys and grow its own agent loop; excluded by ADR-0012.
- **The agent with its usual tools, in the user's folders:** it could run commands and change files; the working folder and the deny list keep it to FossCut's tools.
- **Pointing the configuration at the `fosscut` CLI:** the app does not ship it; its own executable is always there.
- **The brief on standard input:** standard input is the watchdog's pipe.
- **Opening a terminal with the agent in it:** no progress or cancel in the app, and different on every platform.

## Validation

- **Engine tests with fake agents:** refusal with the command and notes, invalid requests, a missing agent, the report from the event stream, the final report, failure with the log, the turn cap, cancelling the process group, and the job (one per project).
- **Desktop tests:** the setup, the confirmation, the job and cancelling it, a project from a generated video, and a fake agent that starts the app's executable in `mcp` mode as `mcp.json` says and edits the hosted project as `agent:fake-agent`.
- **UI tests** of the flow, reviewed in the browser stand-in at 1440x900 and 960x640.
- **Before Accepted:** a maintainer runs Claude Code from the app on a real talking-head video on macOS, and sees the hand-off on a computer without an agent; Windows and Linux later.
