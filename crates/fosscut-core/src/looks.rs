//! Built-in looks (ADR-0011, item 7: presets are data): sets of ordinary effects with
//! parameters chosen for a style, written for FossCut. Applying a look copies its effects
//! into the clip, so projects never depend on the look afterwards.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ApiError, codes};

const SOURCE: &str = include_str!("../presets/looks/looks.json");

/// One effect of a look.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LookEffect {
    /// The effect type.
    #[serde(rename = "type")]
    pub effect_type: String,
    /// Its parameters.
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

/// A built-in look.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Look {
    /// Stable name, for example `teal_orange`.
    pub name: String,
    /// Display name.
    pub title: String,
    /// What it looks like and what it suits.
    pub description: String,
    /// The effects it adds, in order.
    pub effects: Vec<LookEffect>,
}

/// Every built-in look. The document is compiled in and checked by tests.
pub fn looks() -> Vec<Look> {
    serde_json::from_str(SOURCE).unwrap_or_default()
}

/// The look called `name`.
pub fn look(name: &str) -> Result<Look, ApiError> {
    let all = looks();
    all.iter().find(|l| l.name == name).cloned().ok_or_else(|| {
        let names: Vec<&str> = all.iter().map(|l| l.name.as_str()).collect();
        ApiError::new(codes::LOOK_UNKNOWN, format!("there is no look {name}"))
            .with_hint(format!("Looks: {}.", names.join(", ")))
    })
}
