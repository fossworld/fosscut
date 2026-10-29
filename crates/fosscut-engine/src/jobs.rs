//! Background jobs (ADR-0006, item 8): an export runs on its own thread and is followed by
//! id, so a long-lived host (the MCP server, later the desktop app) answers at once and
//! reports progress, the result and cancellation. Jobs live as long as their [`Jobs`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fosscut_model::Project;
use fosscut_render::Renderer;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// What the job does: `export`.
    pub kind: String,
    /// Where it stands.
    pub state: JobState,
    /// Frames (or audio seconds) done and in total; zero until rendering starts.
    pub progress: Progress,
    /// Time since the job started, in milliseconds.
    pub elapsed_ms: u64,
    /// The file being written.
    pub output: String,
    /// The verified export, once the job succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<ExportReport>,
    /// The typed error, once the job failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
}

/// The jobs of one host session.
#[derive(Default)]
pub struct Jobs {
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
}

struct Job {
    id: String,
    output: String,
    request_id: Option<String>,
    started: Instant,
    cancel: AtomicBool,
    progress: Mutex<Progress>,
    outcome: Mutex<Option<Result<ExportReport, EngineError>>>,
    finished: Condvar,
}

impl Job {
    fn status(&self, outcome: &Option<Result<ExportReport, EngineError>>) -> JobStatus {
        let (state, report, error) = match outcome {
            None => (JobState::Running, None, None),
            Some(Ok(report)) => (JobState::Succeeded, Some(report.clone()), None),
            Some(Err(EngineError::Cancelled)) => (
                JobState::Cancelled,
                None,
                Some(JobError::from(&EngineError::Cancelled)),
            ),
            Some(Err(error)) => (JobState::Failed, None, Some(JobError::from(error))),
        };
        JobStatus {
            job: self.id.clone(),
            kind: "export".into(),
            state,
            progress: *lock(&self.progress),
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            output: self.output.clone(),
            report,
            error,
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
        let mut jobs = lock(&self.jobs);
        if let Some(running) = jobs
            .values()
            .find(|job| job.output == output && lock(&job.outcome).is_none())
        {
            if running.request_id.is_some() && running.request_id == request.request_id {
                return Ok(running.status(&None));
            }
            return Err(EngineError::OutputBusy {
                path: output,
                job: running.id.clone(),
            });
        }
        let job = Arc::new(Job {
            id: format!("job_{:04}", jobs.len() + 1),
            output,
            request_id: request.request_id.clone(),
            started: Instant::now(),
            cancel: AtomicBool::new(false),
            progress: Mutex::new(Progress::default()),
            outcome: Mutex::new(None),
            finished: Condvar::new(),
        });
        jobs.insert(job.id.clone(), Arc::clone(&job));
        drop(jobs);

        let worker = Arc::clone(&job);
        std::thread::spawn(move || {
            let result =
                Renderer::headless()
                    .map_err(EngineError::from)
                    .and_then(|mut renderer| {
                        export(&project, &request, &mut renderer, &mut |progress| {
                            *lock(&worker.progress) = progress;
                            !worker.cancel.load(Ordering::Relaxed)
                        })
                    });
            *lock(&worker.outcome) = Some(result);
            worker.finished.notify_all();
        });
        Ok(job.status(&None))
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
