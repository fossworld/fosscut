# Security policy

## Reporting a vulnerability

Please report security problems privately through GitHub: open this repository's **Security** tab and choose **Report a vulnerability**. Only maintainers can see the report. Do not open a public issue for a vulnerability.

Include what you found, how to reproduce it (a minimal project, media file or command sequence), the affected version or commit, and the impact you expect. We aim to acknowledge reports within seven days and to agree on a disclosure date with you once a fix is ready.

## Supported versions

FossCut has no stable release yet. Fixes are made on `main`. Once releases exist, the latest minor version receives security fixes.

## Areas of particular interest

- Parsing of project files, patches, selectors and imported timelines ([ADR-0004](docs/adr/0004-open-project-format.md), [ADR-0007](docs/adr/0007-timeline-projection-and-patches.md)).
- Media probing and decoding of untrusted files ([ADR-0009](docs/adr/0009-media-io-and-codecs.md)).
- The MCP server and the local IPC between front ends and the engine: authentication of local clients, file access limited to granted paths, no arbitrary file reads or command execution ([ADR-0002](docs/adr/0002-agent-first-command-api.md), [ADR-0006](docs/adr/0006-agent-safety-contract.md)).
- Handling of AI provider credentials ([ADR-0012](docs/adr/0012-ai-capabilities-bring-your-own-brain.md)).
- Third-party effect packages and plug-ins ([ADR-0011](docs/adr/0011-effects-and-animatable-properties.md)).
