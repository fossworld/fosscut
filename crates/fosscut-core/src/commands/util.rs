//! Shared lookups and checks for commands.

use fosscut_model::{Clip, ClipContent, Id, IdKind, Project, Timeline, Track, TrackKind};
use fosscut_time::{Flicks, Rate, TimeInput, TimeReport};
use serde_json::json;

use crate::{ApiError, codes};

/// The requested timeline, or the primary timeline.
pub(crate) fn timeline_id(project: &Project, requested: Option<&Id>) -> Result<Id, ApiError> {
    let id = requested
        .cloned()
        .unwrap_or_else(|| project.manifest.primary_timeline.clone());
    if project.timelines.contains_key(&id) {
        Ok(id)
    } else {
        Err(ApiError::new(
            codes::TIMELINE_NOT_FOUND,
            format!("timeline {id} does not exist"),
        ))
    }
}

pub(crate) fn timeline_mut<'a>(
    project: &'a mut Project,
    id: &Id,
) -> Result<&'a mut Timeline, ApiError> {
    project.timelines.get_mut(id).ok_or_else(|| {
        ApiError::new(
            codes::TIMELINE_NOT_FOUND,
            format!("timeline {id} does not exist"),
        )
    })
}

/// Resolves a time input and snaps it to the nearest frame of `rate` (ADR-0005).
pub(crate) fn resolve(input: &TimeInput, rate: Rate) -> Result<Flicks, ApiError> {
    Ok(rate.snap(input.resolve(rate)?))
}

/// Where a clip lives.
pub(crate) struct ClipLocation {
    pub timeline: Id,
    pub track: usize,
    pub clip: usize,
}

pub(crate) fn locate_clip(project: &Project, clip: &Id) -> Result<ClipLocation, ApiError> {
    for (timeline_id, timeline) in &project.timelines {
        for (track_index, track) in timeline.tracks.iter().enumerate() {
            if let Some(clip_index) = track.clips.iter().position(|c| &c.id == clip) {
                return Ok(ClipLocation {
                    timeline: timeline_id.clone(),
                    track: track_index,
                    clip: clip_index,
                });
            }
        }
    }
    Err(ApiError::new(
        codes::CLIP_NOT_FOUND,
        format!("clip {clip} does not exist"),
    ))
}

pub(crate) fn track_index(timeline: &Timeline, track: &Id) -> Result<usize, ApiError> {
    timeline
        .tracks
        .iter()
        .position(|t| &t.id == track)
        .ok_or_else(|| {
            ApiError::new(
                codes::TRACK_NOT_FOUND,
                format!("track {track} does not exist in timeline {}", timeline.id),
            )
        })
}

pub(crate) fn ensure_unlocked(track: &Track) -> Result<(), ApiError> {
    if track.locked {
        Err(ApiError::new(
            codes::TRACK_LOCKED,
            format!("track {} is locked", track.id),
        ))
    } else {
        Ok(())
    }
}

/// The track kind a clip content needs.
pub(crate) fn track_kind_for(content: &ClipContent) -> TrackKind {
    match content {
        ClipContent::Video { .. }
        | ClipContent::Image { .. }
        | ClipContent::Adjustment
        | ClipContent::Shape(_)
        | ClipContent::Particles { .. }
        | ClipContent::Model { .. }
        | ClipContent::Camera(_)
        | ClipContent::Timeline { .. } => TrackKind::Video,
        ClipContent::Audio { .. } => TrackKind::Audio,
        ClipContent::Text(_) => TrackKind::Text,
        ClipContent::Caption(_) => TrackKind::Caption,
    }
}

pub(crate) fn ensure_kind(track: &Track, content: &ClipContent) -> Result<(), ApiError> {
    let needed = track_kind_for(content);
    if track.kind == needed {
        Ok(())
    } else {
        Err(ApiError::new(
            codes::TRACK_KIND_MISMATCH,
            format!(
                "{} clips need a {needed:?} track, but {} is a {:?} track",
                content.kind_name(),
                track.id,
                track.kind
            ),
        ))
    }
}

/// Fails with `fosscut.clip.overlap` if `[start, end)` intersects a clip other than `ignore`.
pub(crate) fn ensure_free(
    track: &Track,
    start: Flicks,
    end: Flicks,
    ignore: Option<&Id>,
    rate: Rate,
) -> Result<(), ApiError> {
    if let Some(other) = track
        .clips
        .iter()
        .find(|c| Some(&c.id) != ignore && c.start < end && start < c.end())
    {
        let track_end = track
            .clips
            .iter()
            .filter(|c| Some(&c.id) != ignore)
            .map(Clip::end)
            .max()
            .unwrap_or(Flicks::ZERO);
        return Err(ApiError::new(
            codes::CLIP_OVERLAP,
            format!("the clip would overlap {} on track {}", other.id, track.id),
        )
        .with_details(json!({
            "conflicting_clip": other.id,
            "conflicting_start": TimeReport::new(other.start, rate),
            "conflicting_end": TimeReport::new(other.end(), rate),
            "free_from": TimeReport::new(track_end, rate),
        })));
    }
    Ok(())
}

