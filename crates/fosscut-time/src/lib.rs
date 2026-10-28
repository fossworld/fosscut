//! Exact time types for FossCut (ADR-0005).
//!
//! Timeline positions and durations are [`Flicks`]: integer counts of 1/705,600,000
//! of a second. Every common video rate (including the NTSC 1000/1001 rates) and every
//! common audio sample rate divides a second of flicks exactly, so frame arithmetic
//! never drifts. Frame rates are exact rationals ([`Rate`]). At the API boundary a
//! time must carry an explicit unit ([`TimeInput`]); responses report all units at
//! once ([`TimeReport`]).
//!
//! All time arithmetic in FossCut goes through this crate. It has no native
//! dependencies and builds for `wasm32-unknown-unknown`.

mod error;
mod flicks;
mod input;
mod rate;
mod report;
mod timecode;

pub use error::TimeError;
pub use flicks::{FLICKS_PER_SECOND, Flicks};
pub use input::TimeInput;
pub use rate::Rate;
pub use report::TimeReport;
pub use timecode::{format_timecode, parse_timecode};

/// Rounds `numerator / denominator` to the nearest integer, halves rounding up.
///
/// `denominator` must be positive.
pub(crate) fn div_round(numerator: i128, denominator: i128) -> i128 {
    debug_assert!(denominator > 0);
    (2 * numerator + denominator).div_euclid(2 * denominator)
}
