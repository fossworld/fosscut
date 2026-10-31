use thiserror::Error;

/// Errors from probing and decoding media.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MediaError {
    /// The file could not be opened or is not a media file FossCut can read.
    #[error("cannot open {path}: {message}")]
    OpenFailed {
        /// The file that was requested.
        path: String,
        /// What the backend reported.
        message: String,
    },
    /// The file has no video stream.
    #[error("{path} has no video stream")]
    NoVideoStream {
        /// The file that was requested.
        path: String,
    },
    /// No decoder is available for the stream's codec.
    #[error("no decoder for codec {codec}")]
    DecoderUnavailable {
        /// Codec name as reported by the backend.
        codec: String,
    },
    /// Hardware decoding was required but is not available for this stream.
    #[error("hardware decoding is not available: {message}")]
    HardwareUnavailable {
        /// Why hardware decoding could not be used.
        message: String,
    },
    /// Decoding failed part way through the stream.
    #[error("decoding failed: {message}")]
    DecodeFailed {
        /// What the backend reported.
        message: String,
    },
    /// Seeking failed.
    #[error("seeking failed: {message}")]
    SeekFailed {
        /// What the backend reported.
        message: String,
    },
    /// The file has no audio stream.
    #[error("{path} has no audio stream")]
    NoAudioStream {
        /// The file that was requested.
        path: String,
    },
    /// No encoder on this machine can produce the requested codec.
    #[error("this machine cannot encode {codec}; it can encode: {}", available.join(", "))]
    EncoderUnavailable {
        /// The requested codec.
        codec: String,
        /// Codecs this machine can encode instead.
        available: Vec<String>,
    },
    /// The requested combination of settings is not possible.
    #[error("unsupported output settings: {message}")]
    Unsupported {
        /// What is not possible.
        message: String,
    },
    /// Writing the output file failed.
    #[error("cannot write {path}: {message}")]
    WriteFailed {
        /// The output file.
        path: String,
        /// What the backend reported.
        message: String,
    },
    /// Encoding failed part way through.
    #[error("encoding failed: {message}")]
    EncodeFailed {
        /// What the backend reported.
        message: String,
    },
    /// This FFmpeg build has no input format of this name, so it cannot record from it.
    #[error("this FFmpeg build has no {format} input; its capture inputs are: {}", available.join(", "))]
    InputFormatUnavailable {
        /// The requested input format, for example `avfoundation`.
        format: String,
        /// The capture input formats this build has instead.
        available: Vec<String>,
    },
    /// A capture device could not be opened or stopped delivering media: it does not
    /// exist, is in use, or recording from it is not permitted.
    #[error("cannot record from {input} ({format}): {message}")]
    DeviceUnavailable {
        /// The input format.
        format: String,
        /// What the format calls the device.
        input: String,
        /// What the backend reported.
        message: String,
    },
    /// A recording ended before its devices delivered the media it needs.
    #[error("nothing was recorded: {message}")]
    NothingRecorded {
        /// What was missing.
        message: String,
    },
    /// The caller asked the work to stop; nothing complete was written.
    #[error("the work was cancelled")]
    Cancelled,
    /// The requested time is after the last frame of the stream.
    #[error("no frame at {requested_flicks} flicks: the stream ends at {end_flicks} flicks")]
    EndOfStream {
        /// The requested time in flicks.
        requested_flicks: i64,
        /// The end of the last decoded frame in flicks.
        end_flicks: i64,
    },
}

impl MediaError {
    /// The stable error code used in FossCut API responses (ADR-0006).
    pub fn code(&self) -> &'static str {
        match self {
            MediaError::OpenFailed { .. } => "fosscut.media.open_failed",
            MediaError::NoVideoStream { .. } => "fosscut.media.no_video_stream",
            MediaError::DecoderUnavailable { .. } => "fosscut.media.decoder_unavailable",
            MediaError::HardwareUnavailable { .. } => "fosscut.media.hardware_unavailable",
            MediaError::DecodeFailed { .. } => "fosscut.media.decode_failed",
            MediaError::SeekFailed { .. } => "fosscut.media.seek_failed",
            MediaError::EndOfStream { .. } => "fosscut.media.end_of_stream",
            MediaError::NoAudioStream { .. } => "fosscut.media.no_audio_stream",
            MediaError::EncoderUnavailable { .. } => "fosscut.media.encoder_unavailable",
            MediaError::Unsupported { .. } => "fosscut.media.unsupported",
            MediaError::WriteFailed { .. } => "fosscut.media.write_failed",
            MediaError::EncodeFailed { .. } => "fosscut.media.encode_failed",
            MediaError::InputFormatUnavailable { .. } => "fosscut.media.input_format_unavailable",
            MediaError::DeviceUnavailable { .. } => "fosscut.media.device_unavailable",
            MediaError::NothingRecorded { .. } => "fosscut.media.nothing_recorded",
            MediaError::Cancelled => "fosscut.media.cancelled",
        }
    }
}
