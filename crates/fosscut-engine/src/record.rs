//! Recording from a microphone, camera or screen straight into a file of the project
//! (CapCut parity CC-027), over the capture backend of `fosscut-media` (ADR-0009).
//!
//! A [`RecordRequest`] says what to record and where. [`plan`] chooses the devices and the
//! file's settings without recording, so front ends can show what will be recorded;
//! [`plan_for`] does the same over a given device list and is the part that tests check
//! choices with. [`record`] plans and records, then probes the file and checks it against
//! the settings the way exports do. Long-lived hosts record in the background with
//! [`Jobs::start_recording`](crate::jobs::Jobs::start_recording).
//!
//! Recording writes a file only; front ends import it into the project afterwards with
//! [`media_add_args`](crate::import::media_add_args). [`default_output`] names a file in
//! the bundle's `recordings/` folder.
//!
//! What is recorded:
//!
//! - A microphone recording is a 48 kHz stereo 16-bit WAV file (lossless, for voiceovers).
//!   A mono microphone plays on both sides.
//! - A camera or screen recording uses the most compatible video preset this machine can
//!   encode ([`Preset::default_video`]: H.264 in MP4 where there is an encoder), with that
//!   preset's sound codec (AAC in MP4) when a microphone is recorded with it. Pictures are
//!   fitted into the size without distortion, with black bars where the shapes differ, at
//!   a constant frame rate.
//!
//! The output's extension must be the one the recording's preset writes (`.wav`, or for
//! pictures the preset's, usually `.mp4`): FossCut never writes another format than the
//! name says, and never substitutes a codec.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use fosscut_media::ffmpeg::{audio_encoder, encoder_capabilities, video_encoder};
use fosscut_media::{OutputReport, OutputSettings};
use fosscut_model::TimelineFormat;
use fosscut_time::{FLICKS_PER_SECOND, Flicks, Rate};
use serde::{Deserialize, Serialize};

pub use fosscut_media::{CaptureInput, Device, DeviceKind, Recording};

use crate::{Encoders, EngineError, Preset, Progress, compare};

/// The folder of a project bundle that recordings are written to by default.
pub const RECORDINGS: &str = "recordings";

/// The size of a screen recording when the request gives none.
pub const SCREEN_SIZE: (u32, u32) = (1920, 1080);

/// The size of a camera recording when the request gives none.
pub const CAMERA_SIZE: (u32, u32) = (1280, 720);

/// The largest width or height a recording may have.
pub const MAX_SIDE: u32 = 8192;

/// The microphone choice that records no sound with a camera or screen.
pub const NO_MICROPHONE: &str = "none";

/// The error code of a device that cannot be found or opened, from `fosscut-media`.
const DEVICE_UNAVAILABLE: &str = "fosscut.media.device_unavailable";

/// What to record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    /// Sound from a microphone, as a WAV voiceover.
    Microphone,
    /// Pictures from a camera, with a microphone by default.
    Camera,
    /// Pictures of a screen, without sound by default.
    Screen,
}

impl RecordKind {
    /// Every kind, in the order front ends list them.
    pub const ALL: [RecordKind; 3] = [
        RecordKind::Microphone,
        RecordKind::Camera,
        RecordKind::Screen,
    ];

    /// The kind's name: `microphone`, `camera` or `screen`.
    pub fn name(self) -> &'static str {
        match self {
            RecordKind::Microphone => "microphone",
            RecordKind::Camera => "camera",
            RecordKind::Screen => "screen",
        }
    }

    /// The kind named `microphone`, `camera` or `screen` (any case).
    pub fn parse(name: &str) -> Option<RecordKind> {
        let name = name.trim();
        RecordKind::ALL
            .into_iter()
            .find(|kind| kind.name().eq_ignore_ascii_case(name))
    }

    /// The kind of device it records from.
    pub fn device_kind(self) -> DeviceKind {
        match self {
            RecordKind::Microphone => DeviceKind::Microphone,
            RecordKind::Camera => DeviceKind::Camera,
            RecordKind::Screen => DeviceKind::Screen,
        }
    }

    /// How [`default_output`] names its files: `Voiceover`, `Camera` or `Screen`.
    pub fn label(self) -> &'static str {
        match self {
            RecordKind::Microphone => "Voiceover",
            RecordKind::Camera => "Camera",
            RecordKind::Screen => "Screen",
        }
    }

    /// The extension of the files it writes on this machine: `wav` for a microphone, the
    /// default video preset's (usually `mp4`) for a camera or screen.
    pub fn extension(self) -> &'static str {
        match self {
            RecordKind::Microphone => Preset::Wav.extension(),
            RecordKind::Camera | RecordKind::Screen => {
                Preset::default_video().map_or("mp4", Preset::extension)
            }
        }
    }
}

