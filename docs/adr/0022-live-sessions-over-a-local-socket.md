# ADR-0022: Host live sessions over a local socket with per-session keys

- **Status:** Proposed
- **Date:** 2026-10-30
- **Related:** [ADR-0002](0002-agent-first-command-api.md), [ADR-0003](0003-rust-workspace-and-crate-layering.md), [ADR-0004](0004-open-project-format.md), [ADR-0006](0006-agent-safety-contract.md), [ADR-0010](0010-desktop-shell-ui-and-web-path.md), [ADR-0014](0014-licensing.md), [ADR-0016](0016-testing-strategy.md), private research: `docs/capcut-rea/AGENT-SUBSYSTEM.md`

## Context

ADR-0002 item 3 makes the desktop app the host of an open project: when the CLI or the MCP server targets that project, it connects to the app's session over a local socket, so the user watches the agent's edits live and can undo them. Its Consequences leave the protocol open: a local socket, a per-session token, and no network listener by default. Milestone 5 builds the app, so the protocol has to be decided first.

The forces:

- **One writer.** A `Session` holds the project and its operation log in memory and enforces revisions, idempotency and undo (ADR-0006). Two sessions writing one bundle lose edits and duplicate log sequence numbers.
- **Today's lock is not enough.** The CLI and MCP server take a `.lock` file for each request and reopen the project from disk (ADR-0002, implementation notes). The lock is judged stale by its age, so two waiters can both remove it, and a crashed writer blocks the project for two minutes. It cannot be held for the hours an app keeps a project open.
- **Live view.** The app must learn about every change as it is committed, not by polling the disk.
- **No caller changes.** The CLI and the MCP server reach projects only through `fosscut_engine::project`. Frame grabs, contact sheets, exports, analysis and caption generation read the project and run in the caller's process.
- **Processes see different environments.** An app started from the dock has no shell variables, agent clients start `fosscut mcp` with a reduced environment, and one bundle can have several paths (symlinks, bind mounts). Anything that only works when every process computes the same path is fragile.
- **Security.** Other users of the machine, other machines and web pages must not reach a session. Bundles can sit in synced or shared folders, so nothing secret may be stored inside them, and ADR-0004 keeps machine-specific data out of the project.
- **Research (behaviour only).** In the editor studied, the built-in agent edits the running editor through a command-line tool that sends JSON requests to it over a local socket, with a message size cap and a request timeout, and never touches project files (`docs/capcut-rea/AGENT-SUBSYSTEM.md`).

## Decision

1. **Hosts.** A host is the process that holds a project's `Session` in memory and is its only writer: the desktop app for each project it has open, or `fosscut host` (headless, for tests and scripts). Hosting lives in `fosscut-engine` (module `live`), so the app and the CLI share it. A second host for the same bundle fails with `fosscut.session.already_hosted`, and the app then connects as a client or brings its existing window forward.
2. **The writer lock.** Each bundle has an empty file `.writer.lock` that is created once and never deleted. Writers lock it with the operating system's file lock (`File::lock` and `try_lock`: `flock` on Unix, `LockFileEx` on Windows), which follows the file through any path and is released by the kernel when the process dies. A host holds it for as long as it hosts; a local writer holds it for one request. It replaces the `.lock` file. `.writer.lock` is a runtime file, not part of the project format, and sync tools may copy it harmlessly.
3. **Discovery records.** A host announces a project with a record in the user's runtime directory, never in the bundle:
   - **Directory:** `~/Library/Application Support/fosscut/run` on macOS, `%LOCALAPPDATA%\fosscut\run` on Windows, `$XDG_RUNTIME_DIR/fosscut` on Linux (else `~/.local/state/fosscut/run`). `FOSSCUT_RUNTIME_DIR` overrides it for tests. On Unix it is created 0700, and FossCut refuses to use one owned by another user or open to others.
   - **File:** `sessions/<key>.json`. The key comes from the bundle directory's device and inode on Unix, and from the BLAKE3 hash of its canonical path on Windows.
   - **Contents:** `protocol` (1), `endpoint`, the session key (item 5), `pid`, `host` (name and version), `bundle` (canonical path), `project` (id) and `started` (Unix milliseconds).
   - **Writing:** created new with owner-only permissions, then renamed into place. Records are written and removed only by a process that holds the writer lock.
