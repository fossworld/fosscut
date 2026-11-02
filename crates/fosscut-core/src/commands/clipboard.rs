//! Copying and pasting clips as documents, and copying attributes between clips.

use std::collections::BTreeMap;

use fosscut_model::{Clip, ClipContent, Id, IdKind};
use fosscut_time::{Flicks, TimeInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::clip::ClipPlacement;
use super::util::{
    ensure_free, ensure_kind, ensure_unlocked, ensure_valid_range, find_or_create_track,
    insert_sorted, locate_clip, renew_effect_ids, resolve, tidy_groups, timeline_id, timeline_mut,
    track_index, track_kind_for,
};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// A copied clip, as the `clips.copy` query returns it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CopiedClip {
    /// The track it was copied from; pasting uses it when it exists and fits.
    #[serde(default)]
    pub track: Option<Id>,
    /// The clip as stored in the project.
    pub clip: Clip,
}

/// Pastes copied clips at a time, keeping their arrangement.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipsPaste {
    /// Clips from the `clips.copy` query (of this or another project; their media must be
    /// in this one).
    pub clips: Vec<CopiedClip>,
    /// Where the earliest of them starts; the others keep their offsets from it.
    pub at: TimeInput,
    /// Put them all on this track; otherwise each goes to the track it came from when that
    /// exists here and fits, or to the first track of its kind.
    #[serde(default)]
    pub track: Option<Id>,
    /// The timeline; the primary one when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
}

/// Result of `clips.paste`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Pasted {
    /// The new clips, with new ids.
    pub clips: Vec<ClipPlacement>,
}

impl Command for ClipsPaste {
    const NAME: &'static str = "clips.paste";
    const SUMMARY: &'static str = "Paste clips copied with the clips.copy query at a time, keeping their arrangement: new ids, the same media, effects, masks and keyframes, and groups among them kept. Never overwrites: fails with fosscut.clip.overlap.";
    type Output = Pasted;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Pasted, ApiError> {
        if self.clips.is_empty() {
            return Err(invalid("clips.paste: give the clips to paste"));
        }
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let rate = ctx.project.timelines[&timeline].format.rate;
        let at = resolve(&self.at, rate)?;
        let earliest = self
            .clips
            .iter()
            .map(|c| c.clip.start)
            .min()
            .unwrap_or(Flicks::ZERO);
        let mut groups: BTreeMap<Id, Id> = BTreeMap::new();
        let mut placements = Vec::new();
        let mut copies: BTreeMap<Id, Id> = BTreeMap::new();
        for item in self.clips {
            let mut clip = item.clip;
            let old = clip.id.clone();
            clip.id = ctx.project.allocate_id(IdKind::Clip);
            copies.insert(old, clip.id.clone());
            renew_effect_ids(ctx.project, &mut clip);
            if let Some(group) = clip.group.take() {
                let fresh = match groups.get(&group) {
                    Some(id) => id.clone(),
                    None => {
                        let id = ctx.project.allocate_id(IdKind::Group);
                        groups.insert(group, id.clone());
                        id
                    }
                };
                clip.group = Some(fresh);
            }
            clip.start = at + (clip.start - earliest);
            ensure_valid_range(clip.start, clip.duration)?;
            let doc = &ctx.project.timelines[&timeline];
            let fits = |id: &Id| {
                doc.track(id)
                    .is_some_and(|t| ensure_kind(t, &clip.content).is_ok())
            };
            let track_id = match (&self.track, &item.track) {
                (Some(track), _) => track.clone(),
                (None, Some(track)) if fits(track) => track.clone(),
                _ => find_or_create_track(ctx.project, &timeline, track_kind_for(&clip.content))?,
            };
            let doc = timeline_mut(ctx.project, &timeline)?;
            let index = track_index(doc, &track_id)?;
            let track = &mut doc.tracks[index];
            ensure_unlocked(track)?;
            ensure_kind(track, &clip.content)?;
            ensure_free(track, clip.start, clip.end(), None, rate)?;
            placements.push(ClipPlacement::of(&clip, &track.id, rate));
            insert_sorted(track, clip);
        }
        tidy_groups(timeline_mut(ctx.project, &timeline)?);
        // Pasted followers follow the pasted copies of their paths, or the paths on this
        // timeline; others stop following (ADR-0036).
        super::vector::remap_follows(timeline_mut(ctx.project, &timeline)?, &copies);
        Ok(Pasted { clips: placements })
    }
}

/// The attributes `clip.copy_attributes` copies, with the property paths they cover.
const ATTRIBUTES: &[(&str, &[&str])] = &[
    ("transform", &["transform."]),
    ("crop", &["crop."]),
    ("opacity", &["opacity"]),
    ("blend", &["blend"]),
    ("volume", &["volume"]),
    ("audio", &["audio."]),
    ("effects", &[]),
    ("masks", &[]),
    ("animations", &[]),
];

