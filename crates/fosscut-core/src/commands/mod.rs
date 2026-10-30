//! Built-in commands.

mod caption;
mod clip;
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
pub(crate) use keyframe::keyframe_reports;
pub use keyframe::{
    ClipProperties, ClipSet, Easing, KeyframeRemove, KeyframeReport, KeyframeSet, KeyframesChanged,
};
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
    registry.register::<TextAdd>();
    registry.register::<CaptionsAdd>();
    registry.register::<CaptionsEdit>();
    registry.register::<CaptionsStyle>();
    registry.register::<TimelineDuplicate>();
    registry.register::<TimelineRemoveRanges>();
}
