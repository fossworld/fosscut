use std::ops::{Add, Neg, Sub};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{TimeError, div_round};

/// Number of flicks in one second.
pub const FLICKS_PER_SECOND: i64 = 705_600_000;

/// A point or duration on a timeline, in flicks (1/705,600,000 of a second).
///
/// Serialized as a plain integer.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Flicks(pub i64);

impl Flicks {
    /// Zero time.
    pub const ZERO: Flicks = Flicks(0);

    /// Creates a value from a raw flick count.
    pub const fn new(value: i64) -> Self {
        Flicks(value)
    }

    /// The raw flick count.
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Whole milliseconds as flicks; exact, since a millisecond is 705,600 flicks.
    /// Saturates on overflow.
    pub const fn from_millis(millis: i64) -> Self {
        Flicks(millis.saturating_mul(FLICKS_PER_SECOND / 1000))
    }

    /// Whole seconds as flicks, or `None` on overflow.
    pub fn from_seconds(seconds: i64) -> Option<Self> {
        seconds.checked_mul(FLICKS_PER_SECOND).map(Flicks)
    }

    /// Checked addition.
    pub fn checked_add(self, other: Flicks) -> Option<Flicks> {
        self.0.checked_add(other.0).map(Flicks)
    }

    /// Checked subtraction.
    pub fn checked_sub(self, other: Flicks) -> Option<Flicks> {
        self.0.checked_sub(other.0).map(Flicks)
    }

    /// True when the value is greater than zero.
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// True when the value is less than zero.
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Approximate value in seconds, for display and logging only.
    pub fn as_seconds_f64(self) -> f64 {
        self.0 as f64 / FLICKS_PER_SECOND as f64
    }

    /// Decimal seconds rounded to microseconds, without trailing zeros (`"3.933333"`, `"8"`).
    pub fn to_seconds_string(self) -> String {
        let micros = div_round(
            i128::from(self.0).abs() * 1_000_000,
            i128::from(FLICKS_PER_SECOND),
        );
        let whole = micros / 1_000_000;
        let fraction = micros % 1_000_000;
        let sign = if self.0 < 0 && micros != 0 { "-" } else { "" };
        if fraction == 0 {
            format!("{sign}{whole}")
        } else {
            let digits = format!("{fraction:06}");
            format!("{sign}{whole}.{}", digits.trim_end_matches('0'))
        }
    }

    /// Parses a decimal seconds string (`"3.9333"`, `"-1.5"`, `"12"`) exactly, rounding to
    /// the nearest flick.
    pub fn parse_seconds(input: &str) -> Result<Flicks, TimeError> {
        let invalid = |reason| TimeError::InvalidSeconds {
            input: input.to_string(),
            reason,
        };
        let text = input.trim();
        let (negative, unsigned) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let (whole, fraction) = match unsigned.split_once('.') {
            Some((w, f)) => (w, f),
            None => (unsigned, ""),
        };
        if whole.is_empty() && fraction.is_empty() {
            return Err(invalid("expected digits"));
        }
        if !whole.chars().all(|c| c.is_ascii_digit())
            || !fraction.chars().all(|c| c.is_ascii_digit())
        {
            return Err(invalid("expected a decimal number such as 3.9333"));
        }
        if whole.len() > 18 || fraction.len() > 18 {
            return Err(invalid("too many digits"));
        }
        let whole_value: i128 = if whole.is_empty() {
            0
        } else {
            whole.parse().map_err(|_| invalid("expected digits"))?
        };
        let fraction_value: i128 = if fraction.is_empty() {
            0
        } else {
            fraction.parse().map_err(|_| invalid("expected digits"))?
        };
        let scale = 10_i128.pow(fraction.len() as u32);
        let total = whole_value
            .checked_mul(scale)
            .and_then(|v| v.checked_add(fraction_value))
            .and_then(|v| v.checked_mul(i128::from(FLICKS_PER_SECOND)))
            .ok_or(TimeError::Overflow)?;
        let flicks = div_round(total, scale);
        let signed = if negative { -flicks } else { flicks };
        i64::try_from(signed)
            .map(Flicks)
            .map_err(|_| TimeError::Overflow)
    }

    /// Converts floating-point seconds, rounding to the nearest flick.
    pub fn from_seconds_f64(seconds: f64) -> Result<Flicks, TimeError> {
        if !seconds.is_finite() {
            return Err(TimeError::InvalidSeconds {
                input: seconds.to_string(),
                reason: "not a finite number",
            });
        }
        let value = (seconds * FLICKS_PER_SECOND as f64).round();
        if value.abs() >= i64::MAX as f64 {
            return Err(TimeError::Overflow);
        }
        Ok(Flicks(value as i64))
    }
}

impl Add for Flicks {
    type Output = Flicks;
    fn add(self, rhs: Flicks) -> Flicks {
        Flicks(self.0 + rhs.0)
    }
}

impl Sub for Flicks {
    type Output = Flicks;
    fn sub(self, rhs: Flicks) -> Flicks {
        Flicks(self.0 - rhs.0)
    }
}

impl Neg for Flicks {
    type Output = Flicks;
    fn neg(self) -> Flicks {
        Flicks(-self.0)
    }
}

impl JsonSchema for Flicks {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Flicks".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "integer",
            "description": "Time in flicks: 1/705,600,000 of a second (ADR-0005). 30 fps frames are 23,520,000 flicks long."
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_strings_round_trip_common_values() {
        assert_eq!(
            Flicks::parse_seconds("8").unwrap(),
            Flicks(8 * FLICKS_PER_SECOND)
        );
        assert_eq!(Flicks::parse_seconds("1.5").unwrap(), Flicks(1_058_400_000));
        assert_eq!(Flicks::parse_seconds(".5").unwrap(), Flicks(352_800_000));
        assert_eq!(
            Flicks::parse_seconds("-0.25").unwrap(),
            Flicks(-176_400_000)
        );
        assert_eq!(Flicks(8 * FLICKS_PER_SECOND).to_seconds_string(), "8");
        assert_eq!(Flicks(2_775_360_000).to_seconds_string(), "3.933333");
        assert_eq!(Flicks(-176_400_000).to_seconds_string(), "-0.25");
    }

    #[test]
    fn malformed_seconds_are_rejected() {
        for bad in ["", ".", "1.2.3", "abc", "1e3", "1,5"] {
            assert!(Flicks::parse_seconds(bad).is_err(), "{bad:?} should fail");
        }
    }
}
