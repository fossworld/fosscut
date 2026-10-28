use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::ChangeSummary;

/// One command invocation inside a request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandCall {
    /// Command name, for example `clip.split`.
    pub command: String,
    /// Arguments matching the command's input schema.
    #[serde(default)]
    pub args: serde_json::Value,
}

/// A write request: one or more commands applied as a single transaction (ADR-0006).
///
/// JSON may also use the single-command shorthand `{"command": ..., "args": ...}` with
/// the same envelope fields.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Request {
    /// Commands to apply atomically, in order.
    pub commands: Vec<CommandCall>,
    /// Fail with `fosscut.revision.conflict` unless the project is at this revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    /// Validate and report changes without committing.
    #[serde(default)]
    pub dry_run: bool,
    /// Retry key: repeating a request with the same id and commands returns the first result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Permit destructive operations such as removing clips from the primary video track.
    #[serde(default)]
    pub allow_destructive: bool,
    /// Who makes the change: `human`, or `agent:<client name>`.
    #[serde(default = "default_actor")]
    pub actor: String,
    /// Why the change is made, recorded in the operation log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Commit time in milliseconds since the Unix epoch, supplied by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
}

fn default_actor() -> String {
    "unknown".to_string()
}

impl Request {
    /// A request for a single command with default envelope settings.
    pub fn single(command: impl Into<String>, args: serde_json::Value) -> Request {
        Request {
            commands: vec![CommandCall {
                command: command.into(),
                args,
            }],
            expected_revision: None,
            dry_run: false,
            request_id: None,
            allow_destructive: false,
            actor: default_actor(),
            reason: None,
            at_ms: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestRepr {
    #[serde(default)]
    commands: Option<Vec<CommandCall>>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Option<serde_json::Value>,
    #[serde(default)]
    expected_revision: Option<u64>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    allow_destructive: bool,
    #[serde(default = "default_actor")]
    actor: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    at_ms: Option<u64>,
}

impl<'de> Deserialize<'de> for Request {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Request, D::Error> {
        let repr = RequestRepr::deserialize(deserializer)?;
        let commands = match (repr.commands, repr.command) {
            (Some(commands), None) if repr.args.is_none() => commands,
            (None, Some(command)) => vec![CommandCall {
                command,
                args: repr.args.unwrap_or_default(),
            }],
            _ => {
                return Err(serde::de::Error::custom(
                    "use either \"commands\": [...] or a single \"command\" with \"args\"",
                ));
            }
        };
        Ok(Request {
            commands,
            expected_revision: repr.expected_revision,
            dry_run: repr.dry_run,
            request_id: repr.request_id,
            allow_destructive: repr.allow_destructive,
            actor: repr.actor,
            reason: repr.reason,
            at_ms: repr.at_ms,
        })
    }
}

/// The result of a write request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    /// True when the changes were committed (false for dry runs and no-op requests).
    pub committed: bool,
    /// True when this was a dry run.
    pub dry_run: bool,
    /// True when the request repeated an earlier `request_id` and nothing was executed.
    pub replayed: bool,
    /// Project revision after the request (unchanged for dry runs).
    pub revision: u64,
    /// One result per command, matching each command's output schema.
    pub results: Vec<serde_json::Value>,
    /// Objects added, removed or modified (predicted for dry runs).
    pub changes: ChangeSummary,
}
