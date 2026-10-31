//! FossCut's local media analysis (ADR-0012, ADR-0019): facts about footage that agents
//! build edits on, measured on this machine and cached by content, and the local models they
//! run (ADR-0020, ADR-0033).
//!
//! - [`loudness`]: ITU-R BS.1770-4 integrated loudness and sample peak;
//! - [`levels`]: K-weighted loudness of consecutive windows, for peaks and variation;
//! - [`beats`]: the tempo, beats and downbeats of music, by dynamic programming;
//! - [`silence`]: ranges quieter than a threshold for at least a minimum duration;
//! - [`scenes`]: hard shot cuts from luma-histogram differences;
//! - [`speech`]: words with timings, filler candidates and pauses, recognised by whisper.cpp
//!   with a model from [`models`] (ADR-0020);
//! - [`models`]: the manifests of every model FossCut can install (speech, mattes, pose,
//!   voices, translation and the ONNX Runtime library), and their verified installation;
//! - [`onnx`]: sessions on ONNX Runtime, loaded at run time, for the vision, voice and
//!   translation models (ADR-0033);
//! - [`motion`]: camera motion between frames, the camera path, and corrections that steady
//!   it;
//! - [`track`]: where a region of the first frame is in each later frame;
//! - [`sync`]: how far apart two recordings of the same scene are, from their sound;
//! - [`pose`]: body landmarks and a person mask in every frame, with MediaPipe Pose
//!   Landmarker (ADR-0033, item 4);
//! - [`matte`]: the subject's alpha in every frame, from the person mask or BiRefNet
//!   (ADR-0033, item 3);
//! - [`vision`]: what those two share: RGB frames, resampling, packed mattes, smoothing in
//!   time and their data files;
//! - [`framing`]: crops of another aspect that follow a subject (from pose, mattes or
//!   tracking) through each shot, for reframing and camera tracking;
//! - [`highlights`]: candidate shorts of whole sentences in long footage, scored from the
//!   transcript, levels and scene cuts, with their reasons.
//!
//! The analyzers are pure and stream over samples or frames; [`decode`] feeds them from a
//! media file and [`cache`] stores their results outside the project, keyed by the file's
//! content identity, the analyzer version and its options.

#![forbid(unsafe_code)]

pub mod beats;
pub mod cache;
pub mod decode;
pub mod framing;
mod gray;
pub mod highlights;
pub mod levels;
pub mod loudness;
pub mod matte;
pub mod models;
pub mod motion;
pub mod onnx;
pub mod pose;
pub mod scenes;
pub mod silence;
pub mod speech;
pub mod sync;
pub mod track;
pub mod translate;
pub mod vision;
pub mod voice;

use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use beats::{Beat, BeatOptions, Beats};
pub use levels::{LevelOptions, Levels};
pub use loudness::Loudness;
pub use matte::{Matte, MatteModel, MatteOptions, MatteTrack};
pub use motion::{
    CameraPath, Correction, FrameMotion, MotionEstimator, PathFrame, Stabilization, stabilize,
};
pub use pose::{Pose, PoseOptions, PoseTrack};
pub use scenes::{Cut, SceneOptions, Scenes};
pub use silence::{SilenceOptions, Silences};
pub use speech::{Transcript, TranscriptOptions};
pub use sync::{SyncResult, audio_offset};
pub use track::{Region, TrackPoint, Tracker};

/// A moment in an asset, from its start: exact flicks, with decimal seconds for reading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Moment {
    /// Exact time in flicks.
    pub flicks: Flicks,
    /// The same time in seconds, rounded to microseconds.
    pub seconds: String,
}

impl Moment {
    /// The moment `time` after the asset's start.
    pub fn new(time: Flicks) -> Moment {
        Moment {
            flicks: time,
            seconds: time.to_seconds_string(),
        }
    }
}

/// A span of an asset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Span {
    /// Where it starts.
    pub start: Moment,
    /// Where it ends (exclusive).
    pub end: Moment,
}

impl Span {
    /// The span from `start` to `end`.
    pub fn new(start: Flicks, end: Flicks) -> Span {
        Span {
            start: Moment::new(start),
            end: Moment::new(end),
        }
    }
}

