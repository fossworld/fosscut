use fosscut_model::DocumentKey;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ApiError, ChangeSummary, CommandCall, codes};

/// What an operation log entry records.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A committed transaction.
    Commit,
    /// An undo of `target_seq`.
    Undo,
    /// A redo of `target_seq`.
    Redo,
}

/// The content of one document before and after a transaction (`null` when absent).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentChange {
    /// Which document.
    pub doc: DocumentKey,
    /// Content before the transaction.
    pub before: serde_json::Value,
    /// Content after the transaction.
    pub after: serde_json::Value,
}

/// One line of `history/ops.jsonl` (ADR-0006).
///
/// Entries are appended before documents are written, so an interrupted save can be
/// completed on the next open, and they hold enough state to undo and redo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Sequence number, starting at 1.
    pub seq: u64,
    /// Commit, undo or redo.
    pub kind: EntryKind,
    /// Project revision before the entry.
    pub revision_before: u64,
    /// Project revision after the entry.
    pub revision_after: u64,
    /// Who made the change.
    pub actor: String,
    /// Why, when the caller said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Retry key of the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// The commands of the request.
    pub commands: Vec<CommandCall>,
    /// For undo and redo, the entry they apply to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_seq: Option<u64>,
    /// Commit time in milliseconds since the Unix epoch, when the host supplied it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
    /// Command results, returned again when the request is retried.
    pub results: Vec<serde_json::Value>,
    /// Object-level summary.
    pub changes: ChangeSummary,
    /// Document states for recovery, undo and redo.
    pub documents: Vec<DocumentChange>,
}

/// Result of `history.undo` and `history.redo`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HistoryOutput {
    /// Sequence number of the transaction that was undone or redone.
    pub target_seq: u64,
    /// Objects changed by this undo or redo.
    pub changes: ChangeSummary,
}

/// Parses the operation log. A torn final line (from an interrupted append) is ignored.
pub(crate) fn parse_log(bytes: &[u8]) -> Result<Vec<LogEntry>, ApiError> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut entries = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        match serde_json::from_str::<LogEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(_) if index + 1 == lines.len() && !text.ends_with('\n') => break,
            Err(e) => {
                return Err(ApiError::new(
                    codes::HISTORY_CORRUPT,
                    format!("history/ops.jsonl line {}: {e}", index + 1),
                ));
            }
        }
    }
    Ok(entries)
}

/// The undo and redo stacks implied by the log.
pub(crate) fn stacks(entries: &[LogEntry]) -> (Vec<u64>, Vec<u64>) {
    let mut undo = Vec::new();
    let mut redo = Vec::new();
    for entry in entries {
        match entry.kind {
            EntryKind::Commit => {
                undo.push(entry.seq);
                redo.clear();
            }
            EntryKind::Undo => {
                undo.pop();
                if let Some(target) = entry.target_seq {
                    redo.push(target);
                }
            }
            EntryKind::Redo => {
                redo.pop();
                if let Some(target) = entry.target_seq {
                    undo.push(target);
                }
            }
        }
    }
    (undo, redo)
}
