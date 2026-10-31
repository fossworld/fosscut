//! Recording from microphones, cameras and screens into project files (CapCut parity
//! CC-027): device choice and planning over fake device lists, file names in the bundle,
//! and real recordings through FFmpeg's `lavfi` input standing in for hardware (a 440 Hz
//! `sine` for a microphone, `testsrc2` for a camera), in the foreground and as background
//! jobs that stop and keep what they recorded. CI sets `FOSSCUT_REQUIRE_FFMPEG=1`, which
//! turns a missing `lavfi` input or ffprobe into a failure instead of a skip.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fosscut_engine::Preset;
use fosscut_engine::import::media_add_args;
use fosscut_engine::jobs::{JobState, Jobs};
use fosscut_engine::record::{
    CaptureInput, Device, DeviceKind, RecordKind, RecordRequest, choose_device, default_output,
    plan, plan_for, progress, record,
};
use fosscut_media::capture::input_formats;
use fosscut_media::{AudioCodec, Container};
use fosscut_time::{FLICKS_PER_SECOND, Flicks, Rate};

const TONE: &str = "sine=frequency=440:sample_rate=48000";
const PICTURES: &str = "testsrc2=size=320x240:rate=30";
const WAIT: Duration = Duration::from_secs(60);

fn required() -> bool {
    std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Whether FFmpeg's `lavfi` input and ffprobe (for importing) are here; skips otherwise.
fn have_ffmpeg() -> bool {
    let lavfi = input_formats().iter().any(|f| f == "lavfi");
    let ffprobe =
        Command::new(std::env::var("FOSSCUT_FFPROBE").unwrap_or_else(|_| "ffprobe".into()))
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success());
    assert!(
        (lavfi && ffprobe) || !required(),
        "FOSSCUT_REQUIRE_FFMPEG is set but lavfi ({lavfi}) or ffprobe ({ffprobe}) is missing"
    );
    if !(lavfi && ffprobe) {
        eprintln!("skipping: no lavfi input or no ffprobe");
    }
    lavfi && ffprobe
}

fn dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("fosscut-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    })
}

/// A fresh, empty project bundle folder.
fn bundle(name: &str) -> PathBuf {
    let bundle = dir().join(format!("{name}.fosscut"));
    let _ = std::fs::remove_dir_all(&bundle);
    std::fs::create_dir_all(&bundle).unwrap();
    bundle
}

fn second() -> Flicks {
    Flicks::from_seconds(1).unwrap()
}

fn seconds(time: Flicks) -> f64 {
    time.get() as f64 / FLICKS_PER_SECOND as f64
}

fn lavfi(input: &str) -> CaptureInput {
    CaptureInput {
        format: "lavfi".into(),
        input: input.into(),
        options: Vec::new(),
    }
}

fn video_preset() -> Preset {
    Preset::default_video().unwrap()
}

fn partial(output: &Path) -> PathBuf {
    output.with_file_name(format!(
        ".{}.partial",
        output.file_name().unwrap().to_string_lossy()
    ))
}

fn device(kind: DeviceKind, name: &str, format: &str, input: &str) -> Device {
    Device {
        kind,
        name: name.into(),
        format: format.into(),
        input: input.into(),
    }
}

/// Devices as AVFoundation lists them on a Mac.
fn mac() -> Vec<Device> {
    vec![
        device(
            DeviceKind::Camera,
            "FaceTime HD Camera",
            "avfoundation",
            "0:",
        ),
        device(
            DeviceKind::Camera,
            "OBS Virtual Camera",
            "avfoundation",
            "1:",
        ),
        device(DeviceKind::Screen, "Capture screen 0", "avfoundation", "2:"),
        device(
            DeviceKind::Microphone,
            "MacBook Pro Microphone",
            "avfoundation",
            ":0",
        ),
        device(DeviceKind::Microphone, "Studio Mic", "avfoundation", ":1"),
        device(DeviceKind::Microphone, "Studio Mic 2", "avfoundation", ":2"),
    ]
}

