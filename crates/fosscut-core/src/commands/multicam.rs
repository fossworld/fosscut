//! Multicam editing (ADR-0029): angles recorded at the same time become one compound clip
//! whose timeline holds each angle on its own track, lined up by their offsets; cutting
//! between angles splits that clip and chooses the angle each part shows.

use fosscut_model::{
    AssetKind, Clip, ClipContent, Id, IdKind, Timeline, TimelineEntry, TimelineRole, Track,
    TrackKind,
};
use fosscut_time::{Flicks, TimeInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::util::{
    ensure_free, ensure_unlocked, find_or_create_track, insert_sorted, locate_clip, resolve,
    timeline_id, timeline_mut, track_index,
};
use super::{ClipPlacement, ClipSplit};
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::MULTICAM_INVALID, message)
}

/// Makes a multicam clip from angles recorded at the same time.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MulticamCreate {
    /// The angles: video assets, at least two.
    pub angles: Vec<Id>,
    /// Where each angle starts in the shared scene, in the same order: an angle that
    /// started recording later has a larger offset. All zero when omitted; `fosscut
    /// multicam sync` and the MCP `multicam` tool measure them from the sound.
    #[serde(default)]
    pub offsets: Vec<TimeInput>,
    /// Which angle's sound plays, by its number from 1 in `angles` (as `multicam.switch`
    /// numbers them); the first when omitted.
    #[serde(default)]
    pub sound: Option<usize>,
    /// The multicam timeline's name; "Multicam" and a number when omitted.
    #[serde(default)]
    pub name: Option<String>,
    /// Timeline to place the multicam clip on; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Track for the multicam clip; the first video track when omitted.
    #[serde(default)]
    pub track: Option<Id>,
    /// Where it starts; appended after the track's last clip when omitted.
    #[serde(default)]
    pub start: Option<TimeInput>,
}

/// What `multicam.create` made.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MulticamCreated {
    /// The multicam timeline.
    pub timeline: Id,
    /// Its angle tracks, in the order of `angles`.
    pub angles: Vec<Id>,
    /// The multicam clip, showing the first angle.
    pub clip: ClipPlacement,
}

impl Command for MulticamCreate {
    const NAME: &'static str = "multicam.create";
    const SUMMARY: &'static str = "Make a multicam clip from angles recorded at the same time: a compound clip whose timeline holds each angle on its own track, lined up by their offsets, with one angle's sound. Cut between angles with multicam.switch.";
    type Output = MulticamCreated;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MulticamCreated, ApiError> {
        if self.angles.len() < 2 {
            return Err(invalid("multicam.create: give at least two angles"));
        }
        if !self.offsets.is_empty() && self.offsets.len() != self.angles.len() {
            return Err(invalid(format!(
                "multicam.create: {} angles need {} offsets, not {}",
                self.angles.len(),
                self.angles.len(),
                self.offsets.len()
            )));
        }
        let sound = self.sound.unwrap_or(1);
        if sound == 0 || sound > self.angles.len() {
            return Err(invalid(format!(
                "multicam.create: there is no angle {sound} for the sound; angles are numbered from 1"
            )));
        }
        let sound = sound - 1;
        let parent = timeline_id(ctx.project, self.timeline.as_ref())?;
        let format = ctx.project.timelines[&parent].format.clone();
        let rate = format.rate;
        let mut assets = Vec::new();
        for id in &self.angles {
            let asset = ctx.project.media.get(id).cloned().ok_or_else(|| {
                ApiError::new(codes::ASSET_NOT_FOUND, format!("asset {id} does not exist"))
            })?;
            if asset.kind != AssetKind::Video {
                return Err(invalid(format!(
                    "multicam.create: {id} is not a video asset"
                )));
            }
            assets.push(asset);
        }
        // Offsets keep their exact value, between frames too: angles line up to the
        // sample below.
        let offsets = self
            .offsets
            .iter()
            .map(|o| {
                o.resolve(rate).map_err(|e| {
                    ApiError::new(codes::INVALID_ARGUMENTS, format!("multicam.create: {e}"))
                })
            })
            .collect::<Result<Vec<Flicks>, ApiError>>()?;
        let offset = |index: usize| offsets.get(index).copied().unwrap_or(Flicks::ZERO);
        // The scene starts where the earliest angle does.
        let earliest = (0..assets.len()).map(offset).min().unwrap_or(Flicks::ZERO);
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
            .unwrap_or_else(|| format!("Multicam {}", count + 1));
        let inner = ctx.project.allocate_id(IdKind::Timeline);
        let mut tracks = Vec::with_capacity(assets.len());
        let mut angle_ids = Vec::with_capacity(assets.len());
        let mut length = Flicks::ZERO;
        for (index, asset) in assets.iter().enumerate() {
            // Clips start on frames, while offsets measured from sound fall between them:
            // the angle starts on the frame at or after its offset and skips the rest of
            // the offset at the start of its media, so it lines up to the sample.
            let exact = offset(index) - earliest;
            let mut at = rate.frames_to_flicks(rate.frame_at(exact));
            if at < exact {
                at = rate.frames_to_flicks(rate.frame_at(exact) + 1);
            }
            let source_in = at - exact;
            let duration = asset
                .info
                .duration
                .map(|d| rate.frames_to_flicks(rate.frame_at(d - source_in)))
                .unwrap_or(Flicks::ZERO);
            if !duration.is_positive() {
                return Err(invalid(format!(
                    "multicam.create: {} has no length",
                    asset.id
                )));
            }
            length = length.max(at + duration);
            let track = ctx.project.allocate_id(IdKind::Track);
            let clip = Clip::new(
                ctx.project.allocate_id(IdKind::Clip),
                at,
                duration,
                ClipContent::Video {
                    asset: asset.id.clone(),
                    source_in,
                },
            );
            angle_ids.push(track.clone());
            tracks.push(Track {
                id: track,
                kind: TrackKind::Video,
                name: Some(format!("Angle {}: {}", index + 1, asset.name)),
                // Only the chosen angle is heard.
                muted: index != sound,
                hidden: false,
                locked: false,
                caption_style: None,
                clips: vec![clip],
            });
        }
        // Track order puts the first angle on top, as it shows when nothing is chosen.
        tracks.reverse();
        ctx.project.manifest.timelines.push(TimelineEntry {
            id: inner.clone(),
            name: name.clone(),
            role: TimelineRole::Compound,
        });
        ctx.project.timelines.insert(
            inner.clone(),
            Timeline {
                id: inner.clone(),
                name,
                format,
                tracks,
                markers: Vec::new(),
            },
        );
        let track = match &self.track {
            Some(track) => track.clone(),
            None => find_or_create_track(ctx.project, &parent, TrackKind::Video)?,
        };
        let id = ctx.project.allocate_id(IdKind::Clip);
        let doc = timeline_mut(ctx.project, &parent)?;
        let index = track_index(doc, &track)?;
        let track_doc = &mut doc.tracks[index];
        ensure_unlocked(track_doc)?;
        let start = match &self.start {
            Some(start) => resolve(start, rate)?,
            None => track_doc.end(),
        };
        ensure_free(track_doc, start, start + length, None, rate)?;
        let clip = Clip {
            angle: angle_ids.first().cloned(),
            ..Clip::new(
                id,
                start,
                length,
                ClipContent::Timeline {
                    timeline: inner.clone(),
                    source_in: Flicks::ZERO,
                },
            )
        };
        let placement = ClipPlacement::of(&clip, &track, rate);
        insert_sorted(track_doc, clip);
        Ok(MulticamCreated {
            timeline: inner,
            angles: angle_ids,
            clip: placement,
        })
    }
}

