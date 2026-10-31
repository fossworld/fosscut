//! Recording through FFmpeg's input devices (CapCut parity CC-027). CI machines have no
//! microphones or cameras, so FFmpeg's `lavfi` input stands in for them: a 440 Hz `sine`
//! for a microphone and `testsrc2` for a camera, paced to real time with `realtime` and
//! `arealtime` where a test stops a recording by the clock. CI sets
//! `FOSSCUT_REQUIRE_FFMPEG=1`, which turns a build without `lavfi` into a failure instead
//! of a skip.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use fosscut_media::capture::{input_formats, list_devices, record};
use fosscut_media::ffmpeg::{FfmpegAudioDecoder, FfmpegVideoDecoder, encoder_capabilities};
use fosscut_media::{
    AudioCodec, AudioSettings, CaptureInput, Container, DecoderOptions, HardwareDecoding,
    MediaError, OutputReport, OutputSettings, Recording, VideoCodec, VideoDecoder, VideoSettings,
};
use fosscut_time::{FLICKS_PER_SECOND, Flicks, Rate};

const TONE: &str = "sine=frequency=440:sample_rate=48000";
const PICTURES: &str = "testsrc2=size=320x240:rate=30";

fn have_lavfi() -> bool {
    let found = input_formats().iter().any(|f| f == "lavfi");
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        found || !required,
        "FOSSCUT_REQUIRE_FFMPEG is set but the linked FFmpeg has no lavfi input"
    );
    if !found {
        eprintln!("skipping: the linked FFmpeg has no lavfi input");
    }
    found
}

fn dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-capture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    })
}

fn second() -> Flicks {
    Flicks::from_seconds(1).unwrap()
}

fn seconds(time: Flicks) -> f64 {
    time.get() as f64 / FLICKS_PER_SECOND as f64
}

fn partial(output: &Path) -> PathBuf {
    output.with_file_name(format!(
        ".{}.partial",
        output.file_name().unwrap().to_string_lossy()
    ))
}

/// A video codec this machine can put in MP4: hardware H.264 or HEVC where there is
/// one, otherwise AV1 or VP9 in software.
fn mp4_codec() -> VideoCodec {
    [
        VideoCodec::H264,
        VideoCodec::Hevc,
        VideoCodec::Av1,
        VideoCodec::Vp9,
    ]
    .into_iter()
    .find(|c| encoder_capabilities().video(*c).is_some())
    .expect("an MP4 video encoder")
}

fn video(width: u32, height: u32) -> VideoSettings {
    VideoSettings {
        codec: mp4_codec(),
        width,
        height,
        rate: Rate::FPS_30,
        bit_rate: None,
    }
}

fn recording(input: &str, output: &str, settings: OutputSettings) -> Recording {
    Recording {
        format: "lavfi".into(),
        input: input.into(),
        options: Vec::new(),
        microphone: None,
        output: dir().join(output),
        settings,
        duration: Some(second()),
    }
}

/// Records, checking that progress only grows and ends at what was recorded.
fn run(recording: &Recording) -> OutputReport {
    let stop = AtomicBool::new(false);
    let mut reports: Vec<Flicks> = Vec::new();
    let report = record(recording, &stop, &mut |t| reports.push(t)).unwrap();
    assert!(!reports.is_empty(), "progress is reported");
    assert!(reports.windows(2).all(|w| w[0] < w[1]), "{reports:?}");
    if let Some(duration) = recording.duration {
        assert!(*reports.last().unwrap() >= duration, "{reports:?}");
    }
    assert!(recording.output.exists());
    assert!(!partial(&recording.output).exists(), "no partial file");
    report
}

fn within(actual: Option<Flicks>, expected: Flicks, tolerance: Flicks, what: &str) {
    let actual = actual.unwrap_or_else(|| panic!("{what}: no duration"));
    let off = (actual - expected).get().abs();
    println!("{what}: {:.4} s", seconds(actual));
    assert!(
        off <= tolerance.get(),
        "{what}: {:.4} s, expected {:.4} s",
        seconds(actual),
        seconds(expected)
    );
}

/// The samples from `start` for `duration`, and the sign changes of their first channel:
/// twice the frequency per second.
fn zero_crossings(
    path: &Path,
    channels: u16,
    start: Flicks,
    duration: Flicks,
) -> (usize, Vec<f32>) {
    let mut decoder = FfmpegAudioDecoder::open(path, 48_000, channels).unwrap();
    let buffer = decoder.read(start, duration).unwrap();
    let first: Vec<f32> = buffer
        .samples
        .iter()
        .step_by(usize::from(channels))
        .copied()
        .collect();
    let crossings = first
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    (crossings, buffer.samples)
}