/// Devices as Video4Linux, PulseAudio and X11 list them on Linux.
fn linux() -> Vec<Device> {
    vec![
        device(
            DeviceKind::Camera,
            "Integrated Camera",
            "v4l2",
            "/dev/video0",
        ),
        device(
            DeviceKind::Microphone,
            "USB Microphone",
            "pulse",
            "alsa_input.usb-mic",
        ),
        device(DeviceKind::Screen, "Screen :0.0", "x11grab", ":0.0"),
    ]
}

fn options(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn camera_output(name: &str) -> PathBuf {
    PathBuf::from(format!("/nowhere/{name}.{}", video_preset().extension()))
}

#[test]
fn devices_are_chosen_by_name_number_or_part_of_a_name() {
    let devices = mac();
    let camera = |wanted: Option<&str>| {
        choose_device(&devices, DeviceKind::Camera, wanted)
            .map(|d| d.name.as_str())
            .map_err(|e| (e.code().to_string(), e.to_string()))
    };
    // The first device of the kind by default.
    assert_eq!(camera(None), Ok("FaceTime HD Camera"));
    // An exact name in any case, a number from 1, or a part of exactly one name.
    assert_eq!(camera(Some("facetime hd CAMERA")), Ok("FaceTime HD Camera"));
    assert_eq!(camera(Some("2")), Ok("OBS Virtual Camera"));
    assert_eq!(camera(Some(" 1 ")), Ok("FaceTime HD Camera"));
    assert_eq!(camera(Some("obs")), Ok("OBS Virtual Camera"));
    // An exact name wins over a part of a longer name.
    let microphone = choose_device(&devices, DeviceKind::Microphone, Some("studio mic")).unwrap();
    assert_eq!(microphone.input, ":1");
    let microphone = choose_device(&devices, DeviceKind::Microphone, Some("mic 2")).unwrap();
    assert_eq!(microphone.name, "Studio Mic 2");

    // A part of several names is ambiguous; the message lists them with their numbers.
    let (code, message) = camera(Some("camera")).unwrap_err();
    assert_eq!(code, "fosscut.request.invalid_arguments");
    assert!(
        message.contains("\"FaceTime HD Camera\" (1)")
            && message.contains("\"OBS Virtual Camera\" (2)"),
        "{message}"
    );
    for wanted in ["3", "0", "webcam"] {
        let (code, message) = camera(Some(wanted)).unwrap_err();
        assert_eq!(code, "fosscut.request.invalid_arguments", "{wanted}");
        assert!(
            message.contains("FaceTime HD Camera") && message.contains("OBS Virtual Camera"),
            "{wanted}: {message}"
        );
        assert!(!message.contains("Capture screen"), "{message}");
    }

    // A kind the machine does not have is a device that is not there.
    let error = choose_device(&linux()[..2], DeviceKind::Screen, None).unwrap_err();
    assert_eq!(error.code(), "fosscut.media.device_unavailable");
    assert!(
        error.to_string().contains("no screen") && error.to_string().contains("USB Microphone"),
        "{error}"
    );
    let error = choose_device(&[], DeviceKind::Camera, Some("1")).unwrap_err();
    assert_eq!(error.code(), "fosscut.media.device_unavailable");
}

#[test]
fn a_mac_camera_and_microphone_combine_into_one_input() {
    let devices = mac();
    let request = RecordRequest::new(RecordKind::Camera, camera_output("camera"));
    let planned = plan_for(&request, &devices).unwrap();
    assert_eq!(planned.device.as_deref(), Some("FaceTime HD Camera"));
    assert_eq!(
        planned.microphone.as_deref(),
        Some("MacBook Pro Microphone")
    );
    assert_eq!(planned.preset, video_preset().name());
    let recording = &planned.recording;
    assert_eq!(
        (recording.format.as_str(), recording.input.as_str()),
        ("avfoundation", "0:0")
    );
    assert_eq!(recording.options, options(&[("framerate", "30")]));
    assert_eq!(recording.microphone, None);
    assert_eq!(recording.duration, None);
    let video = recording.settings.video.as_ref().unwrap();
    assert_eq!(
        (video.width, video.height, video.rate),
        (1280, 720, Rate::FPS_30)
    );
    let audio = recording.settings.audio.as_ref().unwrap();
    assert_eq!((audio.sample_rate, audio.channels), (48_000, 2));
    assert!(planned.encoders.video.is_some() && planned.encoders.audio.is_some());

    // Another microphone by name, a camera by number at another rate and size.
    let mut request = request.clone();
    request.device = Some("2".into());
    request.microphone = Some("Studio Mic 2".into());
    request.rate = Some(Rate::new(60, 1).unwrap());
    request.size = Some((1920, 1080));
    request.duration = Some(second());
    let planned = plan_for(&request, &devices).unwrap();
    assert_eq!(planned.recording.input, "1:2");
    assert_eq!(planned.recording.options, options(&[("framerate", "60")]));
    assert_eq!(planned.recording.duration, Some(second()));
    let video = planned.recording.settings.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (1920, 1080));

    // No sound when asked for none.
    request.microphone = Some("NONE".into());
    let planned = plan_for(&request, &devices).unwrap();
    assert_eq!(planned.recording.input, "1:");
    assert_eq!(planned.microphone, None);
    assert!(planned.recording.settings.audio.is_none());
    assert!(planned.encoders.audio.is_none());
}