4. **Endpoints.** A Unix domain socket with a short random name in the runtime directory on macOS and Linux (socket paths are limited to about 100 bytes; a longer path is a typed error). On Windows, a named pipe with a random name that rejects remote clients and grants access only to the system and the owner; clients open it at the identification impersonation level, so a server cannot act as them. Hosts never open TCP or UDP listeners.
5. **Authentication.** Each hosted project gets a fresh 32-byte key from the operating system's random source, stored only in its record. Both sides prove they know it without sending it:
   - the client's `hello` carries a random nonce;
   - the host answers with its own nonce and a BLAKE3 keyed hash over a direction label, the protocol version, both nonces and the bundle key;
   - the client checks it, then sends its own hash in the other direction.
   A wrong proof closes the connection with `fosscut.session.unauthorized`. Before authentication, messages are capped at 4 KiB, handshakes have a deadline, and unauthenticated connections are limited.
6. **Messages.** JSON-RPC 2.0, one UTF-8 JSON object per line, at most 64 MiB per message.
   - `hello` and `auth`: the handshake, with `protocol`, the client's name (`agent:<client>`, `cli`, `app`) and the bundle path, which the host checks is its bundle. The host answers with its name, the project id and the current revision.
   - `execute`: a core `Request` (ADR-0006), answered with its `Response`. Undo and redo are commands, so they need nothing more. The host sets the commit time. `actor` and the client's name are labels for the log, never permissions.
   - `query`: a query name and arguments.
   - `snapshot`: the project's documents, keyed by their paths in the bundle, and the revision they belong to. Clients load them through the format's normal loader, which validates them.
   - `subscribe`: after it, the host sends a `changed` notification after every commit: revision, sequence number, kind (edit, undo, redo), actor and command names. A subscriber that falls behind is dropped; it never delays a commit.
   - **Errors:** a FossCut `ApiError` travels as the JSON-RPC error's `data`, so a client returns exactly the error a local session would.
7. **Hosting order.** A host takes the writer lock (waiting for a local write in progress), loads the project, starts listening, then writes its record. On shutdown it refuses new writes, finishes the one in progress, removes its record and releases the lock. Before each commit it checks that the project on disk is still at its revision and that `.writer.lock` is the file it locked; otherwise it stops writing with `fosscut.session.diverged`, since a sync tool or version control replaced files under it.
8. **Clients.** `fosscut_engine::project` decides per call, until a 10 s deadline:
   - **The writer lock is free:** no host is running. Remove any leftover record and socket, write locally under the lock, release it.
   - **The lock is held and a record exists:** connect, authenticate and send the request to the host.
   - **The lock is held and no record exists yet** (a host starting, or a local write): wait and try again. At the deadline: `fosscut.project.busy`.

   Reads use the host when a record exists, and the disk otherwise. `project::snapshot` returns the project from the host or the disk. The CLI connects per call. The MCP server keeps one connection per project while it works on it, so the app can list it as connected.
9. **Where work runs.** The host only serialises reads and writes of project state. Frames, contact sheets, exports and analysis run in the client's process on a snapshot, as they do without a host, and resolve media against the bundle path as before.
10. **Failures.** A protocol version the host does not speak is `fosscut.session.protocol`, with the host's version in the details. Requests time out after 120 s. A host lost after a request was sent is `fosscut.session.unavailable` with `details.outcome = "unknown"` and the request's `request_id`. Clients never retry a sent request themselves; an agent may retry with the same `request_id` (ADR-0006 item 3), and should read the history first when it sent none.
11. **Dependencies.** Unix sockets and the Windows client use the standard library. The Windows pipe server uses the `interprocess` crate, the only safe way to set the pipe's access list; no FossCut crate needs `unsafe` for this. Its dependencies add the 0BSD license, a public-domain-equivalent license, to the allowed list (ADR-0014). The writer lock needs Rust 1.89, the new minimum.

## Consequences

