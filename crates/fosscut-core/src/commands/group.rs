//! Clip groups and extracted sound (ADR-0024).

use fosscut_model::{AssetKind, Clip, ClipContent, Id, IdKind, Track, TrackKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::clip::ClipPlacement;
use super::util::{ensure_unlocked, insert_sorted, locate_clip, tidy_groups, timeline_mut};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// Clips and the group they are in now.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Grouped {
    /// The group, absent when the clips are in none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Id>,
    /// The clips.
    pub clips: Vec<Id>,
}

/// The timeline of every clip in `clips`, which must all be on one.
fn one_timeline(ctx: &Context<'_>, clips: &[Id], command: &str) -> Result<Id, ApiError> {
    let mut timeline = None;
    for clip in clips {
        let location = locate_clip(ctx.project, clip)?;
        if timeline.get_or_insert(location.timeline.clone()) != &location.timeline {
            return Err(invalid(format!(
                "{command}: the clips must be on one timeline"
            )));
        }
    }
    timeline.ok_or_else(|| invalid(format!("{command}: give the clips")))
}

/// Puts clips in a new group, so moves, splits, trims and removals edit them together.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipGroup {
    /// Two or more clips of one timeline; they leave any group they were in.
    pub clips: Vec<Id>,
}

impl Command for ClipGroup {
    const NAME: &'static str = "clip.group";
    const SUMMARY: &'static str = "Group two or more clips of a timeline so clip.move, clip.split, clip.trim and clip.remove edit them together (pass linked: false to edit one). They leave any group they were in.";
    type Output = Grouped;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Grouped, ApiError> {
        let mut clips = self.clips.clone();
        clips.sort();
        clips.dedup();
        if clips.len() < 2 {
            return Err(invalid("clip.group: a group needs two or more clips"));
        }
        let timeline_id = one_timeline(ctx, &clips, Self::NAME)?;
        let group = ctx.project.allocate_id(IdKind::Group);
        let timeline = timeline_mut(ctx.project, &timeline_id)?;
        for clip in timeline.tracks.iter_mut().flat_map(|t| &mut t.clips) {
            if clips.contains(&clip.id) {
                clip.group = Some(group.clone());
            }
        }
        tidy_groups(timeline);
        Ok(Grouped {
            group: Some(group),
            clips,
        })
    }
}

/// Takes clips out of their groups.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipUngroup {
    /// The clips; a group left with one clip dissolves.
    pub clips: Vec<Id>,
}

impl Command for ClipUngroup {
    const NAME: &'static str = "clip.ungroup";
    const SUMMARY: &'static str = "Take clips out of their groups, so each edits on its own again.";
    type Output = Grouped;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Grouped, ApiError> {
        let timeline_id = one_timeline(ctx, &self.clips, Self::NAME)?;
        let timeline = timeline_mut(ctx.project, &timeline_id)?;
        let mut changed = Vec::new();
        for clip in timeline.tracks.iter_mut().flat_map(|t| &mut t.clips) {
            if self.clips.contains(&clip.id) && clip.group.take().is_some() {
                changed.push(clip.id.clone());
            }
        }
        if changed.is_empty() {
            return Err(invalid("clip.ungroup: none of the clips is in a group"));
        }
        tidy_groups(timeline);
        Ok(Grouped {
            group: None,
            clips: changed,
        })
    }
}

/// The video clip and the audio clip that now plays its sound.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AudioExtracted {
    /// The video clip, no longer heard.
    pub video: Id,
    /// The new audio clip.
    pub audio: ClipPlacement,
    /// The group they share.
    pub group: Id,
}

/// Moves a video clip's sound to an audio clip of its own, linked to it.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipExtractAudio {
    /// A video clip whose media has sound.
    pub clip: Id,
}

impl Command for ClipExtractAudio {
    const NAME: &'static str = "clip.extract_audio";
    const SUMMARY: &'static str = "Extract a video clip's sound into an audio clip of its own, on the first audio track free over its span (or a new one), grouped with the video so they stay in sync. The video clip is no longer heard; clip.restore_audio undoes it.";
    type Output = AudioExtracted;

