//! Analysis through the engine (ADR-0019, item 4): the analyzers each asset allows, cached
//! results returned without decoding, refreshes, typed errors, background jobs, model
//! installs (item 5) and transcripts (ADR-0020).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{project, renderer, workspace};
use fosscut_analysis::{Analyzer, Options};
use fosscut_engine::analysis::{
    AnalysisRequest, ModelSource, analyze, analyzers, cached, install_model, models,
};
use fosscut_engine::jobs::{JobState, Jobs};
use fosscut_model::{AssetKind, Id, IdKind, Project};
use fosscut_time::{FLICKS_PER_SECOND, Flicks};

fn request(project: &Project, analyzers: Vec<Analyzer>, cache: &str) -> AnalysisRequest {
    let asset = project.media.assets[0].id.clone();
    AnalysisRequest {
        bundle: workspace().join("Show.fosscut"),
        asset,
        analyzers,
        options: Options::default(),
        refresh: false,
        cache: workspace().join(cache),
        models: workspace().join("no-models"),
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

#[test]
fn transcripts_need_an_installed_model() {
    if renderer().is_none() {
        return;
    }
    let project = project("../clip.mp4");
    let speech = request(&project, vec![Analyzer::Transcript], "cache-speech");
    let missing = analyze(&project, &speech, &mut |_| true).unwrap_err();
    assert_eq!(missing.code(), "fosscut.model.not_installed");
    assert!(
        missing
            .hint()
            .unwrap()
            .contains("whisper-tiny.en (77.7 MB)"),
        "{:?}",
        missing.hint()
    );
    assert_eq!(
        cached(&project, &speech).unwrap_err().code(),
        missing.code()
    );
    let mut unknown = speech.clone();
    unknown.options.transcript.model = Some("whisper-huge".into());
    let error = analyze(&project, &unknown, &mut |_| true).unwrap_err();
    assert_eq!(error.code(), "fosscut.model.unknown");
    assert!(error.hint().unwrap().contains("whisper-base.en"));
}

#[test]
fn models_install_only_when_confirmed_and_verified() {
    let dir = workspace().join("models-install");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(models(&dir).len(), 4);
    assert!(models(&dir).iter().all(|m| !m.installed));
    let download = ModelSource::Download(None);

    let unknown = install_model(&dir, "whisper-huge", &download, true, false).unwrap_err();
    assert_eq!(unknown.code(), "fosscut.model.unknown");

    // A download needs confirmation; the error carries what to tell the user.
    let unconfirmed = install_model(&dir, "whisper-tiny.en", &download, false, false).unwrap_err();
    assert_eq!(unconfirmed.code(), "fosscut.model.confirmation_required");
    assert_eq!(
        unconfirmed.to_string(),
        "installing whisper-tiny.en downloads 77.7 MB (MIT license) and needs confirmation"
    );
    assert_eq!(
        unconfirmed.details(),
        serde_json::json!({ "name": "whisper-tiny.en", "size_bytes": 77_704_715, "license": "MIT" })
    );
    assert!(unconfirmed.hint().unwrap().contains("--yes"));

    // A dry run reports where it would go.
    let dry = install_model(&dir, "whisper-tiny.en", &download, false, true).unwrap();
    assert_eq!(dry.status, "dry_run");
    assert!(!dry.model.installed && dry.path.ends_with("ggml-tiny.en.bin"));

    // Local files need no confirmation but must match the published checksum.
    let gone = install_model(
        &dir,
        "whisper-tiny.en",
        &ModelSource::File(dir.join("missing.bin")),
        false,
        false,
    )
    .unwrap_err();
    assert_eq!(gone.code(), "fosscut.model.download_failed");
    assert!(gone.hint().unwrap().contains("--from"));
    let wrong = workspace().join("wrong-model.bin");
    std::fs::write(&wrong, b"not a model").unwrap();
    let mismatch = install_model(
        &dir,
        "whisper-tiny.en",
        &ModelSource::File(wrong),
        false,
        false,
    )
    .unwrap_err();
    assert_eq!(mismatch.code(), "fosscut.model.checksum_mismatch");
    assert_eq!(mismatch.details()["actual"], "11 bytes");
    assert!(!models(&dir)[3].installed);

    // An installed model is left as it is, without downloading.
    std::fs::File::create(dir.join("ggml-tiny.en.bin"))
        .unwrap()
        .set_len(77_704_715)
        .unwrap();
    let unchanged = install_model(&dir, "whisper-tiny.en", &download, false, false).unwrap();
    assert_eq!(unchanged.status, "unchanged");
    assert!(unchanged.model.installed);
}

/// A models folder holding only `whisper-tiny.en`, linked or copied from
/// `FOSSCUT_MODELS_DIR`, when FFmpeg and the model are available. CI installs the model and
/// sets `FOSSCUT_REQUIRE_SPEECH`, which turns a missing model into a failure.
fn only_tiny() -> Option<std::path::PathBuf> {
    let source = std::env::var_os("FOSSCUT_MODELS_DIR")
        .map(std::path::PathBuf::from)
        .map(|dir| dir.join("ggml-tiny.en.bin"))
        .filter(|file| std::fs::metadata(file).is_ok_and(|m| m.len() == 77_704_715));
    let required =
        std::env::var_os("FOSSCUT_REQUIRE_SPEECH").is_some_and(|v| !v.is_empty() && v != "0");
    assert!(
        source.is_some() || !required,
        "FOSSCUT_REQUIRE_SPEECH is set but whisper-tiny.en is not in FOSSCUT_MODELS_DIR"
    );
    let source = source?;
    renderer()?;
    let dir = workspace().join("models-tiny");
    let target = dir.join("ggml-tiny.en.bin");
    if !target.exists() {
        std::fs::create_dir_all(&dir).unwrap();
        if std::fs::hard_link(&source, &target).is_err() {
            std::fs::copy(&source, &target).unwrap();
        }
    }
    Some(dir)
}

#[test]
fn transcripts_are_measured_and_cached_per_model() {
    let Some(models) = only_tiny() else {
        return;
    };
    let speech = workspace().join("speech.flac");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/speech/librispeech-5105-28233-0005/audio.flac"
        ),
        &speech,
    )
    .unwrap();
    let mut project = project("../speech.flac");
    let asset = &mut project.media.assets[0];
    asset.kind = AssetKind::Audio;
    asset.info.video = None;
    asset.info.duration = Some(Flicks::new(1139 * (FLICKS_PER_SECOND / 100)));
    asset.info.audio.as_mut().unwrap().sample_rate = 16_000;
    let mut work = request(&project, vec![Analyzer::Transcript], "cache-transcript");
    work.models = models;

    let first = analyze(&project, &work, &mut |_| true).unwrap();
    assert_eq!(first.computed, [Analyzer::Transcript]);
    let transcript = first.results.transcript.unwrap();
    assert_eq!(transcript.model, "whisper-tiny.en");
    assert_eq!(transcript.words.len(), 37);

    // Without a model named, the best installed one is chosen before the cache is asked, so
    // naming it or not finds the same entry.
    let mut named = work.clone();
    named.options.transcript.model = Some("whisper-tiny.en".into());
    let again = cached(&project, &named).unwrap().unwrap();
    assert_eq!(again.cached, [Analyzer::Transcript]);
    assert_eq!(again.results.transcript.unwrap().words, transcript.words);
    let mut other = work.clone();
    other.options.transcript.min_pause_ms = 500;
    assert_eq!(
        cached(&project, &other).unwrap(),
        None,
        "options miss the cache"
    );
}
