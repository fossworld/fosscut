use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{FLICKS_PER_SECOND, Flicks, TimeError, div_round};

/// An exact frame rate in frames per second, stored as a reduced fraction.
///
/// Serialized as a string: `"30"` or `"30000/1001"`. Integers are accepted when
/// deserializing. Decimal rates such as `29.97` are rejected because they are inexact.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rate {
    num: u32,
    den: u32,
}

const fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

impl Rate {
    /// 24 frames per second.
    pub const FPS_24: Rate = Rate { num: 24, den: 1 };
    /// 25 frames per second.
    pub const FPS_25: Rate = Rate { num: 25, den: 1 };
    /// 30 frames per second.
    pub const FPS_30: Rate = Rate { num: 30, den: 1 };
    /// 60 frames per second.
    pub const FPS_60: Rate = Rate { num: 60, den: 1 };
    /// NTSC 23.976 (24000/1001) frames per second.
    pub const NTSC_24: Rate = Rate {
        num: 24000,
        den: 1001,
    };
    /// NTSC 29.97 (30000/1001) frames per second.
    pub const NTSC_30: Rate = Rate {
        num: 30000,
        den: 1001,
    };
    /// NTSC 59.94 (60000/1001) frames per second.
    pub const NTSC_60: Rate = Rate {
        num: 60000,
        den: 1001,
    };

    /// Creates a rate `num / den`, reduced to lowest terms.
    pub fn new(num: u32, den: u32) -> Result<Rate, TimeError> {
        if num == 0 || den == 0 {
            return Err(TimeError::InvalidRate {
                input: format!("{num}/{den}"),
                reason: "numerator and denominator must be positive",
            });
        }
        let divisor = gcd(num, den);
        Ok(Rate {
            num: num / divisor,
            den: den / divisor,
        })
    }

    /// Numerator of the reduced fraction.
    pub const fn num(self) -> u32 {
        self.num
    }

    /// Denominator of the reduced fraction.
    pub const fn den(self) -> u32 {
        self.den
    }

    /// True when one frame is a whole number of flicks, which holds for every common rate.
    pub fn is_exact(self) -> bool {
        (i128::from(FLICKS_PER_SECOND) * i128::from(self.den)) % i128::from(self.num) == 0
    }

    /// True for the NTSC rates that use drop-frame timecode (30000/1001 and 60000/1001).
    pub fn is_drop_frame(self) -> bool {
        self.den == 1001 && (self.num == 30000 || self.num == 60000)
    }

    /// Whole frames per timecode second (30 for 29.97, 24 for 23.976).
    pub fn nominal_fps(self) -> u32 {
        self.num.div_ceil(self.den)
    }

    /// Duration of one frame (rounded to the nearest flick for exotic rates).
    pub fn frame_duration(self) -> Flicks {
        self.frames_to_flicks(1)
    }