#[test]
fn a_mac_screen_records_the_pointer_and_no_sound_by_default() {
    let devices = mac();
    let mut request = RecordRequest::new(RecordKind::Screen, camera_output("screen"));
    let planned = plan_for(&request, &devices).unwrap();
    assert_eq!(planned.device.as_deref(), Some("Capture screen 0"));
    assert_eq!(planned.recording.input, "2:");
    assert_eq!(
        planned.recording.options,
        options(&[("framerate", "30"), ("capture_cursor", "1")])
    );
    assert!(planned.recording.settings.audio.is_none());
    let video = planned.recording.settings.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (1920, 1080));

    request.microphone = Some("2".into());
    let planned = plan_for(&request, &devices).unwrap();
    assert_eq!(planned.recording.input, "2:1");
    assert_eq!(planned.microphone.as_deref(), Some("Studio Mic"));
    assert!(planned.recording.settings.audio.is_some());
}

#[test]
fn elsewhere_the_microphone_is_a_separate_input() {
    let devices = linux();
    let request = RecordRequest::new(RecordKind::Camera, camera_output("camera"));
    let planned = plan_for(&request, &devices).unwrap();
    let recording = &planned.recording;
    assert_eq!(
        (recording.format.as_str(), recording.input.as_str()),
        ("v4l2", "/dev/video0")
    );
    // Video4Linux cameras keep their own rate; the backend times frames to the output.
    assert!(recording.options.is_empty());
    assert_eq!(
        recording.microphone,
        Some(CaptureInput {
            format: "pulse".into(),
            input: "alsa_input.usb-mic".into(),
            options: Vec::new(),
        })
    );
    assert_eq!(planned.microphone.as_deref(), Some("USB Microphone"));
    assert!(recording.settings.audio.is_some());

    let mut screen = RecordRequest::new(RecordKind::Screen, camera_output("screen"));
    screen.microphone = Some("usb".into());
    let planned = plan_for(&screen, &devices).unwrap();
    assert_eq!(planned.recording.input, ":0.0");
    assert_eq!(planned.recording.options, options(&[("framerate", "30")]));
    assert_eq!(
        planned
            .recording
            .microphone
            .as_ref()
            .map(|m| m.format.as_str()),
        Some("pulse")
    );

    // A microphone recording is a 48 kHz stereo 16-bit WAV of the chosen microphone.
    let voice = RecordRequest::new(RecordKind::Microphone, "/nowhere/Voice.WAV");
    let planned = plan_for(&voice, &devices).unwrap();
    assert_eq!(planned.device.as_deref(), Some("USB Microphone"));
    assert_eq!(planned.preset, "wav");
    let settings = &planned.recording.settings;
    assert_eq!(settings.container, Container::Wav);
    assert!(settings.video.is_none());
    let audio = settings.audio.as_ref().unwrap();
    assert_eq!(
        (audio.codec, audio.sample_rate, audio.channels),
        (AudioCodec::Pcm16, 48_000, 2)
    );
    let error = plan_for(&voice, &mac()[..3]).unwrap_err();
    assert_eq!(error.code(), "fosscut.media.device_unavailable");
}

