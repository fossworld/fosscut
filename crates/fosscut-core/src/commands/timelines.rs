//! A project's timelines: adding one (empty, or from chosen media), renaming, removing and
//! choosing the primary one.

use fosscut_model::{
    AssetKind, Clip, ClipContent, Id, IdKind, Timeline, TimelineEntry, TimelineFormat, Track,
    TrackKind,
};
use fosscut_time::{Flicks, Rate, TimeReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::util::timeline_id;
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// A timeline after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TimelineReport {
    /// The timeline.
    pub timeline: Id,
    /// Its name.
    pub name: String,
    /// True when it is the primary timeline.
    pub primary: bool,
    /// Its length.
    pub duration: TimeReport,
}

impl TimelineReport {
    fn of(ctx: &Context<'_>, id: &Id) -> TimelineReport {
        let doc = &ctx.project.timelines[id];
        TimelineReport {
            timeline: id.clone(),
            name: doc.name.clone(),
            primary: &ctx.project.manifest.primary_timeline == id,
            duration: TimeReport::new(doc.duration(), doc.format.rate),
        }
    }
}

/// Adds a timeline: empty, or with chosen media placed one after another.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimelineAdd {
    /// Its name; `Timeline N` when omitted.
    #[serde(default)]
    pub name: Option<String>,
    /// Canvas width in pixels; the first video's, or the primary timeline's, when omitted.
    #[serde(default)]
    pub width: Option<u32>,
    /// Canvas height in pixels.
    #[serde(default)]
    pub height: Option<u32>,
    /// Frame rate such as `30` or `30000/1001`; the first video's, or the primary
    /// timeline's, when omitted.
    #[serde(default)]
    pub rate: Option<String>,
    /// Assets to place one after another, pictures on a video track and sound on an audio
    /// track; images last three seconds.
    #[serde(default)]
    pub assets: Vec<Id>,
    /// Make it the primary timeline.
    #[serde(default)]
    pub primary: bool,
}

impl Command for TimelineAdd {
    const NAME: &'static str = "timeline.add";
    const SUMMARY: &'static str = "Add a timeline to the project: empty, or a new timeline from chosen media placed one after another, with the canvas and rate of the first video unless given. primary makes it the one commands and exports use by default.";
    type Output = TimelineReport;

    fn apply(self, ctx: &mut Context<'_>) -> Result<TimelineReport, ApiError> {
        let mut assets = Vec::new();
        for id in &self.assets {
            let asset = ctx.project.media.get(id).cloned().ok_or_else(|| {
                ApiError::new(codes::ASSET_NOT_FOUND, format!("asset {id} does not exist"))
            })?;
            if asset.kind == AssetKind::Lut {
                return Err(invalid(format!(
                    "timeline.add: {id} is a LUT, not media to place"
                )));
            }
            assets.push(asset);
        }
        let base = ctx
            .project
            .primary_timeline()
            .map(|t| t.format.clone())
            .unwrap_or(TimelineFormat {
                width: 1920,
                height: 1080,
                rate: Rate::FPS_30,
                sample_rate: 48_000,
            });
        let first_video = assets
            .iter()
            .find(|a| a.kind == AssetKind::Video)
            .and_then(|a| a.info.video.as_ref());
        let shown = first_video.map(|v| {
            if v.rotation.rem_euclid(180) == 90 {
                (v.height, v.width)
            } else {
                (v.width, v.height)
            }
        });
        let rate = match &self.rate {
            Some(text) => text
                .parse::<Rate>()
                .map_err(|e| invalid(format!("timeline.add: rate {text}: {e}")))?,
            None => first_video.and_then(|v| v.rate).unwrap_or(base.rate),
        };
        let format = TimelineFormat {
            width: self.width.or(shown.map(|s| s.0)).unwrap_or(base.width),
            height: self.height.or(shown.map(|s| s.1)).unwrap_or(base.height),
            rate,
            sample_rate: base.sample_rate,
        };
        if format.width == 0 || format.height == 0 {
            return Err(invalid("timeline.add: width and height must be at least 1"));
        }
        let count = ctx.project.manifest.timelines.len();
        let name = self
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("Timeline {}", count + 1));
        let id = ctx.project.allocate_id(IdKind::Timeline);
        let mut doc = Timeline {
            id: id.clone(),
            name: name.clone(),
            format,
            tracks: Vec::new(),
            markers: Vec::new(),
        };
        let mut pictures = Vec::new();
        let mut sounds = Vec::new();
        let mut at = Flicks::ZERO;
        for asset in &assets {
            let length = match asset.info.duration {
                Some(d) if asset.kind != AssetKind::Image => {
                    rate.frames_to_flicks(rate.frame_at(d))
                }
                _ => rate.snap(Flicks::from_millis(3000)),
            };
            if !length.is_positive() {
                continue;
            }
            let clip_id = ctx.project.allocate_id(IdKind::Clip);
            let (content, list) = match asset.kind {
                AssetKind::Video => (
                    ClipContent::Video {
                        asset: asset.id.clone(),
                        source_in: Flicks::ZERO,
                    },
                    &mut pictures,
                ),
                AssetKind::Image => (
                    ClipContent::Image {
                        asset: asset.id.clone(),
                    },
                    &mut pictures,
                ),
                _ => (
                    ClipContent::Audio {
                        asset: asset.id.clone(),
                        source_in: Flicks::ZERO,
                    },
                    &mut sounds,
                ),
            };
            list.push(Clip::new(clip_id, at, length, content));
            at = at + length;
        }
        for (kind, clips) in [(TrackKind::Video, pictures), (TrackKind::Audio, sounds)] {
            if clips.is_empty() {
                continue;
            }
            let track = ctx.project.allocate_id(IdKind::Track);
            doc.tracks.push(Track {
                id: track,
                kind,
                name: None,
                muted: false,
                hidden: false,
                locked: false,
                caption_style: None,
                clips,
            });
        }
        ctx.project.manifest.timelines.push(TimelineEntry {
            role: Default::default(),
            id: id.clone(),
            name,
        });
        ctx.project.timelines.insert(id.clone(), doc);
        if self.primary {
            ctx.project.manifest.primary_timeline = id.clone();
        }
        Ok(TimelineReport::of(ctx, &id))
    }
}

