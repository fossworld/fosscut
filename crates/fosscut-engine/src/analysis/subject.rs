//! Cached analyses for the effects that follow a subject (ADR-0033, items 3 and 4):
//! `key.subject` reads a clip's matte, and `body.outline`, `body.glow`, `body.shadow` and
//! `body.sparkle` its pose and person mask. Models run only in `media_analyze`; exports, frame
//! grabs, contact sheets and the preview read the results from the analysis cache and hand
//! them to the renderer through [`fosscut_render::DecoderFrames::insert_subject`].
//!
//! An effect reads the newest cached analysis of the right kind for its media, whatever other
//! options it was measured with: a `matte` of the effect's `source` model, or a `pose`. Using
//! one marks it used, so it stays the newest and the cache policy keeps it.
//! Exports and frame grabs fail before rendering with `fosscut.analysis.missing`
//! ([`EngineError::AnalysisMissing`]) when one is not cached, naming the asset, analyzer and
//! options to run; the preview shows such clips without the effect until the analysis
//! arrives. Effects on still images draw nothing for now, since only video is analyzed.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fosscut_analysis::{Analyzer, MatteModel, MatteTrack, PoseTrack, cache};
use fosscut_model::{AssetKind, ClipContent, ContentIdentity, Id, Project};
use fosscut_render::DecoderFrames;
use fosscut_render::plan::{LayerSource, RenderPlan};
use fosscut_render::subject::{Landmark, Matte, Pose, SubjectData, need, need_of};
pub use fosscut_render::subject::{MatteKind, Need};
use fosscut_time::Flicks;
use serde_json::{Value, json};

use crate::EngineError;

/// An analysis a clip's effect needs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Requirement {
    /// The asset whose media it analyzes.
    pub asset: Id,
    /// What it reads.
    pub need: Need,
    /// The effect type that needs it.
    pub effect: String,
}

impl Requirement {
    /// The analyzer that makes it.
    pub fn analyzer(&self) -> Analyzer {
        match self.need {
            Need::Matte(_) => Analyzer::Matte,
            Need::Pose => Analyzer::Pose,
        }
    }

    /// The options to run it with, as `media_analyze` takes them.
    pub fn options(&self) -> Value {
        match self.need {
            Need::Matte(MatteKind::Subject) => json!({ "matte": { "model": "subject" } }),
            Need::Matte(MatteKind::Person) => json!({ "matte": { "model": "person" } }),
            Need::Pose => json!({}),
        }
    }

    /// The error that says it is missing.
    pub fn missing(&self) -> EngineError {
        EngineError::AnalysisMissing {
            asset: self.asset.to_string(),
            analyzer: self.analyzer().name().to_string(),
            options: self.options(),
            effect: self.effect.clone(),
        }
    }
}

/// Every analysis the clips of `timeline` need, on video tracks that are shown, in `range`
/// of it (all of it when `None`), and in the timelines of compound clips there.
pub fn timeline_requirements(
    project: &Project,
    timeline: &Id,
    range: Option<(Flicks, Flicks)>,
) -> Vec<Requirement> {
    let mut found = BTreeSet::new();
    let mut visited = BTreeSet::new();
    gather(project, timeline, range, &mut visited, &mut found);
    found.into_iter().collect()
}

fn gather(
    project: &Project,
    timeline: &Id,
    range: Option<(Flicks, Flicks)>,
    visited: &mut BTreeSet<Id>,
    found: &mut BTreeSet<Requirement>,
) {
    if !visited.insert(timeline.clone()) {
        return;
    }
    let Some(doc) = project.timelines.get(timeline) else {
        return;
    };
    for track in doc.tracks.iter().filter(|t| !t.hidden) {
        for clip in &track.clips {
            if !clip.enabled
                || range.is_some_and(|(from, to)| clip.end() <= from || clip.start >= to)
            {
                continue;
            }
            match &clip.content {
                ClipContent::Video { asset, .. } if is_video(project, asset) => {
                    for effect in clip.effects.iter().filter(|e| e.enabled) {
                        let Some(values) = effect.values_at(Flicks::ZERO) else {
                            continue;
                        };
                        if let Some(need) = need_of(&effect.effect_type, &values) {
                            found.insert(Requirement {
                                asset: asset.clone(),
                                need,
                                effect: effect.effect_type.clone(),
                            });
                        }
                    }
                }
                ClipContent::Timeline { timeline, .. } => {
                    gather(project, timeline, None, visited, found);
                }
                _ => {}
            }
        }
    }
}