#[test]
fn raw_inputs_replace_device_choice() {
    // No devices are needed when every input is raw.
    let mut request = RecordRequest::new(RecordKind::Camera, camera_output("raw"));
    request.input = Some(lavfi(PICTURES));
    let planned = plan_for(&request, &[]).unwrap();
    assert_eq!(planned.device, None);
    assert_eq!(planned.recording.input, PICTURES);
    // A raw camera records no sound unless asked to.
    assert!(planned.recording.settings.audio.is_none());

    request.microphone_input = Some(lavfi(TONE));
    let planned = plan_for(&request, &[]).unwrap();
    assert_eq!(planned.recording.microphone, Some(lavfi(TONE)));
    assert!(planned.recording.settings.audio.is_some());

    request.microphone_input = None;
    request.input_sound = true;
    let planned = plan_for(&request, &[]).unwrap();
    assert_eq!(planned.recording.microphone, None);
    assert!(planned.recording.settings.audio.is_some());

    // A named microphone with a raw camera is a separate input.
    request.input_sound = false;
    request.microphone = Some("studio mic".into());
    let planned = plan_for(&request, &mac()).unwrap();
    assert_eq!(planned.recording.input, PICTURES);
    assert_eq!(
        planned
            .recording
            .microphone
            .as_ref()
            .map(|m| m.input.as_str()),
        Some(":1")
    );
}

#[test]
fn impossible_requests_are_typed_errors() {
    let devices = mac();
    let code =
        |request: &RecordRequest| plan_for(request, &devices).unwrap_err().code().to_string();
    let camera = RecordRequest::new(RecordKind::Camera, camera_output("camera"));
    let voice = RecordRequest::new(RecordKind::Microphone, "/nowhere/voice.wav");
    let invalid = "fosscut.request.invalid_arguments";

    let mut odd = camera.clone();
    odd.size = Some((321, 240));
    assert_eq!(code(&odd), invalid);
    odd.size = Some((0, 240));
    assert_eq!(code(&odd), invalid);
    odd.size = Some((16_384, 240));
    assert_eq!(code(&odd), invalid);

    // The output is named for the file type the recording writes.
    let mut named = camera.clone();
    named.output = PathBuf::from("/nowhere/camera.wav");
    let error = plan_for(&named, &devices).unwrap_err();
    assert_eq!(error.code(), invalid);
    assert!(
        error
            .to_string()
            .contains(&format!(".{}", video_preset().extension())),
        "{error}"
    );
    let mut named = voice.clone();
    named.output = PathBuf::from("/nowhere/voice.mp4");
    assert!(
        plan_for(&named, &devices)
            .unwrap_err()
            .to_string()
            .contains(".wav")
    );
    named.output = PathBuf::from("/nowhere/voice");
    assert_eq!(code(&named), invalid);

    let mut pictures = voice.clone();
    pictures.size = Some((640, 360));
    assert_eq!(code(&pictures), invalid);
    let mut pictures = voice.clone();
    pictures.rate = Some(Rate::FPS_30);
    assert_eq!(code(&pictures), invalid);
    let mut microphone = voice.clone();
    microphone.microphone = Some("Studio Mic".into());
    assert_eq!(code(&microphone), invalid);

    let mut both = camera.clone();
    both.device = Some("1".into());
    both.input = Some(lavfi(PICTURES));
    assert_eq!(code(&both), invalid);
    let mut both = camera.clone();
    both.microphone = Some("1".into());
    both.microphone_input = Some(lavfi(TONE));
    assert_eq!(code(&both), invalid);
    let mut sound = camera.clone();
    sound.input_sound = true;
    assert_eq!(code(&sound), invalid);

    let mut zero = voice.clone();
    zero.duration = Some(Flicks::ZERO);
    assert_eq!(code(&zero), invalid);

    // An existing output is only replaced when allowed; a folder is never written.
    let existing = dir().join("existing.wav");
    std::fs::write(&existing, b"keep").unwrap();
    let mut over = voice.clone();
    over.output = existing.clone();
    assert_eq!(code(&over), "fosscut.guard.destructive");
    over.allow_overwrite = true;
    assert!(plan_for(&over, &devices).is_ok());
    let folder = dir().join("folder.wav");
    std::fs::create_dir_all(&folder).unwrap();
    over.output = folder;
    assert_eq!(code(&over), invalid);
}