/// Renames a timeline.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimelineRename {
    /// The timeline; the primary one when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Its new name.
    pub name: String,
}

impl Command for TimelineRename {
    const NAME: &'static str = "timeline.rename";
    const SUMMARY: &'static str = "Rename a timeline.";
    type Output = TimelineReport;

    fn apply(self, ctx: &mut Context<'_>) -> Result<TimelineReport, ApiError> {
        let id = timeline_id(ctx.project, self.timeline.as_ref())?;
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(invalid("timeline.rename: the name must not be empty"));
        }
        if let Some(doc) = ctx.project.timelines.get_mut(&id) {
            doc.name = name.clone();
        }
        for entry in &mut ctx.project.manifest.timelines {
            if entry.id == id {
                entry.name = name.clone();
            }
        }
        Ok(TimelineReport::of(ctx, &id))
    }
}

/// Makes a timeline the primary one.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimelineSetPrimary {
    /// The timeline.
    pub timeline: Id,
}

impl Command for TimelineSetPrimary {
    const NAME: &'static str = "timeline.set_primary";
    const SUMMARY: &'static str = "Make a timeline the primary one: the timeline the app shows and commands, frames and exports use when given no timeline.";
    type Output = TimelineReport;

    fn apply(self, ctx: &mut Context<'_>) -> Result<TimelineReport, ApiError> {
        let id = timeline_id(ctx.project, Some(&self.timeline))?;
        if ctx.project.manifest.primary_timeline == id {
            return Err(invalid(format!(
                "timeline.set_primary: {id} is already the primary timeline"
            )));
        }
        ctx.project.manifest.primary_timeline = id.clone();
        Ok(TimelineReport::of(ctx, &id))
    }
}

/// Removes a timeline from the project.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimelineRemove {
    /// The timeline. The project keeps at least one; removing the primary one makes the
    /// first remaining timeline primary.
    pub timeline: Id,
}

/// Result of `timeline.remove`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TimelineRemoved {
    /// The timeline removed.
    pub removed: Id,
    /// The primary timeline now.
    pub primary: Id,
}

impl Command for TimelineRemove {
    const NAME: &'static str = "timeline.remove";
    const SUMMARY: &'static str = "Remove a timeline (allow_destructive when it has clips). A project keeps at least one timeline; removing the primary one makes the next primary.";
    type Output = TimelineRemoved;

    fn apply(self, ctx: &mut Context<'_>) -> Result<TimelineRemoved, ApiError> {
        let id = timeline_id(ctx.project, Some(&self.timeline))?;
        let shown_by: Vec<Id> = ctx
            .project
            .timelines
            .values()
            .flat_map(|t| t.tracks.iter().flat_map(|tr| &tr.clips))
            .filter(|c| c.content.nested() == Some(&id))
            .map(|c| c.id.clone())
            .collect();
        if !shown_by.is_empty() {
            return Err(ApiError::new(
                codes::TIMELINE_IN_USE,
                format!("timeline.remove: compound clips show {id}"),
            )
            .with_details(json!({ "clips": shown_by }))
            .with_hint("Remove or decompose those compound clips first."));
        }
        if ctx.project.manifest.timelines.len() < 2 {
            return Err(invalid(
                "timeline.remove: a project keeps at least one timeline",
            ));
        }
        let clips: usize = ctx.project.timelines[&id]
            .tracks
            .iter()
            .map(|t| t.clips.len())
            .sum();
        if clips > 0 {
            ctx.require_destructive(
                "Removing a timeline with clips",
                json!({ "timeline": id, "clips": clips }),
            )?;
        }
        ctx.project.timelines.remove(&id);
        ctx.project.manifest.timelines.retain(|t| t.id != id);
        if ctx.project.manifest.primary_timeline == id
            && let Some(first) = ctx.project.manifest.timelines.first()
        {
            ctx.project.manifest.primary_timeline = first.id.clone();
        }
        Ok(TimelineRemoved {
            removed: id,
            primary: ctx.project.manifest.primary_timeline.clone(),
        })
    }
}
