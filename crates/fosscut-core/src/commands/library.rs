//! The media bin: renaming, removing and relinking assets, replacing them in every clip,
//! and folders. An asset's identity never changes (ADR-0004, item 3); its name, folder and
//! path hints are how a project organizes and finds it.

use fosscut_model::{
    AssetKind, ClipContent, ContentIdentity, Id, PathHints, folder_and_parents, valid_folder,
};
use fosscut_time::TimeReport;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::util::tidy_groups;
use crate::{ApiError, Command, Context, codes};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

fn not_found(asset: &Id) -> ApiError {
    ApiError::new(
        codes::ASSET_NOT_FOUND,
        format!("asset {asset} does not exist"),
    )
    .with_hint("media.list lists the project's assets.")
}

/// The clips showing `asset`, across every timeline.
fn users(project: &fosscut_model::Project, asset: &Id) -> Vec<Id> {
    project
        .timelines
        .values()
        .flat_map(|t| t.tracks.iter().flat_map(|tr| &tr.clips))
        .filter(|c| c.content.asset() == Some(asset))
        .map(|c| c.id.clone())
        .collect()
}

/// An asset after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AssetChanged {
    /// The asset.
    pub asset: Id,
    /// Its display name.
    pub name: String,
    /// Its folder; the top level when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

impl AssetChanged {
    fn of(asset: &fosscut_model::Asset) -> AssetChanged {
        AssetChanged {
            asset: asset.id.clone(),
            name: asset.name.clone(),
            folder: asset.folder.clone(),
        }
    }
}

/// Renames an asset in the media bin; the file is not touched.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaRename {
    /// The asset.
    pub asset: Id,
    /// Its new display name.
    pub name: String,
}

impl Command for MediaRename {
    const NAME: &'static str = "media.rename";
    const SUMMARY: &'static str =
        "Rename an asset in the media bin. Only the display name changes, not the file.";
    type Output = AssetChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<AssetChanged, ApiError> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(invalid("media.rename: the name must not be empty"));
        }
        let asset = ctx
            .project
            .media
            .assets
            .iter_mut()
            .find(|a| a.id == self.asset)
            .ok_or_else(|| not_found(&self.asset))?;
        asset.name = name;
        Ok(AssetChanged::of(asset))
    }
}

/// Removes an asset from the media bin.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaRemove {
    /// The asset.
    pub asset: Id,
    /// Also remove the clips that show it (needs allow_destructive). Without it, an asset
    /// in use fails with `fosscut.asset.in_use`.
    #[serde(default)]
    pub remove_clips: bool,
}

/// Result of `media.remove`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaRemoved {
    /// The asset removed.
    pub asset: Id,
    /// The clips removed with it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<Id>,
}

impl Command for MediaRemove {
    const NAME: &'static str = "media.remove";
    const SUMMARY: &'static str = "Remove an asset from the media bin (never the file). An asset that clips show fails with fosscut.asset.in_use, unless remove_clips (with allow_destructive) removes them too.";
    type Output = MediaRemoved;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MediaRemoved, ApiError> {
        if ctx.project.media.get(&self.asset).is_none() {
            return Err(not_found(&self.asset));
        }
        let clips = users(ctx.project, &self.asset);
        if !clips.is_empty() {
            if !self.remove_clips {
                return Err(ApiError::new(
                    codes::ASSET_IN_USE,
                    format!("media.remove: {} clips show {}", clips.len(), self.asset),
                )
                .with_details(json!({ "clips": clips }))
                .with_hint(
                    "Remove those clips first, or pass remove_clips: true with allow_destructive.",
                ));
            }
            ctx.require_destructive(
                "Removing an asset with the clips that show it",
                json!({ "asset": self.asset, "clips": clips }),
            )?;
            for timeline in ctx.project.timelines.values_mut() {
                for track in &mut timeline.tracks {
                    if track.locked && track.clips.iter().any(|c| clips.contains(&c.id)) {
                        return Err(ApiError::new(
                            codes::TRACK_LOCKED,
                            format!("track {} is locked", track.id),
                        ));
                    }
                    track.clips.retain(|c| !clips.contains(&c.id));
                }
                tidy_groups(timeline);
            }
        }
        ctx.project.media.assets.retain(|a| a.id != self.asset);
        Ok(MediaRemoved {
            asset: self.asset,
            clips,
        })
    }
}

/// Puts assets in a folder of the media bin.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaMove {
    /// The assets.
    pub assets: Vec<Id>,
    /// The folder, such as `B-roll/Day 1`; created with its parents when missing. Empty or
    /// omitted moves them to the top level.
    #[serde(default)]
    pub folder: Option<String>,
}

/// Result of `media.move`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaMoved {
    /// The assets moved.
    pub assets: Vec<AssetChanged>,
}

/// Adds `folder` and its parents to the bin's sorted folder list.
fn ensure_folder(media: &mut fosscut_model::Media, folder: &str) {
    for path in folder_and_parents(folder) {
        if let Err(index) = media.folders.binary_search(&path) {
            media.folders.insert(index, path);
        }
    }
}

