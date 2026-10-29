//! Audio decoding against tones whose every sample is known: exact placement after
//! resampling, channel conversion, silence outside the stream, and errors (ADR-0009).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use fosscut_media::ffmpeg::FfmpegAudioDecoder;
use fosscut_time::Flicks;

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
        let dir = std::env::temp_dir().join(format!("fosscut-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    })
}

/// A 2 s tone: `0.5 sin(2π 440 t)` on the left, `0.25 sin(2π 660 t)` on the right.
fn tone(name: &str, rate: u32, extra: &[&str]) -> PathBuf {
    static CREATING: Mutex<()> = Mutex::new(());
    let _guard = CREATING.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir().join(name);
    if !path.exists() {
        let partial = dir().join(format!("partial-{name}"));
        let source = format!("aevalsrc=0.5*sin(2*PI*440*t)|0.25*sin(2*PI*660*t):s={rate}:d=2");
        let status = Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", &source])
            .args(extra)
            .arg(&partial)
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg could not create {name}");
        std::fs::rename(&partial, &path).unwrap();
    }
    path
}

fn seconds(s: f64) -> Flicks {
    Flicks::from_seconds_f64(s).unwrap()
}

fn expected(channel: usize, t: f64) -> f32 {
    if channel == 0 {
        (0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as f32
    } else {
        (0.25 * (2.0 * std::f64::consts::PI * 660.0 * t).sin()) as f32
    }
}

#[test]
fn lossless_audio_lands_on_exact_samples_after_resampling() {
    if !have_ffmpeg() {
        return;
    }
    let wav = tone("tone-44k.wav", 44_100, &["-c:a", "pcm_s16le"]);
    let mut decoder = FfmpegAudioDecoder::open(&wav, 48_000, 2).unwrap();
    assert_eq!(decoder.codec(), "pcm_s16le");
    assert!(decoder.path().ends_with("tone-44k.wav"));
    let start = 0.75;
    let buffer = decoder.read(seconds(start), seconds(0.25)).unwrap();
    assert_eq!((buffer.sample_rate, buffer.channels), (48_000, 2));
    assert_eq!(buffer.frames(), 12_000, "a quarter second at 48 kHz");
    let mut worst = 0.0_f32;
    for i in 0..buffer.frames() {
        let t = start + i as f64 / 48_000.0;
        for c in 0..2 {
            worst = worst.max((buffer.samples[i * 2 + c] - expected(c, t)).abs());
        }
    }
    // 16-bit quantization plus resampling: far below one percent of full scale, and a
    // one-sample misplacement would show up as about 0.03 at 440 Hz.
    assert!(worst < 0.01, "largest error {worst}");
}

#[test]
fn reads_are_repeatable_in_any_order() {
    if !have_ffmpeg() {
        return;
    }
    let wav = tone("tone-48k.wav", 48_000, &["-c:a", "pcm_s16le"]);
    let mut decoder = FfmpegAudioDecoder::open(&wav, 48_000, 2).unwrap();
    let later = decoder.read(seconds(1.5), seconds(0.1)).unwrap();
    let earlier = decoder.read(seconds(0.2), seconds(0.1)).unwrap();
    let again = decoder.read(seconds(1.5), seconds(0.1)).unwrap();
    assert_eq!(later, again);
    assert!((earlier.samples[0] - expected(0, 0.2)).abs() < 0.01);
}

#[test]
fn mono_output_and_lossy_sources_work() {
    if !have_ffmpeg() {
        return;
    }
    let aac = tone("tone.m4a", 48_000, &["-c:a", "aac", "-b:a", "192k"]);
    let mut stereo = FfmpegAudioDecoder::open(&aac, 48_000, 2).unwrap();
    assert_eq!(stereo.codec(), "aac");
    let buffer = stereo.read(seconds(0.5), seconds(0.5)).unwrap();
    assert_eq!(buffer.frames(), 24_000);
    let rms = |c: usize| {
        let sum: f64 = (0..buffer.frames())
            .map(|i| f64::from(buffer.samples[i * 2 + c]).powi(2))
            .sum();
        (sum / buffer.frames() as f64).sqrt()
    };
    // A sine of amplitude a has an RMS of a / sqrt(2).
    assert!((rms(0) - 0.5 / 2f64.sqrt()).abs() < 0.01, "left {}", rms(0));
    assert!(
        (rms(1) - 0.25 / 2f64.sqrt()).abs() < 0.01,
        "right {}",
        rms(1)
    );

    let mut mono = FfmpegAudioDecoder::open(&aac, 24_000, 1).unwrap();
    let buffer = mono.read(seconds(0.5), seconds(0.5)).unwrap();
    assert_eq!((buffer.channels, buffer.frames()), (1, 12_000));
}

#[test]
fn time_outside_the_stream_is_silence() {
    if !have_ffmpeg() {
        return;
    }
    let wav = tone("tone-48k.wav", 48_000, &["-c:a", "pcm_s16le"]);
    let mut decoder = FfmpegAudioDecoder::open(&wav, 48_000, 2).unwrap();
    let tail = decoder.read(seconds(1.9), seconds(0.3)).unwrap();
    assert_eq!(tail.frames(), 14_400);
    let audible = tail.samples[..2 * 4_000].iter().any(|s| s.abs() > 0.1);
    let after_end = tail.samples[2 * 6_000..].iter().all(|s| *s == 0.0);
    assert!(audible && after_end, "sound until 2.0 s, silence after");
    let before = decoder.read(seconds(-0.1), seconds(0.2)).unwrap();
    assert_eq!(before.frames(), 9_600);
    assert!(
        before.samples[..2 * 4_000].iter().all(|s| *s == 0.0),
        "silence before 0"
    );
    assert!((before.samples[2 * 4_800 + 2 * 100] - expected(0, 100.0 / 48_000.0)).abs() < 0.01);
    assert_eq!(
        decoder.read(seconds(1.0), Flicks::ZERO).unwrap().frames(),
        0
    );
}

#[test]
fn files_without_audio_and_bad_settings_are_typed_errors() {
    if !have_ffmpeg() {
        return;
    }
    let video = dir().join("silent.mp4");
    if !video.exists() {
        let status = Command::new("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:d=1",
            ])
            .args(["-c:v", "mpeg4"])
            .arg(&video)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let error = FfmpegAudioDecoder::open(&video, 48_000, 2).err().unwrap();
    assert_eq!(error.code(), "fosscut.media.no_audio_stream");
    let wav = tone("tone-48k.wav", 48_000, &["-c:a", "pcm_s16le"]);
    let error = FfmpegAudioDecoder::open(&wav, 48_000, 6).err().unwrap();
    assert_eq!(error.code(), "fosscut.media.unsupported");
}
