//! Recording from microphones, cameras and screens straight into a file (CapCut parity
//! CC-027), with backend-neutral descriptions of devices and recordings.
//!
//! A device is named the way FFmpeg's input devices name it: an input format such as
//! `avfoundation` (macOS), `dshow` or `gdigrab` (Windows), `v4l2`, `pulse`, `alsa` or
//! `x11grab` (Linux), and the input that format calls the device. [`list_devices`] finds
//! the devices of this machine and [`record`] writes what one or two of them capture
//! through the same encoders, settings validation and probing as exports (ADR-0009).
//! FFmpeg's `lavfi` input works like a device, which is how recording is tested without
//! hardware.

use std::path::PathBuf;

use fosscut_time::Flicks;
use serde::{Deserialize, Serialize};

use crate::OutputSettings;

#[cfg(feature = "ffmpeg")]
pub use crate::ffmpeg::{input_formats, list_devices, record};

/// What a capture device records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    /// Sound: a microphone, a line input or a system audio monitor.
    Microphone,
    /// Pictures from a camera.
    Camera,
    /// Pictures of a screen.
    Screen,
}

/// A capture device.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Device {
    /// What the device records.
    pub kind: DeviceKind,
    /// The name people know the device by, for example `FaceTime HD Camera`.
    pub name: String,
    /// FFmpeg input format, such as `avfoundation`.
    pub format: String,
    /// What that format calls it, such as `:0` or `0:`. On macOS a camera or screen
    /// (`0:`) and a microphone (`:1`) combine into one input (`0:1`) that records both.
    pub input: String,
}

impl Device {
    /// The input that opens this device, with no options.
    pub fn capture_input(&self) -> CaptureInput {
        CaptureInput {
            format: self.format.clone(),
            input: self.input.clone(),
            options: Vec::new(),
        }
    }
}

/// One device input to open.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CaptureInput {
    /// FFmpeg input format, such as `avfoundation`.
    pub format: String,
    /// What that format calls the device.
    pub input: String,
    /// Extra input options such as `framerate`, `video_size` or `sample_rate`. An option
    /// the format does not know is an error rather than ignored.
    pub options: Vec<(String, String)>,
}

/// What to record and where.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recording {
    /// FFmpeg input format, such as `avfoundation`.
    pub format: String,
    /// What that format calls the device.
    pub input: String,
    /// Extra input options such as `framerate`, `video_size` or `sample_rate`. An option
    /// the format does not know is an error rather than ignored.
    pub options: Vec<(String, String)>,
    /// A second input whose sound is recorded instead of the first input's, for a camera
    /// or screen with a microphone where they are separate inputs (Linux `v4l2` or
    /// `x11grab` with `pulse`, Windows `gdigrab` with `dshow`). The two are lined up by
    /// when each delivered its first media.
    pub microphone: Option<CaptureInput>,
    /// The file to write. It is written under a temporary name next to it and moved into
    /// place only when complete.
    pub output: PathBuf,
    /// What the file contains. Pictures are scaled to fit the video size without
    /// distortion, with black bars where the aspect ratios differ, and timed to its
    /// constant frame rate (frames are repeated or dropped as the device delivers them);
    /// sound is converted to the audio sample rate and channels.
    pub settings: OutputSettings,
    /// Stop after this long; `None` records until `stop` is set or the input ends.
    pub duration: Option<Flicks>,
}

impl Recording {
    /// The first input as a [`CaptureInput`].
    pub fn capture_input(&self) -> CaptureInput {
        CaptureInput {
            format: self.format.clone(),
            input: self.input.clone(),
            options: self.options.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioCodec, AudioSettings, Container};

    #[test]
    fn devices_and_recordings_name_their_inputs() {
        let microphone = Device {
            kind: DeviceKind::Microphone,
            name: "MacBook Pro Microphone".into(),
            format: "avfoundation".into(),
            input: ":1".into(),
        };
        assert_eq!(
            microphone.capture_input(),
            CaptureInput {
                format: "avfoundation".into(),
                input: ":1".into(),
                options: Vec::new(),
            }
        );
        let recording = Recording {
            format: "lavfi".into(),
            input: "sine".into(),
            options: vec![("sample_rate".into(), "48000".into())],
            microphone: None,
            output: PathBuf::from("voice.wav"),
            settings: OutputSettings {
                container: Container::Wav,
                video: None,
                audio: Some(AudioSettings::stereo(AudioCodec::Pcm16)),
            },
            duration: None,
        };
        let input = recording.capture_input();
        assert_eq!(
            (input.format.as_str(), input.input.as_str()),
            ("lavfi", "sine")
        );
        assert_eq!(input.options, recording.options);
    }
}