/// Whether `asset` is video that can be analyzed.
fn is_video(project: &Project, asset: &Id) -> bool {
    project
        .media
        .get(asset)
        .is_some_and(|a| a.kind == AssetKind::Video && a.info.video.is_some())
}

/// Every analysis the layers of `plan` need, inside transitions and compound clips too.
pub fn plan_requirements(project: &Project, plan: &RenderPlan) -> Vec<Requirement> {
    let mut found = BTreeSet::new();
    for layer in plan.layers.iter().flat_map(|layer| layer.flattened()) {
        let LayerSource::Frame(source) = &layer.source else {
            continue;
        };
        if !is_video(project, &source.asset) {
            continue;
        }
        for effect in &layer.effects {
            if let Some(need) = need(effect) {
                found.insert(Requirement {
                    asset: source.asset.clone(),
                    need,
                    effect: effect.effect_type.clone(),
                });
            }
        }
    }
    found.into_iter().collect()
}

/// A cached matte track, with the frame unpacked last.
struct MatteData {
    track: MatteTrack,
    hash: String,
    last: Mutex<Option<(usize, Arc<Matte>)>>,
}

impl MatteData {
    fn at(&self, time: Flicks) -> Option<Arc<Matte>> {
        let index = self.track.at(time)?;
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((cached, matte)) = last.as_ref()
            && *cached == index
        {
            return Some(matte.clone());
        }
        let matte = Arc::new(Matte {
            width: self.track.size.0,
            height: self.track.size.1,
            alpha: self.track.alpha(index)?,
        });
        *last = Some((index, matte.clone()));
        Some(matte)
    }
}

/// A cached pose track, with the mask unpacked last.
struct PoseData {
    track: PoseTrack,
    hash: String,
    last: Mutex<Option<(usize, Arc<Matte>)>>,
}

impl PoseData {
    fn at(&self, time: Flicks) -> Option<Arc<Pose>> {
        let at = self.track.at(time)?;
        let mask = {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            match last.as_ref() {
                Some((cached, mask)) if *cached == at.frame => Some(mask.clone()),
                _ => {
                    let mask = self.track.mask(at.frame).map(|alpha| {
                        Arc::new(Matte {
                            width: self.track.mask_size.0,
                            height: self.track.mask_size.1,
                            alpha,
                        })
                    });
                    if let Some(mask) = &mask {
                        *last = Some((at.frame, mask.clone()));
                    }
                    mask
                }
            }
        };
        Some(Arc::new(Pose {
            presence: at.presence,
            landmarks: at
                .landmarks
                .iter()
                .map(|l| Landmark {
                    x: l.x,
                    y: l.y,
                    visibility: l.visibility.min(l.presence),
                })
                .collect(),
            mask,
        }))
    }
}

/// One asset's cached mattes and pose, as the renderer reads them.
#[derive(Clone, Default)]
pub struct Subject {
    person: Option<Arc<MatteData>>,
    subject: Option<Arc<MatteData>>,
    pose: Option<Arc<PoseData>>,
}

impl Subject {
    /// What it holds, by the hashes of the data files, for keying rendered frames.
    pub fn describe(&self) -> String {
        let hash = |h: Option<&str>| h.map_or("-", |h| &h[..h.len().min(16)]).to_string();
        format!(
            "person={} subject={} pose={}",
            hash(self.person.as_ref().map(|d| d.hash.as_str())),
            hash(self.subject.as_ref().map(|d| d.hash.as_str())),
            hash(self.pose.as_ref().map(|d| d.hash.as_str())),
        )
    }

