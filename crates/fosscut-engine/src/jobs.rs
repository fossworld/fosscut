//! Background jobs (ADR-0006, item 8; ADR-0019, item 4; ADR-0032): an export, an
//! analysis, a recording, a proxy or a remuxed copy is made on its own thread and followed
//! by id, so a long-lived host (the MCP server, the desktop app) answers at once and reports
//! progress, the result and cancellation. Jobs live as long as their [`Jobs`]. A host can
//! give its [`Jobs`] the user's cache policy, which is applied after every job.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fosscut_model::{Id, Project};
use fosscut_render::Renderer;
use fosscut_time::Flicks;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::analysis::{AnalysisReport, AnalysisRequest, analyze};
use crate::cache::{CachePolicy, source_of};
use crate::proxy::{ProxyReport, ProxySize, make_proxy, proxy_path, video_asset};
use crate::record::{RecordRequest, RecordingReport, record_plan};
use crate::remux::{RemuxReport, remux, remux_path};
use crate::{EngineError, ExportReport, ExportRequest, Progress, export};

/// Where a job stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Still working.
    Running,
    /// Finished; the result is in the status.
    Succeeded,
    /// Stopped with an error; the error is in the status.
    Failed,
    /// Stopped on request; nothing was written. A recording that is stopped keeps what
    /// it recorded and ends [`JobState::Succeeded`] instead.
    Cancelled,
}

/// A typed error as a job reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobError {
    /// Stable code, for example `fosscut.export.missing_media`.
    pub code: String,
    /// What went wrong.
    pub message: String,
    /// What to do about it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Machine-readable details.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl From<&EngineError> for JobError {
    fn from(error: &EngineError) -> JobError {
        JobError {
            code: error.code().to_string(),
            message: error.to_string(),
            hint: error.hint(),
            details: error.details(),
        }
    }
}

/// A job's state, progress and outcome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobStatus {
    /// The job id, for example `job_0001`.
    pub job: String,
    /// What the job does: `export`, `analysis`, `recording`, `proxy` or `remux`.
    pub kind: String,
    /// Where it stands.
    pub state: JobState,
    /// Frames (or audio seconds) done and in total; zero until rendering starts. For a
    /// recording, whole seconds recorded and the duration in seconds (0 when it records
    /// until stopped), as [`crate::record::progress`] counts them. For a proxy, frames
    /// written and the source's frames; for a remux, milliseconds copied and the duration
    /// in milliseconds.
    pub progress: Progress,
    /// Time since the job started, in milliseconds.
    pub elapsed_ms: u64,
    /// The file an export, a recording, a proxy or a remux writes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The asset an analysis, a proxy or a remux reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// The verified export, once an export succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<ExportReport>,
    /// The results, once an analysis succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis: Option<AnalysisReport>,
    /// The recorded file, once a recording succeeded (also when it was stopped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<RecordingReport>,
    /// The proxy, once a proxy job succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyReport>,
    /// The remuxed copy, once a remux job succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remux: Option<RemuxReport>,
    /// The typed error, once the job failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
}

/// The jobs of one host session.
#[derive(Default)]
pub struct Jobs {
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
    /// Applied after every job finishes.
    policy: Arc<Mutex<Option<CachePolicy>>>,
}

/// What a finished job produced.
enum Done {
    Export(ExportReport),
    Analysis(AnalysisReport),
    Recording(RecordingReport),
    Proxy(ProxyReport),
    Remux(RemuxReport),
}

type Outcome = Option<Result<Done, EngineError>>;

struct Job {
    id: String,
    kind: &'static str,
    output: Option<String>,
    asset: Option<String>,
    /// Identifies the work, so the same request joins a running job: the export's request
    /// id, or the analysis's asset, analyzers and options.
    key: Option<String>,
    started: Instant,
    cancel: AtomicBool,
    progress: Mutex<Progress>,
    outcome: Mutex<Outcome>,
    finished: Condvar,
}