    fn apply(self, ctx: &mut Context<'_>) -> Result<AudioExtracted, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let video = ctx.project.timelines[&location.timeline].tracks[location.track].clips
            [location.clip]
            .clone();
        let ClipContent::Video { asset, source_in } = &video.content else {
            return Err(invalid(format!(
                "clip.extract_audio: {} is not a video clip",
                video.id
            )));
        };
        let has_sound = ctx
            .project
            .media
            .get(asset)
            .is_some_and(|a| a.kind == AssetKind::Video && a.has_audio());
        if !has_sound {
            return Err(invalid(format!(
                "clip.extract_audio: the media of {} has no sound",
                video.id
            )));
        }
        if video.audio.detached {
            return Err(invalid(format!(
                "clip.extract_audio: the sound of {} is already extracted",
                video.id
            ))
            .with_hint("Use clip.restore_audio to put it back first."));
        }
        let audio_id = ctx.project.allocate_id(IdKind::Clip);
        let group = video
            .group
            .clone()
            .unwrap_or_else(|| ctx.project.allocate_id(IdKind::Group));
        let new_track = ctx.project.allocate_id(IdKind::Track);
        let mut audio = Clip::new(
            audio_id,
            video.start,
            video.duration,
            ClipContent::Audio {
                asset: asset.clone(),
                source_in: *source_in,
            },
        );
        audio.volume = video.volume;
        audio.audio = video.audio.clone();
        audio.enabled = video.enabled;
        audio.speed = video.speed.clone();
        audio.group = Some(group.clone());
        audio.keyframes = video
            .keyframes
            .iter()
            .filter(|(path, _)| *path == "volume" || path.starts_with("audio."))
            .map(|(path, keys)| (path.clone(), keys.clone()))
            .collect();
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let rate = timeline.format.rate;
        let track = timeline.tracks.iter().position(|t| {
            t.kind == TrackKind::Audio
                && !t.locked
                && t.clips
                    .iter()
                    .all(|c| c.end() <= audio.start || audio.end() <= c.start)
        });
        let track = match track {
            Some(index) => index,
            None => {
                timeline.tracks.push(Track {
                    id: new_track,
                    kind: TrackKind::Audio,
                    name: None,
                    muted: false,
                    hidden: false,
                    locked: false,
                    caption_style: None,
                    clips: Vec::new(),
                });
                timeline.tracks.len() - 1
            }
        };
        let placement = ClipPlacement::of(&audio, &timeline.tracks[track].id, rate);
        insert_sorted(&mut timeline.tracks[track], audio);
        let video = &mut timeline.tracks[location.track].clips[location.clip];
        video.audio.detached = true;
        video.group = Some(group.clone());
        Ok(AudioExtracted {
            video: video.id.clone(),
            audio: placement,
            group,
        })
    }
}

/// A video clip heard again.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AudioRestored {
    /// The video clip.
    pub video: Id,
    /// The extracted audio clips removed.
    pub removed: Vec<Id>,
}

/// Puts a video clip's extracted sound back.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipRestoreAudio {
    /// The video clip whose sound was extracted.
    pub clip: Id,
}

impl Command for ClipRestoreAudio {
    const NAME: &'static str = "clip.restore_audio";
    const SUMMARY: &'static str = "Put a video clip's extracted sound back: remove the audio clips of its group that play the same media, and hear the video clip again.";
    type Output = AudioRestored;

    fn apply(self, ctx: &mut Context<'_>) -> Result<AudioRestored, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let timeline = timeline_mut(ctx.project, &location.timeline)?;
        let video = &timeline.tracks[location.track].clips[location.clip];
        if !video.audio.detached {
            return Err(invalid(format!(
                "clip.restore_audio: the sound of {} is not extracted",
                video.id
            )));
        }
        let (asset, group) = (video.content.asset().cloned(), video.group.clone());
        let mut removed = Vec::new();
        for track in &mut timeline.tracks {
            let extracted = |c: &Clip| {
                matches!(c.content, ClipContent::Audio { .. })
                    && c.content.asset() == asset.as_ref()
                    && group.is_some()
                    && c.group == group
            };
            if track.clips.iter().any(extracted) {
                ensure_unlocked(track)?;
            }
            removed.extend(
                track
                    .clips
                    .iter()
                    .filter(|c| extracted(c))
                    .map(|c| c.id.clone()),
            );
            track.clips.retain(|c| !extracted(c));
        }
        let video = &mut timeline.tracks[location.track].clips[location.clip];
        video.audio.detached = false;
        let id = video.id.clone();
        tidy_groups(timeline);
        Ok(AudioRestored { video: id, removed })
    }
}