fn checked_folder(folder: &str, command: &str) -> Result<(), ApiError> {
    if valid_folder(folder) {
        Ok(())
    } else {
        Err(invalid(format!(
            "{command}: {folder:?} is not a folder path: names separated by /, none empty"
        )))
    }
}

impl Command for MediaMove {
    const NAME: &'static str = "media.move";
    const SUMMARY: &'static str = "Put assets in a media bin folder such as B-roll/Day 1 (created when missing), or back at the top level.";
    type Output = MediaMoved;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MediaMoved, ApiError> {
        if self.assets.is_empty() {
            return Err(invalid("media.move: give the assets to move"));
        }
        let folder = self.folder.filter(|f| !f.is_empty());
        if let Some(folder) = &folder {
            checked_folder(folder, Self::NAME)?;
            ensure_folder(&mut ctx.project.media, folder);
        }
        let mut moved = Vec::new();
        for id in &self.assets {
            let asset = ctx
                .project
                .media
                .assets
                .iter_mut()
                .find(|a| &a.id == id)
                .ok_or_else(|| not_found(id))?;
            asset.folder = folder.clone();
            moved.push(AssetChanged::of(asset));
        }
        Ok(MediaMoved { assets: moved })
    }
}

/// The folders of the bin after a change.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Folders {
    /// Every folder, sorted.
    pub folders: Vec<String>,
}

/// Adds, renames or removes a folder of the media bin.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaFolder {
    /// `add`, `rename` or `remove`.
    pub action: FolderAction,
    /// The folder, such as `B-roll/Day 1`.
    pub folder: String,
    /// For `rename`: the new path. Subfolders and the assets in them move along.
    #[serde(default)]
    pub to: Option<String>,
}

/// What `media.folder` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FolderAction {
    /// Create it, with its parents.
    Add,
    /// Rename or move it.
    Rename,
    /// Remove it; its assets and subfolders move to its parent.
    Remove,
}

impl Command for MediaFolder {
    const NAME: &'static str = "media.folder";
    const SUMMARY: &'static str = "Add, rename (or move) or remove a media bin folder. Removing one moves its assets and subfolders up to its parent; assets are never removed.";
    type Output = Folders;

    fn apply(self, ctx: &mut Context<'_>) -> Result<Folders, ApiError> {
        checked_folder(&self.folder, Self::NAME)?;
        let media = &mut ctx.project.media;
        let exists = media.folders.contains(&self.folder);
        let within =
            |path: &str, folder: &str| path == folder || path.starts_with(&format!("{folder}/"));
        match self.action {
            FolderAction::Add => ensure_folder(media, &self.folder),
            FolderAction::Rename | FolderAction::Remove if !exists => {
                return Err(invalid(format!(
                    "media.folder: there is no folder {}",
                    self.folder
                )));
            }
            FolderAction::Rename => {
                let to = self
                    .to
                    .as_deref()
                    .ok_or_else(|| invalid("media.folder: rename needs to"))?;
                checked_folder(to, Self::NAME)?;
                if within(to, &self.folder) {
                    return Err(invalid("media.folder: a folder cannot move into itself"));
                }
                let moved = |path: &str| format!("{to}{}", &path[self.folder.len()..]);
                let renamed: Vec<String> = media
                    .folders
                    .iter()
                    .filter(|f| within(f, &self.folder))
                    .map(|f| moved(f))
                    .collect();
                media.folders.retain(|f| !within(f, &self.folder));
                for folder in renamed {
                    ensure_folder(media, &folder);
                }
                for asset in &mut media.assets {
                    if let Some(folder) = asset.folder.clone()
                        && within(&folder, &self.folder)
                    {
                        asset.folder = Some(moved(&folder));
                    }
                }
            }
            FolderAction::Remove => {
                let parent = self.folder.rsplit_once('/').map(|(p, _)| p.to_string());
                let up = |path: &str| match &parent {
                    Some(parent) => format!("{parent}{}", &path[self.folder.len()..]),
                    None => path[self.folder.len()..]
                        .trim_start_matches('/')
                        .to_string(),
                };
                let kept: Vec<String> = media
                    .folders
                    .iter()
                    .filter(|f| f.as_str() != self.folder && within(f, &self.folder))
                    .map(|f| up(f))
                    .collect();
                media.folders.retain(|f| !within(f, &self.folder));
                for folder in kept.into_iter().filter(|f| !f.is_empty()) {
                    ensure_folder(media, &folder);
                }
                for asset in &mut media.assets {
                    if let Some(folder) = asset.folder.clone()
                        && within(&folder, &self.folder)
                    {
                        asset.folder = Some(up(&folder)).filter(|f| !f.is_empty());
                    }
                }
            }
        }
        Ok(Folders {
            folders: ctx.project.media.folders.clone(),
        })
    }
}

