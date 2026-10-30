//! Background export jobs (ADR-0006, item 8): they answer at once, report progress and the
//! verified result, cancel cleanly, fail with typed errors and refuse a second writer.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{frames, project, renderer, workspace};
use fosscut_engine::jobs::{JobState, Jobs};
use fosscut_engine::{ExportRequest, Preset};
use fosscut_model::{Clip, Id, IdKind, Project};

const WAIT: Duration = Duration::from_secs(120);

fn request(project: &Project, name: &str) -> ExportRequest {
    let timeline = project.manifest.primary_timeline.clone();
    let preset = Preset::default_video().unwrap();
    let output = workspace().join(format!("{name}.{}", preset.extension()));
    let _ = std::fs::remove_file(&output);
    ExportRequest {
        bundle: workspace().join("Show.fosscut"),
        settings: preset.settings(&project.timelines[&timeline].format),
        timeline,
        output,
        allow_overwrite: false,
        expected_revision: None,
        request_id: None,
        cache: workspace().join("cache"),
        options: Default::default(),
    }
}

/// A 12 s timeline: the fixture's clip repeated, long enough to still be running when a
/// test acts on it.
fn long_project() -> Project {
    let mut project = project("../clip.mp4");
    let timeline = project.manifest.primary_timeline.clone();
    let track = &mut project.timelines.get_mut(&timeline).unwrap().tracks[0];
    let first = track.clips[0].clone();
    track.clips = (0..8)
        .map(|n| Clip {
            id: Id::new(IdKind::Clip, 100 + n),
            start: frames(45 * n as i64),
            duration: frames(45),
            ..first.clone()
        })
        .collect();
    project
}

#[test]
fn exports_run_in_the_background_and_report_their_result() {
    if renderer().is_none() {
        return;
    }
    let jobs = Jobs::new();
    let project = project("../clip.mp4");
    let started = jobs
        .start_export(project.clone(), request(&project, "job-done"))
        .unwrap();
    assert_eq!(started.job, "job_0001");
    assert_eq!(started.kind, "export");
    assert_eq!(started.state, JobState::Running);

    let done = jobs.status(&started.job, WAIT).unwrap();
    assert_eq!(done.state, JobState::Succeeded, "{done:?}");
    assert_eq!((done.progress.done, done.progress.total), (30, 30));
    let report = done.report.unwrap();
    assert_eq!(report.frames, 30);
    assert!(std::path::Path::new(&report.output).is_file());
    assert!(done.error.is_none());
    assert_eq!(jobs.list().len(), 1);

    // Cancelling a finished job changes nothing.
    let after = jobs.cancel(&started.job, Duration::ZERO).unwrap();
    assert_eq!(after.state, JobState::Succeeded);
}

#[test]
fn running_jobs_cancel_and_guard_their_output() {
    if renderer().is_none() {
        return;
    }
    let jobs = Jobs::new();
    let project = long_project();
    let mut first = request(&project, "job-long");
    first.request_id = Some("long".into());
    let running = jobs.start_export(project.clone(), first.clone()).unwrap();

    // Without waiting, the status comes back at once.
    let now = jobs.status(&running.job, Duration::ZERO).unwrap();
    assert_eq!(now.state, JobState::Running);

    // The same request joins the running job; another writer to its output is refused.
    let again = jobs.start_export(project.clone(), first.clone()).unwrap();
    assert_eq!(again.job, running.job);
    let mut other = first.clone();
    other.request_id = None;
    let busy = jobs.start_export(project.clone(), other).unwrap_err();
    assert_eq!(busy.code(), "fosscut.export.output_busy");

    let cancelled = jobs.cancel(&running.job, WAIT).unwrap();
    assert_eq!(cancelled.state, JobState::Cancelled, "{cancelled:?}");
    assert_eq!(cancelled.error.unwrap().code, "fosscut.export.cancelled");
    assert!(cancelled.progress.done < cancelled.progress.total);
    assert!(!first.output.exists(), "a cancelled export writes nothing");
    let leftovers = std::fs::read_dir(workspace())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains("job-long"))
        .count();
    assert_eq!(leftovers, 0, "no temporary file is left");
}

#[test]
fn failures_and_unknown_jobs_are_typed() {
    if renderer().is_none() {
        return;
    }
    let jobs = Jobs::new();
    let missing = project("missing.mp4");
    let started = jobs
        .start_export(missing.clone(), request(&missing, "job-missing"))
        .unwrap();
    let failed = jobs.status(&started.job, WAIT).unwrap();
    assert_eq!(failed.state, JobState::Failed);
    let error = failed.error.unwrap();
    assert_eq!(error.code, "fosscut.export.missing_media");
    assert!(error.hint.is_some());
    assert_eq!(error.details["asset"], "asset_0001");

    for result in [
        jobs.status("job_0099", Duration::ZERO),
        jobs.cancel("job_0099", Duration::ZERO),
    ] {
        assert_eq!(result.unwrap_err().code(), "fosscut.job.not_found");
    }
    let serialized = serde_json::to_value(jobs.list()).unwrap();
    assert_eq!(serialized[0]["state"], "failed");
    assert!(serialized[0].get("report").is_none());
}