impl Job {
    fn status(&self, outcome: &Outcome) -> JobStatus {
        let (state, done, error) = match outcome {
            None => (JobState::Running, None, None),
            Some(Ok(done)) => (JobState::Succeeded, Some(done), None),
            Some(Err(
                error @ (EngineError::Cancelled
                | EngineError::AnalysisCancelled
                | EngineError::JobCancelled { .. }),
            )) => (JobState::Cancelled, None, Some(JobError::from(error))),
            Some(Err(error)) => (JobState::Failed, None, Some(JobError::from(error))),
        };
        JobStatus {
            job: self.id.clone(),
            kind: self.kind.into(),
            state,
            progress: *lock(&self.progress),
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            output: self.output.clone(),
            asset: self.asset.clone(),
            report: match done {
                Some(Done::Export(report)) => Some(report.clone()),
                _ => None,
            },
            analysis: match done {
                Some(Done::Analysis(report)) => Some(report.clone()),
                _ => None,
            },
            recording: match done {
                Some(Done::Recording(report)) => Some(report.clone()),
                _ => None,
            },
            proxy: match done {
                Some(Done::Proxy(report)) => Some(report.clone()),
                _ => None,
            },
            remux: match done {
                Some(Done::Remux(report)) => Some(report.clone()),
                _ => None,
            },
            error,
        }
    }

    fn running(&self) -> bool {
        lock(&self.outcome).is_none()
    }
}

impl Job {
    fn new(
        index: usize,
        kind: &'static str,
        output: Option<String>,
        asset: Option<String>,
        key: Option<String>,
    ) -> Job {
        Job {
            id: format!("job_{:04}", index + 1),
            kind,
            output,
            asset,
            key,
            started: Instant::now(),
            cancel: AtomicBool::new(false),
            progress: Mutex::new(Progress::default()),
            outcome: Mutex::new(None),
            finished: Condvar::new(),
        }
    }
}