/// Checks that `path` holds lavfi's 440 Hz tone from `start` for `duration`: 880 sign
/// changes a second, give or take `slack`, at an eighth of full scale.
fn assert_tone(path: &Path, channels: u16, start: Flicks, duration: Flicks, slack: usize) {
    let (crossings, samples) = zero_crossings(path, channels, start, duration);
    let expected = (880.0 * seconds(duration)).round() as usize;
    println!(
        "{}: {crossings} zero crossings in {:.2} s from {:.2} s ({expected} for 440 Hz)",
        path.display(),
        seconds(duration),
        seconds(start)
    );
    assert!(
        crossings.abs_diff(expected) <= slack,
        "{crossings} zero crossings, {expected} for 440 Hz"
    );
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!((0.10..=0.15).contains(&peak), "peak {peak}");
}

fn mean_luma(frame: &fosscut_media::VideoFrame, x0: u32, x1: u32) -> f64 {
    let luma = &frame.planes[0];
    let mut sum = 0u64;
    let mut count = 0u64;
    for y in 0..frame.height as usize {
        for x in x0 as usize..x1 as usize {
            sum += u64::from(luma.data[y * luma.stride + x]);
            count += 1;
        }
    }
    sum as f64 / count as f64
}

#[test]
fn devices_are_listed_with_a_kind_name_format_and_input() {
    let started = Instant::now();
    let devices = list_devices().unwrap();
    println!(
        "{} devices in {:?}: {devices:#?}",
        devices.len(),
        started.elapsed()
    );
    let formats = input_formats();
    for device in &devices {
        assert!(!device.name.trim().is_empty(), "{device:?}");
        assert!(!device.input.is_empty(), "{device:?}");
        assert!(
            formats.contains(&device.format),
            "{device:?} in {formats:?}"
        );
        assert_eq!(device.capture_input().format, device.format);
    }
}

#[test]
fn a_microphone_records_a_tone_to_wav() {
    if !have_lavfi() {
        return;
    }
    let settings = OutputSettings {
        container: Container::Wav,
        video: None,
        audio: Some(AudioSettings {
            channels: 1,
            ..AudioSettings::stereo(AudioCodec::Pcm16)
        }),
    };
    let recording = recording(TONE, "voice.wav", settings);
    let report = run(&recording);
    assert!(Container::Wav.matches_probe(&report.container, None));
    assert!(report.video.is_none());
    let audio = report.audio.as_ref().unwrap();
    assert_eq!(
        (audio.codec.as_str(), audio.sample_rate, audio.channels),
        ("pcm_s16le", 48_000, 1)
    );
    // PCM is exact: 48,000 samples.
    within(audio.duration, second(), Flicks::ZERO, "WAV duration");
    // The tone starts at zero, so a second of it changes sign 879 times.
    assert_tone(&recording.output, 1, Flicks::ZERO, second(), 1);
}

#[test]
fn a_microphone_records_to_stereo_m4a() {
    if !have_lavfi() {
        return;
    }
    let settings = OutputSettings {
        container: Container::Mp4,
        video: None,
        audio: Some(AudioSettings::stereo(AudioCodec::Aac)),
    };
    let recording = recording(TONE, "voice.m4a", settings);
    let report = run(&recording);
    assert!(Container::Mp4.matches_probe(&report.container, report.brand.as_deref()));
    let audio = report.audio.as_ref().unwrap();
    assert_eq!(
        (audio.codec.as_str(), audio.sample_rate, audio.channels),
        ("aac", 48_000, 2)
    );
    within(
        audio.duration.or(report.duration),
        second(),
        Flicks::from_millis(10),
        "M4A duration",
    );
    // FfmpegAudioDecoder reads the first few milliseconds of AAC in MP4 as silence, so
    // the tone is counted away from the ends.
    let (tenth, middle) = (Flicks::from_millis(100), Flicks::from_millis(800));
    assert_tone(&recording.output, 2, tenth, middle, 2);
    // The mono microphone plays at full level on both sides.
    let (_, samples) = zero_crossings(&recording.output, 2, Flicks::ZERO, second());
    let difference = samples
        .as_chunks::<2>()
        .0
        .iter()
        .fold(0.0f32, |m, [left, right]| m.max((left - right).abs()));
    assert!(difference < 1e-3, "channels differ by {difference}");
}