/// The analyzers FossCut runs (ADR-0019, item 2).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Analyzer {
    /// Integrated loudness (LUFS) and sample peak (dBFS).
    Loudness,
    /// Ranges below a level for at least a minimum duration.
    Silences,
    /// Hard shot cuts.
    Scenes,
    /// Speech as words with timings, filler candidates and pauses.
    Transcript,
    /// Body landmarks and the person mask in every frame (ADR-0033).
    Pose,
    /// The subject's alpha in every frame (ADR-0033).
    Matte,
    /// Tempo, beats and downbeats of music.
    Beats,
    /// Loudness of consecutive windows.
    Levels,
}

/// What an analyzer reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    /// The audio stream.
    Audio,
    /// The video stream.
    Video,
}

impl Analyzer {
    /// Every analyzer.
    pub const ALL: [Analyzer; 8] = [
        Analyzer::Loudness,
        Analyzer::Silences,
        Analyzer::Scenes,
        Analyzer::Transcript,
        Analyzer::Pose,
        Analyzer::Matte,
        Analyzer::Beats,
        Analyzer::Levels,
    ];

    /// Stable name, as agents pass it.
    pub fn name(self) -> &'static str {
        match self {
            Analyzer::Loudness => "loudness",
            Analyzer::Silences => "silences",
            Analyzer::Scenes => "scenes",
            Analyzer::Transcript => "transcript",
            Analyzer::Pose => "pose",
            Analyzer::Matte => "matte",
            Analyzer::Beats => "beats",
            Analyzer::Levels => "levels",
        }
    }

    /// Whether it runs when no analyzers are named: loudness, silences and scenes. Speech,
    /// pose and mattes need installed models and take longer; beats and levels serve
    /// particular edits (music, shorts) and are named when wanted.
    pub fn by_default(self) -> bool {
        matches!(
            self,
            Analyzer::Loudness | Analyzer::Silences | Analyzer::Scenes
        )
    }

    /// Whether its results include a data file beside the cache entry, with what is too
    /// large for the result itself (per-frame landmarks and masks).
    pub fn has_data(self) -> bool {
        matches!(self, Analyzer::Pose | Analyzer::Matte)
    }

    /// The installed models it runs with `options`, besides ONNX Runtime for the vision
    /// models; speech models are chosen by [`decode::speech_model`].
    pub fn vision_model(self, options: &Options) -> Option<&'static str> {
        match self {
            Analyzer::Pose => Some(pose::MODEL),
            Analyzer::Matte => Some(options.matte.model.model()),
            _ => None,
        }
    }

    /// The analyzer called `name`.
    pub fn parse(name: &str) -> Option<Analyzer> {
        Analyzer::ALL.into_iter().find(|a| a.name() == name)
    }

    /// Bumped whenever results for the same input and options can change; part of the
    /// cache key.
    pub fn version(self) -> u32 {
        match self {
            // 2: background and loud levels leave digital silence out (ADR-0020).
            Analyzer::Transcript => 2,
            _ => 1,
        }
    }

    /// What it measures, for `capabilities`.
    pub fn summary(self) -> &'static str {
        match self {
            Analyzer::Loudness => {
                "Integrated loudness in LUFS (ITU-R BS.1770-4) and sample peak in dBFS."
            }
            Analyzer::Silences => {
                "Ranges whose level stays below a threshold for at least a minimum duration: pauses to cut or keep."
            }
            Analyzer::Scenes => "Hard shot cuts with their frame, time and confidence.",
            Analyzer::Transcript => {
                "Speech as words with timings, filler candidates and pauses, recognised locally by whisper.cpp; needs an installed speech model and must be named."
            }
            Analyzer::Pose => {
                "A person's 33 body landmarks and person mask in every frame, with MediaPipe Pose Landmarker, for the body effects (body.outline, body.glow, body.shadow, body.sparkle); needs the mediapipe-pose-full model and must be named."
            }
            Analyzer::Matte => {
                "The subject's alpha in every frame, for key.subject: model person (the pose model's person mask, fast) or subject (BiRefNet, any subject, 9 to 14 s a frame, so analyze every few frames with every); needs mediapipe-pose-full or birefnet-lite and must be named."
            }
            Analyzer::Beats => {
                "The tempo in bpm, every beat with its strength, and the downbeats when the accents show them, for cutting to music and beat markers; must be named."
            }
            Analyzer::Levels => {
                "K-weighted loudness in LUFS of consecutive windows (1 s by default), for loudness peaks and variation, such as when looking for highlights; must be named."
            }
        }
    }

    /// The stream it reads.
    pub fn needs(self) -> Needs {
        match self {
            Analyzer::Loudness
            | Analyzer::Silences
            | Analyzer::Transcript
            | Analyzer::Beats
            | Analyzer::Levels => Needs::Audio,
            Analyzer::Scenes | Analyzer::Pose | Analyzer::Matte => Needs::Video,
        }
    }
}

