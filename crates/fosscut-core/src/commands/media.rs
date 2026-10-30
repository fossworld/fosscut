use fosscut_model::{Asset, AssetKind, ContentIdentity, Id, IdKind, MediaInfo, PathHints};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ApiError, Command, Context, codes};

/// Adds a probed media file to the project's media table.
///
/// Front ends probe the file (the CLI uses ffprobe) and pass the stream information; the
/// core never touches media files itself.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaAdd {
    /// Display name, usually the file name.
    pub name: String,
    /// Media kind.
    pub kind: AssetKind,
    /// Where the file is.
    pub path: PathHints,
    /// Content identity; when it matches an existing asset, that asset is returned instead.
    #[serde(default)]
    pub identity: Option<ContentIdentity>,
    /// Probed stream information.
    #[serde(default)]
    pub info: MediaInfo,
}

/// Result of `media.add`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaAdded {
    /// The asset id.
    pub asset: Id,
    /// True when an identical file was already in the project and nothing was added.
    pub existing: bool,
}

impl Command for MediaAdd {
    const NAME: &'static str = "media.add";
    const SUMMARY: &'static str = "Add a probed media file to the media table (front ends call this after probing; the CLI's `media import` does both).";
    type Output = MediaAdded;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MediaAdded, ApiError> {
        if self.name.trim().is_empty() {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "media.add: name must not be empty",
            ));
        }
        if self.path.absolute.is_none() && self.path.relative.is_none() {
            return Err(ApiError::new(
                codes::INVALID_ARGUMENTS,
                "media.add: path needs an absolute or relative path",
            ));
        }
        let info = &self.info;
        let complete = match self.kind {
            AssetKind::Video => {
                info.video.is_some() && info.duration.is_some_and(|d| d.is_positive())
            }
            AssetKind::Audio => {
                info.audio.is_some() && info.duration.is_some_and(|d| d.is_positive())
            }
            AssetKind::Image => info.video.is_some(),
            AssetKind::Lut => info.lut.is_some(),
        };
        if !complete {
            return Err(ApiError::new(
                codes::MEDIA_INCOMPLETE,
                format!(
                    "{:?} assets need stream information and, for time-based media, a positive duration",
                    self.kind
                ),
            ));
        }
        if let Some(identity) = &self.identity
            && let Some(existing) = ctx
                .project
                .media
                .assets
                .iter()
                .find(|a| a.identity.as_ref() == Some(identity))
        {
            return Ok(MediaAdded {
                asset: existing.id.clone(),
                existing: true,
            });
        }
        let id = ctx.project.allocate_id(IdKind::Asset);
        ctx.project.media.assets.push(Asset {
            folder: None,
            id: id.clone(),
            kind: self.kind,
            name: self.name,
            path: self.path,
            identity: self.identity,
            info: self.info,
        });
        Ok(MediaAdded {
            asset: id,
            existing: false,
        })
    }
}
