//! Leaving hardware encoders out (ADR-0032, item 6): a codec is then written by a software
//! encoder or is unavailable, and never replaced by another codec. The choice belongs to
//! the process, so this is the only test in its binary.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

use fosscut_media::ffmpeg::{
    FfmpegWriter, WriterOptions, encoder_capabilities, hardware_encoding, set_hardware_encoding,
    video_encoder, video_encoder_for,
};
use fosscut_media::{
    Container, HardwareEncoding, MediaError, OutputSettings, VideoCodec, VideoSettings,
};
use fosscut_time::Rate;

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

fn h264() -> OutputSettings {
    OutputSettings {
        container: Container::Mp4,
        video: Some(VideoSettings {
            codec: VideoCodec::H264,
            width: 320,
            height: 180,
            rate: Rate::FPS_30,
            bit_rate: None,
        }),
        audio: None,
    }
}

#[test]
fn hardware_encoders_can_be_left_out_and_codecs_are_never_replaced() {
    if !have_ffmpeg() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("fosscut-hw-encoding-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(hardware_encoding(), HardwareEncoding::Auto, "the default");
    let with_hardware = video_encoder(VideoCodec::H264).cloned();
    let prores = video_encoder(VideoCodec::ProRes422).cloned();
    assert!(prores.is_some(), "every supported build has prores_ks");

    set_hardware_encoding(HardwareEncoding::Off);
    assert_eq!(hardware_encoding(), HardwareEncoding::Off);
    let software = video_encoder(VideoCodec::H264);
    assert_eq!(
        software,
        video_encoder_for(VideoCodec::H264, HardwareEncoding::Off)
    );
    assert!(software.is_none_or(|e| !e.hardware), "{software:?}");
    for (codec, info) in &encoder_capabilities().video {
        assert!(!info.hardware, "{codec:?} uses {}", info.encoder);
    }
    assert_eq!(
        video_encoder(VideoCodec::ProRes422).cloned(),
        prores,
        "software codecs stay"
    );
    let path = dir.join("software.mp4");
    match software {
        None => {
            // H.264 is only hardware encoded here: it becomes unavailable.
            let error = FfmpegWriter::create(&path, &h264()).err().unwrap();
            assert_eq!(error.code(), "fosscut.media.encoder_unavailable");
            let MediaError::EncoderUnavailable { codec, available } = error else {
                unreachable!()
            };
            assert_eq!(codec, "h264");
            assert!(!available.contains(&"h264".to_string()), "{available:?}");
            assert!(available.contains(&"prores".to_string()), "{available:?}");
            assert!(!path.exists());
        }
        Some(_) => {
            FfmpegWriter::create(&path, &h264()).unwrap();
        }
    }

    // An explicit choice holds whatever the process's is.
    if let Some(hardware) = with_hardware.as_ref().filter(|e| e.hardware) {
        assert_eq!(
            video_encoder_for(VideoCodec::H264, HardwareEncoding::Auto),
            Some(hardware)
        );
        let explicit = FfmpegWriter::create_with(
            &dir.join("hardware.mp4"),
            &h264(),
            &WriterOptions {
                hardware: HardwareEncoding::Auto,
                color: None,
            },
        );
        assert!(explicit.is_ok(), "{:?}", explicit.err());
    }
    set_hardware_encoding(HardwareEncoding::Auto);
    assert_eq!(video_encoder(VideoCodec::H264).cloned(), with_hardware);
    std::fs::remove_dir_all(&dir).unwrap();
}
