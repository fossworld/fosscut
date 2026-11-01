//! Writing files with every encoder this machine has, then probing and decoding them back:
//! exact frame counts and timestamps, color tags, RGBA conversion, and typed errors when a
//! codec is unavailable or settings are impossible (ADR-0009, items 5 and 6).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use fosscut_media::ffmpeg::{FfmpegVideoDecoder, FfmpegWriter, encoder_capabilities, probe_output};
use fosscut_media::{
    AudioBuffer, AudioCodec, AudioSettings, ColorInfo, Container, DecoderOptions, HardwareDecoding,
    Matrix, OutputSettings, PixelFormat, Plane, Range, RgbaFrame, Transfer, VideoCodec,
    VideoDecoder, VideoFrame, VideoSettings,
};
use fosscut_time::{Flicks, Rate};

const SIZE: (u32, u32) = (320, 180);
const FRAMES: i64 = 30;

fn dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-encode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    })
}

/// An NV12 frame whose luma is `16 + 6 n`, so frame `n` can be recognised after lossy
/// encoding.
fn frame(n: i64) -> VideoFrame {
    let (w, h) = (SIZE.0 as usize, SIZE.1 as usize);
    let luma = vec![(16 + 6 * n) as u8; w * h];
    let chroma = vec![128u8; w * (h / 2)];
    VideoFrame {
        pts: Rate::FPS_30.frames_to_flicks(n),
        duration: Rate::FPS_30.frame_duration(),
        width: SIZE.0,
        height: SIZE.1,
        format: PixelFormat::Nv12,
        planes: vec![
            Plane {
                stride: w,
                rows: h,
                data: Arc::from(luma),
            },
            Plane {
                stride: w,
                rows: h / 2,
                data: Arc::from(chroma),
            },
        ],
        color: ColorInfo {
            matrix: Matrix::Bt709,
            range: Range::Limited,
            primaries: fosscut_media::Primaries::Bt709,
            transfer: Transfer::Bt709,
        },
    }
}

fn tone(seconds: f64) -> AudioBuffer {
    let frames = (48_000.0 * seconds) as usize;
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let v = 0.3 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin();
        samples.extend([v, v]);
    }
    AudioBuffer {
        sample_rate: 48_000,
        channels: 2,
        samples,
    }
}

/// The container and audio codec each video codec is tested in (GIF has no sound).
fn layout(codec: VideoCodec) -> (Container, Option<AudioCodec>) {
    match codec {
        VideoCodec::Vp9 => (Container::Webm, Some(AudioCodec::Opus)),
        VideoCodec::Gif => (Container::Gif, None),
        codec if codec.probe_name() == "prores" => (Container::Mov, Some(AudioCodec::Aac)),
        _ => (Container::Mp4, Some(AudioCodec::Aac)),
    }
}

fn settings(codec: VideoCodec, rate: Rate) -> OutputSettings {
    let (container, audio) = layout(codec);
    let audio = audio.filter(|a| encoder_capabilities().audio(*a).is_some());
    OutputSettings {
        container,
        video: Some(VideoSettings {
            codec,
            width: SIZE.0,
            height: SIZE.1,
            rate,
            bit_rate: None,
        }),
        audio: audio.map(AudioSettings::stereo),
    }
}

fn write(path: &Path, settings: &OutputSettings, frames: i64) {
    let mut writer = FfmpegWriter::create(path, settings).unwrap();
    for n in 0..frames {
        writer.push_frame(&frame(n)).unwrap();
        if settings.audio.is_some() {
            // Audio in uneven chunks, as a mixer would deliver it.
            writer
                .push_audio(&tone(1.0 / 30.0 + if n % 2 == 0 { 0.003 } else { -0.003 }))
                .unwrap();
        }
    }
    assert_eq!(writer.frames_written(), frames as u64);
    writer.finish().unwrap();
}

#[test]
fn capabilities_list_only_usable_permissive_encoders() {
    let capabilities = encoder_capabilities();
    assert!(
        !capabilities.video.is_empty(),
        "every supported FFmpeg build can encode ProRes or AV1"
    );
    assert!(capabilities.audio(AudioCodec::Aac).is_some());
    for (_, info) in capabilities.video.iter() {
        assert!(
            !info.encoder.contains("x264") && !info.encoder.contains("x265"),
            "GPL encoders are never used: {}",
            info.encoder
        );
    }
    let h264 = capabilities.video(VideoCodec::H264);
    assert!(h264.is_none_or(|e| e.encoder != "libx264"));
}

