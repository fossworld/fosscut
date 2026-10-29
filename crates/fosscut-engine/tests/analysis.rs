//! Analysis through the engine (ADR-0019, item 4): the analyzers each asset allows, cached
//! results returned without decoding, refreshes, typed errors and background jobs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{project, renderer, workspace};
use fosscut_analysis::{Analyzer, Options};
use fosscut_engine::analysis::{AnalysisRequest, analyze, analyzers, cached};
use fosscut_engine::jobs::{JobState, Jobs};
use fosscut_model::{AssetKind, Id, IdKind, Project};

fn request(project: &Project, analyzers: Vec<Analyzer>, cache: &str) -> AnalysisRequest {
    let asset = project.media.assets[0].id.clone();
    AnalysisRequest {
        bundle: workspace().join("Show.fosscut"),
        asset,
        analyzers,
        options: Options::default(),
        refresh: false,
        cache: workspace().join(cache),
    }
}

#[test]
fn results_are_measured_once_then_cached() {
    // The fixture needs FFmpeg; the renderer check also asks for it.
    if renderer().is_none() {
        return;
    }
    let project = project("../clip.mp4");
    let all = request(&project, Vec::new(), "cache-measured");
    assert_eq!(cached(&project, &all).unwrap(), None, "nothing cached yet");

    // With no analyzers named, a video with sound gets all three.
    let first = analyze(&project, &all, &mut |_| true).unwrap();
    assert_eq!(
        first.computed,
        [Analyzer::Loudness, Analyzer::Silences, Analyzer::Scenes]
    );
    assert!(first.cached.is_empty());
    let loudness = first.results.loudness.as_ref().unwrap();
    assert!(loudness.integrated_lufs.unwrap() < -10.0);
    assert!(first.results.silences.as_ref().unwrap().ranges.is_empty());
    assert_eq!(first.results.scenes.as_ref().unwrap().frames, 60);

    // Now everything comes from the cache, without decoding.
    let again = cached(&project, &all).unwrap().unwrap();
    assert_eq!(again.results, first.results);
    assert_eq!(again.cached, first.computed);
    let mut decoded = false;
    let hit = analyze(&project, &all, &mut |_| {
        decoded = true;
        true
    })
    .unwrap();
    assert!(!decoded && hit.computed.is_empty());

    // Other options miss the cache; a refresh measures again.
    let mut stricter = request(&project, vec![Analyzer::Silences], "cache-measured");
    stricter.options.silences.threshold_dbfs = -20.0;
    assert_eq!(cached(&project, &stricter).unwrap(), None);
    let mut refresh = request(&project, vec![Analyzer::Loudness], "cache-measured");
    refresh.refresh = true;
    assert_eq!(
        analyze(&project, &refresh, &mut |_| true).unwrap().computed,
        [Analyzer::Loudness]
    );
}

#[test]
fn impossible_analyses_are_typed() {
    if renderer().is_none() {
        return;
    }
    let project = project("../clip.mp4");
    let mut unknown = request(&project, Vec::new(), "cache-errors");
    unknown.asset = Id::new(IdKind::Asset, 77);
    assert_eq!(
        analyze(&project, &unknown, &mut |_| true)
            .unwrap_err()
            .code(),
        "fosscut.asset.not_found"
    );

    let mut audio_only = project.clone();
    audio_only.media.assets[0].kind = AssetKind::Audio;
    audio_only.media.assets[0].info.video = None;
    let scenes = request(&audio_only, vec![Analyzer::Scenes], "cache-errors");
    assert_eq!(
        analyze(&audio_only, &scenes, &mut |_| true)
            .unwrap_err()
            .code(),
        "fosscut.analysis.no_video"
    );
    let defaults = request(&audio_only, Vec::new(), "cache-errors");
    let audio = analyze(&audio_only, &defaults, &mut |_| true).unwrap();
    assert_eq!(
        audio.computed,
        [Analyzer::Loudness, Analyzer::Silences],
        "only what the asset allows"
    );

    let mut silent = project.clone();
    silent.media.assets[0].info.audio = None;
    let loudness = request(&silent, vec![Analyzer::Loudness], "cache-errors");
    assert_eq!(
        analyze(&silent, &loudness, &mut |_| true)
            .unwrap_err()
            .code(),
        "fosscut.analysis.no_audio"
    );

    let missing = common::project("missing.mp4");
    let gone = request(&missing, vec![Analyzer::Loudness], "cache-missing");
    assert_eq!(
        analyze(&missing, &gone, &mut |_| true).unwrap_err().code(),
        "fosscut.export.missing_media"
    );

    let stopped = request(&project, vec![Analyzer::Scenes], "cache-stopped");
    assert_eq!(
        analyze(&project, &stopped, &mut |_| false)
            .unwrap_err()
            .code(),
        "fosscut.analysis.cancelled"
    );
    assert_eq!(
        cached(&project, &stopped).unwrap(),
        None,
        "a cancelled analysis caches nothing"
    );

    let names = analyzers(&["silences".to_string(), "faces".to_string()]).unwrap_err();
    assert_eq!(names.code(), "fosscut.analysis.unknown_analyzer");
    assert!(names.hint().unwrap().contains("scenes"));
    assert_eq!(
        analyzers(&["scenes".to_string()]).unwrap(),
        [Analyzer::Scenes]
    );
}

#[test]
fn analyses_run_as_jobs() {
    if renderer().is_none() {
        return;
    }
    let jobs = Jobs::new();
    let project = project("../clip.mp4");
    let work = request(
        &project,
        vec![Analyzer::Loudness, Analyzer::Scenes],
        "cache-jobs",
    );
    let started = jobs.start_analysis(project.clone(), work.clone());
    assert_eq!(started.kind, "analysis");
    assert_eq!(started.asset.as_deref(), Some("asset_0001"));
    assert!(started.output.is_none());
    let done = jobs.status(&started.job, Duration::from_secs(120)).unwrap();
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    let report = done.analysis.unwrap();
    assert_eq!(report.computed, [Analyzer::Loudness, Analyzer::Scenes]);
    assert!(done.report.is_none());
    assert_eq!(done.progress.done, done.progress.total);

    // A second identical request after it finished is answered from the cache.
    let again = jobs.start_analysis(project, work);
    let second = jobs.status(&again.job, Duration::from_secs(120)).unwrap();
    assert_eq!(
        second.analysis.unwrap().cached,
        [Analyzer::Loudness, Analyzer::Scenes]
    );
    let serialized = serde_json::to_value(jobs.list()).unwrap();
    assert_eq!(serialized[0]["kind"], "analysis");
    assert!(serialized[0].get("output").is_none());
}
