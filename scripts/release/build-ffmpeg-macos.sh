#!/bin/bash
# Builds FFmpeg 8.1 for macOS as shared libraries that FossCut can ship (ADR-0009, ADR-0014):
# LGPL only, with no GPL or nonfree parts and no libraries from the build machine beyond
# macOS's own (VideoToolbox, AudioToolbox, AVFoundation, zlib and bzip2). The libraries
# are named under @rpath, so an app bundle or an archive can carry them beside the binary.
#
# Usage: build-ffmpeg-macos.sh <prefix>
# Afterwards <prefix>/lib holds the libraries and pkg-config files, <prefix>/bin/ffprobe the
# probe FossCut runs when importing (it finds the libraries in lib/ beside it, or in an app
# bundle's Frameworks), and <prefix>/LICENSE.txt FFmpeg's LGPL.

set -euo pipefail

version=8.1
sha256=b072aed6871998cce9b36e7774033105ca29e33632be5b6347f3206898e0756a
prefix="$1"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSL --retry 3 "https://ffmpeg.org/releases/ffmpeg-$version.tar.xz" -o "$work/ffmpeg.tar.xz"
echo "$sha256  $work/ffmpeg.tar.xz" | shasum -a 256 -c -
tar -xJf "$work/ffmpeg.tar.xz" -C "$work"
cd "$work/ffmpeg-$version"

./configure \
  --prefix="$prefix" \
  --install-name-dir='@rpath' \
  --enable-shared \
  --disable-static \
  --disable-programs \
  --enable-ffprobe \
  --extra-ldexeflags='-Wl,-rpath,@executable_path/lib -Wl,-rpath,@executable_path/../Frameworks' \
  --disable-doc \
  --disable-autodetect \
  --enable-videotoolbox \
  --enable-audiotoolbox \
  --enable-avfoundation \
  --enable-zlib \
  --enable-bzlib
# configure fails if anything would make the build GPL or nonfree; check it anyway.
if grep -qE '^#define CONFIG_(GPL|NONFREE) 1' config.h; then
  echo "FFmpeg would not be LGPL" >&2
  exit 1
fi
make -j"$(sysctl -n hw.ncpu)"
make install
cp COPYING.LGPLv2.1 "$prefix/LICENSE.txt"
