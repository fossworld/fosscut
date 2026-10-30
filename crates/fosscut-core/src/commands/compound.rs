//! Compound clips (ADR-0025): clips that show another timeline of the project.

use std::collections::BTreeMap;

use fosscut_model::{
    Clip, ClipContent, Id, IdKind, Project, Timeline, TimelineEntry, TimelineRole, Track, TrackKind,
};
use fosscut_time::Flicks;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::clip::ClipPlacement;
use super::util::{
    ensure_free, ensure_unlocked, insert_sorted, locate_clip, renew_effect_ids, tidy_groups,
    timeline_mut,
};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// The content of a compound clip showing `nested` from `source_in`, checked for use on
/// `timeline`: a compound timeline with the same frame rate, other than `timeline`.
pub(crate) fn compound_content(
    project: &Project,
    timeline: &Id,
    nested: &Id,
    source_in: Flicks,
) -> Result<ClipContent, ApiError> {
    let inner = project.timelines.get(nested).ok_or_else(|| {
        ApiError::new(
            codes::TIMELINE_NOT_FOUND,
            format!("timeline {nested} does not exist"),
        )
    })?;
    let compound = project
        .manifest
        .timelines
        .iter()
        .any(|t| &t.id == nested && t.role == TimelineRole::Compound);
    if !compound || nested == timeline {
        return Err(invalid(format!(
            "{nested} is not a compound timeline that can be placed here"
        ))
        .with_hint("project.info lists each timeline's role; clip.compound makes one."));
    }
    if inner.format.rate != project.timelines[timeline].format.rate {
        return Err(invalid(format!("{nested} has another frame rate")));
    }
    Ok(ClipContent::Timeline {
        timeline: nested.clone(),
        source_in,
    })
}

/// Result of `clip.compound`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Compounded {
    /// The new compound clip.
    pub compound: ClipPlacement,
    /// The compound timeline that holds the clips now.
    pub timeline: Id,
}

/// Turns chosen clips into one compound clip.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipCompound {
    /// Clips of one timeline, on any tracks.
    pub clips: Vec<Id>,
    /// The compound timeline's name; `Compound N` when omitted.
    #[serde(default)]
    pub name: Option<String>,
}