#[test]
fn requests_and_kinds_read_from_json() {
    let request: RecordRequest = serde_json::from_value(serde_json::json!({
        "what": "camera",
        "output": "Take.mp4",
        "device": "FaceTime",
        "microphone": "none",
        "size": [640, 360],
        "rate": "30000/1001",
        "duration": FLICKS_PER_SECOND,
    }))
    .unwrap();
    assert_eq!(request.what, RecordKind::Camera);
    assert_eq!(request.size, Some((640, 360)));
    assert_eq!(request.rate, Some(Rate::NTSC_30));
    assert_eq!(request.duration, Some(second()));
    assert!(!request.allow_overwrite && !request.input_sound);
    let back = serde_json::to_value(&request).unwrap();
    assert!(back.get("input").is_none() && back.get("input_sound").is_none());
    assert_eq!(
        serde_json::from_value::<RecordRequest>(back).unwrap(),
        request
    );

    for kind in RecordKind::ALL {
        assert_eq!(RecordKind::parse(kind.name()), Some(kind));
        assert_eq!(serde_json::to_value(kind).unwrap(), kind.name());
    }
    assert_eq!(RecordKind::parse(" Screen "), Some(RecordKind::Screen));
    assert_eq!(RecordKind::parse("webcam"), None);
}

#[test]
fn progress_counts_whole_seconds() {
    let at = |millis: i64| Flicks::from_millis(millis);
    let counted = |recorded: i64, duration: Option<i64>| {
        let p = progress(at(recorded), duration.map(at));
        (p.done, p.total)
    };
    assert_eq!(counted(0, Some(1000)), (0, 1));
    assert_eq!(counted(999, Some(1000)), (0, 1));
    assert_eq!(counted(1000, Some(1000)), (1, 1));
    assert_eq!(counted(200, Some(500)), (0, 1));
    assert_eq!(counted(1500, Some(2500)), (1, 3));
    // Reaching the duration completes the count.
    assert_eq!(counted(2500, Some(2500)), (3, 3));
    // Open-ended recordings have no total.
    assert_eq!(counted(3200, None), (3, 0));
}