#[test]
fn a_camera_records_test_pictures_to_mp4() {
    if !have_lavfi() {
        return;
    }
    let settings = OutputSettings {
        container: Container::Mp4,
        video: Some(video(320, 240)),
        audio: None,
    };
    let recording = recording(PICTURES, "camera.mp4", settings);
    let report = run(&recording);
    assert!(Container::Mp4.matches_probe(&report.container, report.brand.as_deref()));
    assert!(report.audio.is_none());
    let got = report.video.as_ref().unwrap();
    println!("camera: {} at {:?}", got.codec, got.rate);
    assert_eq!(got.codec, mp4_codec().probe_name());
    assert_eq!((got.width, got.height), (320, 240));
    assert_eq!(got.rate, Some(Rate::FPS_30));
    assert_eq!(got.frame_count, 30);
    within(
        got.duration.or(report.duration),
        second(),
        Rate::FPS_30.frame_duration(),
        "MP4 duration",
    );
    // The pictures came through: testsrc2 is colourful, not black.
    let mut decoder = FfmpegVideoDecoder::open(
        &recording.output,
        DecoderOptions {
            hardware: HardwareDecoding::Off,
        },
    )
    .unwrap();
    let frame = decoder.frame_at(Flicks::from_millis(500)).unwrap();
    let luma = mean_luma(&frame, 0, frame.width);
    assert!(luma > 40.0, "mean luma {luma}");
}

#[test]
fn pictures_of_another_shape_fit_between_black_bars() {
    if !have_lavfi() {
        return;
    }
    // 4:3 pictures in a 16:9 frame are 480 pixels wide, with 80-pixel bars at the sides.
    let settings = OutputSettings {
        container: Container::Mp4,
        video: Some(video(640, 360)),
        audio: None,
    };
    let mut recording = recording(PICTURES, "fitted.mp4", settings);
    recording.duration = Some(Flicks::from_millis(200));
    let report = run(&recording);
    let got = report.video.as_ref().unwrap();
    assert_eq!((got.width, got.height), (640, 360));
    assert_eq!(got.frame_count, 6);
    let mut decoder = FfmpegVideoDecoder::open(
        &recording.output,
        DecoderOptions {
            hardware: HardwareDecoding::Off,
        },
    )
    .unwrap();
    let frame = decoder.frame_at(Flicks::ZERO).unwrap();
    let (left, middle, right) = (
        mean_luma(&frame, 0, 72),
        mean_luma(&frame, 96, 544),
        mean_luma(&frame, 568, 640),
    );
    println!("luma: bars {left:.1} and {right:.1}, picture {middle:.1}");
    assert!(left < 20.0 && right < 20.0, "bars {left} {right}");
    assert!(middle > 40.0, "picture {middle}");
}

#[test]
fn one_input_records_a_camera_and_a_microphone_together() {
    if !have_lavfi() {
        return;
    }
    let settings = OutputSettings {
        container: Container::Mp4,
        video: Some(video(320, 240)),
        audio: Some(AudioSettings::stereo(AudioCodec::Aac)),
    };
    // One lavfi graph with a video and an audio output, like `avfoundation` with `0:1`.
    let graph = format!("{PICTURES} [out0]; {TONE} [out1]");
    let recording = recording(&graph, "together.mp4", settings);
    let report = run(&recording);
    check_camera_with_sound(&recording, &report, "one input");
}

#[test]
fn a_camera_and_a_separate_microphone_record_together() {
    if !have_lavfi() {
        return;
    }
    let settings = OutputSettings {
        container: Container::Mp4,
        video: Some(video(320, 240)),
        audio: Some(AudioSettings::stereo(AudioCodec::Aac)),
    };
    let mut recording = recording(PICTURES, "two-inputs.mp4", settings);
    recording.microphone = Some(CaptureInput {
        format: "lavfi".into(),
        input: TONE.into(),
        options: Vec::new(),
    });
    let report = run(&recording);
    check_camera_with_sound(&recording, &report, "two inputs");
}