/// Cuts to another angle of a multicam clip from a time on.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MulticamSwitch {
    /// The multicam clip (or any part of it after earlier switches).
    pub clip: Id,
    /// Timeline position of the cut; the clip's start switches the whole part.
    pub at: TimeInput,
    /// The angle to show: its track id, or its number from 1 in `multicam.create`'s order.
    pub angle: AngleChoice,
}

/// An angle by track id or by number.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum AngleChoice {
    /// The angle's number, from 1.
    Number(usize),
    /// The angle's track id.
    Track(Id),
}

/// What `multicam.switch` did.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MulticamSwitched {
    /// The part that now shows the angle.
    pub clip: ClipPlacement,
    /// The angle's track.
    pub angle: Id,
}

impl Command for MulticamSwitch {
    const NAME: &'static str = "multicam.switch";
    const SUMMARY: &'static str = "Cut a multicam clip to another angle from a time on: it splits there and the part after shows the angle (by number or track id).";
    type Output = MulticamSwitched;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MulticamSwitched, ApiError> {
        let location = locate_clip(ctx.project, &self.clip)?;
        let clip =
            &ctx.project.timelines[&location.timeline].tracks[location.track].clips[location.clip];
        let rate = ctx.project.timelines[&location.timeline].format.rate;
        let ClipContent::Timeline { timeline, .. } = &clip.content else {
            return Err(invalid(format!(
                "multicam.switch: {} is a {} clip, not a multicam clip",
                self.clip,
                clip.content.kind_name()
            )));
        };
        let inner = &ctx.project.timelines[timeline];
        // Angles in multicam.create's order: the tracks bottom to top reversed.
        let angles: Vec<Id> = inner
            .tracks
            .iter()
            .rev()
            .filter(|t| t.kind == TrackKind::Video)
            .map(|t| t.id.clone())
            .collect();
        let angle = match &self.angle {
            AngleChoice::Number(n) => angles
                .get(n.wrapping_sub(1))
                .cloned()
                .ok_or_else(|| invalid(format!("multicam.switch: there is no angle {n}"))),
            AngleChoice::Track(id) if angles.contains(id) => Ok(id.clone()),
            AngleChoice::Track(id) => Err(invalid(format!(
                "multicam.switch: {id} is not an angle of {}",
                self.clip
            ))),
        }?;
        let at = resolve(&self.at, rate)?;
        let (start, end) = (clip.start, clip.end());
        if at < start || at >= end {
            return Err(invalid(format!(
                "multicam.switch: the cut must be inside {}",
                self.clip
            )));
        }
        let target = if at == start {
            self.clip.clone()
        } else {
            let split = ClipSplit {
                clip: self.clip.clone(),
                at: TimeInput::Flicks(at.get()),
                linked: false,
            }
            .apply(ctx)?;
            split.right.clip
        };
        let location = locate_clip(ctx.project, &target)?;
        let doc = timeline_mut(ctx.project, &location.timeline)?;
        let track_id = doc.tracks[location.track].id.clone();
        let clip = &mut doc.tracks[location.track].clips[location.clip];
        clip.angle = Some(angle.clone());
        let placement = ClipPlacement::of(clip, &track_id, rate);
        Ok(MulticamSwitched {
            clip: placement,
            angle,
        })
    }
}