/// An analyzer as `capabilities` lists it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AnalyzerInfo {
    /// Name to pass, for example `silences`.
    pub name: &'static str,
    /// What it measures.
    pub summary: &'static str,
    /// The stream it reads: `audio` or `video`.
    pub reads: &'static str,
    /// Its version; results are cached per version.
    pub version: u32,
    /// The JSON Schema of its options, if it has any.
    pub options: serde_json::Value,
}

/// Every analyzer, for `capabilities`.
pub fn analyzers() -> Vec<AnalyzerInfo> {
    Analyzer::ALL
        .into_iter()
        .map(|analyzer| AnalyzerInfo {
            name: analyzer.name(),
            summary: analyzer.summary(),
            reads: match analyzer.needs() {
                Needs::Audio => "audio",
                Needs::Video => "video",
            },
            version: analyzer.version(),
            options: match analyzer {
                Analyzer::Loudness => serde_json::Value::Null,
                Analyzer::Silences => {
                    serde_json::to_value(schemars::schema_for!(SilenceOptions)).unwrap_or_default()
                }
                Analyzer::Scenes => {
                    serde_json::to_value(schemars::schema_for!(SceneOptions)).unwrap_or_default()
                }
                Analyzer::Transcript => {
                    serde_json::to_value(schemars::schema_for!(TranscriptOptions))
                        .unwrap_or_default()
                }
                Analyzer::Pose => {
                    serde_json::to_value(schemars::schema_for!(PoseOptions)).unwrap_or_default()
                }
                Analyzer::Matte => {
                    serde_json::to_value(schemars::schema_for!(MatteOptions)).unwrap_or_default()
                }
                Analyzer::Beats => {
                    serde_json::to_value(schemars::schema_for!(BeatOptions)).unwrap_or_default()
                }
                Analyzer::Levels => {
                    serde_json::to_value(schemars::schema_for!(LevelOptions)).unwrap_or_default()
                }
            },
        })
        .collect()
}

/// Options for every analyzer; each has defaults.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// Options of `silences`.
    #[serde(default)]
    pub silences: SilenceOptions,
    /// Options of `scenes`.
    #[serde(default)]
    pub scenes: SceneOptions,
    /// Options of `transcript`.
    #[serde(default)]
    pub transcript: TranscriptOptions,
    /// Options of `pose`.
    #[serde(default)]
    pub pose: PoseOptions,
    /// Options of `matte`.
    #[serde(default)]
    pub matte: MatteOptions,
    /// Options of `beats`.
    #[serde(default)]
    pub beats: BeatOptions,
    /// Options of `levels`.
    #[serde(default)]
    pub levels: LevelOptions,
}

impl Options {
    /// The options that affect `analyzer`'s results, as JSON for its cache key.
    pub fn for_analyzer(&self, analyzer: Analyzer) -> serde_json::Value {
        match analyzer {
            Analyzer::Loudness => serde_json::Value::Null,
            Analyzer::Silences => serde_json::to_value(&self.silences).unwrap_or_default(),
            Analyzer::Scenes => serde_json::to_value(&self.scenes).unwrap_or_default(),
            Analyzer::Transcript => serde_json::to_value(&self.transcript).unwrap_or_default(),
            Analyzer::Pose => serde_json::to_value(&self.pose).unwrap_or_default(),
            Analyzer::Matte => serde_json::to_value(self.matte.clamped()).unwrap_or_default(),
            Analyzer::Beats => serde_json::to_value(self.beats.clamped()).unwrap_or_default(),
            Analyzer::Levels => serde_json::to_value(self.levels.clamped()).unwrap_or_default(),
        }
    }
}