fn check_camera_with_sound(recording: &Recording, report: &OutputReport, what: &str) {
    let got = report.video.as_ref().unwrap();
    assert_eq!((got.width, got.height), (320, 240), "{what}");
    assert_eq!(got.rate, Some(Rate::FPS_30), "{what}");
    assert_eq!(got.frame_count, 30, "{what}");
    let sound = report.audio.as_ref().unwrap();
    assert_eq!(
        (sound.codec.as_str(), sound.sample_rate, sound.channels),
        ("aac", 48_000, 2),
        "{what}"
    );
    within(
        sound.duration,
        second(),
        Flicks::from_millis(10),
        &format!("{what}: sound"),
    );
    // A separate input may start a few milliseconds late, padded with silence, so the
    // tone is counted over the second half.
    let half = Flicks::from_millis(500);
    assert_tone(&recording.output, 2, half, half, 2);
}

/// Records until another thread sets `stop` half a second after the recording reported
/// its first progress (opening encoders and devices comes first and takes longer on a busy
/// machine). Returns the report and how long `record` took to return after the stop.
/// How long a stopped recording could have run: from its first progress report to the stop
/// at least, and from the call to `record` to the stop at most (a source may deliver
/// frames while it starts, before the first report).
struct Ran {
    least: Duration,
    most: Duration,
}

/// Records until about half a second after the recording starts, then stops. Returns the
/// report, how long `record` took to return after the stop, and how long the recording
/// could have run: on a loaded machine the half-second sleep can run late, and a
/// real-time source records for as long as it actually ran.
fn stopped_after_half_a_second(recording: &Recording) -> (OutputReport, Duration, Ran) {
    let stop = AtomicBool::new(false);
    let started = std::sync::OnceLock::new();
    let mut stopped_at = None;
    let mut ran = None;
    let report = std::thread::scope(|scope| {
        let stopper = scope.spawn(|| {
            while started.get().is_none() {
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(500));
            stop.store(true, Ordering::Relaxed);
            Instant::now()
        });
        let called = Instant::now();
        let report = record(recording, &stop, &mut |_| {
            started.get_or_init(Instant::now);
        })
        .unwrap();
        let returned = Instant::now();
        let stopped = stopper.join().unwrap();
        stopped_at = Some(returned.duration_since(stopped));
        ran = started.get().map(|start| Ran {
            least: stopped.duration_since(*start),
            most: stopped.duration_since(called),
        });
        report
    });
    assert!(recording.output.exists());
    assert!(!partial(&recording.output).exists(), "no partial file");
    (report, stopped_at.unwrap(), ran.unwrap())
}

/// Checks a stopped recording's `duration` against how long it could have run. Nothing may
/// be recorded more than 100 ms past the stop. The lavfi stand-ins pace themselves to the
/// clock only while they get to run, and on a loaded machine they fall behind it (by 110
/// ms in a half second on CI), which a real device does not; so the recording must hold
/// at least half the time it ran, which still shows it recorded up to the stop.
fn ran_for(duration: Option<Flicks>, ran: &Ran, what: &str) {
    let actual = duration.unwrap_or_else(|| panic!("{what}: no duration"));
    let slack = Duration::from_millis(100);
    let (low, high) = (ran.least / 2, ran.most + slack);
    let seconds_of = |d: Duration| d.as_secs_f64();
    println!(
        "{what}: {:.4} s, ran {:.4} to {:.4} s",
        seconds(actual),
        seconds_of(ran.least),
        seconds_of(ran.most)
    );
    assert!(
        (seconds_of(low)..=seconds_of(high)).contains(&seconds(actual)),
        "{what}: {:.4} s, ran {:.4} to {:.4} s",
        seconds(actual),
        seconds_of(ran.least),
        seconds_of(ran.most)
    );
}

/// How soon `record` returns after a stop, closing and probing the file included.
const STOP_RETURNS_WITHIN: Duration = Duration::from_secs(2);

#[test]
fn stopping_ends_a_recording_without_a_duration() {
    if !have_lavfi() {
        return;
    }
    let settings = OutputSettings {
        container: Container::Wav,
        video: None,
        audio: Some(AudioSettings::stereo(AudioCodec::Pcm16)),
    };
    let mut voice = recording(&format!("{TONE},arealtime"), "stopped.wav", settings);
    voice.duration = None;
    let (report, latency, ran) = stopped_after_half_a_second(&voice);
    let audio = report.audio.as_ref().unwrap();
    println!("stopped sound: returned {latency:?} after the stop");
    ran_for(audio.duration, &ran, "stopped sound");
    // What is recorded ends at the stop (checked above). Returning also closes and probes
    // the file, which a loaded machine slows (0.49 s on CI), so the bound only catches a
    // recording that does not stop.
    assert!(latency < STOP_RETURNS_WITHIN, "{latency:?}");

    let settings = OutputSettings {
        container: Container::Mp4,
        video: Some(video(320, 240)),
        audio: Some(AudioSettings::stereo(AudioCodec::Aac)),
    };
    let graph = format!("{PICTURES},realtime [out0]; {TONE},arealtime [out1]");
    let mut camera = recording(&graph, "stopped.mp4", settings);
    camera.duration = None;
    let (report, latency, ran) = stopped_after_half_a_second(&camera);
    let got = report.video.as_ref().unwrap();
    println!(
        "stopped camera: {} frames, returned {latency:?} after the stop",
        got.frame_count
    );
    ran_for(got.duration.or(report.duration), &ran, "stopped pictures");
    assert!(latency < STOP_RETURNS_WITHIN, "{latency:?}");
    let sound = report.audio.as_ref().unwrap();
    // Sound and pictures end together, within a frame.
    within(
        sound.duration,
        Rate::FPS_30.frames_to_flicks(got.frame_count as i64),
        Rate::FPS_30.frame_duration(),
        "stopped camera sound",
    );
}

