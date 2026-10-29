//! Media analysis for front ends (ADR-0019, item 4): an asset's cached results come back at
//! once; anything missing is measured from its file and cached for next time.

use std::path::PathBuf;
use std::time::Instant;

use fosscut_analysis::decode::{self, Media};
use fosscut_analysis::{AnalysisError, Needs, cache};
pub use fosscut_analysis::{Analyzer, Options, Results, SceneOptions, SilenceOptions};
use fosscut_model::{AssetKind, ContentIdentity, Id, Project};
use serde::{Deserialize, Serialize};

use crate::export::resolve;
use crate::{EngineError, Progress};

/// What to analyze, and where results are cached.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisRequest {
    /// The project bundle; relative media paths resolve against it.
    pub bundle: PathBuf,
    /// The asset (`asset_…`).
    pub asset: Id,
    /// Analyzers to run; every analyzer the asset's streams allow when empty.
    pub analyzers: Vec<Analyzer>,
    /// Their options.
    pub options: Options,
    /// Measure again even when results are cached.
    pub refresh: bool,
    /// The cache directory, normally [`crate::cache_dir`].
    pub cache: PathBuf,
}

/// An asset's analysis results.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalysisReport {
    /// The asset.
    pub asset: String,
    /// Results by analyzer, with times from the start of the asset's file.
    pub results: Results,
    /// Analyzers answered from the cache.
    pub cached: Vec<Analyzer>,
    /// Analyzers measured now.
    pub computed: Vec<Analyzer>,
    /// Wall-clock time in milliseconds.
    pub elapsed_ms: u64,
}

/// The analyzers to run, their cache keys and what is cached already.
struct Plan {
    analyzers: Vec<Analyzer>,
    identity: ContentIdentity,
    cached: Results,
    missing: Vec<Analyzer>,
}

fn plan(project: &Project, request: &AnalysisRequest) -> Result<Plan, EngineError> {
    let asset = project
        .media
        .get(&request.asset)
        .ok_or_else(|| EngineError::AssetNotFound {
            asset: request.asset.to_string(),
        })?;
    let has_audio = asset.info.audio.is_some();
    let has_video = asset.kind == AssetKind::Video && asset.info.video.is_some();
    let analyzers: Vec<Analyzer> = if request.analyzers.is_empty() {
        Analyzer::ALL
            .into_iter()
            .filter(|a| match a.needs() {
                Needs::Audio => has_audio,
                Needs::Video => has_video,
            })
            .collect()
    } else {
        let mut chosen = request.analyzers.clone();
        chosen.sort_unstable();
        chosen.dedup();
        chosen
    };
    for analyzer in &analyzers {
        match analyzer.needs() {
            Needs::Audio if !has_audio => {
                return Err(EngineError::NoAudio {
                    asset: request.asset.to_string(),
                });
            }
            Needs::Video if !has_video => {
                return Err(EngineError::NoVideo {
                    asset: request.asset.to_string(),
                });
            }
            _ => {}
        }
    }
    let identity = match &asset.identity {
        Some(identity) => identity.clone(),
        None => {
            let path = resolve(project, &request.bundle, &request.asset)?;
            crate::import::identity(&path).map_err(|e| EngineError::Failed {
                code: e.code,
                message: e.message,
            })?
        }
    };
    let mut cached = Results::default();
    let mut missing = Vec::new();
    for &analyzer in &analyzers {
        let options = request.options.for_analyzer(analyzer);
        let path = cache::entry_path(&request.cache, &identity, analyzer, &options);
        let hit = (!request.refresh)
            .then(|| cache::read(&path, analyzer, &options))
            .flatten()
            .is_some_and(|value| cached.set(analyzer, value));
        if !hit {
            missing.push(analyzer);
        }
    }
    Ok(Plan {
        analyzers,
        identity,
        cached,
        missing,
    })
}

/// The report, if every requested analyzer is cached; nothing is decoded.
pub fn cached(
    project: &Project,
    request: &AnalysisRequest,
) -> Result<Option<AnalysisReport>, EngineError> {
    let started = Instant::now();
    let plan = plan(project, request)?;
    Ok(plan.missing.is_empty().then(|| AnalysisReport {
        asset: request.asset.to_string(),
        results: plan.cached,
        cached: plan.analyzers,
        computed: Vec::new(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}

/// Analyzes `request.asset`, measuring whatever is not cached and caching it; `progress`
/// may cancel by returning `false`.
pub fn analyze(
    project: &Project,
    request: &AnalysisRequest,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<AnalysisReport, EngineError> {
    let started = Instant::now();
    let plan = plan(project, request)?;
    let mut results = plan.cached;
    if !plan.missing.is_empty() {
        let path = resolve(project, &request.bundle, &request.asset)?;
        let info = &project
            .media
            .get(&request.asset)
            .map(|a| a.info.clone())
            .unwrap_or_default();
        let media = Media {
            path: &path,
            duration: info.duration.unwrap_or_default(),
            audio_channels: info.audio.as_ref().map(|a| a.channels),
            video_rate: info.video.as_ref().and_then(|v| v.rate),
        };
        let measured = decode::analyze(media, &plan.missing, &request.options, &mut |p| {
            progress(Progress {
                done: p.done,
                total: p.total,
            })
        })
        .map_err(|e| match e {
            AnalysisError::NoAudio => EngineError::NoAudio {
                asset: request.asset.to_string(),
            },
            AnalysisError::NoVideo => EngineError::NoVideo {
                asset: request.asset.to_string(),
            },
            AnalysisError::Cancelled => EngineError::AnalysisCancelled,
            AnalysisError::Media(media) => EngineError::from(media),
        })?;
        for &analyzer in &plan.missing {
            if let Some(value) = measured.get(analyzer) {
                let options = request.options.for_analyzer(analyzer);
                let path = cache::entry_path(&request.cache, &plan.identity, analyzer, &options);
                // The cache only saves time; failing to write it does not fail the analysis.
                let _ = cache::write(&path, analyzer, &options, &value);
                results.set(analyzer, value);
            }
        }
    }
    let cached = plan
        .analyzers
        .iter()
        .copied()
        .filter(|a| !plan.missing.contains(a))
        .collect();
    Ok(AnalysisReport {
        asset: request.asset.to_string(),
        results,
        cached,
        computed: plan.missing,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Parses analyzer names, failing with the list of known ones.
pub fn analyzers(names: &[String]) -> Result<Vec<Analyzer>, EngineError> {
    names
        .iter()
        .map(|name| {
            Analyzer::parse(name).ok_or_else(|| EngineError::UnknownAnalyzer {
                name: name.clone(),
                known: Analyzer::ALL.iter().map(|a| a.name().to_string()).collect(),
            })
        })
        .collect()
}