- **Live edits with no new tools:** every capability of the CLI and MCP works against an open app. The app's own edits run through the same session, so agents meet the user's changes as revision conflicts and re-read, as the safety contract already requires.
- **One writer, enforced by the kernel:** a crashed host or writer frees the project at once, and paths that differ between processes can only end in a busy or unavailable error, never in a second writer.
- **Security boundary:** owner-only directories, files and pipes, and a key that never crosses the wire. A process running as the same user can still read the record and connect; it could edit the bundle directly anyway.
- **New maintenance:** a versioned wire protocol, two transports, and end-to-end tests that start host processes. Changing a message's shape is a protocol version bump; snapshot tests of every message make that visible.
- **Snapshots cost a copy:** each frame grab or export from a client copies the project's documents. This is small next to rendering.
- **Not covered:** two machines editing one bundle in a synced folder. The lock and records are per machine; `fosscut.session.diverged` reports what it can detect.

## Alternatives considered

- **TCP on 127.0.0.1 with a token:** one transport everywhere, but reachable by every local user and, through DNS rebinding and request forgery, by web pages.
- **The discovery record as the only fence:** no lock, but processes with different environments or paths would not find the same record and would write at once.
- **Keeping the age-based `.lock`:** no new dependency on file locking, but its stale takeover is racy and a crash blocks the project.
- **The app as an MCP server only:** agents would connect to the app directly, but the CLI would need a second path, and `fosscut mcp` would behave differently depending on whether the app runs.
- **gRPC or another RPC framework:** typed streams, but a large dependency and code generation for a handful of methods whose requests and responses are already the core's JSON types.
- **Sending the key in `hello`:** simpler, but a process that took over a dead host's endpoint would receive it.

## Implementation notes

- **Where it lives:**
  - `fosscut_engine::live`: the host, records, wire format, handshake and transports.
  - `fosscut_engine::project::Projects`: the routing. The CLI uses it per call, and the MCP server keeps one per process.
  - `fosscut host [--events] [--exit-on-eof]`: hosting without the app, for tests and scripts.
  - `project info` (CLI) and `project_info` and `project_open` (MCP) report `session`, live or not, with the host's name.
- **Readers never repair.** Opening a project from disk completes a save that a crash interrupted, by writing the missing documents. A reader doing that next to a writer collides with it: two hundred concurrent clients made a reader's repair overwrite the host's half-written manifest.
  - Reads from disk now use `Session::open_read_only`, which completes such a save in memory only.
  - Only writers, holding the lock, repair.
- **The lock decides before every write, even with a kept connection.** On macOS, writing into a socket whose host has shut it down can still succeed, so only the reply's absence shows the host is gone, and then the outcome is unknown.
  - Before reusing a connection, a client first tries the writer lock. A free lock means no host runs, so the write goes local.
  - A kept connection the host has hung up on is dropped unused.
- **One direction at a time.** Windows serialises synchronous I/O on one pipe handle, so neither side reads and writes a connection at once.
  - `subscribe` turns its connection into a stream of notifications.
  - Windows pipe handles have no read timeout; a client there waits until the host answers or the pipe breaks.
  - Closing wakes the host's accept loop with a connection that stays open until the loop ends. Windows discards a pipe client that leaves before the host waits for it, and the first CI run on Windows hung closing a host that had just started.
- **Change kinds** are the operation log's: `commit`, `undo` and `redo`.
- **Paths cross the wire absolute.** The host checks `hello`'s bundle from its own working directory, so a relative `-p Demo.fosscut` named another folder there and every edit came back `fosscut.project.busy`. The first run of the desktop app showed it; the CLI tests now start the host from another folder than their clients.

## Validation

- **Engine tests**, in-process with explicit runtime directories: directory and record permissions, the record key for symlinked paths, a wrong key, a fake host failing the client's check, a protocol mismatch, both size caps, identical errors locally and through a host, and snapshot tests of every message.
- **Fencing, without sleeps:** a held lock delays a host until released; a second host is refused; a leftover record with a free lock is cleaned up; 200 concurrent clients all succeed and the record survives.
- **End to end:** with `fosscut host` as a child process, CLI edits go through the host and appear as events with their actor, undo works, two clients racing on `expected_revision` get one commit and one `fosscut.revision.conflict`, and after the host is killed the next write runs locally with continuous sequence numbers.
- **MCP:** the round 1 edit sequence (import, title, split, undo) against a running host, observed through `subscribe`; then a real-agent run.
- **Exit check of milestone 5** (ADR-0002 Validation): the same sequence while the desktop app has the project open appears live in the app and undoes cleanly.
