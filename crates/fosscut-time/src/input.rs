use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Flicks, Rate, TimeError, parse_timecode};

/// A time value as given by a user or agent, always with an explicit unit (ADR-0005).
///
/// JSON forms: `{"frames": 118}`, `{"seconds": "3.9333"}` (a number is also accepted),
/// `{"timecode": "00:00:03:28"}` and `{"flicks": 2775360000}`. Command-line forms:
/// `118f`, `3.9333s`, `00:00:03:28` and `2775360000fl`. Bare numbers are rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimeInput {
    /// A frame index or frame count at the timeline rate.
    Frames(i64),
    /// Decimal seconds, kept as text so that parsing is exact.
    Seconds(String),
    /// SMPTE timecode at the timeline rate.
    Timecode(String),
    /// Raw flicks.
    Flicks(i64),
}

impl TimeInput {
    /// Converts the input to flicks using `rate` for frame-based units.
    pub fn resolve(&self, rate: Rate) -> Result<Flicks, TimeError> {
        match self {
            TimeInput::Frames(frames) => Ok(rate.frames_to_flicks(*frames)),
            TimeInput::Seconds(text) => Flicks::parse_seconds(text),
            TimeInput::Timecode(text) => Ok(rate.frames_to_flicks(parse_timecode(text, rate)?)),
            TimeInput::Flicks(value) => Ok(Flicks(*value)),
        }
    }
}

impl fmt::Display for TimeInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeInput::Frames(n) => write!(f, "{n}f"),
            TimeInput::Seconds(s) => write!(f, "{s}s"),
            TimeInput::Timecode(t) => write!(f, "{t}"),
            TimeInput::Flicks(n) => write!(f, "{n}fl"),
        }
    }
}

impl FromStr for TimeInput {
    type Err = TimeError;

    fn from_str(input: &str) -> Result<TimeInput, TimeError> {
        let text = input.trim();
        let unit_required = || TimeError::UnitRequired {
            input: input.to_string(),
        };
        if text.contains(':') || text.contains(';') {
            return Ok(TimeInput::Timecode(text.to_string()));
        }
        let lower = text.to_ascii_lowercase();
        if let Some(number) = lower.strip_suffix("fl") {
            return number
                .trim()
                .parse()
                .map(TimeInput::Flicks)
                .map_err(|_| unit_required());
        }
        if let Some(number) = lower.strip_suffix('f') {
            return number
                .trim()
                .parse()
                .map(TimeInput::Frames)
                .map_err(|_| unit_required());
        }
        if let Some(number) = lower.strip_suffix('s') {
            let number = number.trim();
            Flicks::parse_seconds(number)?;
            return Ok(TimeInput::Seconds(number.to_string()));
        }
        Err(unit_required())
    }
}

impl Serialize for TimeInput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            TimeInput::Frames(n) => map.serialize_entry("frames", n)?,
            TimeInput::Seconds(s) => map.serialize_entry("seconds", s)?,
            TimeInput::Timecode(t) => map.serialize_entry("timecode", t)?,
            TimeInput::Flicks(n) => map.serialize_entry("flicks", n)?,
        }
        map.end()
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SecondsValue {
    Text(String),
    Number(serde_json_number::Number),
}

/// Minimal number holder so that `{"seconds": 1.5}` keeps its decimal text.
mod serde_json_number {
    use serde::{Deserialize, Deserializer};

    pub struct Number(pub String);

    impl<'de> Deserialize<'de> for Number {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Number, D::Error> {
            struct NumberVisitor;
            impl serde::de::Visitor<'_> for NumberVisitor {
                type Value = Number;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("a number")
                }
                fn visit_i64<E>(self, v: i64) -> Result<Number, E> {
                    Ok(Number(v.to_string()))
                }
                fn visit_u64<E>(self, v: u64) -> Result<Number, E> {
                    Ok(Number(v.to_string()))
                }
                fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Number, E> {
                    if v.is_finite() {
                        Ok(Number(v.to_string()))
                    } else {
                        Err(E::custom("not a finite number"))
                    }
                }
            }
            deserializer.deserialize_any(NumberVisitor)
        }
    }
}

impl<'de> Deserialize<'de> for TimeInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<TimeInput, D::Error> {
        struct InputVisitor;

        fn unit_required<E: de::Error>(shown: String) -> E {
            E::custom(TimeError::UnitRequired { input: shown })
        }

