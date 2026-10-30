//! Built-in commands.

mod caption;
mod clip;
mod effect;
mod keyframe;
mod media;
mod text;
mod timeline;
mod track;
mod util;

pub use caption::{
    CaptionPage, CaptionsAdd, CaptionsAdded, CaptionsEdit, CaptionsStyle, CaptionsStyled, Paging,
    WordInput,
};
pub use clip::{
    ClipAdd, ClipMove, ClipPlacement, ClipRemove, ClipRemoved, ClipSplit, ClipSplitResult, ClipTrim,
};
pub use effect::{
    AdjustmentAdd, AdjustmentAdded, EffectAdd, EffectCopy, EffectInput, EffectMove, EffectRemove,
    EffectReport, EffectSet, EffectsChanged, EffectsCopied, LookApply,
};
pub use keyframe::{
    ClipProperties, ClipSet, Easing, KeyframeRemove, KeyframeReport, KeyframeSet, KeyframesChanged,
};
pub(crate) use keyframe::{effect_keyframe_reports, keyframe_reports};
pub use media::{MediaAdd, MediaAdded};
pub use text::TextAdd;
pub use timeline::{
    Fit, RangeInput, RangeReport, RangesRemoved, TimelineDuplicate, TimelineDuplicated,
    TimelineRemoveRanges,
};
pub use track::{TrackAdd, TrackAdded};

use crate::Registry;

pub(crate) fn register(registry: &mut Registry) {
    registry.register::<MediaAdd>();
    registry.register::<TrackAdd>();
    registry.register::<ClipAdd>();
    registry.register::<ClipSplit>();
    registry.register::<ClipTrim>();
    registry.register::<ClipMove>();
    registry.register::<ClipRemove>();
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
    registry.register::<TextAdd>();
    registry.register::<CaptionsAdd>();
    registry.register::<CaptionsEdit>();
    registry.register::<CaptionsStyle>();
    registry.register::<TimelineDuplicate>();
    registry.register::<TimelineRemoveRanges>();
}