#[test]
fn default_outputs_are_new_files_in_the_recordings_folder() {
    let bundle = bundle("Names");
    let now = UNIX_EPOCH + Duration::from_secs(1_793_394_843);
    let recordings = bundle.join("recordings");
    let first = default_output(&bundle, RecordKind::Microphone, now);
    assert_eq!(first, recordings.join("Voiceover 2026-10-30 21-14-03.wav"));
    assert!(!recordings.exists(), "naming creates nothing");

    std::fs::create_dir_all(&recordings).unwrap();
    std::fs::write(&first, b"take 1").unwrap();
    let second = default_output(&bundle, RecordKind::Microphone, now);
    assert_eq!(
        second,
        recordings.join("Voiceover 2026-10-30 21-14-03 2.wav")
    );
    std::fs::write(&second, b"take 2").unwrap();
    let third = default_output(&bundle, RecordKind::Microphone, now);
    assert_eq!(
        third,
        recordings.join("Voiceover 2026-10-30 21-14-03 3.wav")
    );
    // A recording in progress to a name takes it too.
    std::fs::write(partial(&third), b"recording").unwrap();
    let fourth = default_output(&bundle, RecordKind::Microphone, now);
    assert_eq!(
        fourth,
        recordings.join("Voiceover 2026-10-30 21-14-03 4.wav")
    );
    assert_eq!(
        std::fs::read(&first).unwrap(),
        b"take 1",
        "nothing replaced"
    );

    let extension = video_preset().extension();
    assert_eq!(
        default_output(&bundle, RecordKind::Camera, now),
        recordings.join(format!("Camera 2026-10-30 21-14-03.{extension}"))
    );
    assert_eq!(
        default_output(&bundle, RecordKind::Screen, now),
        recordings.join(format!("Screen 2026-10-30 21-14-03.{extension}"))
    );
    assert_eq!(RecordKind::Camera.extension(), extension);
    assert_eq!(RecordKind::Microphone.extension(), "wav");
}

#[test]
fn a_microphone_records_a_voiceover_that_imports_as_audio() {
    if !have_ffmpeg() {
        return;
    }
    let bundle = bundle("Voice");
    let output = default_output(&bundle, RecordKind::Microphone, SystemTime::now());
    let mut request = RecordRequest::new(RecordKind::Microphone, &output);
    request.input = Some(lavfi(TONE));
    request.duration = Some(second());
    let planned = plan(&request).unwrap();
    assert_eq!((planned.device, planned.preset.as_str()), (None, "wav"));

    let mut reported = Vec::new();
    let report = record(&request, &AtomicBool::new(false), &mut |t| reported.push(t)).unwrap();
    assert!(!reported.is_empty(), "progress is reported");
    assert!(*reported.last().unwrap() >= second(), "{reported:?}");
    // The recordings folder was created, and only the finished file is left in it.
    assert!(output.is_file());
    assert!(!partial(&output).exists());
    assert_eq!(report.output, output.display().to_string());
    assert_eq!(report.what, RecordKind::Microphone);
    assert!(!report.stopped);
    assert_eq!(report.encoders.audio.as_deref(), Some("pcm_s16le"));
    assert!(report.verified.video.is_none());
    let audio = report.verified.audio.as_ref().unwrap();
    assert_eq!(
        (audio.codec.as_str(), audio.sample_rate, audio.channels),
        ("pcm_s16le", 48_000, 2)
    );
    // A second of sound, to within one sample.
    let sample = FLICKS_PER_SECOND / 48_000;
    println!("voiceover: {:.5} s", seconds(report.duration));
    assert!(
        (report.duration - second()).get().abs() <= sample,
        "{:.5} s",
        seconds(report.duration)
    );

    let args = media_add_args(&output, None, &bundle).unwrap();
    assert_eq!(args["kind"], "audio");
    assert_eq!(args["info"]["audio"]["sample_rate"], 48_000);
    assert_eq!(args["info"]["audio"]["channels"], 2);
    assert!(args["info"].get("video").is_none());
    let relative = args["path"]["relative"].as_str().unwrap();
    assert!(relative.starts_with("recordings/Voiceover "), "{relative}");

    // The same output is not replaced by another recording.
    let again = record(&request, &AtomicBool::new(false), &mut |_| {}).unwrap_err();
    assert_eq!(again.code(), "fosscut.guard.destructive");
}

