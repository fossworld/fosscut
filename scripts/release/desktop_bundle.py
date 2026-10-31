#!/usr/bin/env python3
"""Writes the Tauri configuration overlay that bundles FossCut's desktop app for a release.

FossCut links FFmpeg 8 dynamically and runs its ffprobe to import media (ADR-0009). Every
installer carries an LGPL build of both, since an app opened from the Finder or a desktop menu
does not see the shell's path:

- Windows: the DLLs and ffprobe.exe beside the executable.
- Linux: the libraries and ffprobe in /usr/lib/fosscut, on the executable's rpath (ffprobe
  finds the libraries beside itself).
- macOS: the libraries in Contents/Frameworks and ffprobe in Contents/MacOS, both found
  through @rpath. The app is signed ad hoc, as the project has no Developer ID yet.

Usage: desktop_bundle.py <windows|linux|macos> <ffmpeg dir> <overlay.json>
The FFmpeg directory holds bin/ (ffprobe, and the DLLs on Windows) and lib/, as
.github/actions/setup-release-ffmpeg leaves it.
"""

import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

DESKTOP = Path(__file__).resolve().parents[2] / "apps" / "desktop"
# Where the files are copied for the bundler; git-ignored.
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


def named(folder: Path, pattern: str) -> list[Path]:
    """The files of `folder` whose names match `pattern`, sorted."""
    return sorted(p for p in folder.iterdir() if re.fullmatch(pattern, p.name))


def required(files: list[Path], what: str, where: Path) -> list[Path]:
    if not files:
        sys.exit(f"no {what} in {where}")
    return files


def overlay(platform: str, ffmpeg: Path) -> dict:
    bundle: dict = {"active": True}
    if platform == "windows":
        bin_dir = ffmpeg / "bin"
        shipped = required(named(bin_dir, r".+\.dll"), "DLLs", bin_dir)
        shipped += required(named(bin_dir, r"ffprobe\.exe"), "ffprobe.exe", bin_dir)
        # Resources beside tauri.conf.json are installed beside the executable.
        for file in shipped:
            shutil.copy2(file, DESKTOP / file.name)
        bundle["targets"] = ["nsis"]
        bundle["resources"] = [file.name for file in shipped]
    elif platform == "linux":
        # The names the loader asks for (libavcodec.so.62), not the development links.
        lib_dir = ffmpeg / "lib"
        libs = required(named(lib_dir, r"lib\w+\.so\.\d+"), "shared libraries", lib_dir)
        probe = required(named(ffmpeg / "bin", r"ffprobe"), "ffprobe", ffmpeg / "bin")
        copies = stage(libs + probe)
        subprocess.run(["patchelf", "--set-rpath", "$ORIGIN", str(STAGING / "ffprobe")], check=True)
        bundle["targets"] = ["deb"]
        bundle["linux"] = {
            "deb": {"files": {f"/usr/lib/fosscut/{c.name}": str(c) for c in copies}}
        }
    elif platform == "macos":
        # The names the binary asks for through @rpath (libavcodec.62.dylib).
        libs = required(named(ffmpeg / "lib", r"lib\w+\.\d+\.dylib"), "libraries", ffmpeg / "lib")
        probe = required(named(ffmpeg / "bin", r"ffprobe"), "ffprobe", ffmpeg / "bin")
        copies = stage(libs + probe)
        bundle["targets"] = ["dmg"]
        bundle["macOS"] = {
            "signingIdentity": "-",
            "minimumSystemVersion": "13.0",
            "frameworks": [str(c) for c in copies if c.suffix == ".dylib"],
            "files": {"MacOS/ffprobe": str(STAGING / "ffprobe")},
        }
    else:
        sys.exit(f"unknown platform {platform}")
    return {"bundle": bundle}


def main() -> None:
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    platform, ffmpeg, out = sys.argv[1:]
    config = overlay(platform, Path(ffmpeg))
    Path(out).write_text(json.dumps(config, indent=2) + "\n")
    print(json.dumps(config, indent=2))


if __name__ == "__main__":
    main()