#[test]
fn stopping_before_anything_arrives_records_nothing() {
    if !have_lavfi() {
        return;
    }
    let settings = OutputSettings {
        container: Container::Wav,
        video: None,
        audio: Some(AudioSettings::stereo(AudioCodec::Pcm16)),
    };
    let mut voice = recording(TONE, "never.wav", settings);
    voice.duration = None;
    let stop = AtomicBool::new(true);
    let error = record(&voice, &stop, &mut |_| {}).unwrap_err();
    assert_eq!(error.code(), "fosscut.media.nothing_recorded", "{error}");
    assert!(!voice.output.exists());
    assert!(!partial(&voice.output).exists());
}

#[test]
fn unknown_formats_devices_and_options_are_typed_errors() {
    if !have_lavfi() {
        return;
    }
    let stop = AtomicBool::new(false);
    let audio_only = OutputSettings {
        container: Container::Wav,
        video: None,
        audio: Some(AudioSettings::stereo(AudioCodec::Pcm16)),
    };
    let expect = |recording: &Recording, code: &str| -> MediaError {
        let error = record(recording, &stop, &mut |_| {}).unwrap_err();
        assert_eq!(error.code(), code, "{error}");
        assert!(!recording.output.exists(), "{code}: no output");
        assert!(
            !partial(&recording.output).exists(),
            "{code}: no partial file"
        );
        error
    };

    let mut unknown = recording(TONE, "unknown.wav", audio_only.clone());
    unknown.format = "no-such-device".into();
    let error = expect(&unknown, "fosscut.media.input_format_unavailable");
    let MediaError::InputFormatUnavailable { available, .. } = error else {
        unreachable!()
    };
    assert!(available.iter().any(|f| f == "lavfi"), "{available:?}");

    let missing = recording("no_such_filter=1", "missing.wav", audio_only.clone());
    expect(&missing, "fosscut.media.device_unavailable");

    let mut option = recording(TONE, "option.wav", audio_only.clone());
    option.options = vec![("no_such_option".into(), "1".into())];
    let error = expect(&option, "fosscut.media.unsupported");
    assert!(error.to_string().contains("no_such_option"), "{error}");

    let silent = recording(PICTURES, "silent.wav", audio_only.clone());
    expect(&silent, "fosscut.media.no_audio_stream");

    let blind = recording(
        TONE,
        "blind.mp4",
        OutputSettings {
            container: Container::Mp4,
            video: Some(video(320, 240)),
            audio: None,
        },
    );
    expect(&blind, "fosscut.media.no_video_stream");

    let mut separate = recording(PICTURES, "deaf.mp4", audio_only.clone());
    separate.settings = OutputSettings {
        container: Container::Mp4,
        video: Some(video(320, 240)),
        audio: Some(AudioSettings::stereo(AudioCodec::Aac)),
    };
    separate.microphone = Some(CaptureInput {
        format: "lavfi".into(),
        input: PICTURES.into(),
        options: Vec::new(),
    });
    expect(&separate, "fosscut.media.no_audio_stream");

    // Impossible settings fail before any device is opened.
    let odd = recording(
        PICTURES,
        "odd.mp4",
        OutputSettings {
            container: Container::Mp4,
            video: Some(video(321, 240)),
            audio: None,
        },
    );
    expect(&odd, "fosscut.media.unsupported");

    let mut endless = recording(TONE, "zero.wav", audio_only);
    endless.duration = Some(Flicks::ZERO);
    expect(&endless, "fosscut.media.unsupported");
}
