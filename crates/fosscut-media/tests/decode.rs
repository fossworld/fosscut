//! Decoding real files made with the FFmpeg command-line tool: exact timestamps, exact
//! seeking, color tags, rotation, hardware decoding and errors (ADR-0009).
//!
//! Each fixture's luma encodes its frame number (`16 + 1.5 n`), so a test can tell which
//! frame it got. CI sets `FOSSCUT_REQUIRE_FFMPEG=1`, which turns a missing `ffmpeg` into a
//! failure instead of a skip.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use fosscut_media::ffmpeg::{FfmpegVideoDecoder, probe_video};
use fosscut_media::{
    Acceleration, DecoderOptions, HardwareDecoding, Matrix, MediaError, PixelFormat, Range,
    VideoDecoder,
};
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
    if !found {
        eprintln!("skipping: ffmpeg is not installed");
    }
    found
}

fn fixtures_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    })
}

/// A clip whose luma is `16 + 1.5 n` in frame `n`, with a keyframe every second.
/// `params` tags the frames with FFmpeg's `setparams` filter, which labels color without
/// converting pixel values (output options such as `-color_range` would convert them).
fn frame_index_clip(name: &str, rate: &str, seconds: u32, params: &str, extra: &[&str]) -> PathBuf {
    // Tests run as threads in one process under `cargo test`: create each fixture once,
    // under a lock, and move it into place only when complete.
    static CREATING: Mutex<()> = Mutex::new(());
    let _guard = CREATING.lock().unwrap_or_else(|e| e.into_inner());
    let path = fixtures_dir().join(name);
    if path.exists() {
        return path;
    }
    let partial = fixtures_dir().join(format!("partial-{name}"));
    let mut source = format!(
        "color=c=black:s=320x180:r={rate}:d={seconds},format=yuv420p,geq=lum='16+N*1.5':cb=128:cr=128"
    );
    if !params.is_empty() {
        source.push_str(",setparams=");
        source.push_str(params);
    }
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", &source])
        .args([
            "-c:v", "libx264", "-preset", "veryfast", "-qp", "0", "-g", "30", "-bf", "2",
        ])
        .args(extra)
        .arg(&partial)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg could not create {name}");
    std::fs::rename(&partial, &path).unwrap();
    path
}

fn tagged_clip() -> PathBuf {
    frame_index_clip(
        "index-30.mp4",
        "30",
        4,
        "range=tv:colorspace=bt709:color_primaries=bt709:color_trc=bt709",
        &[],
    )
}

fn frame_number(luma: f64) -> i64 {
    ((luma - 16.0) / 1.5).round() as i64
}

fn software() -> DecoderOptions {
    DecoderOptions {
        hardware: HardwareDecoding::Off,
    }
}

#[test]
fn probing_reports_size_rate_duration_and_color_tags() {
    if !have_ffmpeg() {
        return;
    }
    let info = probe_video(&tagged_clip()).unwrap();
    assert_eq!(info.codec, "h264");
    assert_eq!((info.width, info.height), (320, 180));
    assert_eq!(info.rate, Some(Rate::FPS_30));
    assert_eq!(info.frame_count, Some(120));
    assert_eq!(info.duration, Flicks::from_seconds(4));
    assert_eq!(info.rotation, 0);
    assert_eq!(info.color.matrix, Matrix::Bt709);
    assert_eq!(info.color.range, Range::Limited);
    assert_eq!(info.source_pixel_format, "yuv420p");
}

#[test]
fn sequential_frames_have_exact_timestamps_and_content() {
    if !have_ffmpeg() {
        return;
    }
    let mut decoder = FfmpegVideoDecoder::open(&tagged_clip(), software()).unwrap();
    let mut count = 0;
    while let Some(frame) = decoder.next_frame().unwrap() {
        assert_eq!(
            frame.pts,
            Rate::FPS_30.frames_to_flicks(count),
            "frame {count}"
        );
        assert_eq!(frame.duration, Rate::FPS_30.frame_duration());
        assert_eq!(
            frame_number(frame.mean_luma()),
            count,
            "content of frame {count}"
        );
        assert_eq!(frame.format, PixelFormat::Yuv420p);
        assert_eq!(frame.planes[1].rows, 90);
        count += 1;
    }
    assert_eq!(count, 120);
    assert_eq!(decoder.acceleration(), Acceleration::Software);
}

