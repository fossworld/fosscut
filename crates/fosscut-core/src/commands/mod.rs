//! Built-in commands.

mod caption;
mod clip;
mod media;
mod text;
mod track;
mod util;

pub use caption::{
    CaptionPage, CaptionsAdd, CaptionsAdded, CaptionsEdit, CaptionsStyle, CaptionsStyled, Paging,
    WordInput,
};
pub use clip::{
    ClipAdd, ClipMove, ClipPlacement, ClipRemove, ClipRemoved, ClipSplit, ClipSplitResult,
};
pub use media::{MediaAdd, MediaAdded};
pub use text::TextAdd;
pub use track::{TrackAdd, TrackAdded};

use crate::Registry;

pub(crate) fn register(registry: &mut Registry) {
    registry.register::<MediaAdd>();
    registry.register::<TrackAdd>();
    registry.register::<ClipAdd>();
    registry.register::<ClipSplit>();
    registry.register::<ClipMove>();
    registry.register::<ClipRemove>();
    registry.register::<TextAdd>();
    registry.register::<CaptionsAdd>();
    registry.register::<CaptionsEdit>();
    registry.register::<CaptionsStyle>();
}