#[test]
fn every_available_codec_writes_exact_frames_tags_and_audio() {
    let capabilities = encoder_capabilities();
    // GIF stores centisecond delays and no color tags; tests/formats.rs covers it.
    for (codec, info) in capabilities
        .video
        .iter()
        .filter(|(c, _)| *c != VideoCodec::Gif)
    {
        let settings = settings(*codec, Rate::FPS_30);
        let path = dir().join(format!(
            "all-{}.{}",
            codec.probe_name(),
            settings.container.extension()
        ));
        write(&path, &settings, FRAMES);
        let report = probe_output(&path).unwrap();
        let video = report.video.as_ref().unwrap();
        let what = format!("{} with {}", codec.probe_name(), info.encoder);
        assert_eq!(video.codec, codec.probe_name(), "{what}: codec");
        if let Some(profile) = codec.profile_name() {
            assert_eq!(video.profile.as_deref(), Some(profile), "{what}: profile");
        }
        assert_eq!((video.width, video.height), SIZE, "{what}: size");
        assert_eq!(video.rate, Some(Rate::FPS_30), "{what}: rate");
        assert_eq!(video.frame_count, FRAMES as u64, "{what}: frame count");
        assert_eq!(video.color.matrix, Matrix::Bt709, "{what}: matrix tag");
        assert_eq!(video.color.range, Range::Limited, "{what}: range tag");
        assert_eq!(
            video.color.transfer,
            Transfer::Bt709,
            "{what}: transfer tag"
        );
        if let Some(duration) = video.duration {
            assert_eq!(
                duration,
                Flicks::from_seconds(1).unwrap(),
                "{what}: duration"
            );
        }
        assert!(report.size_bytes > 0);
        if settings.audio.is_some() {
            let audio = report.audio.as_ref().unwrap();
            assert_eq!(
                Some(audio.codec.as_str()),
                layout(*codec).1.map(AudioCodec::probe_name),
                "{what}: audio codec"
            );
            assert_eq!(
                (audio.sample_rate, audio.channels),
                (48_000, 2),
                "{what}: audio format"
            );
        }

        // Decode frame 17 back: the right frame, in the right place.
        let mut decoder = FfmpegVideoDecoder::open(
            &path,
            DecoderOptions {
                hardware: HardwareDecoding::Off,
            },
        )
        .unwrap();
        let back = decoder.frame_at(Rate::FPS_30.frames_to_flicks(17)).unwrap();
        let number = ((back.mean_luma() - 16.0) / 6.0).round() as i64;
        assert_eq!(number, 17, "{what}: decoded frame 17 shows frame {number}");
    }
}

#[test]
fn ntsc_rates_are_stamped_exactly() {
    let capabilities = encoder_capabilities();
    let codec = [VideoCodec::ProRes422, VideoCodec::Av1, VideoCodec::H264]
        .into_iter()
        .find(|c| capabilities.video(*c).is_some())
        .expect("ProRes, AV1 or H.264 is available");
    let rate = Rate::NTSC_30;
    let mut settings = settings(codec, rate);
    settings.audio = None;
    let path = dir().join(format!("ntsc.{}", settings.container.extension()));
    write(&path, &settings, FRAMES);
    let video = probe_output(&path).unwrap().video.unwrap();
    assert_eq!(video.rate, Some(rate));
    assert_eq!(video.frame_count, FRAMES as u64);
    if let Some(duration) = video.duration {
        assert_eq!(
            duration,
            rate.frames_to_flicks(FRAMES),
            "30 frames at 30000/1001"
        );
    }
}

#[test]
fn rgba_frames_become_limited_range_bt709() {
    let capabilities = encoder_capabilities();
    let codec = [VideoCodec::ProRes422, VideoCodec::Av1, VideoCodec::H264]
        .into_iter()
        .find(|c| capabilities.video(*c).is_some())
        .unwrap();
    let mut settings = settings(codec, Rate::FPS_30);
    settings.audio = None;
    let path = dir().join(format!("rgba.{}", settings.container.extension()));
    let mut writer = FfmpegWriter::create(&path, &settings).unwrap();
    // 75% red in non-linear BT.709 R'G'B'.
    let red = RgbaFrame {
        width: SIZE.0,
        height: SIZE.1,
        rgba: [191u8, 0, 0, 255].repeat((SIZE.0 * SIZE.1) as usize),
    };
    for _ in 0..5 {
        writer.push_rgba(&red).unwrap();
    }
    writer.finish().unwrap();
    let mut decoder = FfmpegVideoDecoder::open(
        &path,
        DecoderOptions {
            hardware: HardwareDecoding::Off,
        },
    )
    .unwrap();
    let back = decoder.next_frame().unwrap().unwrap();
    // SMPTE 75% red in BT.709 limited range: Y 51, Cb 109, Cr 212.
    let luma = back.mean_luma();
    assert!((luma - 51.0).abs() < 3.0, "luma {luma}");
    let chroma = &back.planes[1];
    let (cb, cr) = match back.format {
        PixelFormat::Nv12 => (chroma.data[10], chroma.data[11]),
        PixelFormat::Yuv420p => (chroma.data[10], back.planes[2].data[10]),
        PixelFormat::Rgba => panic!("an opaque H.264 frame decodes as Y'CbCr"),
    };
    assert!(
        cb.abs_diff(109) <= 3 && cr.abs_diff(212) <= 3,
        "Cb {cb}, Cr {cr}"
    );
}