    /// Start time of frame `frames`, rounded to the nearest flick for exotic rates.
    pub fn frames_to_flicks(self, frames: i64) -> Flicks {
        let value = div_round(
            i128::from(frames) * i128::from(FLICKS_PER_SECOND) * i128::from(self.den),
            i128::from(self.num),
        );
        Flicks(value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
    }

    /// Index of the frame that contains `time` (rounds toward negative infinity).
    pub fn frame_at(self, time: Flicks) -> i64 {
        let value = (i128::from(time.0) * i128::from(self.num))
            .div_euclid(i128::from(FLICKS_PER_SECOND) * i128::from(self.den));
        value as i64
    }

    /// Index of the frame boundary nearest to `time`.
    pub fn nearest_frame(self, time: Flicks) -> i64 {
        div_round(
            i128::from(time.0) * i128::from(self.num),
            i128::from(FLICKS_PER_SECOND) * i128::from(self.den),
        ) as i64
    }

    /// `time` moved to the nearest frame boundary.
    pub fn snap(self, time: Flicks) -> Flicks {
        self.frames_to_flicks(self.nearest_frame(time))
    }

    /// True when `time` lies exactly on a frame boundary.
    pub fn is_on_frame(self, time: Flicks) -> bool {
        self.frames_to_flicks(self.frame_at(time)) == time
    }
}

impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl FromStr for Rate {
    type Err = TimeError;

    fn from_str(input: &str) -> Result<Rate, TimeError> {
        let invalid = |reason| TimeError::InvalidRate {
            input: input.to_string(),
            reason,
        };
        let text = input.trim();
        let (num, den) = match text.split_once('/') {
            Some((n, d)) => (n.trim(), d.trim()),
            None => (text, "1"),
        };
        if num.contains('.') || den.contains('.') {
            return Err(invalid(
                "use an exact fraction such as 30000/1001 instead of a decimal",
            ));
        }
        let num: u32 = num
            .parse()
            .map_err(|_| invalid("expected an integer or a fraction such as 30000/1001"))?;
        let den: u32 = den
            .parse()
            .map_err(|_| invalid("expected an integer or a fraction such as 30000/1001"))?;
        Rate::new(num, den)
    }
}

impl Serialize for Rate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Rate, D::Error> {
        struct RateVisitor;

        impl serde::de::Visitor<'_> for RateVisitor {
            type Value = Rate;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a frame rate such as \"30\" or \"30000/1001\"")
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Rate, E> {
                let num = u32::try_from(value).map_err(E::custom)?;
                Rate::new(num, 1).map_err(E::custom)
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Rate, E> {
                let num = u32::try_from(value).map_err(E::custom)?;
                Rate::new(num, 1).map_err(E::custom)
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Rate, E> {
                value.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_any(RateVisitor)
    }
}

impl JsonSchema for Rate {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Rate".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "Exact frame rate as an integer or a fraction string, for example \"30\" or \"30000/1001\" (29.97). Decimals are not accepted.",
            "oneOf": [
                { "type": "string", "pattern": "^[1-9][0-9]*(/[1-9][0-9]*)?$" },
                { "type": "integer", "minimum": 1 }
            ]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_reduce_and_display() {
        assert_eq!(Rate::new(60, 2).unwrap(), Rate::FPS_30);
        assert_eq!(Rate::NTSC_30.to_string(), "30000/1001");
        assert_eq!("30".parse::<Rate>().unwrap(), Rate::FPS_30);
        assert!("29.97".parse::<Rate>().is_err());
        assert!("0".parse::<Rate>().is_err());
    }

    #[test]
    fn common_rates_are_exact() {
        for rate in [24, 25, 30, 48, 50, 60, 90, 100, 120] {
            assert!(Rate::new(rate, 1).unwrap().is_exact(), "{rate} fps");
        }
        for rate in [Rate::NTSC_24, Rate::NTSC_30, Rate::NTSC_60] {
            assert!(rate.is_exact(), "{rate}");
        }
    }

    #[test]
    fn frame_boundaries_are_exact_at_30_fps() {
        let rate = Rate::FPS_30;
        assert_eq!(rate.frame_duration(), Flicks(23_520_000));
        let split = rate.frames_to_flicks(3 * 30 + 28);
        assert_eq!(split, Flicks(2_775_360_000));
        assert_eq!(rate.frame_at(split), 118);
        assert!(rate.is_on_frame(split));
        assert_eq!(rate.snap(Flicks(split.0 + 1000)), split);
    }

    #[test]
    fn serde_uses_strings_and_accepts_integers() {
        assert_eq!(
            serde_json::to_string(&Rate::NTSC_30).unwrap(),
            "\"30000/1001\""
        );
        assert_eq!(serde_json::from_str::<Rate>("30").unwrap(), Rate::FPS_30);
        assert_eq!(
            serde_json::from_str::<Rate>("\"24000/1001\"").unwrap(),
            Rate::NTSC_24
        );
        assert!(serde_json::from_str::<Rate>("29.97").is_err());
    }
}
