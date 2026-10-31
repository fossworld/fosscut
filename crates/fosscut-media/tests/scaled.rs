//! Writing frames of another size tagged with their own colors (ADR-0032, item 2), as
//! proxies are made: the picture is scaled by area, keeps its matrix and tags, and a
//! full-range picture becomes limited range.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fosscut_media::ffmpeg::{FfmpegVideoDecoder, FfmpegWriter, WriterOptions, probe_output};
use fosscut_media::{
    ColorInfo, Container, DecoderOptions, HardwareDecoding, HardwareEncoding, Matrix,
    OutputSettings, PixelFormat, Plane, Primaries, Range, RgbaFrame, Transfer, VideoCodec,
    VideoDecoder, VideoFrame, VideoSettings,
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
    found
}

fn path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fosscut-scaled-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

const SD_601: ColorInfo = ColorInfo {
    matrix: Matrix::Bt601,
    range: Range::Limited,
    primaries: Primaries::Bt601Ntsc,
    transfer: Transfer::Bt709,
};

/// A 320x180 YUV 4:2:0 frame: luma `luma` in the left half and `luma / 2` in the right,
/// neutral chroma.
fn frame(n: i64, luma: u8, color: ColorInfo) -> VideoFrame {
    let (width, height) = (320usize, 180usize);
    let mut y = vec![luma; width * height];
    for row in y.chunks_mut(width) {
        for value in &mut row[width / 2..] {
            *value = luma / 2;
        }
    }
    let plane = |data: Vec<u8>, stride: usize, rows: usize| Plane {
        stride,
        rows,
        data: Arc::from(data),
    };
    VideoFrame {
        pts: Rate::FPS_30.frames_to_flicks(n),
        duration: Rate::FPS_30.frame_duration(),
        width: width as u32,
        height: height as u32,
        format: PixelFormat::Yuv420p,
        planes: vec![
            plane(y, width, height),
            plane(vec![128; width / 2 * height / 2], width / 2, height / 2),
            plane(vec![128; width / 2 * height / 2], width / 2, height / 2),
        ],
        color,
    }
}

fn proxy(container: Container, codec: VideoCodec) -> OutputSettings {
    OutputSettings {
        container,
        video: Some(VideoSettings {
            codec,
            width: 160,
            height: 90,
            rate: Rate::FPS_30,
            bit_rate: None,
        }),
        audio: None,
    }
}

fn options(color: ColorInfo) -> WriterOptions {
    WriterOptions {
        hardware: HardwareEncoding::Off,
        color: Some(color),
    }
}

/// Mean luma of the left and right halves of the frame at `time`, decoded in software.
fn halves(file: &Path, time: Flicks) -> (VideoFrame, f64, f64) {
    let mut decoder = FfmpegVideoDecoder::open(
        file,
        DecoderOptions {
            hardware: HardwareDecoding::Off,
        },
    )
    .unwrap();
    let frame = decoder.frame_at(time).unwrap();
    let (width, plane) = (frame.width as usize, &frame.planes[0]);
    let mean = |from: usize, to: usize| {
        let mut sum = 0.0;
        for y in 0..plane.rows {
            sum += plane.row(y)[from..to]
                .iter()
                .map(|&v| f64::from(v))
                .sum::<f64>();
        }
        sum / ((to - from) * plane.rows) as f64
    };
    // Away from the edge between the halves, which averaging blurs.
    let (left, right) = (mean(4, width / 2 - 4), mean(width / 2 + 4, width - 4));
    (frame, left, right)
}

#[test]
fn frames_of_another_size_are_scaled_and_keep_their_colors() {
    if !have_ffmpeg() {
        return;
    }
    let file = path("proxy-601.mov");
    let settings = proxy(Container::Mov, VideoCodec::ProResProxy);
    let mut writer = FfmpegWriter::create_with(&file, &settings, &options(SD_601)).unwrap();
    for n in 0..10 {
        writer.push_frame_scaled(&frame(n, 180, SD_601)).unwrap();
    }
    assert_eq!(writer.frames_written(), 10);
    writer.finish().unwrap();
    let report = probe_output(&file).unwrap();
    let video = report.video.unwrap();
    assert_eq!(
        (video.width, video.height, video.frame_count),
        (160, 90, 10)
    );
    assert_eq!(video.profile.as_deref(), Some("Proxy"));
    assert_eq!(video.color.matrix, Matrix::Bt601, "{:?}", video.color);
    assert_eq!(video.color.primaries, Primaries::Bt601Ntsc);

    let (decoded, left, right) = halves(&file, Rate::FPS_30.frames_to_flicks(4));
    assert_eq!((decoded.width, decoded.height), (160, 90));
    assert_eq!(decoded.color, SD_601, "decoded with the source's matrix");
    assert!((left - 180.0).abs() < 1.5, "left {left}");
    assert!((right - 90.0).abs() < 1.5, "right {right}");
}

#[test]
fn full_range_frames_become_limited_range() {
    if !have_ffmpeg() {
        return;
    }
    let file = path("proxy-full.mov");
    let limited = ColorInfo {
        matrix: Matrix::Bt709,
        range: Range::Limited,
        primaries: Primaries::Bt709,
        transfer: Transfer::Bt709,
    };
    let full = ColorInfo {
        range: Range::Full,
        ..limited
    };
    let settings = proxy(Container::Mov, VideoCodec::ProResProxy);
    let mut writer = FfmpegWriter::create_with(&file, &settings, &options(limited)).unwrap();
    for n in 0..3 {
        writer.push_frame_scaled(&frame(n, 254, full)).unwrap();
    }
    writer.finish().unwrap();
    let (decoded, left, right) = halves(&file, Flicks::ZERO);
    assert_eq!(decoded.color.range, Range::Limited);
    // Full-range 254 and 127 are limited-range 234 and 125.
    assert!((left - 234.1).abs() < 1.5, "left {left}");
    assert!((right - 125.1).abs() < 1.5, "right {right}");
}

#[test]
fn scaled_frames_and_other_tags_are_refused_where_they_cannot_work() {
    if !have_ffmpeg() {
        return;
    }
    let gif = proxy(Container::Gif, VideoCodec::Gif);
    let mut writer = FfmpegWriter::create(&path("scaled.gif"), &gif).unwrap();
    let error = writer
        .push_frame_scaled(&frame(0, 100, SD_601))
        .unwrap_err();
    assert_eq!(error.code(), "fosscut.media.unsupported");

    let settings = proxy(Container::Mov, VideoCodec::ProResProxy);
    let mut tagged =
        FfmpegWriter::create_with(&path("tagged.mov"), &settings, &options(SD_601)).unwrap();
    let rgba = RgbaFrame {
        width: 160,
        height: 90,
        rgba: vec![255; 160 * 90 * 4],
    };
    assert_eq!(
        tagged.push_rgba(&rgba).unwrap_err().code(),
        "fosscut.media.unsupported",
        "RGBA is converted as BT.709, so other tags would be wrong"
    );
    let mut plain = FfmpegWriter::create(&path("plain.mov"), &settings).unwrap();
    plain.push_rgba(&rgba).unwrap();
    plain.push_frame_scaled(&frame(1, 100, SD_601)).unwrap();
    plain.finish().unwrap();
    assert_eq!(
        probe_output(&path("plain.mov"))
            .unwrap()
            .video
            .unwrap()
            .frame_count,
        2
    );
}
