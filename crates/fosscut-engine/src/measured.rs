//! The newest cached analyses of an asset, whatever options they were measured with, for the
//! planners that read them ([`crate::reframe`], [`crate::shorts`]), as the effects that follow
//! a subject do ([`crate::analysis::subject`]). Reading one marks it used, so the cache
//! policy keeps it (ADR-0032).

use std::path::Path;

use fosscut_analysis::{Analyzer, MatteModel, MatteTrack, PoseTrack, cache};
use fosscut_model::{Id, Project};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::EngineError;

/// The newest cached result of `analyzer` for `asset` whose options `accept` takes, with
/// the path of its entry; `None` when there is none.
pub(crate) fn newest<T: DeserializeOwned>(
    project: &Project,
    bundle: &Path,
    cache: &Path,
    asset: &Id,
    analyzer: Analyzer,
    accept: &dyn Fn(&Value) -> bool,
) -> Result<Option<(T, std::path::PathBuf)>, EngineError> {
    let identity = crate::analysis::identity(project, bundle, asset)?;
    Ok(cache::entries(cache, &identity, analyzer)
        .into_iter()
        .filter(|entry| accept(&entry.options))
        .find_map(|entry| {
            let result: T = serde_json::from_value(entry.result).ok()?;
            crate::cache::touch(&entry.path);
            Some((result, entry.path))
        }))
}

/// The error that names an analysis to run first, for `needed_by` (such as `reframe`).
pub(crate) fn missing(
    asset: &Id,
    analyzer: Analyzer,
    options: Value,
    needed_by: &str,
) -> EngineError {
    EngineError::AnalysisMissing {
        asset: asset.to_string(),
        analyzer: analyzer.name().to_string(),
        options,
        effect: needed_by.to_string(),
    }
}

/// The newest cached pose track of `asset`, with its data file.
pub(crate) fn pose(
    project: &Project,
    bundle: &Path,
    cache: &Path,
    asset: &Id,
) -> Result<Option<PoseTrack>, EngineError> {
    let identity = crate::analysis::identity(project, bundle, asset)?;
    Ok(cache::entries(cache, &identity, Analyzer::Pose)
        .into_iter()
        .find_map(|entry| {
            let result: fosscut_analysis::Pose = serde_json::from_value(entry.result).ok()?;
            let data = cache::read_data(&entry.path, &result.data)?;
            crate::cache::touch(&entry.path);
            crate::cache::touch(&cache::data_path(&entry.path));
            PoseTrack::decode(&data).ok()
        }))
}

/// The newest cached matte track of `asset` made by `model`, with its data file.
pub(crate) fn matte(
    project: &Project,
    bundle: &Path,
    cache: &Path,
    asset: &Id,
    model: MatteModel,
) -> Result<Option<MatteTrack>, EngineError> {
    let identity = crate::analysis::identity(project, bundle, asset)?;
    Ok(cache::entries(cache, &identity, Analyzer::Matte)
        .into_iter()
        .filter(|entry| entry.options["model"] == model.name())
        .find_map(|entry| {
            let result: fosscut_analysis::Matte = serde_json::from_value(entry.result).ok()?;
            let data = cache::read_data(&entry.path, &result.data)?;
            crate::cache::touch(&entry.path);
            crate::cache::touch(&cache::data_path(&entry.path));
            MatteTrack::decode(&data).ok()
        }))
}

/// The options that make a matte of `model`, as `media_analyze` takes them.
pub(crate) fn matte_options(model: MatteModel) -> Value {
    json!({ "matte": { "model": model.name() } })
}

/// Any options.
pub(crate) fn any(_: &Value) -> bool {
    true
}