pub(crate) fn ensure_valid_range(start: Flicks, duration: Flicks) -> Result<(), ApiError> {
    if start.is_negative() {
        return Err(ApiError::new(
            codes::CLIP_INVALID_RANGE,
            "a clip cannot start before zero",
        ));
    }
    if !duration.is_positive() {
        return Err(ApiError::new(
            codes::CLIP_INVALID_RANGE,
            "a clip must be at least one frame long",
        ));
    }
    Ok(())
}

/// Inserts a clip keeping the track sorted by start time.
pub(crate) fn insert_sorted(track: &mut Track, clip: Clip) {
    let index = track.clips.partition_point(|c| c.start <= clip.start);
    track.clips.insert(index, clip);
}

/// The first track of `kind` in a timeline, created on top if none exists.
pub(crate) fn find_or_create_track(
    project: &mut Project,
    timeline: &Id,
    kind: TrackKind,
) -> Result<Id, ApiError> {
    if let Some(track) = project
        .timelines
        .get(timeline)
        .and_then(|t| t.tracks.iter().find(|t| t.kind == kind))
    {
        return Ok(track.id.clone());
    }
    let id = project.allocate_id(IdKind::Track);
    timeline_mut(project, timeline)?.tracks.push(Track {
        id: id.clone(),
        kind,
        name: None,
        muted: false,
        hidden: false,
        locked: false,
        caption_style: None,
        clips: Vec::new(),
    });
    Ok(id)
}

/// The first unlocked track of `kind` with nothing between `start` and `end`, or a new one
/// added after the tracks of that kind.
pub(crate) fn free_track(
    project: &mut Project,
    timeline: &Id,
    kind: TrackKind,
    (start, end): (Flicks, Flicks),
) -> Result<Id, ApiError> {
    let doc = timeline_mut(project, timeline)?;
    if let Some(track) = doc.tracks.iter().find(|t| {
        t.kind == kind && !t.locked && t.clips.iter().all(|c| c.end() <= start || c.start >= end)
    }) {
        return Ok(track.id.clone());
    }
    let after = doc.tracks.iter().rposition(|t| t.kind == kind);
    let id = project.allocate_id(IdKind::Track);
    let doc = timeline_mut(project, timeline)?;
    doc.tracks.insert(
        after.map_or(doc.tracks.len(), |i| i + 1),
        Track {
            id: id.clone(),
            kind,
            name: None,
            muted: false,
            hidden: false,
            locked: false,
            caption_style: None,
            clips: Vec::new(),
        },
    );
    Ok(id)
}

/// Gives the effects and masks of `clip` new ids, for a clip that copies another's (the
/// right part of a split, a piece of a cut, a duplicate): effect ids are unique in a
/// project.
pub(crate) fn renew_effect_ids(project: &mut Project, clip: &mut Clip) {
    for effect in &mut clip.effects {
        effect.id = project.allocate_id(fosscut_model::IdKind::Effect);
    }
    for mask in &mut clip.masks {
        mask.id = project.allocate_id(fosscut_model::IdKind::Mask);
    }
}

/// The other clips in `clip`'s group on its timeline (ADR-0024), when `linked`.
pub(crate) fn group_members(project: &Project, clip: &Id, linked: bool) -> Vec<Id> {
    if !linked {
        return Vec::new();
    }
    let Ok(location) = locate_clip(project, clip) else {
        return Vec::new();
    };
    let timeline = &project.timelines[&location.timeline];
    let Some(group) = &timeline.tracks[location.track].clips[location.clip].group else {
        return Vec::new();
    };
    timeline
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .filter(|c| c.group.as_ref() == Some(group) && &c.id != clip)
        .map(|c| c.id.clone())
        .collect()
}

/// Takes the group id off clips left alone in their group, so every group keeps two or
/// more clips (ADR-0024, item 1).
pub(crate) fn tidy_groups(timeline: &mut Timeline) {
    let mut counts: std::collections::BTreeMap<Id, usize> = std::collections::BTreeMap::new();
    for clip in timeline.tracks.iter().flat_map(|t| &t.clips) {
        if let Some(group) = &clip.group {
            *counts.entry(group.clone()).or_default() += 1;
        }
    }
    for clip in timeline.tracks.iter_mut().flat_map(|t| &mut t.clips) {
        if clip.group.as_ref().is_some_and(|g| counts[g] < 2) {
            clip.group = None;
        }
    }
}
