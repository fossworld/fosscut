use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Crockford base32 alphabet in lowercase (no i, l, o, u).
const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// The kind of object an [`Id`] names; it becomes the id prefix.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum IdKind {
    /// A project (`project_…`).
    Project,
    /// A timeline (`timeline_…`).
    Timeline,
    /// A track (`track_…`).
    Track,
    /// A clip (`clip_…`).
    Clip,
    /// A media asset (`asset_…`).
    Asset,
    /// An effect on a clip (`effect_…`).
    Effect,
}

impl IdKind {
    /// The id prefix for this kind.
    pub const fn prefix(self) -> &'static str {
        match self {
            IdKind::Project => "project",
            IdKind::Timeline => "timeline",
            IdKind::Track => "track",
            IdKind::Clip => "clip",
            IdKind::Asset => "asset",
            IdKind::Effect => "effect",
        }
    }
}

/// A short, stable, readable object id such as `clip_0007` (ADR-0004).
///
/// Ids are `<kind>_<base32>`, unique within a project and never reused.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Id(String);

impl Id {
    /// Encodes `number` as an id of `kind`, using at least four base32 digits.
    pub fn new(kind: IdKind, number: u64) -> Id {
        let mut digits = Vec::new();
        let mut value = number;
        loop {
            digits.push(ALPHABET[(value % 32) as usize]);
            value /= 32;
            if value == 0 {
                break;
            }
        }
        while digits.len() < 4 {
            digits.push(b'0');
        }
        digits.reverse();
        let suffix: String = digits.into_iter().map(char::from).collect();
        Id(format!("{}_{suffix}", kind.prefix()))
    }

    /// Validates an id string.
    pub fn parse(text: &str) -> Result<Id, String> {
        let (prefix, suffix) = text.split_once('_').ok_or_else(|| {
            format!("invalid id {text:?}: expected <kind>_<base32>, for example clip_0007")
        })?;
        let valid_prefix = !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_lowercase());
        let valid_suffix =
            (4..=13).contains(&suffix.len()) && suffix.bytes().all(|b| ALPHABET.contains(&b));
        if valid_prefix && valid_suffix {
            Ok(Id(text.to_string()))
        } else {
            Err(format!(
                "invalid id {text:?}: expected <kind>_<base32>, for example clip_0007"
            ))
        }
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The kind prefix (`clip` for `clip_0007`).
    pub fn prefix(&self) -> &str {
        self.0.split_once('_').map(|(p, _)| p).unwrap_or_default()
    }

    /// True when the id has the prefix of `kind`.
    pub fn is(&self, kind: IdKind) -> bool {
        self.prefix() == kind.prefix()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Id {
    type Error = String;
    fn try_from(value: String) -> Result<Id, String> {
        Id::parse(&value)
    }
}

impl From<Id> for String {
    fn from(id: Id) -> String {
        id.0
    }
}

impl std::str::FromStr for Id {
    type Err = String;
    fn from_str(s: &str) -> Result<Id, String> {
        Id::parse(s)
    }
}

impl JsonSchema for Id {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Id".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[a-z]+_[0-9a-hjkmnp-tv-z]{4,13}$",
            "description": "Stable object id: kind prefix and lowercase Crockford base32, for example clip_0007 or track_000a."
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_validates() {
        assert_eq!(Id::new(IdKind::Clip, 7).as_str(), "clip_0007");
        assert_eq!(Id::new(IdKind::Track, 32).as_str(), "track_0010");
        assert_eq!(Id::new(IdKind::Asset, 1_048_576).as_str(), "asset_10000");
        assert!(Id::parse("clip_0007").is_ok());
        assert!(Id::parse("clip_07").is_err());
        assert!(Id::parse("Clip_0007").is_err());
        assert!(Id::parse("clip_000i").is_err());
        assert!(Id::new(IdKind::Clip, 3).is(IdKind::Clip));
    }
}
