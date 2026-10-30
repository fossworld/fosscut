//! The FossCut command layer (ADR-0002, ADR-0006).
//!
//! Every change to a project is a typed command executed through [`Session::execute`],
//! and every read is a query through [`Session::query`]. The GUI, the CLI and the MCP
//! server are thin front ends over this crate.
//!
//! Each write honours the agent safety contract:
//!
//! - `expected_revision` rejects edits based on a stale read (`fosscut.revision.conflict`);
//! - `dry_run` validates and reports the predicted changes without committing;
//! - `request_id` makes retries safe: a repeated request returns the original result;
//! - a request's commands apply as one atomic transaction and one undo step;
//! - destructive operations need `allow_destructive` (`fosscut.guard.destructive`);
//! - every commit is appended to `history/ops.jsonl` before documents are written.
//!
//! This crate performs no file, clock or thread access of its own; storage goes through
//! [`fosscut_model::ProjectStore`] and timestamps come from the caller, so it builds for
//! `wasm32-unknown-unknown`.

#![forbid(unsafe_code)]

mod changes;
mod command;
mod commands;
mod envelope;
mod error;
mod history;
mod looks;
mod presets;
mod queries;
mod session;

pub use changes::ChangeSummary;
pub use command::{Command, CommandDescriptor, Context, QueryDescriptor, Registry};
pub use commands::{
    AdjustmentAdd, AdjustmentAdded, EffectAdd, EffectCopy, EffectInput, EffectMove, EffectRemove,
    EffectReport, EffectSet, EffectsChanged, EffectsCopied, LookApply, MaskAdd, MaskFields,
    MaskRemove, MaskSet, MasksChanged,
};
pub use commands::{
    Aligned, AttributesCopied, AudioExtracted, AudioRestored, ClipAlign, ClipCopyAttributes,
    ClipExtractAudio, ClipGroup, ClipReplace, ClipRestoreAudio, ClipUngroup, ClipsPaste,
    CopiedClip, Grouped, Horizontal, MarkerAdd, MarkerRemove, MarkerReport, MarkerSet, Pasted,
    SplitAll, TimelineSplit, Vertical,
};
pub use commands::{AnimationInput, AnimationSet, MotionChanged, TransitionRemove, TransitionSet};
pub use commands::{
    AssetChanged, FolderAction, Folders, MediaFolder, MediaMove, MediaMoved, MediaRelink,
    MediaRemove, MediaRemoved, MediaRename, MediaReplace, MediaReplaced, TimelineAdd,
    TimelineRemove, TimelineRemoved, TimelineRename, TimelineReport, TimelineSetPrimary,
};
pub use commands::{
    CaptionPage, CaptionsAdd, CaptionsAdded, CaptionsEdit, CaptionsStyle, CaptionsStyled, ClipAdd,
    ClipMove, ClipPlacement, ClipProperties, ClipRemove, ClipRemoved, ClipSet, ClipSplit,
    ClipSplitResult, ClipTrim, Easing, Fit, KeyframeRemove, KeyframeReport, KeyframeSet,
    KeyframesChanged, MediaAdd, MediaAdded, Paging, RangeInput, RangeReport, RangesRemoved,
    TimelineDuplicate, TimelineDuplicated, TimelineRemoveRanges, TrackAdd, TrackAdded, WordInput,
};
pub use commands::{ClipCompound, ClipDecompose, Compounded, Decomposed};
pub use commands::{ClipFreeze, ClipSpeed, Frozen, SpeedChanged};
pub use commands::{
    RevealInput, TextAdd, TextChanged, TextPresetRemove, TextPresetSave, TextPresets, TextSet,
    TextTemplate,
};
pub use envelope::{CommandCall, Request, Response};
pub use error::{ApiError, ErrorCode, codes, error_catalogue};
pub use history::{DocumentChange, EntryKind, HistoryOutput, LogEntry};
pub use looks::{Look, LookEffect, look, looks};
pub use presets::{
    CaptionPreset, LineLook, TemplateLine, TemplateReveal, TextStylePreset, TitleTemplate,
    caption_animators, caption_preset, caption_presets, text_style_preset, text_style_presets,
    text_template, text_templates,
};
pub use session::Session;

/// Version of the command and query API reported by `capabilities`.
pub const API_VERSION: &str = "0.1.0";

/// Command name for undoing the most recent transaction.
pub const UNDO: &str = "history.undo";
/// Command name for redoing the most recently undone transaction.
pub const REDO: &str = "history.redo";
