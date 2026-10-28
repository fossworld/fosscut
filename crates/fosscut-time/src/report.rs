use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Flicks, Rate, format_timecode};

/// A time reported in every unit at once, so agents never have to convert (ADR-0005).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TimeReport {
    /// Frame index at the timeline rate (the frame containing this time).
    pub frames: i64,
    /// SMPTE timecode at the timeline rate.
    pub timecode: String,
    /// Decimal seconds rounded to microseconds.
    pub seconds: String,
    /// Exact value in flicks.
    pub flicks: Flicks,
}

impl TimeReport {
    /// Describes `time` at `rate`.
    pub fn new(time: Flicks, rate: Rate) -> TimeReport {
        let frames = rate.frame_at(time);
        TimeReport {
            frames,
            timecode: format_timecode(frames, rate),
            seconds: time.to_seconds_string(),
            flicks: time,
        }
    }
}