/// What to record and where.
///
/// Devices are chosen by name or number among the devices of their kind ([`choose_device`]).
/// A raw `input` replaces that choice for advanced use (and for tests, with FFmpeg's `lavfi`
/// input); `what` still decides the file type.
///
/// Sound with a camera or screen comes from, in this order: the raw input itself when
/// `input_sound` is set; `microphone_input`; the microphone named by `microphone`; or by
/// default, for a camera chosen from the devices, the first microphone (none when the
/// machine has none). A screen, or a camera given as a raw `input`, records no sound
/// unless asked to. `microphone: "none"` always records no sound.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordRequest {
    /// Microphone, camera or screen.
    pub what: RecordKind,
    /// The device by name (any case: an exact name first, then a part of exactly one name)
    /// or by its number from 1 among the devices of its kind; the first device of the
    /// kind when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// For a camera or screen, the microphone to record with it, chosen like `device`
    /// among microphones, or `none` for no sound. Not allowed for a microphone recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<String>,
    /// A raw FFmpeg input that replaces the choice of `device`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<CaptureInput>,
    /// For a camera or screen, a raw FFmpeg input for the sound that replaces the choice of
    /// `microphone`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone_input: Option<CaptureInput>,
    /// For a camera or screen given as a raw `input`, that input delivers the sound too (a
    /// combined `avfoundation` input such as `0:1`, or a `lavfi` graph with an audio
    /// output).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub input_sound: bool,
    /// Stop after this long; record until stopped when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<Flicks>,
    /// The file to write. Its folder is created when missing.
    pub output: PathBuf,
    /// For a camera or screen, the picture size in pixels (even numbers): 1920x1080 for a
    /// screen and 1280x720 for a camera when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<(u32, u32)>,
    /// For a camera or screen, the frame rate: 30 frames per second when `None`. A camera
    /// chosen from `avfoundation` devices is asked for this rate, which it has to support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<Rate>,
    /// Replace `output` if it exists (ADR-0006, item 6).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_overwrite: bool,
}

impl RecordRequest {
    /// A request for `what` into `output`, with every choice left to its default.
    pub fn new(what: RecordKind, output: impl Into<PathBuf>) -> RecordRequest {
        RecordRequest {
            what,
            device: None,
            microphone: None,
            input: None,
            microphone_input: None,
            input_sound: false,
            duration: None,
            output: output.into(),
            size: None,
            rate: None,
            allow_overwrite: false,
        }
    }
}

/// What a request will record, as [`plan`] resolves it without recording.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordPlan {
    /// Microphone, camera or screen.
    pub what: RecordKind,
    /// The name of the device chosen; `None` for a raw input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// The name of the microphone recorded with a camera or screen; `None` when there is
    /// no sound or it comes from a raw input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<String>,
    /// The export preset whose codecs the file uses, for example `wav` or `mp4-h264`.
    pub preset: String,
    /// The encoders that will be used.
    pub encoders: Encoders,
    /// The inputs, options, output, settings and duration given to the capture backend.
    pub recording: Recording,
}

/// A finished recording: what was recorded, and the file as probed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingReport {
    /// The written file.
    pub output: String,
    /// Microphone, camera or screen.
    pub what: RecordKind,
    /// The name of the device recorded; `None` for a raw input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// The name of the microphone recorded with a camera or screen; `None` when there was
    /// no sound or it came from a raw input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<String>,
    /// The export preset whose codecs the file uses.
    pub preset: String,
    /// The settings it was written with.
    pub settings: OutputSettings,
    /// Encoders used.
    pub encoders: Encoders,
    /// How long the recording is: its pictures' duration, or its sound's.
    pub duration: Flicks,
    /// True when it was stopped before its duration (always, for an open-ended recording
    /// that was stopped) rather than ending at its duration or the end of its input.
    pub stopped: bool,
    /// What the file contains, as probed after writing; it matched the settings.
    pub verified: OutputReport,
    /// Wall-clock time in milliseconds, from opening the devices to probing the file.
    pub elapsed_ms: u64,
}

