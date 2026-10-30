//! Built-in commands.

mod arrange;
mod caption;
mod clip;
mod clipboard;
mod compound;
mod effect;
mod group;
mod keyframe;
mod library;
mod marker;
mod mask;
mod media;
mod motion;
mod speed;
mod text;
mod timeline;
mod timelines;
mod track;
mod util;

pub use arrange::{Aligned, ClipAlign, ClipReplace, Horizontal, SplitAll, TimelineSplit, Vertical};
pub use caption::{
    CaptionPage, CaptionsAdd, CaptionsAdded, CaptionsEdit, CaptionsStyle, CaptionsStyled, Paging,
    WordInput,
};
pub use clip::{
    ClipAdd, ClipMove, ClipPlacement, ClipRemove, ClipRemoved, ClipSplit, ClipSplitResult, ClipTrim,
};
pub(crate) use clipboard::copied;
pub use clipboard::{AttributesCopied, ClipCopyAttributes, ClipsPaste, CopiedClip, Pasted};
pub use compound::{ClipCompound, ClipDecompose, Compounded, Decomposed};
pub use effect::{
    AdjustmentAdd, AdjustmentAdded, EffectAdd, EffectCopy, EffectInput, EffectMove, EffectRemove,
    EffectReport, EffectSet, EffectsChanged, EffectsCopied, LookApply,
};
pub use group::{
    AudioExtracted, AudioRestored, ClipExtractAudio, ClipGroup, ClipRestoreAudio, ClipUngroup,
    Grouped,
};
pub use keyframe::{
    ClipProperties, ClipSet, Easing, KeyframeRemove, KeyframeReport, KeyframeSet, KeyframesChanged,
};
pub(crate) use keyframe::{effect_keyframe_reports, keyframe_reports};
pub use library::{
    AssetChanged, FolderAction, Folders, MediaFolder, MediaMove, MediaMoved, MediaRelink,
    MediaRemove, MediaRemoved, MediaRename, MediaReplace, MediaReplaced,
};
pub use marker::{MarkerAdd, MarkerRemove, MarkerReport, MarkerSet};
pub use mask::{MaskAdd, MaskFields, MaskRemove, MaskSet, MasksChanged};
pub use media::{MediaAdd, MediaAdded};
pub(crate) use motion::motion_capabilities;
pub use motion::{AnimationInput, AnimationSet, MotionChanged, TransitionRemove, TransitionSet};
pub(crate) use speed::speed_capabilities;
pub use speed::{ClipFreeze, ClipSpeed, Frozen, SpeedChanged};
pub use text::TextAdd;
pub use timeline::{
    Fit, RangeInput, RangeReport, RangesRemoved, TimelineDuplicate, TimelineDuplicated,
    TimelineRemoveRanges,
};
pub use timelines::{
    TimelineAdd, TimelineRemove, TimelineRemoved, TimelineRename, TimelineReport,
    TimelineSetPrimary,
};
pub use track::{TrackAdd, TrackAdded};

use crate::Registry;

pub(crate) fn register(registry: &mut Registry) {
    registry.register::<MediaAdd>();
    registry.register::<MediaRename>();
    registry.register::<MediaRemove>();
    registry.register::<MediaMove>();
    registry.register::<MediaFolder>();
    registry.register::<MediaRelink>();
    registry.register::<MediaReplace>();
    registry.register::<TrackAdd>();
    registry.register::<ClipAdd>();
    registry.register::<ClipSplit>();
    registry.register::<ClipTrim>();
    registry.register::<ClipMove>();
    registry.register::<ClipRemove>();
    registry.register::<ClipSpeed>();
    registry.register::<ClipFreeze>();
    registry.register::<ClipReplace>();
    registry.register::<ClipCompound>();
    registry.register::<ClipDecompose>();
    registry.register::<ClipAlign>();
    registry.register::<ClipGroup>();
    registry.register::<ClipUngroup>();
    registry.register::<ClipExtractAudio>();
    registry.register::<ClipRestoreAudio>();
    registry.register::<ClipCopyAttributes>();
    registry.register::<ClipsPaste>();
    registry.register::<MarkerAdd>();
    registry.register::<MarkerSet>();
    registry.register::<MarkerRemove>();
    registry.register::<ClipSet>();
    registry.register::<KeyframeSet>();
    registry.register::<KeyframeRemove>();
    registry.register::<EffectAdd>();
    registry.register::<EffectSet>();
    registry.register::<EffectRemove>();
    registry.register::<EffectMove>();
    registry.register::<EffectCopy>();
    registry.register::<AdjustmentAdd>();
    registry.register::<LookApply>();
    registry.register::<MaskAdd>();
    registry.register::<MaskSet>();
    registry.register::<MaskRemove>();
    registry.register::<TransitionSet>();
    registry.register::<TransitionRemove>();
    registry.register::<AnimationSet>();
    registry.register::<TextAdd>();
    registry.register::<CaptionsAdd>();
    registry.register::<CaptionsEdit>();
    registry.register::<CaptionsStyle>();
    registry.register::<TimelineDuplicate>();
    registry.register::<TimelineRemoveRanges>();
    registry.register::<TimelineSplit>();
    registry.register::<TimelineAdd>();
    registry.register::<TimelineRename>();
    registry.register::<TimelineSetPrimary>();
    registry.register::<TimelineRemove>();
}