#[test]
fn frame_at_returns_exactly_the_displayed_frame_in_any_order() {
    if !have_ffmpeg() {
        return;
    }
    let mut decoder = FfmpegVideoDecoder::open(&tagged_clip(), software()).unwrap();
    let rate = Rate::FPS_30;
    let half_frame = Flicks::new(rate.frame_duration().get() / 2);
    for n in [0, 1, 118, 29, 30, 31, 119, 59, 60, 2, 90, 89, 118, 118] {
        let time = rate.frames_to_flicks(n) + half_frame;
        let frame = decoder.frame_at(time).unwrap();
        assert_eq!(frame.pts, rate.frames_to_flicks(n), "pts for frame {n}");
        assert_eq!(frame_number(frame.mean_luma()), n, "content for frame {n}");
    }
    let exact = decoder.frame_at(rate.frames_to_flicks(45)).unwrap();
    assert_eq!(
        frame_number(exact.mean_luma()),
        45,
        "a frame's own start time"
    );
}

#[test]
fn ntsc_rates_keep_exact_timestamps() {
    if !have_ffmpeg() {
        return;
    }
    let clip = frame_index_clip("index-ntsc.mp4", "30000/1001", 2, "", &[]);
    let rate = Rate::NTSC_30;
    let info = probe_video(&clip).unwrap();
    assert_eq!(info.rate, Some(rate));
    let mut decoder = FfmpegVideoDecoder::open(&clip, software()).unwrap();
    for n in 0..20 {
        let frame = decoder.next_frame().unwrap().unwrap();
        assert_eq!(frame.pts, rate.frames_to_flicks(n));
    }
    let frame = decoder.frame_at(rate.frames_to_flicks(50)).unwrap();
    assert_eq!(frame.pts, rate.frames_to_flicks(50));
    assert_eq!(frame_number(frame.mean_luma()), 50);
}

#[test]
fn reading_past_the_end_is_a_typed_error() {
    if !have_ffmpeg() {
        return;
    }
    let mut decoder = FfmpegVideoDecoder::open(&tagged_clip(), software()).unwrap();
    let last = decoder
        .frame_at(Rate::FPS_30.frames_to_flicks(119))
        .unwrap();
    assert_eq!(frame_number(last.mean_luma()), 119);
    let error = decoder
        .frame_at(Flicks::from_seconds(5).unwrap())
        .unwrap_err();
    assert_eq!(error.code(), "fosscut.media.end_of_stream");
    let MediaError::EndOfStream { end_flicks, .. } = error else {
        unreachable!()
    };
    assert_eq!(end_flicks, Flicks::from_seconds(4).unwrap().get());
    let again = decoder.frame_at(Rate::FPS_30.frames_to_flicks(10)).unwrap();
    assert_eq!(
        frame_number(again.mean_luma()),
        10,
        "seeking back after the end"
    );
}

#[test]
fn full_range_and_converted_formats_are_handled() {
    if !have_ffmpeg() {
        return;
    }
    let full = frame_index_clip("index-full.mp4", "30", 1, "range=pc", &[]);
    let mut decoder = FfmpegVideoDecoder::open(&full, software()).unwrap();
    let frame = decoder.next_frame().unwrap().unwrap();
    assert_eq!(frame.color.range, Range::Full);

    let high = frame_index_clip("index-422.mp4", "30", 1, "", &["-pix_fmt", "yuv422p"]);
    let mut decoder = FfmpegVideoDecoder::open(&high, software()).unwrap();
    let frame = decoder.frame_at(Rate::FPS_30.frames_to_flicks(7)).unwrap();
    assert_eq!(
        frame.format,
        PixelFormat::Nv12,
        "4:2:2 is converted to NV12"
    );
    assert_eq!(frame_number(frame.mean_luma()), 7);
    assert_eq!(frame.planes[1].stride, 320, "interleaved chroma row");
}

#[test]
fn rotation_comes_from_the_display_matrix() {
    if !have_ffmpeg() {
        return;
    }
    let plain = tagged_clip();
    let rotated = fixtures_dir().join("rotated.mp4");
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-display_rotation", "90", "-i"])
        .arg(&plain)
        .args(["-c", "copy"])
        .arg(&rotated)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(probe_video(&rotated).unwrap().rotation, 270);
}

#[test]
fn hardware_decoding_matches_software() {
    if !have_ffmpeg() {
        return;
    }
    let clip = tagged_clip();
    let mut hardware = FfmpegVideoDecoder::open(&clip, DecoderOptions::default()).unwrap();
    let mut software = FfmpegVideoDecoder::open(&clip, software()).unwrap();
    for n in [0, 45, 118] {
        let time = Rate::FPS_30.frames_to_flicks(n);
        let a = hardware.frame_at(time).unwrap();
        let b = software.frame_at(time).unwrap();
        assert_eq!(a.pts, b.pts);
        assert!(
            (a.mean_luma() - b.mean_luma()).abs() < 1.0,
            "frame {n}: {} vs {}",
            a.mean_luma(),
            b.mean_luma()
        );
    }
    if cfg!(target_os = "macos") {
        assert_eq!(hardware.acceleration(), Acceleration::VideoToolbox);
    } else {
        assert_eq!(hardware.acceleration(), Acceleration::Software);
        let required = FfmpegVideoDecoder::open(
            &clip,
            DecoderOptions {
                hardware: HardwareDecoding::Require,
            },
        );
        assert_eq!(
            required.err().map(|e| e.code()),
            Some("fosscut.media.hardware_unavailable")
        );
    }
}