/// The capture devices on this machine (cameras, screens and microphones) in the order
/// their input formats list them. Empty when there are none.
pub fn devices() -> Result<Vec<Device>, EngineError> {
    Ok(fosscut_media::capture::list_devices()?)
}

/// The device of `kind` that `wanted` names among `devices`: the one whose name is
/// `wanted` in any case, else the one numbered `wanted` from 1 among devices of the kind,
/// else the only one whose name contains `wanted` in any case. The first device of the
/// kind when `wanted` is `None`.
///
/// A choice that matches no device, or a part of several names, fails with
/// `fosscut.request.invalid_arguments` and a message that lists the devices of the kind;
/// no device of the kind at all fails with `fosscut.media.device_unavailable`.
pub fn choose_device<'a>(
    devices: &'a [Device],
    kind: DeviceKind,
    wanted: Option<&str>,
) -> Result<&'a Device, EngineError> {
    let of_kind: Vec<&Device> = devices.iter().filter(|d| d.kind == kind).collect();
    let Some(first) = of_kind.first() else {
        return Err(EngineError::Failed {
            code: DEVICE_UNAVAILABLE.into(),
            message: format!(
                "this machine has no {} to record from{}",
                singular(kind),
                found(devices)
            ),
        });
    };
    let Some(wanted) = wanted.map(str::trim) else {
        return Ok(first);
    };
    let lower = wanted.to_lowercase();
    if let Some(device) = of_kind.iter().find(|d| d.name.to_lowercase() == lower) {
        return Ok(device);
    }
    if let Ok(number) = wanted.parse::<usize>() {
        return match number.checked_sub(1).and_then(|n| of_kind.get(n)) {
            Some(device) => Ok(device),
            None => Err(EngineError::InvalidArguments {
                message: format!(
                    "there is no {} number {number}; the {} are {}",
                    singular(kind),
                    plural(kind),
                    listing(&of_kind)
                ),
            }),
        };
    }
    let matches: Vec<&Device> = of_kind
        .iter()
        .copied()
        .filter(|d| !lower.is_empty() && d.name.to_lowercase().contains(&lower))
        .collect();
    match matches.as_slice() {
        [device] => Ok(device),
        [] => Err(EngineError::InvalidArguments {
            message: format!(
                "no {} is called {wanted:?}; the {} are {}",
                singular(kind),
                plural(kind),
                listing(&of_kind)
            ),
        }),
        several => Err(EngineError::InvalidArguments {
            message: format!(
                "{wanted:?} matches {} {}: {}; give more of the name or its number",
                several.len(),
                plural(kind),
                listing(several)
            ),
        }),
    }
}

/// Resolves `request` into what will be recorded, listing this machine's devices only
/// when the request chooses one. Nothing is opened or written; this is the recording's
/// dry run.
pub fn plan(request: &RecordRequest) -> Result<RecordPlan, EngineError> {
    let listed = if needs_devices(request) {
        devices()?
    } else {
        Vec::new()
    };
    plan_for(request, &listed)
}