/// Records where an asset's file is now, after it was moved or renamed on disk.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaRelink {
    /// The asset.
    pub asset: Id,
    /// Where the file is now.
    pub path: PathHints,
    /// The file's content identity, as front ends measure it; it must match the asset's.
    #[serde(default)]
    pub identity: Option<ContentIdentity>,
}

impl Command for MediaRelink {
    const NAME: &'static str = "media.relink";
    const SUMMARY: &'static str = "Point an asset at its file's new location (front ends measure the file; the CLI's media relink and the MCP media_import with relink do both). The content must be the same file (fosscut.asset.identity_mismatch); for other media use media.replace.";
    type Output = AssetChanged;

    fn apply(self, ctx: &mut Context<'_>) -> Result<AssetChanged, ApiError> {
        if self.path.absolute.is_none() && self.path.relative.is_none() {
            return Err(invalid(
                "media.relink: path needs an absolute or relative path",
            ));
        }
        let asset = ctx
            .project
            .media
            .assets
            .iter_mut()
            .find(|a| a.id == self.asset)
            .ok_or_else(|| not_found(&self.asset))?;
        if let (Some(known), Some(given)) = (&asset.identity, &self.identity)
            && known != given
        {
            return Err(ApiError::new(
                codes::ASSET_IDENTITY_MISMATCH,
                format!("media.relink: that file is not the media of {}", asset.id),
            )
            .with_details(json!({ "expected": known, "found": given }))
            .with_hint("Import the other file and use media.replace to swap it in."));
        }
        asset.path = self.path;
        if asset.identity.is_none() {
            asset.identity = self.identity;
        }
        Ok(AssetChanged::of(asset))
    }
}

/// Shows other media in every clip that shows an asset.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaReplace {
    /// The asset to replace.
    pub asset: Id,
    /// The asset that takes its place in every clip.
    pub with: Id,
}

/// Result of `media.replace`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MediaReplaced {
    /// The clips that show the new asset now.
    pub clips: Vec<Id>,
}

impl Command for MediaReplace {
    const NAME: &'static str = "media.replace";
    const SUMMARY: &'static str = "Replace an asset with another in every clip that shows it, keeping each clip's place, length and source position. The new media must be long enough (fosscut.clip.source_range). The old asset stays in the bin.";
    type Output = MediaReplaced;

    fn apply(self, ctx: &mut Context<'_>) -> Result<MediaReplaced, ApiError> {
        let old = ctx
            .project
            .media
            .get(&self.asset)
            .cloned()
            .ok_or_else(|| not_found(&self.asset))?;
        let new = ctx
            .project
            .media
            .get(&self.with)
            .cloned()
            .ok_or_else(|| not_found(&self.with))?;
        let fits = match old.kind {
            AssetKind::Video | AssetKind::Image | AssetKind::Lut => new.kind == old.kind,
            AssetKind::Audio => new.has_audio(),
        };
        if !fits || self.asset == self.with {
            return Err(invalid(format!(
                "media.replace: {:?} asset {} cannot take the place of {:?} asset {}",
                new.kind, new.id, old.kind, old.id
            )));
        }
        let mut clips = Vec::new();
        for timeline in ctx.project.timelines.values_mut() {
            let rate = timeline.format.rate;
            for track in &mut timeline.tracks {
                for clip in &mut track.clips {
                    if clip.content.asset() != Some(&self.asset) {
                        continue;
                    }
                    if track.locked {
                        return Err(ApiError::new(
                            codes::TRACK_LOCKED,
                            format!("track {} is locked", track.id),
                        ));
                    }
                    let used = clip.source_length();
                    match &mut clip.content {
                        ClipContent::Video { asset, source_in }
                        | ClipContent::Audio { asset, source_in } => {
                            if let Some(total) = new.info.duration
                                && *source_in + used > total
                            {
                                return Err(ApiError::new(
                                    codes::CLIP_SOURCE_RANGE,
                                    format!(
                                        "media.replace: {} is too short for {}",
                                        new.name, clip.id
                                    ),
                                )
                                .with_details(json!({
                                    "clip": clip.id,
                                    "needs": TimeReport::new(*source_in + used, rate),
                                    "has": TimeReport::new(total, rate),
                                })));
                            }
                            *asset = new.id.clone();
                        }
                        ClipContent::Image { asset } => *asset = new.id.clone(),
                        _ => {}
                    }
                    clips.push(clip.id.clone());
                }
            }
        }
        // LUT assets are referenced by effects, not clips.
        if old.kind == AssetKind::Lut {
            for clip in ctx
                .project
                .timelines
                .values_mut()
                .flat_map(|t| t.tracks.iter_mut().flat_map(|tr| tr.clips.iter_mut()))
            {
                let mut changed = false;
                for effect in &mut clip.effects {
                    for value in effect.params.values_mut() {
                        if let fosscut_model::ParamValue::Asset(reference) = value
                            && reference.asset == self.asset
                        {
                            reference.asset = self.with.clone();
                            changed = true;
                        }
                    }
                }
                if changed {
                    clips.push(clip.id.clone());
                }
            }
        }
        Ok(MediaReplaced { clips })
    }
}
