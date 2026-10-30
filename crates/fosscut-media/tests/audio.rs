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

/// A file made with `ffmpeg -y -v error <args> <name>`, created once per test run.
fn generated(name: &str, args: &[&str]) -> PathBuf {
    static CREATING: Mutex<()> = Mutex::new(());
    let _guard = CREATING.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir().join(name);
    if !path.exists() {
        let partial = dir().join(format!("partial-{name}"));
        let status = Command::new("ffmpeg")
            .args(["-y", "-v", "error"])
            .args(args)
            .arg(&partial)
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg could not create {name}");
        std::fs::rename(&partial, &path).unwrap();
    }
    path
}

/// A 2 s tone: `0.5 sin(2π 440 t)` on the left, `0.25 sin(2π 660 t)` on the right.
fn tone(name: &str, rate: u32, extra: &[&str]) -> PathBuf {
    let source = format!("aevalsrc=0.5*sin(2*PI*440*t)|0.25*sin(2*PI*660*t):s={rate}:d=2");
    let mut args = vec!["-f", "lavfi", "-i", &source];
    args.extend_from_slice(extra);
    generated(name, &args)
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

#[test]
fn mono_sources_play_at_full_level_on_both_channels() {
    if !have_ffmpeg() {
        return;
    }
    let path = dir().join("mono.wav");
    if !path.exists() {
        let status = Command::new("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.5*sin(2*PI*440*t):s=48000:d=1",
            ])
            .args(["-c:a", "pcm_s16le"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let mut decoder = FfmpegAudioDecoder::open(&path, 48_000, 2).unwrap();
    let buffer = decoder.read(seconds(0.25), seconds(0.25)).unwrap();
    for i in (0..buffer.frames()).step_by(97) {
        let want = expected(0, 0.25 + i as f64 / 48_000.0);
        assert!((buffer.samples[i * 2] - want).abs() < 0.01, "left at {i}");
        assert!(
            (buffer.samples[i * 2 + 1] - want).abs() < 0.01,
            "right at {i}"
        );
    }
}

/// Whether this FFmpeg build has the encoder `name`.
fn has_encoder(name: &str) -> bool {
    static LIST: OnceLock<String> = OnceLock::new();
    let list = LIST.get_or_init(|| {
        let output = Command::new("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    });
    list.lines()
        .any(|line| line.split_whitespace().nth(1) == Some(name))
}

/// Where the features of [`markers`] sit, in samples at 48 kHz.
const BURST_A: usize = 0;
const CLICK_B: usize = 4_800;
const BURST_C: usize = 24_000;
const CLICK_D: usize = 27_840;

/// `samples` at 48 kHz as a time.
fn at_48k(samples: usize) -> Flicks {
    Flicks::new(samples as i64 * 14_700)
}

/// A 1 s mono 48 kHz PCM signal with features on known samples: a 1 kHz cosine burst of
/// amplitude 0.5 on samples 0 to 2399 (at full level from the very first sample), a click
/// on sample 4800 (0.100 s), another burst on samples 24000 (0.500 s) to 26399, and a
/// click on sample 27840 (0.580 s).
fn markers() -> PathBuf {
    let source = format!(
        "aevalsrc=0.5*cos(2*PI*1000*t)*(lt(n\\,{a_end})+between(n\\,{BURST_C}\\,{c_end}))\
         +0.9*(eq(n\\,{CLICK_B})+eq(n\\,{CLICK_D})):s=48000:d=1",
        a_end = BURST_A + 2_400,
        c_end = BURST_C + 2_399,
    );
    generated(
        "markers.wav",
        &["-f", "lavfi", "-i", &source, "-c:a", "pcm_s16le"],
    )
}

/// The first sample from `from` louder than 40% of a burst's level.
fn onset(samples: &[f32], from: usize) -> usize {
    from + samples[from..].iter().position(|s| s.abs() > 0.2).unwrap()
}

/// The loudest sample within `radius` of `around`.
fn peak(samples: &[f32], around: usize, radius: usize) -> usize {
    let range = around - radius..around + radius;
    let loudest = samples[range.clone()]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0;
    range.start + loudest
}

/// Where the features of [`markers`] land in `path`, in samples from the start of the file,
/// measured by three reads in turn: one starting right on the second burst (the second
/// burst and click), one from the start (the first burst and click), and one starting
/// 10 ms before the second burst. Every read seeks: into the middle of the stream first,
/// back to the start, then into the middle again.
fn feature_positions(path: &Path) -> [i64; 6] {
    let mut decoder = FfmpegAudioDecoder::open(path, 48_000, 1).unwrap();
    let mut positions = Vec::new();
    for (from, frames, [burst, click]) in [
        (BURST_C, 4_800, [BURST_C, CLICK_D]),
        (0, 9_600, [BURST_A, CLICK_B]),
        (BURST_C - 480, 5_760, [BURST_C, CLICK_D]),
    ] {
        let buffer = decoder.read(at_48k(from), at_48k(frames)).unwrap();
        let burst = onset(&buffer.samples, (burst - from).saturating_sub(400));
        let click = peak(&buffer.samples, click - from, 700);
        positions.extend([from + burst, from + click].map(|p| p as i64));
    }
    positions.try_into().unwrap()
}

#[test]
fn lossy_sources_line_up_with_the_lossless_reference() {
    if !have_ffmpeg() {
        return;
    }
    let reference = feature_positions(&markers());
    let exact = [BURST_C, CLICK_D, BURST_A, CLICK_B, BURST_C, CLICK_D].map(|p| p as i64);
    assert_eq!(reference, exact, "PCM puts every feature on its own sample");
    let input = markers().to_string_lossy().into_owned();
    // Tolerances in samples for the read from the start and for the reads that start in
    // mid-stream. Lossy codecs smear an edge over a few samples, while a first frame
    // decoded without the frames before it comes out silent for hundreds. Matroska keeps
    // times in milliseconds, so in mid-stream WebM audio is placed only to the nearest half
    // millisecond (24 samples): libopus's 312-sample pre-skip puts every packet half a
    // millisecond off a tick.
    let cases: [(&str, &str, &[&str], i64, i64); 7] = [
        ("flac", "markers.flac", &["-c:a", "flac"], 0, 0),
        ("aac", "markers.m4a", &["-c:a", "aac", "-b:a", "128k"], 8, 8),
        ("aac", "markers.mp4", &["-c:a", "aac", "-b:a", "128k"], 8, 8),
        ("aac", "markers.mov", &["-c:a", "aac", "-b:a", "128k"], 8, 8),
        (
            "libmp3lame",
            "markers.mp3",
            &["-c:a", "libmp3lame", "-b:a", "128k"],
            8,
            8,
        ),
        (
            "libopus",
            "markers.ogg",
            &["-c:a", "libopus", "-b:a", "96k"],
            8,
            8,
        ),
        (
            "libopus",
            "markers.webm",
            &["-c:a", "libopus", "-b:a", "96k"],
            8,
            24 + 8,
        ),
    ];
    let mut failures = Vec::new();
    for (encoder, name, codec, from_start, mid_stream) in cases {
        if !has_encoder(encoder) {
            eprintln!("{name}: skipped, this FFmpeg has no {encoder} encoder");
            continue;
        }
        let mut args = vec!["-i", input.as_str()];
        args.extend_from_slice(codec);
        let found = feature_positions(&generated(name, &args));
        let errors: Vec<i64> = found.iter().zip(&exact).map(|(f, r)| f - r).collect();
        eprintln!("{name}: errors in samples {errors:?}");
        let start_ok = errors[2..4].iter().all(|e| e.abs() <= from_start);
        let middle_ok = (errors[..2].iter())
            .chain(&errors[4..])
            .all(|e| e.abs() <= mid_stream);
        if !(start_ok && middle_ok) {
            failures.push(format!("{name}: {errors:?}"));
        }
    }
    assert!(failures.is_empty(), "misplaced audio: {failures:#?}");
}
