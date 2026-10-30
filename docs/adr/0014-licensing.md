# ADR-0014: Dual MIT or Apache-2.0 license, DCO, and a dependency license policy

- **Status:** Proposed (the license choice must be confirmed by the project owner before the first public release)
- **Date:** 2026-10-28
- **Related:** [ADR-0009](0009-media-io-and-codecs.md), [ADR-0012](0012-ai-capabilities-bring-your-own-brain.md), [ADR-0015](0015-clean-room-policy.md), private research: `docs/capcut-rea/ARCHITECTURE.md`, `docs/capcut-rea/RESOURCES.md`

## Context

FossCut is open source and wants wide adoption: by individual creators, by agent platforms that embed it, and possibly by companies building on it. The Rust ecosystem's convention is dual licensing under MIT or Apache-2.0. Media software has license traps: GPL encoders (x264, x265), GPL builds of FFmpeg, non-commercial ML models, and proprietary system or brand fonts that are easy to bundle by accident.

## Decision

1. **Project license: `MIT OR Apache-2.0`** for all code, schemas and documentation written for FossCut, with `LICENSE-MIT` and `LICENSE-APACHE` at the root. Copyright is held by "The FossCut Contributors".
2. **Contributions use the Developer Certificate of Origin** (`Signed-off-by` on every commit), enforced in CI. No contributor license agreement.
3. **Dependency policy, enforced by `cargo-deny` and an npm license check:**
   - allowed: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, MPL-2.0 (file-level copyleft), CC0-1.0, and the Unlicense (a public-domain dedication like CC0, added for the whisper.cpp bindings in [ADR-0020](0020-speech-recognition-with-whisper-cpp.md)), and 0BSD (public-domain-equivalent, added for the local socket transport in [ADR-0022](0022-live-sessions-over-a-local-socket.md));
   - allowed only as dynamically linked system or runtime libraries: LGPL (for example FFmpeg built without GPL parts);
   - not allowed in distributed builds: GPL, AGPL, SSPL, non-commercial or field-of-use restricted licenses.
4. **Models, fonts, LUTs, sound effects and other content** carry a license and source in their manifests. Only content with licenses compatible with commercial use is offered by default. Fonts: OFL or Apache-2.0 only (for example Noto and Source Han); no proprietary fonts.
5. **A `NOTICE` file and an in-app "third-party licenses" view** are generated from dependency metadata at release time.

## Consequences

- Companies and agent platforms can embed FossCut's crates without copyleft obligations, which helps adoption.
- A permissive license also allows closed forks; the project accepts this in exchange for reach.
- Some best-in-class components (x264, certain models) cannot ship by default; users can opt into them locally.

## Alternatives considered

- **GPL-3.0** (as Kdenlive and Shotcut): keeps derivatives open and allows bundling GPL encoders, but blocks embedding in permissive or proprietary agent platforms and conflicts with the Rust ecosystem norm.
- **MPL-2.0:** file-level copyleft, a middle ground; less familiar to Rust contributors.
- **Apache-2.0 only:** explicit patent grant but incompatible with GPL-2.0-only projects; dual licensing avoids that.

## Validation

`cargo-deny` and the npm license check pass in CI; the owner confirms the license choice in the pull request that accepts this ADR.