impl Command for ClipCompound {
    const NAME: &'static str = "clip.compound";
    const SUMMARY: &'static str = "Make a compound clip: move chosen clips into a new compound timeline (keeping their tracks and timing) and put one clip showing it in their place, on the lowest of their video tracks. Trim, speed, effects and transitions then apply to it as a whole.";
    type Output = Compounded;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Compounded, ApiError> {
        if self.clips.is_empty() {
            return Err(invalid("clip.compound: give the clips"));
        }
        let mut timeline_id = None;
        for clip in &self.clips {
            let location = locate_clip(ctx.project, clip)?;
            if timeline_id.get_or_insert(location.timeline.clone()) != &location.timeline {
                return Err(invalid("clip.compound: the clips must be on one timeline"));
            }
        }
        let timeline_id = timeline_id.ok_or_else(|| invalid("clip.compound: give the clips"))?;
        let nested_id = ctx.project.allocate_id(IdKind::Timeline);
        let compound_id = ctx.project.allocate_id(IdKind::Clip);
        let count = ctx
            .project
            .manifest
            .timelines
            .iter()
            .filter(|t| t.role == TimelineRole::Compound)
            .count();
        let name = self
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("Compound {}", count + 1));
        let doc = timeline_mut(ctx.project, &timeline_id)?;
        let format = doc.format.clone();
        let rate = format.rate;
        // Take the clips out, track by track, keeping the tracks' order.
        let mut taken: Vec<(usize, Track, Vec<Clip>)> = Vec::new();
        for (index, track) in doc.tracks.iter_mut().enumerate() {
            if !track.clips.iter().any(|c| self.clips.contains(&c.id)) {
                continue;
            }
            ensure_unlocked(track)?;
            let (chosen, kept): (Vec<Clip>, Vec<Clip>) = std::mem::take(&mut track.clips)
                .into_iter()
                .partition(|c| self.clips.contains(&c.id));
            track.clips = kept;
            taken.push((index, track.clone(), chosen));
        }
        let start = taken
            .iter()
            .flat_map(|(_, _, clips)| clips.iter().map(|c| c.start))
            .min()
            .unwrap_or(Flicks::ZERO);
        let end = taken
            .iter()
            .flat_map(|(_, _, clips)| clips.iter().map(Clip::end))
            .max()
            .unwrap_or(Flicks::ZERO);
        let host = taken
            .iter()
            .find(|(_, track, _)| track.kind == TrackKind::Video)
            .map(|(index, _, _)| *index);
        let mut nested = Timeline {
            id: nested_id.clone(),
            name: name.clone(),
            format,
            tracks: Vec::new(),
            markers: Vec::new(),
        };
        let mut new_tracks = Vec::new();
        for (_, track, clips) in &taken {
            new_tracks.push(Track {
                id: Id::new(IdKind::Track, 0),
                clips: clips
                    .iter()
                    .cloned()
                    .map(|mut c| {
                        c.start = c.start - start;
                        c
                    })
                    .collect(),
                ..track.clone()
            });
        }
        let host = match host {
            Some(index) => index,
            None => {
                let index = doc.tracks.iter().position(|t| t.kind == TrackKind::Video);
                match index {
                    Some(index) => index,
                    None => {
                        doc.tracks.insert(
                            0,
                            Track {
                                id: Id::new(IdKind::Track, 0),
                                kind: TrackKind::Video,
                                name: None,
                                muted: false,
                                hidden: false,
                                locked: false,
                                caption_style: None,
                                clips: Vec::new(),
                            },
                        );
                        0
                    }
                }
            }
        };
        let compound = Clip::new(
            compound_id,
            start,
            end - start,
            ClipContent::Timeline {
                timeline: nested_id.clone(),
                source_in: Flicks::ZERO,
            },
        );
        ensure_unlocked(&doc.tracks[host])?;
        ensure_free(
            &doc.tracks[host],
            compound.start,
            compound.end(),
            None,
            rate,
        )?;
        tidy_groups(doc);
        // Fresh track ids, now that the borrow of the timeline has ended.
        let needs_host_id = doc.tracks[host].id == Id::new(IdKind::Track, 0);
        let mut placement = ClipPlacement::of(&compound, &doc.tracks[host].id, rate);
        insert_sorted(&mut doc.tracks[host], compound);
        if needs_host_id {
            let id = ctx.project.allocate_id(IdKind::Track);
            let doc = timeline_mut(ctx.project, &timeline_id)?;
            doc.tracks[host].id = id.clone();
            placement.track = id;
        }
        for mut track in new_tracks {
            track.id = ctx.project.allocate_id(IdKind::Track);
            nested.tracks.push(track);
        }
        tidy_groups(&mut nested);
        ctx.project.manifest.timelines.push(TimelineEntry {
            id: nested_id.clone(),
            name,
            role: TimelineRole::Compound,
        });
        ctx.project.timelines.insert(nested_id.clone(), nested);
        Ok(Compounded {
            compound: placement,
            timeline: nested_id,
        })
    }
}

/// Result of `clip.decompose`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Decomposed {
    /// The clips put back, with new ids.
    pub clips: Vec<ClipPlacement>,
    /// The compound timeline, when it was removed because nothing else showed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_timeline: Option<Id>,
}

/// Puts a compound clip's contents back in its place.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipDecompose {
    /// The compound clip.
    pub clip: Id,
}