        impl<'de> Visitor<'de> for InputVisitor {
            type Value = TimeInput;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a time with an explicit unit: {\"frames\": n}, {\"seconds\": \"s\"}, {\"timecode\": \"HH:MM:SS:FF\"} or {\"flicks\": n}")
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<TimeInput, E> {
                Err(unit_required(v.to_string()))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<TimeInput, E> {
                Err(unit_required(v.to_string()))
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<TimeInput, E> {
                Err(unit_required(v.to_string()))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<TimeInput, E> {
                v.parse().map_err(E::custom)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<TimeInput, A::Error> {
                let key: String = map
                    .next_key()?
                    .ok_or_else(|| de::Error::custom("empty time object"))?;
                let value = match key.as_str() {
                    "frames" => TimeInput::Frames(map.next_value()?),
                    "flicks" => TimeInput::Flicks(map.next_value()?),
                    "timecode" => TimeInput::Timecode(map.next_value()?),
                    "seconds" => {
                        let text = match map.next_value::<SecondsValue>()? {
                            SecondsValue::Text(t) => t,
                            SecondsValue::Number(n) => n.0,
                        };
                        Flicks::parse_seconds(&text).map_err(de::Error::custom)?;
                        TimeInput::Seconds(text)
                    }
                    other => {
                        return Err(de::Error::unknown_field(
                            other,
                            &["frames", "seconds", "timecode", "flicks"],
                        ));
                    }
                };
                if map.next_key::<String>()?.is_some() {
                    return Err(de::Error::custom(
                        "a time object must have exactly one unit key",
                    ));
                }
                Ok(value)
            }
        }

        deserializer.deserialize_any(InputVisitor)
    }
}

impl JsonSchema for TimeInput {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TimeInput".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "A time with an explicit unit. Frame and timecode values use the timeline frame rate. Bare numbers are rejected.",
            "oneOf": [
                { "type": "object", "properties": { "frames": { "type": "integer" } }, "required": ["frames"], "additionalProperties": false },
                { "type": "object", "properties": { "seconds": { "type": ["string", "number"], "description": "Decimal seconds, for example \"3.9333\"" } }, "required": ["seconds"], "additionalProperties": false },
                { "type": "object", "properties": { "timecode": { "type": "string", "description": "HH:MM:SS:FF, or HH:MM:SS;FF for drop-frame" } }, "required": ["timecode"], "additionalProperties": false },
                { "type": "object", "properties": { "flicks": { "type": "integer" } }, "required": ["flicks"], "additionalProperties": false },
                { "type": "string", "description": "Shorthand: 118f, 3.9333s, 00:00:03:28 or 2775360000fl" }
            ]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_forms_resolve_to_the_same_time() {
        let rate = Rate::FPS_30;
        let expected = rate.frames_to_flicks(118);
        for json in [
            r#"{"frames": 118}"#,
            r#"{"timecode": "00:00:03:28"}"#,
            r#"{"flicks": 2775360000}"#,
            r#""118f""#,
            r#""00:00:03:28""#,
        ] {
            let input: TimeInput = serde_json::from_str(json).unwrap();
            assert_eq!(input.resolve(rate).unwrap(), expected, "{json}");
        }
        let seconds: TimeInput = serde_json::from_str(r#"{"seconds": 1.5}"#).unwrap();
        assert_eq!(seconds, TimeInput::Seconds("1.5".into()));
    }

    #[test]
    fn bare_numbers_are_rejected_with_a_helpful_message() {
        let error = serde_json::from_str::<TimeInput>("118")
            .unwrap_err()
            .to_string();
        assert!(error.contains("explicit unit"), "{error}");
        assert!(matches!(
            "118".parse::<TimeInput>(),
            Err(TimeError::UnitRequired { .. })
        ));
    }

    #[test]
    fn cli_shorthand() {
        assert_eq!("118f".parse::<TimeInput>().unwrap(), TimeInput::Frames(118));
        assert_eq!(
            "3.9333s".parse::<TimeInput>().unwrap(),
            TimeInput::Seconds("3.9333".into())
        );
        assert_eq!(
            "2775360000fl".parse::<TimeInput>().unwrap(),
            TimeInput::Flicks(2_775_360_000)
        );
        assert_eq!(
            "00:00:03;28".parse::<TimeInput>().unwrap(),
            TimeInput::Timecode("00:00:03;28".into())
        );
    }

    #[test]
    fn serializes_to_single_key_objects() {
        assert_eq!(
            serde_json::to_string(&TimeInput::Frames(118)).unwrap(),
            r#"{"frames":118}"#
        );
    }
}
