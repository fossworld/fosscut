//! Built-in commands.

mod arrange;
mod camera;
mod canvas;
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
mod model;
mod motion;
mod multicam;
mod particles;
mod shape;
mod speed;
mod stagger;
mod text;
mod timeline;
mod timelines;
mod track;
mod util;
mod vector;

pub use arrange::{Aligned, ClipAlign, ClipReplace, Horizontal, SplitAll, TimelineSplit, Vertical};
pub use camera::{CameraAdd, CameraReport, CameraSet};
pub use canvas::{CanvasChanged, ClipCanvas};
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
    EffectReport, EffectSet, EffectsChanged, EffectsCopied, LookApply, VoiceApply,
};
pub use group::{
    AudioExtracted, AudioRestored, ClipExtractAudio, ClipGroup, ClipRestoreAudio, ClipUngroup,
    Grouped,
};
pub use keyframe::{
    ClipProperties, ClipSet, Easing, KeyInput, KeyframeRemove, KeyframeReport, KeyframeSet,
    KeyframeSetAll, KeyframesChanged,
};
pub(crate) use keyframe::{effect_keyframe_reports, keyframe_reports};
pub use library::{
    AssetChanged, FolderAction, Folders, MediaFolder, MediaMove, MediaMoved, MediaRelink,
    MediaRemove, MediaRemoved, MediaRename, MediaReplace, MediaReplaced,
};
pub use marker::{MarkerAdd, MarkerRemove, MarkerReport, MarkerSet};
pub use mask::{MaskAdd, MaskFields, MaskRemove, MaskSet, MasksChanged};
pub use media::{MediaAdd, MediaAdded};
pub(crate) use model::model_capabilities;
pub use model::{ModelAdd, ModelChanged, ModelPreset, ModelSet, model_preset, model_presets};
pub(crate) use motion::motion_capabilities;
pub use motion::{AnimationInput, AnimationSet, MotionChanged, TransitionRemove, TransitionSet};
pub use multicam::{
    AngleChoice, MulticamCreate, MulticamCreated, MulticamSwitch, MulticamSwitched,
};
pub(crate) use particles::particle_capabilities;
pub use particles::{
    BurstInput, ParticlePreset, ParticlesAdd, ParticlesChanged, ParticlesSet, particle_preset,
    particle_presets,
};
pub use shape::{ShapeAdd, ShapeChanged, ShapeSet, TemplateApplied, TemplateApply};
pub(crate) use speed::speed_capabilities;
pub use speed::{ClipFreeze, ClipSpeed, Frozen, SpeedChanged};
pub use stagger::{ClipsStagger, StaggerOrder, StaggerPlace, Staggered, StaggeredClip};
pub use text::{
    RevealInput, TextAdd, TextChanged, TextPresetRemove, TextPresetSave, TextPresets, TextSet,
    TextTemplate,
};
pub use timeline::{
    Fit, RangeInput, RangeReport, RangesRemoved, TimelineDuplicate, TimelineDuplicated,
    TimelineRemoveRanges,
};
pub use timelines::{
    TimelineAdd, TimelineRemove, TimelineRemoved, TimelineRename, TimelineReport,
    TimelineSetPrimary,
};
pub use track::{TrackAdd, TrackAdded};
pub(crate) use vector::vector_capabilities;
pub use vector::{ClipFollow, Exploded, ExplodedPart, Followed, VectorExplode, VectorPart};

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
    registry.register::<KeyframeSetAll>();
    registry.register::<ClipCanvas>();
    registry.register::<KeyframeSet>();
    registry.register::<KeyframeRemove>();
    registry.register::<EffectAdd>();
    registry.register::<EffectSet>();
    registry.register::<EffectRemove>();
    registry.register::<EffectMove>();
    registry.register::<EffectCopy>();
    registry.register::<AdjustmentAdd>();
    registry.register::<LookApply>();
    registry.register::<VoiceApply>();
    registry.register::<MaskAdd>();
    registry.register::<MaskSet>();
    registry.register::<MaskRemove>();
    registry.register::<TransitionSet>();
    registry.register::<TransitionRemove>();
    registry.register::<AnimationSet>();
    registry.register::<ClipsStagger>();
    registry.register::<TextAdd>();
    registry.register::<TextSet>();
    registry.register::<TextPresetSave>();
    registry.register::<TextPresetRemove>();
    registry.register::<TextTemplate>();
    registry.register::<ShapeAdd>();
    registry.register::<ShapeSet>();
    registry.register::<ParticlesAdd>();
    registry.register::<ParticlesSet>();
    registry.register::<ModelAdd>();
    registry.register::<ModelSet>();
    registry.register::<CameraAdd>();
    registry.register::<CameraSet>();
    registry.register::<TemplateApply>();
    registry.register::<ClipFollow>();
    registry.register::<VectorExplode>();
    registry.register::<MulticamCreate>();
    registry.register::<MulticamSwitch>();
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