/// Results by analyzer; only the analyzers that ran are present.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Results {
    /// Loudness and peak.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness: Option<Loudness>,
    /// Quiet ranges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silences: Option<Silences>,
    /// Shot cuts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenes: Option<Scenes>,
    /// Words, fillers and pauses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<Transcript>,
    /// Body landmarks and person masks (boxed: results travel in job reports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<Box<Pose>>,
    /// Subject mattes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matte: Option<Box<Matte>>,
    /// Tempo, beats and downbeats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beats: Option<Beats>,
    /// Loudness of consecutive windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Levels>,
    /// The data files of analyzers measured now ([`Analyzer::has_data`]), to be written
    /// beside their cache entries; never part of the reply, which names them by hash.
    #[serde(skip)]
    #[schemars(skip)]
    pub data: std::collections::BTreeMap<Analyzer, std::sync::Arc<[u8]>>,
}

impl Results {
    /// The result of `analyzer` as JSON, if it ran.
    pub fn get(&self, analyzer: Analyzer) -> Option<serde_json::Value> {
        let value = match analyzer {
            Analyzer::Loudness => self.loudness.as_ref().map(serde_json::to_value),
            Analyzer::Silences => self.silences.as_ref().map(serde_json::to_value),
            Analyzer::Scenes => self.scenes.as_ref().map(serde_json::to_value),
            Analyzer::Transcript => self.transcript.as_ref().map(serde_json::to_value),
            Analyzer::Pose => self.pose.as_ref().map(serde_json::to_value),
            Analyzer::Matte => self.matte.as_ref().map(serde_json::to_value),
            Analyzer::Beats => self.beats.as_ref().map(serde_json::to_value),
            Analyzer::Levels => self.levels.as_ref().map(serde_json::to_value),
        };
        value.and_then(Result::ok)
    }

    /// Stores `value` as the result of `analyzer`; false if it does not parse.
    pub fn set(&mut self, analyzer: Analyzer, value: serde_json::Value) -> bool {
        match analyzer {
            Analyzer::Loudness => serde_json::from_value(value)
                .map(|r| self.loudness = Some(r))
                .is_ok(),
            Analyzer::Silences => serde_json::from_value(value)
                .map(|r| self.silences = Some(r))
                .is_ok(),
            Analyzer::Scenes => serde_json::from_value(value)
                .map(|r| self.scenes = Some(r))
                .is_ok(),
            Analyzer::Transcript => serde_json::from_value(value)
                .map(|r| self.transcript = Some(r))
                .is_ok(),
            Analyzer::Pose => serde_json::from_value(value)
                .map(|r| self.pose = Some(r))
                .is_ok(),
            Analyzer::Matte => serde_json::from_value(value)
                .map(|r| self.matte = Some(r))
                .is_ok(),
            Analyzer::Beats => serde_json::from_value(value)
                .map(|r| self.beats = Some(r))
                .is_ok(),
            Analyzer::Levels => serde_json::from_value(value)
                .map(|r| self.levels = Some(r))
                .is_ok(),
        }
    }
}