/// Copies a clip's look, sound or motion to other clips, without their media.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipCopyAttributes {
    /// The clip to copy from.
    pub from: Id,
    /// The clips that receive the attributes.
    pub to: Vec<Id>,
    /// Which: `transform`, `crop`, `opacity`, `blend`, `volume`, `audio` (fades, pan and
    /// channels), `effects`, `masks` and `animations`; all of them when omitted.
    /// Keyframes of these properties come along.
    #[serde(default)]
    pub attributes: Vec<String>,
}

/// Result of `clip.copy_attributes`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttributesCopied {
    /// The clips that received them.
    pub clips: Vec<Id>,
    /// The attributes copied.
    pub attributes: Vec<String>,
}

impl Command for ClipCopyAttributes {
    const NAME: &'static str = "clip.copy_attributes";
    const SUMMARY: &'static str = "Paste attributes: copy one clip's placement, crop, opacity, blend mode, volume and sound settings, effects, masks or animations (with their keyframes) to other clips, keeping their media. Effects and masks skip audio clips.";
    type Output = AttributesCopied;

    fn apply(self, ctx: &mut Context<'_>) -> Result<AttributesCopied, ApiError> {
        if self.to.is_empty() {
            return Err(invalid(
                "clip.copy_attributes: give at least one clip in to",
            ));
        }
        let names: Vec<String> = if self.attributes.is_empty() {
            ATTRIBUTES.iter().map(|(n, _)| n.to_string()).collect()
        } else {
            self.attributes.clone()
        };
        for name in &names {
            if !ATTRIBUTES.iter().any(|(n, _)| n == name) {
                let known: Vec<&str> = ATTRIBUTES.iter().map(|(n, _)| *n).collect();
                return Err(invalid(format!(
                    "clip.copy_attributes: there is no attribute {name}"
                ))
                .with_hint(format!("Attributes: {}.", known.join(", "))));
            }
        }
        let location = locate_clip(ctx.project, &self.from)?;
        let source = ctx.project.timelines[&location.timeline].tracks[location.track].clips
            [location.clip]
            .clone();
        let has = |name: &str| names.iter().any(|n| n == name);
        let paths: Vec<&str> = ATTRIBUTES
            .iter()
            .filter(|(n, _)| has(n))
            .flat_map(|(_, p)| p.iter().copied())
            .collect();
        let covered = |path: &str| {
            paths.iter().any(|p| {
                if p.ends_with('.') {
                    path.starts_with(p)
                } else {
                    path == *p
                }
            })
        };
        for target in &self.to {
            if target == &self.from {
                continue;
            }
            let mut effects = source.effects.clone();
            let mut masks = source.masks.clone();
            for effect in &mut effects {
                effect.id = ctx.project.allocate_id(IdKind::Effect);
            }
            for mask in &mut masks {
                mask.id = ctx.project.allocate_id(IdKind::Mask);
            }
            let to = locate_clip(ctx.project, target)?;
            let timeline = timeline_mut(ctx.project, &to.timeline)?;
            let track = &mut timeline.tracks[to.track];
            ensure_unlocked(track)?;
            let clip = &mut track.clips[to.clip];
            let picture = !matches!(clip.content, ClipContent::Audio { .. });
            if has("transform") {
                clip.transform = source.transform.clone();
            }
            if has("crop") {
                clip.crop = source.crop.clone();
            }
            if has("opacity") {
                clip.opacity = source.opacity;
            }
            if has("blend") {
                clip.blend = source.blend;
            }
            if has("volume") {
                clip.volume = source.volume;
            }
            if has("audio") {
                let detached = clip.audio.detached;
                clip.audio = source.audio.clone();
                clip.audio.detached = detached && matches!(clip.content, ClipContent::Video { .. });
            }
            if has("effects") && picture {
                clip.effects = effects;
            }
            if has("masks") && picture {
                clip.masks = masks;
            }
            if has("animations") {
                clip.animations = source.animations.clone();
            }
            clip.keyframes.retain(|path, _| !covered(path));
            for (path, keys) in &source.keyframes {
                if covered(path) {
                    clip.keyframes.insert(path.clone(), keys.clone());
                }
            }
        }
        Ok(AttributesCopied {
            clips: self.to.into_iter().filter(|c| c != &self.from).collect(),
            attributes: names,
        })
    }
}

/// The clips `ids` as documents for the clipboard, with the tracks they are on.
pub(crate) fn copied(
    project: &fosscut_model::Project,
    ids: &[Id],
) -> Result<Vec<CopiedClip>, ApiError> {
    ids.iter()
        .map(|id| {
            let location = locate_clip(project, id)?;
            let track = &project.timelines[&location.timeline].tracks[location.track];
            Ok(CopiedClip {
                track: Some(track.id.clone()),
                clip: track.clips[location.clip].clone(),
            })
        })
        .collect()
}