/// Locks `mutex`, recovering the data if a job thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Jobs {
    /// No jobs yet.
    pub fn new() -> Jobs {
        Jobs::default()
    }

    /// Applies `policy` ([`crate::cache::enforce`]) after every job finishes, before its
    /// final status is reported (ADR-0032, item 5); `None` stops applying one. Hosts set it
    /// from the user's `cache` preferences, and apply it once themselves when they start.
    pub fn set_cache_policy(&self, policy: Option<CachePolicy>) {
        *lock(&self.policy) = policy;
    }

    /// Starts making the `size` proxy of `asset` ([`crate::proxy::make_proxy`]) in the
    /// cache directory `cache`, on a new thread, and returns the job's status at once. The
    /// same asset content and size while one is running joins that job. An asset that does
    /// not exist, is not a video or whose media is missing fails here and starts no job.
    pub fn start_proxy(
        &self,
        project: Project,
        bundle: PathBuf,
        asset: Id,
        size: ProxySize,
        cache: PathBuf,
    ) -> Result<JobStatus, EngineError> {
        video_asset(&project, &asset)?;
        let (_, identity) = source_of(&project, &bundle, &asset)?;
        let output = proxy_path(&cache, &identity, size);
        let key = format!("proxy:{}", output.display());
        let jobs = lock(&self.jobs);
        if let Some(running) = jobs
            .values()
            .find(|job| job.key.as_deref() == Some(key.as_str()) && job.running())
        {
            return Ok(running.status(&None));
        }
        let job = Job::new(
            jobs.len(),
            "proxy",
            Some(output.display().to_string()),
            Some(asset.to_string()),
            Some(key),
        );
        Ok(self.spawn(jobs, job, move |_, progress| {
            make_proxy(&project, &bundle, &asset, size, &cache, progress).map(Done::Proxy)
        }))
    }

    /// Starts copying the streams of `asset`'s media into QuickTime
    /// ([`crate::remux::remux`]) in the cache directory `cache`, on a new thread, and
    /// returns the job's status at once. The same asset content while one is running joins
    /// that job. An asset that does not exist, is not a video, whose media is missing or
    /// that needs no remux fails here and starts no job.
    pub fn start_remux(
        &self,
        project: Project,
        bundle: PathBuf,
        asset: Id,
        cache: PathBuf,
    ) -> Result<JobStatus, EngineError> {
        video_asset(&project, &asset)?;
        let (source, identity) = source_of(&project, &bundle, &asset)?;
        let container = project
            .media
            .get(&asset)
            .and_then(|a| a.info.container.clone());
        if !crate::remux::needs_remux(&source, container.as_deref()) {
            return Err(EngineError::RemuxNotNeeded {
                asset: asset.to_string(),
                container: container.unwrap_or_else(|| "an unknown container".into()),
            });
        }
        let output = remux_path(&cache, &identity);
        let key = format!("remux:{}", output.display());
        let jobs = lock(&self.jobs);
        if let Some(running) = jobs
            .values()
            .find(|job| job.key.as_deref() == Some(key.as_str()) && job.running())
        {
            return Ok(running.status(&None));
        }
        let job = Job::new(
            jobs.len(),
            "remux",
            Some(output.display().to_string()),
            Some(asset.to_string()),
            Some(key),
        );
        Ok(self.spawn(jobs, job, move |_, progress| {
            remux(&project, &bundle, &asset, &cache, progress).map(Done::Remux)
        }))
    }

    /// Starts exporting `project` as `request` describes, on a new thread with its own
    /// renderer, and returns the job's status at once.
    ///
    /// A request with the `request_id` of a running job returns that job instead of
    /// starting another; any other export to the output of a running job fails with
    /// [`EngineError::OutputBusy`].
    pub fn start_export(
        &self,
        project: Project,
        request: ExportRequest,
    ) -> Result<JobStatus, EngineError> {
        let output = request.output.display().to_string();
        let jobs = lock(&self.jobs);
        if let Some(running) = jobs
            .values()
            .find(|job| job.output.as_deref() == Some(output.as_str()) && job.running())
        {
            if running.key.is_some() && running.key == request.request_id {
                return Ok(running.status(&None));
            }
            return Err(EngineError::OutputBusy {
                path: output,
                job: running.id.clone(),
            });
        }
        let key = request.request_id.clone();
        let job = Job::new(jobs.len(), "export", Some(output), None, key);
        Ok(self.spawn(jobs, job, move |_, progress| {
            let mut renderer = Renderer::headless()?;
            export(&project, &request, &mut renderer, progress).map(Done::Export)
        }))
    }

    /// Starts analyzing as `request` describes, on a new thread, and returns the job's
    /// status at once. The same request while one is running joins that job.
    pub fn start_analysis(&self, project: Project, request: AnalysisRequest) -> JobStatus {
        let asset = request.asset.to_string();
        let key = format!(
            "{asset}:{:?}:{}:{}",
            request.analyzers,
            serde_json::to_string(&request.options).unwrap_or_default(),
            request.refresh
        );
        let jobs = lock(&self.jobs);
        if let Some(running) = jobs
            .values()
            .find(|job| job.key.as_deref() == Some(key.as_str()) && job.running())
        {
            return running.status(&None);
        }
        let job = Job::new(jobs.len(), "analysis", None, Some(asset), Some(key));
        self.spawn(jobs, job, move |_, progress| {
            analyze(&project, &request, progress).map(Done::Analysis)
        })
    }

    /// Starts recording as `request` describes, on a new thread, and returns the job's
    /// status at once. The devices and settings are chosen first
    /// ([`crate::record::plan`]), so a bad choice fails here with a typed error and starts
    /// no job.
    ///
    /// Progress counts whole seconds recorded against the duration (0 when the recording
    /// runs until stopped). [`Jobs::cancel`] stops a recording and keeps what it recorded:
    /// the job then ends [`JobState::Succeeded`] with the report in
    /// [`JobStatus::recording`], not [`JobState::Cancelled`]. A recording stopped before
    /// its devices delivered anything fails with `fosscut.media.nothing_recorded` and
    /// writes nothing. A recording to the output of a running job fails with
    /// [`EngineError::OutputBusy`].
    pub fn start_recording(&self, request: RecordRequest) -> Result<JobStatus, EngineError> {
        let planned = crate::record::plan(&request)?;
        let output = request.output.display().to_string();
        let jobs = lock(&self.jobs);
        if let Some(running) = jobs
            .values()
            .find(|job| job.output.as_deref() == Some(output.as_str()) && job.running())
        {
            return Err(EngineError::OutputBusy {
                path: output,
                job: running.id.clone(),
            });
        }
        let duration = planned.recording.duration;
        let mut job = Job::new(jobs.len(), "recording", Some(output), None, None);
        // The total is known from the start, before anything is recorded.
        job.progress = Mutex::new(crate::record::progress(Flicks::ZERO, duration));
        Ok(self.spawn(jobs, job, move |stop, progress| {
            let mut recorded = |time: Flicks| {
                // Stopping goes through `stop`, so what `progress` answers is not needed.
                let _ = progress(crate::record::progress(time, duration));
            };
            record_plan(&planned, stop, &mut recorded).map(Done::Recording)
        }))
    }

    /// Registers `job` and runs `work` on a new thread, reporting progress and honouring
    /// cancellation: `work` gets the job's cancel flag, which [`Jobs::cancel`] sets, and a
    /// progress callback that answers `false` once it is set. The cache policy, if any, is
    /// applied when `work` returns and before the job reports that it finished.
    fn spawn(
        &self,
        mut jobs: MutexGuard<'_, BTreeMap<String, Arc<Job>>>,
        job: Job,
        work: impl FnOnce(&AtomicBool, &mut dyn FnMut(Progress) -> bool) -> Result<Done, EngineError>
        + Send
        + 'static,
    ) -> JobStatus {
        let job = Arc::new(job);
        jobs.insert(job.id.clone(), Arc::clone(&job));
        drop(jobs);
        let worker = Arc::clone(&job);
        let policy = Arc::clone(&self.policy);
        std::thread::spawn(move || {
            let result = work(&worker.cancel, &mut |progress| {
                *lock(&worker.progress) = progress;
                !worker.cancel.load(Ordering::Relaxed)
            });
            let policy = lock(&policy).clone();
            if let Some(policy) = policy {
                policy.enforce();
            }
            *lock(&worker.outcome) = Some(result);
            worker.finished.notify_all();
        });
        job.status(&None)
    }

    /// The status of job `id`, waiting up to `wait` for it to finish first.
    pub fn status(&self, id: &str, wait: Duration) -> Result<JobStatus, EngineError> {
        let job = self.get(id)?;
        let outcome = lock(&job.outcome);
        let (outcome, _) = job
            .finished
            .wait_timeout_while(outcome, wait, |outcome| outcome.is_none())
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(job.status(&outcome))
    }

    /// Asks job `id` to stop, then waits up to `wait` for it to finish. A finished job is
    /// not affected. An export, analysis, proxy or remux that stops ends
    /// [`JobState::Cancelled`] with nothing written; a recording stops and keeps what it
    /// recorded, ending [`JobState::Succeeded`] with its report (see
    /// [`Jobs::start_recording`]).
    pub fn cancel(&self, id: &str, wait: Duration) -> Result<JobStatus, EngineError> {
        self.get(id)?.cancel.store(true, Ordering::Relaxed);
        self.status(id, wait)
    }

    /// Every job of this session, oldest first.
    pub fn list(&self) -> Vec<JobStatus> {
        lock(&self.jobs)
            .values()
            .map(|job| job.status(&lock(&job.outcome)))
            .collect()
    }

    fn get(&self, id: &str) -> Result<Arc<Job>, EngineError> {
        lock(&self.jobs)
            .get(id)
            .cloned()
            .ok_or_else(|| EngineError::JobNotFound { job: id.into() })
    }
}
