//! Background jobs (ADR-0006, item 8; ADR-0019, item 4): an export or an analysis runs on
//! its own thread and is followed by id, so a long-lived host (the MCP server, later the
//! desktop app) answers at once and reports progress, the result and cancellation. Jobs
//! live as long as their [`Jobs`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fosscut_model::Project;
use fosscut_render::Renderer;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::analysis::{AnalysisReport, AnalysisRequest, analyze};
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
    /// Stopped on request; nothing was written.
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
    /// What the job does: `export` or `analysis`.
    pub kind: String,
    /// Where it stands.
    pub state: JobState,
    /// Frames (or audio seconds) done and in total; zero until rendering starts.
    pub progress: Progress,
    /// Time since the job started, in milliseconds.
    pub elapsed_ms: u64,
    /// The file an export writes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// The asset an analysis reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// The verified export, once an export succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<ExportReport>,
    /// The results, once an analysis succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis: Option<AnalysisReport>,
    /// The typed error, once the job failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
}

/// The jobs of one host session.
#[derive(Default)]
pub struct Jobs {
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
}

/// What a finished job produced.
enum Done {
    Export(ExportReport),
    Analysis(AnalysisReport),
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
            Some(Err(error @ (EngineError::Cancelled | EngineError::AnalysisCancelled))) => {
                (JobState::Cancelled, None, Some(JobError::from(error)))
            }
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
        Ok(self.spawn(jobs, job, move |progress| {
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
        self.spawn(jobs, job, move |progress| {
            analyze(&project, &request, progress).map(Done::Analysis)
        })
    }

    /// Registers `job` and runs `work` on a new thread, reporting progress and honouring
    /// cancellation.
    fn spawn(
        &self,
        mut jobs: MutexGuard<'_, BTreeMap<String, Arc<Job>>>,
        job: Job,
        work: impl FnOnce(&mut dyn FnMut(Progress) -> bool) -> Result<Done, EngineError>
        + Send
        + 'static,
    ) -> JobStatus {
        let job = Arc::new(job);
        jobs.insert(job.id.clone(), Arc::clone(&job));
        drop(jobs);
        let worker = Arc::clone(&job);
        std::thread::spawn(move || {
            let result = work(&mut |progress| {
                *lock(&worker.progress) = progress;
                !worker.cancel.load(Ordering::Relaxed)
            });
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
    /// not affected.
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
