//! Copying streams into QuickTime without re-encoding (ADR-0032, item 3): the copy holds
//! the same packets, so every frame decodes exactly as from the original, at the same time
//! from the start of its stream, and it seeks exactly.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use fosscut_media::ffmpeg::{FfmpegVideoDecoder, probe_output, remux, remux_with_progress};
use fosscut_media::{DecoderOptions, HardwareDecoding, MediaError, VideoDecoder, VideoFrame};
use fosscut_time::{Flicks, Rate};

fn have_ffmpeg() -> bool {
    let found = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        found || !required,
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is not installed"
    );
    found
}

fn dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-remux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    })
}

/// A 3 s, 25 fps test pattern in `name`'s container with `codec` video, two B-frames
/// between references, and MP2 sound when `sound` is set.
fn source(name: &str, codec: &str, sound: bool) -> PathBuf {
    let path = dir().join(name);
    let mut command = Command::new("ffmpeg");
    command
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=320x240:rate=25:duration=3");
    if sound {
        command.args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=330:sample_rate=48000:duration=3",
            "-c:a",
            "mp2",
        ]);
    }
    let status = command
        .args(["-c:v", codec, "-q:v", "3", "-g", "12", "-bf", "2"])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg could not create {name}");
    path
}

fn software() -> DecoderOptions {
    DecoderOptions {
        hardware: HardwareDecoding::Off,
    }
}

/// Every frame of `path`, decoded in order.
fn all_frames(path: &Path) -> Vec<VideoFrame> {
    let mut decoder = FfmpegVideoDecoder::open(path, software()).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = decoder.next_frame().unwrap() {
        frames.push(frame);
    }
    frames
}

/// Checks that `copy` decodes to `original`'s frames in order, at times that differ by one
/// constant, which it returns (zero but for AVI with reordered frames), and seeks exactly.
fn assert_same_frames(original: &Path, copy: &Path) -> Flicks {
    let expected = all_frames(original);
    let copied = all_frames(copy);
    let name = original.display();
    assert_eq!(copied.len(), expected.len(), "{name}");
    assert_eq!(expected.len(), 75, "3 s at 25 fps");
    let offset = expected[0].pts - copied[0].pts;
    for (n, (a, b)) in expected.iter().zip(&copied).enumerate() {
        assert_eq!(a.pts, b.pts + offset, "{name}: frame {n} keeps its time");
        assert_eq!(a.planes, b.planes, "{name}: frame {n} decodes the same");
    }
    // Seeking in the copy lands on exactly the frame the original shows at that time.
    let mut decoder = FfmpegVideoDecoder::open(copy, software()).unwrap();
    let rate = Rate::new(25, 1).unwrap();
    for n in [60, 7, 41, 74, 0, 33] {
        let wanted = &expected[n as usize];
        let at = wanted.pts - offset + Flicks::new(rate.frame_duration().get() / 2);
        let frame = decoder.frame_at(at).unwrap();
        assert_eq!(frame.pts + offset, wanted.pts, "{name}: seek to frame {n}");
        assert_eq!(frame.planes, wanted.planes, "{name}: seek to frame {n}");
    }
    offset
}

#[test]
fn transport_streams_copy_into_quicktime_with_the_same_frames() {
    if !have_ffmpeg() {
        return;
    }
    let original = source("clip.ts", "mpeg2video", true);
    let copy = dir().join("clip-ts.mov");
    let mut calls = Vec::new();
    let report = remux_with_progress(&original, &copy, &mut |done, total| {
        calls.push((done, total));
        true
    })
    .unwrap();
    assert_eq!(report.brand.as_deref(), Some("qt  "));
    let video = report.video.as_ref().unwrap();
    assert_eq!(
        (video.codec.as_str(), video.width, video.height),
        ("mpeg2video", 320, 240)
    );
    assert_eq!(video.frame_count, 75);
    assert_eq!(video.rate, Rate::new(25, 1).ok());
    assert_eq!(
        report.audio.as_ref().map(|a| a.codec.as_str()),
        Some("mp2"),
        "the sound is copied too"
    );
    let (done, total) = *calls.last().unwrap();
    assert!(total >= 2_900 && done == total, "{calls:?}");
    assert_eq!(probe_output(&copy).unwrap(), report);
    assert_eq!(assert_same_frames(&original, &copy), Flicks::ZERO);
}

#[test]
fn program_streams_avi_and_flv_copy_too() {
    if !have_ffmpeg() {
        return;
    }
    for (name, codec, sound) in [
        ("clip.mpg", "mpeg2video", true),
        ("clip.avi", "mpeg4", true),
        ("clip.flv", "libx264", false),
    ] {
        let original = source(name, codec, sound);
        let copy = dir().join(format!("{name}.mov"));
        let report = remux(&original, &copy).unwrap();
        assert!(report.video.is_some(), "{name}");
        assert_eq!(report.audio.is_some(), sound, "{name}");
        let offset = assert_same_frames(&original, &copy);
        // AVI's stream starts one reordering delay early (see `remux`).
        let expected = if name.ends_with(".avi") {
            Rate::new(25, 1).unwrap().frame_duration()
        } else {
            Flicks::ZERO
        };
        assert_eq!(offset, expected, "{name}");
    }
}

#[test]
fn cancelling_stops_and_media_without_video_is_refused() {
    if !have_ffmpeg() {
        return;
    }
    let original = source("cancel.ts", "mpeg2video", true);
    let copy = dir().join("cancel.mov");
    let error = remux_with_progress(&original, &copy, &mut |_, _| false).unwrap_err();
    assert_eq!(error, MediaError::Cancelled);
    assert_eq!(error.code(), "fosscut.media.cancelled");

    let sound = dir().join("tone.wav");
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("sine=frequency=330:sample_rate=48000:duration=1")
        .arg(&sound)
        .status()
        .unwrap();
    assert!(status.success());
    let error = remux(&sound, &dir().join("tone.mov")).unwrap_err();
    assert_eq!(error.code(), "fosscut.media.unsupported", "{error}");
    let missing = remux(&dir().join("missing.ts"), &dir().join("missing.mov")).unwrap_err();
    assert_eq!(missing.code(), "fosscut.media.open_failed");
}
