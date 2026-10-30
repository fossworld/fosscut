//! Splitting a clip at its scene changes (CC-029): the shot cuts `fosscut-analysis` finds in
//! the clip's media (ADR-0019, cached by content) mapped to the timeline through the clip's
//! speed, as `clip.split` calls the front ends apply in one request.

use std::path::PathBuf;

use fosscut_model::{ClipContent, Id, Project};
use fosscut_time::Flicks;
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;
use crate::analysis::{AnalysisRequest, Analyzer, Options, analyze};

/// Where a clip splits, and the calls that split it there.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SceneSplit {
    /// The clip.
    pub clip: Id,
    /// Timeline times of the cuts inside it, in order.
    pub cuts: Vec<Flicks>,
    /// `clip.split` calls, latest first, so the clip keeps its id as the first part.
    pub commands: Vec<Value>,
}

/// Finds the scene changes inside `clip` (measuring its media if the cuts are not cached
/// yet) and returns the splits that make each shot a clip of its own.
pub fn split_at_scenes(
    project: &Project,
    bundle: PathBuf,
    clip: &Id,
    cache: PathBuf,
    models: PathBuf,
    progress: &mut dyn FnMut(crate::Progress) -> bool,
) -> Result<SceneSplit, EngineError> {
    let found = project
        .timelines
        .values()
        .flat_map(|t| t.tracks.iter().flat_map(move |track| track.clips.iter().map(move |c| (t, c))))
        .find(|(_, c)| &c.id == clip);
    let Some((timeline, found)) = found else {
        return Err(EngineError::InvalidArguments {
            message: format!("clip {clip} does not exist"),
        });
    };
    let ClipContent::Video { asset, .. } = &found.content else {
        return Err(EngineError::InvalidArguments {
            message: format!(
                "{clip} is a {} clip; only video clips have scenes",
                found.content.kind_name()
            ),
        });
    };
    let report = analyze(
        project,
        &AnalysisRequest {
            bundle,
            asset: asset.clone(),
            analyzers: vec![Analyzer::Scenes],
            options: Options::default(),
            refresh: false,
            cache,
            models,
        },
        progress,
    )?;
    let rate = timeline.format.rate;
    let mut cuts: Vec<Flicks> = report
        .results
        .scenes
        .map(|s| s.cuts)
        .unwrap_or_default()
        .iter()
        .filter_map(|cut| found.offset_of_source(cut.time.flicks))
        .map(|offset| rate.snap(found.start + offset))
        .filter(|&at| at > found.start && at < found.end())
        .collect();
    cuts.sort();
    cuts.dedup();
    let commands = cuts
        .iter()
        .rev()
        .map(|at| {
            json!({
                "command": "clip.split",
                "args": { "clip": clip, "at": { "flicks": at.get() } },
            })
        })
        .collect();
    Ok(SceneSplit {
        clip: clip.clone(),
        cuts,
        commands,
    })
}