impl Command for ClipDecompose {
    const NAME: &'static str = "clip.decompose";
    const SUMMARY: &'static str = "Break a compound clip apart: put the part of its timeline it shows back in its place, clip by clip, on tracks of the same kinds. The compound timeline goes when nothing else shows it. The compound clip must have no speed, effects, masks, keyframes or placement of its own.";
    type Output = Decomposed;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Decomposed, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let compound = ctx.project.timelines[&location.timeline].tracks[location.track].clips
            [location.clip]
            .clone();
        let ClipContent::Timeline {
            timeline: nested,
            source_in,
        } = compound.content.clone()
        else {
            return Err(invalid(format!(
                "clip.decompose: {} is not a compound clip",
                compound.id
            )));
        };
        let plain = Clip::new(
            compound.id.clone(),
            compound.start,
            compound.duration,
            compound.content.clone(),
        );
        let own = compound.speed.is_some()
            || !compound.effects.is_empty()
            || !compound.masks.is_empty()
            || !compound.keyframes.is_empty()
            || compound.transform != plain.transform
            || compound.crop != plain.crop
            || compound.opacity != plain.opacity
            || compound.blend != plain.blend;
        if own {
            return Err(invalid(format!(
                "clip.decompose: {} has speed, effects, masks, keyframes or placement of its own, which its parts would lose",
                compound.id
            ))
            .with_hint("Reset them first, or keep the compound clip and edit inside its timeline."));
        }
        let inner = ctx.project.timelines.get(&nested).cloned().ok_or_else(|| {
            ApiError::new(
                codes::TIMELINE_NOT_FOUND,
                format!("timeline {nested} does not exist"),
            )
        })?;
        let (from, to) = (source_in, source_in + compound.duration);
        let shift = compound.start - source_in;
        // The parts that show, with new ids, trimmed to what the compound clip shows.
        let mut parts: Vec<(TrackKind, Vec<Clip>)> = Vec::new();
        let mut groups: BTreeMap<Id, Id> = BTreeMap::new();
        for track in &inner.tracks {
            let mut clips = Vec::new();
            for clip in &track.clips {
                let (a, b) = (clip.start.max(from), clip.end().min(to));
                if a >= b {
                    continue;
                }
                let mut part = clip.clone();
                part.retime_part(a - clip.start, b - a);
                part.shift_keyframes(clip.start - a);
                part.start = a + shift;
                part.duration = b - a;
                part.id = ctx.project.allocate_id(IdKind::Clip);
                renew_effect_ids(ctx.project, &mut part);
                if let Some(group) = part.group.take() {
                    let fresh = match groups.get(&group) {
                        Some(id) => id.clone(),
                        None => {
                            let id = ctx.project.allocate_id(IdKind::Group);
                            groups.insert(group, id.clone());
                            id
                        }
                    };
                    part.group = Some(fresh);
                }
                clips.push(part);
            }
            if !clips.is_empty() {
                parts.push((track.kind, clips));
            }
        }
        let fresh_tracks: Vec<Id> = (0..parts.len())
            .map(|_| ctx.project.allocate_id(IdKind::Track))
            .collect();
        let doc = timeline_mut(ctx.project, &location.timeline)?;
        let rate = doc.format.rate;
        ensure_unlocked(&doc.tracks[location.track])?;
        doc.tracks[location.track].clips.remove(location.clip);
        let mut placements = Vec::new();
        let mut used: Vec<usize> = Vec::new();
        for ((kind, clips), fresh) in parts.into_iter().zip(fresh_tracks) {
            // The compound clip's own track first, then any free track of the kind.
            let fits = |track: &Track| {
                track.kind == kind
                    && !track.locked
                    && clips
                        .iter()
                        .all(|c| ensure_free(track, c.start, c.end(), None, rate).is_ok())
            };
            let host = std::iter::once(location.track)
                .chain(0..doc.tracks.len())
                .find(|i| !used.contains(i) && fits(&doc.tracks[*i]));
            let index = match host {
                Some(index) => index,
                None => {
                    doc.tracks.push(Track {
                        id: fresh,
                        kind,
                        name: None,
                        muted: false,
                        hidden: false,
                        locked: false,
                        caption_style: None,
                        clips: Vec::new(),
                    });
                    doc.tracks.len() - 1
                }
            };
            used.push(index);
            for clip in clips {
                placements.push(ClipPlacement::of(&clip, &doc.tracks[index].id, rate));
                insert_sorted(&mut doc.tracks[index], clip);
            }
        }
        tidy_groups(doc);
        let shown_elsewhere = ctx.project.timelines.values().any(|t| {
            t.tracks
                .iter()
                .flat_map(|tr| &tr.clips)
                .any(|c| c.content.nested() == Some(&nested))
        });
        let removed_timeline = if shown_elsewhere || ctx.project.manifest.primary_timeline == nested
        {
            None
        } else {
            ctx.project.timelines.remove(&nested);
            ctx.project.manifest.timelines.retain(|t| t.id != nested);
            Some(nested)
        };
        Ok(Decomposed {
            clips: placements,
            removed_timeline,
        })
    }
}
