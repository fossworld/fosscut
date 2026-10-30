use fosscut_model::{Id, IdKind, Track, TrackKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::util::{timeline_id, timeline_mut};
use crate::{ApiError, Command, Context, codes};

/// Adds an empty track to a timeline.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackAdd {
    /// What the track holds.
    pub kind: TrackKind,
    /// Timeline; the primary timeline when omitted.
    #[serde(default)]
    pub timeline: Option<Id>,
    /// Optional display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Position in the track list (0 is the bottom layer); on top when omitted.
    #[serde(default)]
    pub index: Option<usize>,
}

/// Result of `track.add`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TrackAdded {
    /// The new track id.
    pub track: Id,
    /// Its position in the track list.
    pub index: usize,
    /// The timeline it belongs to.
    pub timeline: Id,
}

impl Command for TrackAdd {
    const NAME: &'static str = "track.add";
    const SUMMARY: &'static str =
        "Add an empty video, audio or text track; later tracks draw on top of earlier ones.";
    type Output = TrackAdded;

    fn apply(self, ctx: &mut Context<'_>) -> Result<TrackAdded, ApiError> {
        let timeline = timeline_id(ctx.project, self.timeline.as_ref())?;
        let id = ctx.project.allocate_id(IdKind::Track);
        let tracks = &mut timeline_mut(ctx.project, &timeline)?.tracks;
        let index = self.index.unwrap_or(tracks.len());
        if index > tracks.len() {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                format!(
                    "track.add: index {index} is beyond the {} existing tracks",
                    tracks.len()
                ),
            ));
        }
        tracks.insert(
            index,
            Track {
                id: id.clone(),
                kind: self.kind,
                name: self.name,
                muted: false,
                hidden: false,
                locked: false,
                caption_style: None,
                clips: Vec::new(),
            },
        );
        Ok(TrackAdded {
            track: id,
            index,
            timeline,
        })
    }
}