/// Why an analysis could not finish.
#[derive(Debug, Error)]
pub enum AnalysisError {
    /// An audio analyzer was asked for a file without audio.
    #[error("the file has no audio stream")]
    NoAudio,
    /// A video analyzer was asked for a file without video.
    #[error("the file has no video stream")]
    NoVideo,
    /// The caller cancelled through the progress callback.
    #[error("the analysis was cancelled")]
    Cancelled,
    /// Decoding failed.
    #[error(transparent)]
    Media(#[from] fosscut_media::MediaError),
    /// The speech model is unknown or not installed.
    #[error(transparent)]
    Model(#[from] models::ModelError),
    /// Speech recognition failed.
    #[error("speech recognition failed: {0}")]
    Speech(String),
    /// This build has no speech recognition (the `speech` feature is off).
    #[error("this build of FossCut has no speech recognition")]
    SpeechUnavailable,
    /// ONNX Runtime is missing, or a vision model could not be loaded into it.
    #[error(transparent)]
    Onnx(#[from] onnx::OnnxError),
    /// A vision model failed while running.
    #[error("the model failed: {0}")]
    Inference(String),
    /// This build has no ONNX Runtime support (the `onnx` feature is off).
    #[error("this build of FossCut has no ONNX Runtime support")]
    OnnxUnavailable,
}

/// How far an analysis has come: seconds of audio and frames of video done, of the total.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// Units done.
    pub done: u64,
    /// Units in total (an estimate for video).
    pub total: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzers_have_stable_names_and_needs() {
        for analyzer in Analyzer::ALL {
            assert_eq!(Analyzer::parse(analyzer.name()), Some(analyzer));
            assert!(analyzer.summary().ends_with('.'));
            assert!(analyzer.version() >= 1);
        }
        assert_eq!(Analyzer::parse("faces"), None);
        let listed = analyzers();
        assert_eq!(listed.len(), Analyzer::ALL.len());
        assert!(listed[0].options.is_null());
        assert!(listed[1].options["properties"]["threshold_dbfs"].is_object());
        assert_eq!(listed[2].reads, "video");
        assert!(listed[3].options["properties"]["model"].is_object());
        assert!(listed[4].options["properties"]["max_fps"].is_object());
        assert!(listed[5].options["properties"]["every"].is_object());
        assert!(listed[6].options["properties"]["tightness"].is_object());
        assert!(listed[7].options["properties"]["window_ms"].is_object());
        assert!(!Analyzer::Beats.by_default() && !Analyzer::Levels.by_default());
        assert_eq!(Analyzer::Beats.needs(), Needs::Audio);
        assert_eq!(Analyzer::Levels.vision_model(&Options::default()), None);
        let mut fixed = Options::default();
        fixed.beats.bpm = Some(1e6);
        assert_eq!(fixed.for_analyzer(Analyzer::Beats)["bpm"], 400.0);
        assert_eq!(fixed.for_analyzer(Analyzer::Levels)["window_ms"], 1000);
        assert!(!Analyzer::Transcript.by_default() && Analyzer::Scenes.by_default());
        assert!(!Analyzer::Pose.by_default() && !Analyzer::Matte.by_default());
        assert!(Analyzer::Pose.has_data() && !Analyzer::Scenes.has_data());
        assert_eq!(Analyzer::Matte.needs(), Needs::Video);
        let mut subject = Options::default();
        subject.matte.model = MatteModel::Subject;
        assert_eq!(
            Analyzer::Matte.vision_model(&subject),
            Some("birefnet-lite")
        );
        assert_eq!(
            Analyzer::Pose.vision_model(&subject),
            Some("mediapipe-pose-full")
        );
        assert_eq!(Analyzer::Loudness.vision_model(&subject), None);
        assert_eq!(
            Options::default().for_analyzer(Analyzer::Matte)["model"],
            "person"
        );
        assert_eq!(subject.for_analyzer(Analyzer::Matte)["model"], "subject");
        assert_eq!(Analyzer::Scenes.needs(), Needs::Video);
        assert_eq!(
            serde_json::to_value(Analyzer::Silences).unwrap(),
            "silences"
        );
    }

    #[test]
    fn results_round_trip_by_analyzer() {
        let mut results = Results::default();
        assert!(results.get(Analyzer::Loudness).is_none());
        let loudness = Loudness {
            integrated_lufs: Some(-23.0),
            sample_peak_dbfs: Some(-20.0),
        };
        assert!(results.set(Analyzer::Loudness, serde_json::to_value(&loudness).unwrap()));
        assert_eq!(results.loudness, Some(loudness));
        assert!(!results.set(Analyzer::Scenes, serde_json::json!({ "nonsense": true })));
        assert!(results.get(Analyzer::Loudness).is_some());
        let options = Options::default();
        assert!(options.for_analyzer(Analyzer::Loudness).is_null());
        assert_eq!(
            options.for_analyzer(Analyzer::Silences)["threshold_dbfs"],
            -40.0
        );
    }
}