#[test]
fn impossible_requests_are_typed_errors() {
    let capabilities = encoder_capabilities();
    let codec = capabilities.video[0].0;
    let base = settings(codec, Rate::FPS_30);

    let mut wrong_container = base.clone();
    wrong_container.container = if codec == VideoCodec::Vp9 {
        Container::Mov
    } else {
        Container::Webm
    };
    wrong_container.audio = Some(AudioSettings::stereo(AudioCodec::Aac));
    let error = FfmpegWriter::create(&dir().join("x.webm"), &wrong_container)
        .err()
        .unwrap();
    assert_eq!(error.code(), "fosscut.media.unsupported");

    let mut odd = base.clone();
    if let Some(video) = &mut odd.video {
        video.width = 321;
    }
    let error = FfmpegWriter::create(&dir().join("odd.mp4"), &odd)
        .err()
        .unwrap();
    assert_eq!(error.code(), "fosscut.media.unsupported");

    let empty = OutputSettings {
        container: Container::Mp4,
        video: None,
        audio: None,
    };
    let error = FfmpegWriter::create(&dir().join("empty.mp4"), &empty)
        .err()
        .unwrap();
    assert_eq!(error.code(), "fosscut.media.unsupported");

    let error = FfmpegWriter::create(Path::new("/no/such/dir/out.mov"), &base)
        .err()
        .unwrap();
    assert_eq!(error.code(), "fosscut.media.write_failed");

    // Every codec this machine cannot encode is refused with the alternatives, never
    // substituted (the round 1 regression: H.264 requested, HEVC delivered).
    for missing in VideoCodec::ALL
        .into_iter()
        .filter(|c| capabilities.video(*c).is_none())
    {
        let mut request = base.clone();
        if let Some(video) = &mut request.video {
            video.codec = missing;
        }
        request.container = layout(missing).0;
        request.audio = None;
        let error = FfmpegWriter::create(&dir().join("missing.mov"), &request)
            .err()
            .unwrap();
        assert_eq!(
            error.code(),
            "fosscut.media.encoder_unavailable",
            "{missing:?}"
        );
        assert!(error.to_string().contains(codec.probe_name()), "{error}");
    }

    let mut writer = FfmpegWriter::create(
        &dir().join(format!("misuse.{}", base.container.extension())),
        &base,
    )
    .unwrap();
    let mut small = frame(0);
    small.width = 64;
    assert_eq!(
        writer.push_frame(&small).unwrap_err().code(),
        "fosscut.media.unsupported"
    );
    if base.audio.is_some() {
        let mono = AudioBuffer::silence(44_100, 1, 100);
        assert_eq!(
            writer.push_audio(&mono).unwrap_err().code(),
            "fosscut.media.unsupported"
        );
    }
    writer.push_frame(&frame(0)).unwrap();
    writer.finish().unwrap();

    let mut silent = base.clone();
    silent.audio = None;
    let mut writer = FfmpegWriter::create(
        &dir().join(format!("silent.{}", base.container.extension())),
        &silent,
    )
    .unwrap();
    assert_eq!(
        writer.push_audio(&tone(0.1)).unwrap_err().code(),
        "fosscut.media.unsupported"
    );
    writer.push_frame(&frame(0)).unwrap();
    writer.finish().unwrap();
}

#[test]
fn audio_only_files_and_missing_files_are_reported() {
    let path = dir().join("tone.m4a");
    let settings = OutputSettings {
        container: Container::Mp4,
        video: None,
        audio: Some(AudioSettings::stereo(AudioCodec::Aac)),
    };
    let mut writer = FfmpegWriter::create(&path, &settings).unwrap();
    assert_eq!(
        writer.push_frame(&frame(0)).unwrap_err().code(),
        "fosscut.media.unsupported"
    );
    writer.push_audio(&tone(0.5)).unwrap();
    writer.finish().unwrap();
    let report = probe_output(&path).unwrap();
    assert!(report.video.is_none());
    let audio = report.audio.unwrap();
    assert_eq!((audio.codec.as_str(), audio.channels), ("aac", 2));
    assert!(report.duration.is_some());
    assert_eq!(
        probe_output(Path::new("/no/such/file.mp4"))
            .unwrap_err()
            .code(),
        "fosscut.media.open_failed"
    );
}