#[test]
fn unreadable_inputs_are_typed_errors() {
    if !have_ffmpeg() {
        return;
    }
    let missing = FfmpegVideoDecoder::open(Path::new("/no/such/file.mp4"), software());
    assert_eq!(
        missing.err().map(|e| e.code()),
        Some("fosscut.media.open_failed")
    );

    let garbage = fixtures_dir().join("garbage.mp4");
    std::fs::write(&garbage, b"this is not a video file at all").unwrap();
    assert_eq!(
        probe_video(&garbage).err().map(|e| e.code()),
        Some("fosscut.media.open_failed")
    );

    let audio = fixtures_dir().join("tone.m4a");
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", "sine=duration=1"])
        .arg(&audio)
        .status()
        .unwrap();
    assert!(status.success());
    let error = probe_video(&audio).unwrap_err();
    assert_eq!(error.code(), "fosscut.media.no_video_stream");
    assert!(error.to_string().contains("tone.m4a"));
}

#[test]
fn codecs_without_hardware_support_fall_back_or_fail_when_hardware_is_required() {
    if !have_ffmpeg() {
        return;
    }
    let clip = fixtures_dir().join("index-mjpeg.mov");
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("color=c=black:s=320x180:r=30:d=1,format=yuv420p,geq=lum='16+N*1.5':cb=128:cr=128,setparams=range=pc")
        .args(["-c:v", "mjpeg", "-q:v", "2"])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(status.success());

    let mut automatic = FfmpegVideoDecoder::open(&clip, DecoderOptions::default()).unwrap();
    assert_eq!(automatic.path(), clip.display().to_string());
    assert_eq!(automatic.info().codec, "mjpeg");
    let frame = automatic
        .frame_at(Rate::FPS_30.frames_to_flicks(12))
        .unwrap();
    assert_eq!(automatic.acceleration(), Acceleration::Software);
    assert_eq!(frame.color.range, Range::Full, "JPEG frames are full range");
    assert!(
        (frame_number(frame.mean_luma()) - 12).abs() <= 1,
        "lossy, but frame 12"
    );

    let required = FfmpegVideoDecoder::open(
        &clip,
        DecoderOptions {
            hardware: HardwareDecoding::Require,
        },
    )
    .and_then(|mut decoder| decoder.next_frame().map(|_| ()));
    assert_eq!(
        required.err().map(|e| e.code()),
        Some("fosscut.media.hardware_unavailable")
    );
}

#[test]
fn times_before_the_first_frame_show_the_first_frame() {
    if !have_ffmpeg() {
        return;
    }
    let mut decoder = FfmpegVideoDecoder::open(&tagged_clip(), software()).unwrap();
    decoder
        .frame_at(Rate::FPS_30.frames_to_flicks(100))
        .unwrap();
    let frame = decoder.frame_at(Flicks::new(-1)).unwrap();
    assert_eq!(frame.pts, Flicks::ZERO);
    assert_eq!(frame_number(frame.mean_luma()), 0);
}

#[test]
fn the_last_frame_stays_available_after_reading_past_the_end() {
    if !have_ffmpeg() {
        return;
    }
    let mut decoder = FfmpegVideoDecoder::open(&tagged_clip(), software()).unwrap();
    let error = decoder
        .frame_at(Flicks::from_seconds(9).unwrap())
        .unwrap_err();
    assert_eq!(error.code(), "fosscut.media.end_of_stream");
    let last = decoder
        .frame_at(Rate::FPS_30.frames_to_flicks(119))
        .unwrap();
    assert_eq!(frame_number(last.mean_luma()), 119);
}

#[test]
fn millisecond_time_bases_still_find_exact_frames() {
    if !have_ffmpeg() {
        return;
    }
    // WebM rounds every timestamp to a millisecond; lossless VP9 keeps the luma exact.
    let clip = frame_index_clip(
        "index.webm",
        "30",
        2,
        "",
        &["-c:v", "libvpx-vp9", "-lossless", "1"],
    );
    let info = probe_video(&clip).unwrap();
    assert_eq!(info.time_base, (1, 1000));
    let mut decoder = FfmpegVideoDecoder::open(&clip, software()).unwrap();
    for n in [0, 1, 2, 17, 29, 30, 58, 59, 3, 44] {
        let frame = decoder.frame_at(Rate::FPS_30.frames_to_flicks(n)).unwrap();
        assert_eq!(frame_number(frame.mean_luma()), n, "frame {n}");
    }
}
