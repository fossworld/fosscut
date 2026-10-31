#!/usr/bin/env python3
"""Writes the Tauri configuration overlay that bundles FossCut's desktop app for a release.

FossCut links FFmpeg 8 dynamically (ADR-0009). On Windows and Linux the release ships the LGPL
shared builds the CLI archives ship, inside the installer: next to the executable on Windows,
and in /usr/lib/fosscut (on the executable's rpath) in the Debian package. Homebrew's FFmpeg is
built with GPL parts, so the macOS app uses the ffmpeg@8 the user installs, and is signed
ad hoc because the project has no Developer ID yet.

Usage: desktop_bundle.py <windows|linux|macos> <ffmpeg dir or -> <overlay.json>
The FFmpeg directory holds the DLLs on Windows and the shared libraries on Linux.
"""

import json
import shutil
import sys
from pathlib import Path

DESKTOP = Path(__file__).resolve().parents[2] / "apps" / "desktop"
# Where the libraries are copied for the bundler; git-ignored.
STAGING = DESKTOP / "bundle-libs"


def stage(files: list[Path]) -> list[Path]:
    """Copies `files` into the staging folder, following links, and returns the copies."""
    if STAGING.exists():
        shutil.rmtree(STAGING)
    STAGING.mkdir(parents=True)
    copies = []
    for file in files:
        copy = STAGING / file.name
        shutil.copy2(file, copy, follow_symlinks=True)
        copies.append(copy)
    return copies


def overlay(platform: str, ffmpeg: Path | None) -> dict:
    bundle: dict = {"active": True}
    if platform == "windows":
        dlls = sorted(ffmpeg.glob("*.dll"))
        if not dlls:
            sys.exit(f"no DLLs in {ffmpeg}")
        # Resources beside tauri.conf.json are installed beside the executable.
        for dll in dlls:
            shutil.copy2(dll, DESKTOP / dll.name)
        bundle["targets"] = ["nsis"]
        bundle["resources"] = [dll.name for dll in dlls]
    elif platform == "linux":
        # The names the loader asks for (libavcodec.so.62), not the development links.
        libs = sorted(
            p
            for p in ffmpeg.glob("*.so.*")
            if p.name.count(".so.") == 1 and p.name.split(".so.")[1].isdigit()
        )
        if not libs:
            sys.exit(f"no shared libraries in {ffmpeg}")
        copies = stage(libs)
        bundle["targets"] = ["deb"]
        bundle["linux"] = {
            "deb": {"files": {f"/usr/lib/fosscut/{c.name}": str(c) for c in copies}}
        }
    elif platform == "macos":
        bundle["targets"] = ["dmg"]
        bundle["macOS"] = {"signingIdentity": "-", "minimumSystemVersion": "13.0"}
    else:
        sys.exit(f"unknown platform {platform}")
    return {"bundle": bundle}


def main() -> None:
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    platform, ffmpeg, out = sys.argv[1:]
    config = overlay(platform, None if ffmpeg == "-" else Path(ffmpeg))
    Path(out).write_text(json.dumps(config, indent=2) + "\n")
    print(json.dumps(config, indent=2))


if __name__ == "__main__":
    main()
