//! Pictures with an alpha channel decode as straight-alpha R'G'B'A (ADR-0039): PNG with
//! alpha keeps its code values, ProRes 4444 converts with its own matrix, and pictures
//! without alpha still decode as Y'CbCr.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use fosscut_media::ffmpeg::FfmpegVideoDecoder;
use fosscut_media::{DecoderOptions, PixelFormat, VideoDecoder};
use fosscut_time::Flicks;

/// Makes `name` in a fresh folder with FFmpeg from a `lavfi` source, or `None` without
/// FFmpeg (a failure when `FOSSCUT_REQUIRE_FFMPEG` is set).
fn make(name: &str, source: &str, args: &[&str]) -> Option<PathBuf> {
    let found = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        found || !required,
        "FOSSCUT_REQUIRE_FFMPEG is set but ffmpeg is missing"
    );
    if !found {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("fosscut-media-alpha-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", source])
        .args(args)
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    Some(path)
}

fn first_pixel(path: &Path) -> (PixelFormat, Vec<u8>) {
    let mut decoder = FfmpegVideoDecoder::open(path, DecoderOptions::default()).unwrap();
    let frame = decoder.frame_at(Flicks::ZERO).unwrap();
    let bytes = frame.planes[0].data[..4].to_vec();
    (frame.format, bytes)
}

#[test]
fn png_with_alpha_keeps_its_code_values_and_alpha() {
    let Some(png) = make(
        "quarter.png",
        "color=c=0x2050FF@0.25:s=8x8,format=rgba",
        &["-frames:v", "1", "-pix_fmt", "rgba"],
    ) else {
        return;
    };
    let (format, pixel) = first_pixel(&png);
    assert_eq!(format, PixelFormat::Rgba);
    // FFmpeg's colour source rounds through Y'CbCr, so the file holds about this blue; its
    // code values come out as stored.
    for (got, want) in pixel.iter().zip([0x20u8, 0x50, 0xFF]) {
        assert!(got.abs_diff(want) <= 3, "{pixel:?}");
    }
    assert!((i32::from(pixel[3]) - 64).abs() <= 1, "{pixel:?}");

    let opaque = make(
        "opaque.png",
        "color=c=0x2050FF:s=8x8",
        &["-frames:v", "1", "-pix_fmt", "rgb24"],
    )
    .unwrap();
    assert_eq!(
        first_pixel(&opaque).0,
        PixelFormat::Nv12,
        "no alpha, no change"
    );
}

#[test]
fn prores_4444_decodes_with_its_alpha() {
    let Some(mov) = make(
        "alpha.mov",
        "color=c=0x00FF00@0.75:s=32x16:r=30:d=0.1,format=yuva444p10le",
        &[
            "-c:v",
            "prores_ks",
            "-profile:v",
            "4444",
            "-pix_fmt",
            "yuva444p10le",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
        ],
    ) else {
        return;
    };
    let (format, pixel) = first_pixel(&mov);
    assert_eq!(format, PixelFormat::Rgba);
    assert!(pixel[0] < 8 && pixel[1] > 247 && pixel[2] < 8, "{pixel:?}");
    assert!((i32::from(pixel[3]) - 191).abs() <= 2, "{pixel:?}");
}