#[test]
fn a_camera_with_a_microphone_records_video_with_sound_that_imports() {
    if !have_ffmpeg() {
        return;
    }
    let bundle = bundle("Camera");
    let output = bundle
        .join("recordings")
        .join(format!("Take.{}", video_preset().extension()));
    let mut request = RecordRequest::new(RecordKind::Camera, &output);
    request.input = Some(lavfi(PICTURES));
    request.microphone_input = Some(lavfi(TONE));
    request.size = Some((640, 360));
    request.rate = Some(Rate::new(25, 1).unwrap());
    request.duration = Some(second());
    let report = record(&request, &AtomicBool::new(false), &mut |_| {}).unwrap();
    assert!(output.is_file());
    assert!(!report.stopped);
    let settings = video_preset().settings(&fosscut_model::TimelineFormat {
        width: 640,
        height: 360,
        rate: Rate::new(25, 1).unwrap(),
        sample_rate: 48_000,
    });
    let video = report.verified.video.as_ref().unwrap();
    println!(
        "camera: {} {}x{} at {:?}, {} frames",
        video.codec, video.width, video.height, video.rate, video.frame_count
    );
    assert_eq!(
        video.codec,
        settings.video.as_ref().unwrap().codec.probe_name()
    );
    assert_eq!((video.width, video.height), (640, 360));
    assert_eq!(video.rate, Some(Rate::new(25, 1).unwrap()));
    assert_eq!(video.frame_count, 25);
    let off = (report.duration - second()).get().abs();
    assert!(
        off <= Rate::new(25, 1).unwrap().frame_duration().get(),
        "{off}"
    );
    let audio = report.verified.audio.as_ref().unwrap();
    assert_eq!(
        (audio.codec.as_str(), audio.sample_rate, audio.channels),
        (
            settings.audio.as_ref().unwrap().codec.probe_name(),
            48_000,
            2
        )
    );

    let args = media_add_args(&output, None, &bundle).unwrap();
    assert_eq!(args["kind"], "video");
    assert_eq!(args["info"]["video"]["width"], 640);
    assert_eq!(args["info"]["video"]["height"], 360);
    assert_eq!(args["info"]["audio"]["sample_rate"], 48_000);
}

#[test]
fn a_screen_records_sound_from_its_own_input() {
    if !have_ffmpeg() {
        return;
    }
    // One lavfi graph with a video and an audio output, like `avfoundation` with `2:1`.
    let output = dir().join(format!("screen.{}", video_preset().extension()));
    let _ = std::fs::remove_file(&output);
    let mut request = RecordRequest::new(RecordKind::Screen, &output);
    request.input = Some(lavfi(&format!("{PICTURES} [out0]; {TONE} [out1]")));
    request.input_sound = true;
    request.size = Some((320, 240));
    request.duration = Some(Flicks::from_millis(500));
    let report = record(&request, &AtomicBool::new(false), &mut |_| {}).unwrap();
    let video = report.verified.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    assert_eq!(video.frame_count, 15);
    assert!(report.verified.audio.is_some());
    assert!(report.settings.audio.is_some());
    assert_eq!(report.preset, video_preset().name());
    assert_eq!((report.device, report.microphone), (None, None));
}