/// Resolves `request` into what will be recorded, choosing devices among `devices`.
///
/// A camera or screen and a microphone that are both `avfoundation` devices (macOS)
/// combine into one input (`0:` and `:1` make `0:1`); elsewhere the microphone is a
/// separate input ([`Recording::microphone`]). A camera or screen chosen from
/// `avfoundation`, `x11grab` or `gdigrab` devices is asked for the frame rate, and an
/// `avfoundation` screen also records the pointer.
///
/// Fails with `fosscut.request.invalid_arguments` for contradictory or impossible choices
/// (an odd size, a duration that is not positive, an output named for another format),
/// `fosscut.guard.destructive` when the output exists and may not be replaced, and
/// `fosscut.export.codec_unavailable` when this machine cannot encode the file.
pub fn plan_for(request: &RecordRequest, devices: &[Device]) -> Result<RecordPlan, EngineError> {
    check_request(request)?;
    let preset = preset_for(request.what)?;
    check_output(&request.output, preset, request.allow_overwrite)?;
    let kind = request.what;
    let (width, height) = request.size.unwrap_or(match kind {
        RecordKind::Screen => SCREEN_SIZE,
        _ => CAMERA_SIZE,
    });
    let rate = request.rate.unwrap_or(Rate::FPS_30);
    let mut settings = preset.settings(&TimelineFormat {
        width,
        height,
        rate,
        sample_rate: 48_000,
    });

    let (mut main, device) = match &request.input {
        Some(raw) => (raw.clone(), None),
        None => {
            let device = choose_device(devices, kind.device_kind(), request.device.as_deref())?;
            let mut input = device.capture_input();
            if kind != RecordKind::Microphone {
                input.options = picture_options(device, rate);
            }
            (input, Some(device))
        }
    };
    let mut separate = None;
    let mut microphone_name = None;
    if kind != RecordKind::Microphone {
        check_size(width, height)?;
        let chosen = microphone_device(request, devices)?;
        let sound = if request.input_sound {
            true
        } else if let Some(raw) = &request.microphone_input {
            separate = Some(raw.clone());
            true
        } else if let Some(microphone) = chosen {
            microphone_name = Some(microphone.name.clone());
            match device.and_then(|d| combined_input(d, microphone)) {
                Some(combined) => main.input = combined,
                None => separate = Some(microphone.capture_input()),
            }
            true
        } else {
            false
        };
        if !sound {
            settings.audio = None;
        }
    }
    check_settings(&settings)?;
    let encoders = Encoders {
        video: settings
            .video
            .as_ref()
            .and_then(|v| video_encoder(v.codec))
            .map(|e| e.encoder.clone()),
        audio: settings
            .audio
            .as_ref()
            .and_then(|a| audio_encoder(a.codec))
            .map(|e| e.encoder.clone()),
    };
    Ok(RecordPlan {
        what: kind,
        device: device.map(|d| d.name.clone()),
        microphone: microphone_name,
        preset: preset.name().to_string(),
        encoders,
        recording: Recording {
            format: main.format,
            input: main.input,
            options: main.options,
            microphone: separate,
            output: request.output.clone(),
            settings,
            duration: request.duration,
        },
    })
}

/// Plans and records `request` until its duration is reached, `stop` is set or the input
/// ends, calling `progress` with the time recorded so far. The file is written under a
/// temporary name next to the output and moved into place only when complete, then probed
/// and checked against its settings (a mismatch fails with
/// `fosscut.export.verification_failed` and keeps the file). A recording that fails, or is
/// stopped before its devices delivered anything (`fosscut.media.nothing_recorded`),
/// leaves no file behind. Device errors keep their `fosscut.media.*` codes, such as
/// `fosscut.media.device_unavailable` and `fosscut.media.input_format_unavailable`.
pub fn record(
    request: &RecordRequest,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(Flicks),
) -> Result<RecordingReport, EngineError> {
    let planned = plan(request)?;
    record_plan(&planned, stop, progress)
}

