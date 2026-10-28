use thiserror::Error;

/// Errors produced while parsing, converting or validating time values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TimeError {
    /// A frame rate was zero, malformed or not a positive rational.
    #[error("invalid frame rate {input:?}: {reason}")]
    InvalidRate {
        /// The rejected input.
        input: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// A time value had no explicit unit.
    #[error(
        "time {input:?} needs an explicit unit: frames (118f or {{\"frames\": 118}}), seconds (3.9333s or {{\"seconds\": \"3.9333\"}}), timecode (00:00:03:28 or {{\"timecode\": \"00:00:03:28\"}}) or flicks (2775360000fl or {{\"flicks\": 2775360000}})"
    )]
    UnitRequired {
        /// The rejected input.
        input: String,
    },
    /// A timecode string could not be parsed for the given rate.
    #[error("invalid timecode {input:?}: {reason}")]
    InvalidTimecode {
        /// The rejected input.
        input: String,
        /// Why it was rejected.
        reason: String,
    },
    /// A seconds value could not be parsed.
    #[error("invalid seconds value {input:?}: {reason}")]
    InvalidSeconds {
        /// The rejected input.
        input: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// The result does not fit the time range.
    #[error("time value out of range")]
    Overflow,
}

impl TimeError {
    /// The stable error code used in FossCut API responses (ADR-0006).
    pub fn code(&self) -> &'static str {
        match self {
            TimeError::InvalidRate { .. } => "fosscut.time.invalid_rate",
            TimeError::UnitRequired { .. } => "fosscut.time.unit_required",
            TimeError::InvalidTimecode { .. } => "fosscut.time.invalid_timecode",
            TimeError::InvalidSeconds { .. } => "fosscut.time.invalid_seconds",
            TimeError::Overflow => "fosscut.time.overflow",
        }
    }
}