/// Polls job `id` until `done` holds for its status, for at most a minute.
fn wait_for(
    jobs: &Jobs,
    id: &str,
    done: impl Fn(&fosscut_engine::jobs::JobStatus) -> bool,
) -> fosscut_engine::jobs::JobStatus {
    let started = std::time::Instant::now();
    loop {
        let status = jobs.status(id, Duration::from_millis(50)).unwrap();
        if done(&status) || started.elapsed() > WAIT {
            return status;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn recording_jobs_stop_on_cancel_and_keep_what_they_recorded() {
    if !have_ffmpeg() {
        return;
    }
    let jobs = Jobs::new();
    // A bad request starts no job.
    let mut odd = RecordRequest::new(
        RecordKind::Camera,
        dir().join(format!("odd.{}", video_preset().extension())),
    );
    odd.input = Some(lavfi(PICTURES));
    odd.size = Some((321, 240));
    assert_eq!(
        jobs.start_recording(odd).unwrap_err().code(),
        "fosscut.request.invalid_arguments"
    );
    assert!(jobs.list().is_empty());

    let output = dir().join("job-stopped.wav");
    let _ = std::fs::remove_file(&output);
    let mut request = RecordRequest::new(RecordKind::Microphone, &output);
    // `arealtime` paces the tone like a real microphone.
    request.input = Some(lavfi(&format!("{TONE},arealtime")));
    request.duration = Some(Flicks::from_seconds(10).unwrap());
    let started = jobs.start_recording(request.clone()).unwrap();
    assert_eq!(started.kind, "recording");
    assert_eq!(started.state, JobState::Running);
    assert_eq!((started.progress.done, started.progress.total), (0, 10));
    assert_eq!(started.output.as_deref(), Some(output.to_str().unwrap()));

    // A second recording to the same output while the first runs is refused.
    let busy = jobs.start_recording(request.clone()).unwrap_err();
    assert_eq!(busy.code(), "fosscut.export.output_busy");
    assert_eq!(jobs.list().len(), 1);

    // Stop once a second is recorded: a slow machine may take a while to start the device,
    // and a recording stopped before any media arrives is refused.
    let waiting = std::time::Instant::now();
    while jobs
        .status(&started.job, Duration::ZERO)
        .unwrap()
        .progress
        .done
        < 1
    {
        assert!(waiting.elapsed() < WAIT, "nothing recorded in {WAIT:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
    let stopped = jobs.cancel(&started.job, WAIT).unwrap();
    assert_eq!(stopped.state, JobState::Succeeded, "{stopped:?}");
    assert!(stopped.error.is_none() && stopped.report.is_none());
    assert_eq!(stopped.progress.total, 10);
    let report = stopped.recording.clone().unwrap();
    println!(
        "stopped job: {:.3} s recorded in {} ms",
        seconds(report.duration),
        report.elapsed_ms
    );
    assert!(report.stopped);
    assert!(report.duration.is_positive() && report.duration < Flicks::from_seconds(5).unwrap());
    assert!(output.is_file());
    assert!(!partial(&output).exists());
    let serialized = serde_json::to_value(&stopped).unwrap();
    assert_eq!(serialized["state"], "succeeded");
    assert_eq!(serialized["recording"]["what"], "microphone");
    assert_eq!(serialized["recording"]["stopped"], true);
    assert!(serialized.get("report").is_none() && serialized.get("analysis").is_none());

    // Cancelling the finished job changes nothing.
    let after = jobs.cancel(&started.job, Duration::ZERO).unwrap();
    assert_eq!(after.state, JobState::Succeeded);
}

#[test]
fn open_ended_recording_jobs_report_seconds_until_stopped() {
    if !have_ffmpeg() {
        return;
    }
    let jobs = Jobs::new();
    let output = dir().join("job-open.wav");
    let _ = std::fs::remove_file(&output);
    let mut request = RecordRequest::new(RecordKind::Microphone, &output);
    request.input = Some(lavfi(&format!("{TONE},arealtime")));
    let started = jobs.start_recording(request).unwrap();
    assert_eq!((started.progress.done, started.progress.total), (0, 0));

    let running = wait_for(&jobs, &started.job, |s| s.progress.done >= 1);
    assert_eq!(running.state, JobState::Running, "{running:?}");
    assert!(
        running.progress.done >= 1 && running.progress.total == 0,
        "{running:?}"
    );

    let stopped = jobs.cancel(&started.job, WAIT).unwrap();
    assert_eq!(stopped.state, JobState::Succeeded, "{stopped:?}");
    let report = stopped.recording.unwrap();
    assert!(report.stopped);
    assert!(
        report.duration >= second(),
        "{:.3} s",
        seconds(report.duration)
    );
    assert!(stopped.progress.done >= 1);
    assert!(output.is_file());
}