    fn has(&self, need: Need) -> bool {
        match need {
            Need::Matte(MatteKind::Person) => self.person.is_some(),
            Need::Matte(MatteKind::Subject) => self.subject.is_some(),
            Need::Pose => self.pose.is_some(),
        }
    }
}

impl SubjectData for Subject {
    fn matte(&self, kind: MatteKind, time: Flicks) -> Option<Arc<Matte>> {
        match kind {
            MatteKind::Person => self.person.as_ref()?.at(time),
            MatteKind::Subject => self.subject.as_ref()?.at(time),
        }
    }

    fn pose(&self, time: Flicks) -> Option<Arc<Pose>> {
        self.pose.as_ref()?.at(time)
    }
}

/// How long the preview waits before looking for an analysis it did not find again.
pub(crate) const PREVIEW_RETRY: Duration = Duration::from_secs(2);

/// The analyses loaded for a front end's renders, by asset.
#[derive(Default)]
pub struct Subjects {
    assets: HashMap<Id, Subject>,
    identities: HashMap<Id, Option<ContentIdentity>>,
    /// When each analysis was last looked for and not found.
    misses: HashMap<(Id, Need), Instant>,
}

impl Subjects {
    /// No analyses loaded yet.
    pub fn new() -> Subjects {
        Subjects::default()
    }

    /// Loads what `requirements` need from the analysis cache in `cache` and gives each
    /// asset's to `frames`. Returns the requirements that are not cached; with `retry_after`
    /// set, one looked for in that time already is not looked for again (for the preview).
    pub fn supply(
        &mut self,
        project: &Project,
        bundle: &Path,
        cache: &Path,
        requirements: &[Requirement],
        frames: &mut DecoderFrames,
        retry_after: Option<Duration>,
    ) -> Vec<Requirement> {
        let mut missing = Vec::new();
        let mut changed = BTreeSet::new();
        for requirement in requirements {
            let asset = &requirement.asset;
            if self
                .assets
                .get(asset)
                .is_some_and(|s| s.has(requirement.need))
            {
                continue;
            }
            let key = (asset.clone(), requirement.need);
            if let (Some(wait), Some(at)) = (retry_after, self.misses.get(&key))
                && at.elapsed() < wait
            {
                missing.push(requirement.clone());
                continue;
            }
            let identity = self
                .identities
                .entry(asset.clone())
                .or_insert_with(|| super::identity(project, bundle, asset).ok())
                .clone();
            let loaded = identity
                .as_ref()
                .is_some_and(|id| self.load(cache, id, asset, requirement.need));
            if loaded {
                self.misses.remove(&key);
                changed.insert(asset.clone());
            } else {
                self.misses.insert(key, Instant::now());
                missing.push(requirement.clone());
            }
        }
        for asset in changed {
            if let Some(subject) = self.assets.get(&asset) {
                frames.insert_subject(asset.clone(), Arc::new(subject.clone()));
            }
        }
        missing
    }

    /// Like [`Subjects::supply`] for an export or frame grab: a missing analysis fails with
    /// `fosscut.analysis.missing`.
    pub fn require(
        &mut self,
        project: &Project,
        bundle: &Path,
        cache: &Path,
        requirements: &[Requirement],
        frames: &mut DecoderFrames,
    ) -> Result<(), EngineError> {
        match self
            .supply(project, bundle, cache, requirements, frames, None)
            .first()
        {
            Some(missing) => Err(missing.missing()),
            None => Ok(()),
        }
    }

    /// What is loaded for `asset`, by data file, for keying rendered frames.
    pub fn describe(&self, asset: &Id) -> Option<String> {
        self.assets.get(asset).map(Subject::describe)
    }

    /// Loads `need` for the media with `identity` from `cache`; true when it was cached.
    fn load(&mut self, cache: &Path, identity: &ContentIdentity, asset: &Id, need: Need) -> bool {
        let subject = self.assets.entry(asset.clone()).or_default();
        match need {
            Need::Matte(kind) => {
                let model = match kind {
                    MatteKind::Person => MatteModel::Person,
                    MatteKind::Subject => MatteModel::Subject,
                };
                let Some(data) = load_matte(cache, identity, model) else {
                    return false;
                };
                let data = Some(Arc::new(data));
                match kind {
                    MatteKind::Person => subject.person = data,
                    MatteKind::Subject => subject.subject = data,
                }
                true
            }
            Need::Pose => match load_pose(cache, identity) {
                Some(data) => {
                    subject.pose = Some(Arc::new(data));
                    true
                }
                None => false,
            },
        }
    }
}