/// Records what [`plan`] resolved; see [`record`].
pub(crate) fn record_plan(
    planned: &RecordPlan,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(Flicks),
) -> Result<RecordingReport, EngineError> {
    let started = Instant::now();
    let recording = &planned.recording;
    if let Some(folder) = recording
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        && !folder.is_dir()
    {
        std::fs::create_dir_all(folder).map_err(|e| EngineError::Failed {
            code: "fosscut.media.write_failed".into(),
            message: format!("cannot create {}: {e}", folder.display()),
        })?;
    }
    let verified = fosscut_media::capture::record(recording, stop, progress)?;
    let video = verified.video.as_ref();
    let video_duration = video.and_then(|v| v.duration);
    // Frames and duration come from the file itself: a recording's length is whatever
    // was recorded, while the container, codecs, size, rate and sound must match.
    let mismatches = compare(
        &recording.settings,
        video.map_or(0, |v| v.frame_count),
        video_duration.unwrap_or(Flicks::ZERO),
        &verified,
    );
    if !mismatches.is_empty() {
        return Err(EngineError::VerificationFailed {
            mismatches,
            report: Box::new(verified),
        });
    }
    let duration = video_duration
        .or(verified.audio.as_ref().and_then(|a| a.duration))
        .or(verified.duration)
        .unwrap_or(Flicks::ZERO);
    let stopped =
        stop.load(Ordering::Relaxed) && recording.duration.is_none_or(|limit| duration < limit);
    Ok(RecordingReport {
        output: recording.output.display().to_string(),
        what: planned.what,
        device: planned.device.clone(),
        microphone: planned.microphone.clone(),
        preset: planned.preset.clone(),
        settings: recording.settings.clone(),
        encoders: planned.encoders.clone(),
        duration,
        stopped,
        verified,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// A recording's progress in whole seconds: `done` is the time recorded so far rounded
/// down, `total` the duration rounded up, or 0 for an open-ended recording. Once the
/// duration is reached, `done` equals `total`.
pub fn progress(recorded: Flicks, duration: Option<Flicks>) -> Progress {
    let second = FLICKS_PER_SECOND as u64;
    let whole = |time: Flicks| time.get().max(0) as u64;
    let total = duration.map_or(0, |d| whole(d).div_ceil(second));
    let reached = duration.is_some_and(|d| recorded >= d);
    Progress {
        done: if reached {
            total
        } else {
            whole(recorded) / second
        },
        total,
    }
}

/// A new file for a recording of `kind` in the bundle's `recordings/` folder, named like
/// `Voiceover 2026-10-30 21-14-03.wav`, `Camera 2026-10-30 21-14-03.mp4` or
/// `Screen 2026-10-30 21-14-03.mp4` (the extension is [`RecordKind::extension`]). The time
/// is `now` in UTC, since the local time zone is not known without another dependency.
/// When that name is taken, or a recording to it is in progress, ` 2`, ` 3` and so on
/// follow the time, so an existing file is never replaced. The folder is not created here;
/// [`record`] creates it.
pub fn default_output(bundle: &Path, kind: RecordKind, now: SystemTime) -> PathBuf {
    let folder = bundle.join(RECORDINGS);
    let extension = kind.extension();
    let base = format!("{} {}", kind.label(), utc_stamp(now));
    let named = |n: u32| {
        let name = if n == 1 {
            base.clone()
        } else {
            format!("{base} {n}")
        };
        folder.join(format!("{name}.{extension}"))
    };
    (1..=u32::MAX)
        .map(&named)
        .find(|path| !path.exists() && !partial_path(path).exists())
        .unwrap_or_else(|| named(1))
}

/// Whether planning `request` needs this machine's device list.
fn needs_devices(request: &RecordRequest) -> bool {
    let named_microphone = request
        .microphone
        .as_deref()
        .is_some_and(|m| !m.trim().eq_ignore_ascii_case(NO_MICROPHONE));
    match request.what {
        RecordKind::Microphone => request.input.is_none(),
        RecordKind::Camera | RecordKind::Screen => request.input.is_none() || named_microphone,
    }
}

/// Rejects contradictory or impossible requests before anything is chosen.
fn check_request(request: &RecordRequest) -> Result<(), EngineError> {
    let invalid = |message: &str| {
        Err(EngineError::InvalidArguments {
            message: message.to_string(),
        })
    };
    if request.device.is_some() && request.input.is_some() {
        return invalid("give a device or a raw input, not both");
    }
    if request.duration.is_some_and(|d| !d.is_positive()) {
        return invalid("a recording's duration must be positive");
    }
    if request.what == RecordKind::Microphone {
        if request.microphone.is_some() || request.microphone_input.is_some() || request.input_sound
        {
            return invalid(
                "a microphone recording takes its microphone from device or input; microphone, microphone_input and input_sound are for cameras and screens",
            );
        }
        if request.size.is_some() || request.rate.is_some() {
            return invalid("a microphone recording has no pictures: leave out size and rate");
        }
        return Ok(());
    }
    if request.microphone.is_some() && request.microphone_input.is_some() {
        return invalid("give a microphone or a raw microphone_input, not both");
    }
    if request.input_sound {
        if request.input.is_none() {
            return invalid("input_sound needs a raw input that delivers the sound");
        }
        if request.microphone.is_some() || request.microphone_input.is_some() {
            return invalid(
                "the sound comes from the input (input_sound) or from a microphone, not both",
            );
        }
    }
    Ok(())
}

/// The preset whose codecs a recording of `kind` uses.
fn preset_for(kind: RecordKind) -> Result<Preset, EngineError> {
    match kind {
        RecordKind::Microphone => Ok(Preset::Wav),
        RecordKind::Camera | RecordKind::Screen => {
            Preset::default_video().ok_or_else(|| EngineError::CodecUnavailable {
                codec: fosscut_media::VideoCodec::H264.probe_name().to_string(),
                available: unique(
                    encoder_capabilities()
                        .video
                        .iter()
                        .map(|(c, _)| c.probe_name().to_string()),
                ),
            })
        }
    }
}

/// Checks that the output is a file named for `preset` that may be written.
fn check_output(output: &Path, preset: Preset, allow_overwrite: bool) -> Result<(), EngineError> {
    let expected = preset.extension();
    let extension = output
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case(expected) {
        return Err(EngineError::InvalidArguments {
            message: format!(
                "this recording is written as {} ({}): name the output .{expected}, not {}",
                expected,
                preset.name(),
                output.display()
            ),
        });
    }
    if output.is_dir() {
        return Err(EngineError::InvalidArguments {
            message: format!("{} is a folder, not a file", output.display()),
        });
    }
    if output.exists() && !allow_overwrite {
        return Err(EngineError::OutputExists {
            path: output.display().to_string(),
        });
    }
    Ok(())
}

/// Checks a picture size: even, non-zero and at most [`MAX_SIDE`] on each side.
fn check_size(width: u32, height: u32) -> Result<(), EngineError> {
    let fine = |side: u32| side > 0 && side.is_multiple_of(2) && side <= MAX_SIDE;
    if fine(width) && fine(height) {
        Ok(())
    } else {
        Err(EngineError::InvalidArguments {
            message: format!(
                "{width}x{height} is not a recording size: use even numbers from 2 to {MAX_SIDE}"
            ),
        })
    }
}

/// Checks that the container holds the codecs and this machine can encode them.
fn check_settings(settings: &OutputSettings) -> Result<(), EngineError> {
    let video = settings.video.as_ref().map(|v| v.codec);
    let audio = settings.audio.as_ref().map(|a| a.codec);
    if !settings.container.supports(video, audio) {
        return Err(EngineError::InvalidArguments {
            message: format!(
                "{} cannot hold these codecs",
                settings.container.extension()
            ),
        });
    }
    if let Some(codec) = audio
        && audio_encoder(codec).is_none()
    {
        return Err(EngineError::CodecUnavailable {
            codec: codec.probe_name().to_string(),
            available: unique(
                encoder_capabilities()
                    .audio
                    .iter()
                    .map(|(c, _)| c.probe_name().to_string()),
            ),
        });
    }
    if let Some(codec) = video
        && video_encoder(codec).is_none()
    {
        return Err(EngineError::CodecUnavailable {
            codec: codec.probe_name().to_string(),
            available: unique(
                encoder_capabilities()
                    .video
                    .iter()
                    .map(|(c, _)| c.probe_name().to_string()),
            ),
        });
    }
    Ok(())
}

/// The microphone device a camera or screen request records with, if any.
fn microphone_device<'a>(
    request: &RecordRequest,
    devices: &'a [Device],
) -> Result<Option<&'a Device>, EngineError> {
    if request.input_sound || request.microphone_input.is_some() {
        return Ok(None);
    }
    match request.microphone.as_deref().map(str::trim) {
        Some(name) if name.eq_ignore_ascii_case(NO_MICROPHONE) => Ok(None),
        Some(name) => choose_device(devices, DeviceKind::Microphone, Some(name)).map(Some),
        // A camera chosen from the devices takes the first microphone, when there is one.
        None if request.what == RecordKind::Camera && request.input.is_none() => {
            Ok(devices.iter().find(|d| d.kind == DeviceKind::Microphone))
        }
        None => Ok(None),
    }
}

/// Input options that make a picture device deliver `rate`: screen grabbers and
/// `avfoundation` cameras (which otherwise ask for 29.97 frames per second, a rate most
/// cameras refuse) take `framerate`, and an `avfoundation` screen records the pointer.
fn picture_options(device: &Device, rate: Rate) -> Vec<(String, String)> {
    let mut options = Vec::new();
    let rated = match device.format.as_str() {
        "avfoundation" => true,
        "x11grab" | "gdigrab" => device.kind == DeviceKind::Screen,
        _ => false,
    };
    if rated {
        options.push(("framerate".to_string(), rate.to_string()));
    }
    if device.format == "avfoundation" && device.kind == DeviceKind::Screen {
        options.push(("capture_cursor".to_string(), "1".to_string()));
    }
    options
}

/// The one `avfoundation` input that records `picture` (`0:`) with `microphone` (`:1`):
/// `0:1`. `None` when they cannot combine.
fn combined_input(picture: &Device, microphone: &Device) -> Option<String> {
    if picture.format != "avfoundation" || microphone.format != "avfoundation" {
        return None;
    }
    let video = picture.input.strip_suffix(':')?;
    let audio = microphone.input.strip_prefix(':')?;
    (!video.is_empty() && !audio.is_empty() && !video.contains(':') && !audio.contains(':'))
        .then(|| format!("{video}:{audio}"))
}

/// `"name" (1), "other" (2)`: devices with their numbers, for messages.
fn listing(devices: &[&Device]) -> String {
    devices
        .iter()
        .enumerate()
        .map(|(n, d)| format!("{:?} ({})", d.name, n + 1))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `; devices found: "Mic" (microphone)`, or nothing when there are none.
fn found(devices: &[Device]) -> String {
    if devices.is_empty() {
        return "; FFmpeg found no capture devices".to_string();
    }
    let shown = devices
        .iter()
        .map(|d| format!("{:?} ({})", d.name, singular(d.kind)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("; devices found: {shown}")
}

fn singular(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Microphone => "microphone",
        DeviceKind::Camera => "camera",
        DeviceKind::Screen => "screen",
    }
}

fn plural(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Microphone => "microphones",
        DeviceKind::Camera => "cameras",
        DeviceKind::Screen => "screens",
    }
}

/// `names` in their first order without repeats (the ProRes profiles share a codec name).
fn unique(names: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for name in names {
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen
}

/// The temporary file the capture backend writes a recording to: `.name.partial` next to
/// the output.
fn partial_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    output.with_file_name(format!(".{name}.partial"))
}

/// `2026-10-30 21-14-03`: `now` in UTC, with dashes in the time so it is a valid file name
/// everywhere. Times before 1970 count as 1970.
fn utc_stamp(now: SystemTime) -> String {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (year, month, day) = civil_date(seconds.div_euclid(86_400));
    let time = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}-{:02}-{:02}",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_convert_from_unix_days() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(31), (1970, 2, 1));
        assert_eq!(civil_date(365), (1971, 1, 1));
        // 2000 is a leap year (divisible by 400), 2100 is not.
        assert_eq!(civil_date(11_016), (2000, 2, 29));
        assert_eq!(civil_date(11_017), (2000, 3, 1));
        assert_eq!(civil_date(47_540), (2100, 2, 28));
        assert_eq!(civil_date(47_541), (2100, 3, 1));
    }

    #[test]
    fn stamps_are_utc_with_dashes() {
        let at = |seconds: u64| UNIX_EPOCH + std::time::Duration::from_secs(seconds);
        assert_eq!(utc_stamp(at(0)), "1970-01-01 00-00-00");
        assert_eq!(utc_stamp(at(1_709_251_199)), "2024-02-29 23-59-59");
        assert_eq!(utc_stamp(at(1_793_394_843)), "2026-10-30 21-14-03");
        let before = UNIX_EPOCH - std::time::Duration::from_secs(10);
        assert_eq!(utc_stamp(before), "1970-01-01 00-00-00");
    }

    #[test]
    fn avfoundation_inputs_combine_only_with_each_other() {
        let device = |kind, format: &str, input: &str| Device {
            kind,
            name: input.into(),
            format: format.into(),
            input: input.into(),
        };
        let camera = device(DeviceKind::Camera, "avfoundation", "0:");
        let screen = device(DeviceKind::Screen, "avfoundation", "3:");
        let microphone = device(DeviceKind::Microphone, "avfoundation", ":1");
        assert_eq!(combined_input(&camera, &microphone).as_deref(), Some("0:1"));
        assert_eq!(combined_input(&screen, &microphone).as_deref(), Some("3:1"));
        let pulse = device(DeviceKind::Microphone, "pulse", "default");
        assert_eq!(combined_input(&camera, &pulse), None);
        let v4l2 = device(DeviceKind::Camera, "v4l2", "/dev/video0");
        assert_eq!(combined_input(&v4l2, &microphone), None);
        let odd = device(DeviceKind::Camera, "avfoundation", "0:2");
        assert_eq!(combined_input(&odd, &microphone), None);
    }

    #[test]
    fn the_device_unavailable_code_is_the_media_crates() {
        let error = fosscut_media::MediaError::DeviceUnavailable {
            format: "lavfi".into(),
            input: "x".into(),
            message: "gone".into(),
        };
        assert_eq!(error.code(), DEVICE_UNAVAILABLE);
    }
}
