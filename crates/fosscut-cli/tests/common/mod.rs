//! Fixtures shared by the end-to-end tests of the `fosscut` binary.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub mod mcp;

/// A fresh, empty directory for one test.
pub fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Whether the `ffmpeg` and `ffprobe` programs run.
pub fn has_ffmpeg() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|tool| {
        Command::new(tool)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// CI sets `FOSSCUT_REQUIRE_FFMPEG=1` so that a missing FFmpeg fails the test instead of
/// skipping it silently.
pub fn ffmpeg_required() -> bool {
    std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Whether FFmpeg and a GPU adapter are available; false skips a test locally, while CI
/// (`FOSSCUT_REQUIRE_FFMPEG`, `FOSSCUT_REQUIRE_GPU`) fails instead.
pub fn can_render() -> bool {
    if !has_ffmpeg() {
        assert!(
            !ffmpeg_required(),
            "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
        );
        return false;
    }
    if fosscut_render::Renderer::headless().is_err() {
        let required =
            std::env::var_os("FOSSCUT_REQUIRE_GPU").is_some_and(|v| !v.is_empty() && v != "0");
        assert!(
            !required,
            "FOSSCUT_REQUIRE_GPU is set but there is no adapter"
        );
        return false;
    }
    true
}

/// The round 1 source clip in `dir`: `test-pattern.mp4`, 8 s of 1280x720 test pattern at
/// 30 fps with a 440 Hz tone at 1/8 amplitude.
pub fn test_pattern(dir: &Path) {
    let status = Command::new("ffmpeg")
        .current_dir(dir)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30:duration=8",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=8",
        ])
        .args([
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
            "-c:a",
            "aac",
            "-shortest",
            "test-pattern.mp4",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg could not create the fixture");
}