/// Whether what `requirement` needs is in the analysis cache in `cache`, with its data file;
/// nothing is read beyond the cache entries.
pub fn is_cached(
    project: &Project,
    bundle: &Path,
    cache: &Path,
    requirement: &Requirement,
) -> bool {
    let Ok(identity) = super::identity(project, bundle, &requirement.asset) else {
        return false;
    };
    let model = match requirement.need {
        Need::Matte(MatteKind::Person) => Some(MatteModel::Person),
        Need::Matte(MatteKind::Subject) => Some(MatteModel::Subject),
        Need::Pose => None,
    };
    cache::entries(cache, &identity, requirement.analyzer())
        .into_iter()
        .filter(|entry| model.is_none_or(|m| entry.options["model"] == m.name()))
        .any(|entry| {
            let file = match requirement.need {
                Need::Pose => serde_json::from_value::<fosscut_analysis::Pose>(entry.result)
                    .ok()
                    .map(|p| p.data),
                Need::Matte(_) => serde_json::from_value::<fosscut_analysis::Matte>(entry.result)
                    .ok()
                    .map(|m| m.data),
            };
            file.is_some_and(|file| cache::has_data(&entry.path, &file))
        })
}

/// Marks the entry at `entry` and its data file used, so the cache policy keeps them
/// (ADR-0032) and they stay the newest of their kind.
fn used(entry: &Path) {
    crate::cache::touch(entry);
    crate::cache::touch(&cache::data_path(entry));
}

/// The newest cached matte of `model` for `identity`, whatever its other options: the one
/// written or used most recently.
fn load_matte(cache: &Path, identity: &ContentIdentity, model: MatteModel) -> Option<MatteData> {
    cache::entries(cache, identity, Analyzer::Matte)
        .into_iter()
        .filter(|entry| entry.options["model"] == model.name())
        .find_map(|entry| {
            let result: fosscut_analysis::Matte = serde_json::from_value(entry.result).ok()?;
            let data = cache::read_data(&entry.path, &result.data)?;
            used(&entry.path);
            Some(MatteData {
                track: MatteTrack::decode(&data).ok()?,
                hash: result.data.blake3,
                last: Mutex::new(None),
            })
        })
}

/// The newest cached pose for `identity`, whatever its options.
fn load_pose(cache: &Path, identity: &ContentIdentity) -> Option<PoseData> {
    cache::entries(cache, identity, Analyzer::Pose)
        .into_iter()
        .find_map(|entry| {
            let result: fosscut_analysis::Pose = serde_json::from_value(entry.result).ok()?;
            let data = cache::read_data(&entry.path, &result.data)?;
            used(&entry.path);
            Some(PoseData {
                track: PoseTrack::decode(&data).ok()?,
                hash: result.data.blake3,
                last: Mutex::new(None),
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_name_what_to_run() {
        let subject = Requirement {
            asset: Id::new(fosscut_model::IdKind::Asset, 3),
            need: Need::Matte(MatteKind::Subject),
            effect: "key.subject".into(),
        };
        assert_eq!(subject.analyzer(), Analyzer::Matte);
        assert_eq!(
            subject.options(),
            json!({ "matte": { "model": "subject" } })
        );
        let error = subject.missing();
        assert_eq!(error.code(), "fosscut.analysis.missing");
        assert_eq!(error.details()["asset"], "asset_0003");
        let pose = Requirement {
            need: Need::Pose,
            effect: "body.glow".into(),
            ..subject
        };
        assert_eq!(pose.analyzer(), Analyzer::Pose);
        assert_eq!(pose.options(), json!({}));
        assert_eq!(Subject::default().describe(), "person=- subject=- pose=-");
    }
}
