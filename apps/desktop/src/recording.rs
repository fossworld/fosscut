//! Recording from the window (CapCut parity CC-027): a microphone, camera or screen records
//! in the background on the app's jobs into the open project's `recordings/` folder. Once
//! the recording succeeds (a stopped recording succeeds with what it recorded) the file is
//! imported and placed where the playhead was when it started, as the user's edits, exactly
//! once however often and from wherever its status is read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use fosscut_core::{ApiError, Request, codes};
use fosscut_engine::jobs::{JobState, JobStatus, Jobs};
use fosscut_engine::record::{self, CaptureInput, RecordKind, RecordRequest, RecordingReport};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::workspace::Workspace;

/// What to record (`record_start`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecordChoice {
    /// `microphone`, `camera` or `screen`.
    pub what: String,
    /// The device, by name or by number from 1 among the devices of its kind; the first
    /// one when omitted.
    pub device: Option<String>,
    /// For a camera or screen, the microphone recorded with it (by name or number), or
    /// `none`; a camera records the first microphone, a screen none, when omitted.
    pub microphone: Option<String>,
    /// For a camera or screen, the picture size as `WIDTHxHEIGHT`.
    pub size: Option<String>,
    /// For a camera or screen, the frame rate, such as `30` or `29.97`.
    pub fps: Option<String>,
    /// Where the recording is placed once imported, in timeline flicks: the playhead when
    /// it started. It is imported without being placed when omitted.
    pub at: Option<i64>,
    /// A raw FFmpeg input as `FORMAT:INPUT` instead of a device (`lavfi:sine`), for tests.
    pub input: Option<String>,
}

/// What became of a succeeded recording's file.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Imported {
    /// The asset it was imported as.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// The clip placed at the playhead the recording started from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<String>,
    /// Why importing or placing it failed; the file stays in the bundle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_error: Option<ApiError>,
}

/// A recording's status and, once it succeeded, what was imported from it.
#[derive(Clone, Debug, Serialize)]
pub struct RecordState {
    /// The job's status; `recording` holds the report once it succeeded.
    #[serde(flatten)]
    pub status: JobStatus,
    /// The import, once the recording succeeded.
    #[serde(flatten)]
    pub imported: Imported,
}

/// A recording the window started, and what is done with its file.
#[derive(Clone, Debug)]
enum Finish {
    /// Recording: the bundle it records into and where it goes on the timeline.
    Pending { bundle: PathBuf, at: Option<i64> },
    /// Succeeded and imported, or failed to import.
    Finished(Imported),
}

/// The recordings started from the window, by job id.
#[derive(Default)]
pub struct Recordings {
    finishes: Mutex<HashMap<String, Finish>>,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// A text the window sent, or `None` when it sent nothing or only spaces.
fn given(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|t| !t.is_empty())
}

/// `FORMAT:INPUT`, as the CLI's `--input` takes it.
fn capture_input(text: &str) -> Result<CaptureInput, ApiError> {
    let (format, input) = text
        .split_once(':')
        .ok_or_else(|| invalid(format!("{text}: give an input as FORMAT:INPUT")))?;
    Ok(CaptureInput {
        format: format.to_string(),
        input: input.to_string(),
        options: Vec::new(),
    })
}

/// `WIDTHxHEIGHT`.
fn picture_size(text: &str) -> Result<(u32, u32), ApiError> {
    text.split_once(['x', 'X'])
        .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
        .ok_or_else(|| {
            invalid(format!(
                "size {text}: give WIDTHxHEIGHT, for example 1280x720"
            ))
        })
}

/// Where a recording goes: `default` (in the project's `recordings/` folder), or a file of
/// the same name in `folder`, numbered when one is there already.
fn recording_output(default: PathBuf, folder: Option<&Path>) -> Result<PathBuf, ApiError> {
    let Some(folder) = folder else {
        return Ok(default);
    };
    std::fs::create_dir_all(folder).map_err(|e| {
        invalid(format!("cannot create {}: {e}", folder.display()))
            .with_hint("Choose another recordings folder in Settings.")
    })?;
    let stem = default
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let extension = default
        .extension()
        .map_or_else(String::new, |e| e.to_string_lossy().into_owned());
    let named = |n: u32| {
        let name = if n == 1 {
            stem.clone()
        } else {
            format!("{stem} {n}")
        };
        folder.join(format!("{name}.{extension}"))
    };
    Ok((1..=u32::MAX)
        .map(named)
        .find(|path| !path.exists())
        .unwrap_or_else(|| folder.join(format!("{stem}.{extension}"))))
}

impl Recordings {
    /// Starts recording what `choice` says into the open project's `recordings/` folder,
    /// on `jobs`, after choosing the devices and settings (a bad choice fails here).
    pub fn start(
        &self,
        workspace: &Workspace,
        jobs: &Jobs,
        choice: &RecordChoice,
    ) -> Result<RecordState, ApiError> {
        self.start_in(workspace, jobs, choice, None)
    }

    /// [`Recordings::start`], recording into `folder` (created when missing) instead of
    /// the project's `recordings/` folder when given: the preferred recordings folder.
    pub fn start_in(
        &self,
        workspace: &Workspace,
        jobs: &Jobs,
        choice: &RecordChoice,
        folder: Option<&Path>,
    ) -> Result<RecordState, ApiError> {
        let what = RecordKind::parse(&choice.what).ok_or_else(|| {
            invalid(format!(
                "{}: record a microphone, camera or screen",
                choice.what
            ))
        })?;
        let (bundle, _, _) = workspace.snapshot().ok_or_else(|| {
            invalid("no project is open").with_hint("Open or create a project to record into.")
        })?;
        let output = recording_output(
            record::default_output(&bundle, what, SystemTime::now()),
            folder,
        )?;
        let mut request = RecordRequest::new(what, output);
        request.device = given(choice.device.as_deref()).map(str::to_string);
        request.microphone = given(choice.microphone.as_deref()).map(str::to_string);
        request.input = given(choice.input.as_deref())
            .map(capture_input)
            .transpose()?;
        request.size = given(choice.size.as_deref())
            .map(picture_size)
            .transpose()?;
        request.rate = given(choice.fps.as_deref())
            .map(fosscut_engine::frame_rate)
            .transpose()?;
        let status = jobs.start_recording(request)?;
        guard(&self.finishes).insert(
            status.job.clone(),
            Finish::Pending {
                bundle,
                at: choice.at,
            },
        );
        Ok(RecordState {
            status,
            imported: Imported::default(),
        })
    }

    /// A recording's status, importing its file the first time it is seen succeeded.
    pub fn status(
        &self,
        workspace: &Workspace,
        jobs: &Jobs,
        job: &str,
    ) -> Result<RecordState, ApiError> {
        let status = jobs.status(job, Duration::ZERO)?;
        Ok(self.finish(workspace, status))
    }

    /// Stops a recording, keeping what it recorded, and waits up to `wait` for its file;
    /// the file is imported if it is ready by then (else at a later status).
    pub fn stop(
        &self,
        workspace: &Workspace,
        jobs: &Jobs,
        job: &str,
        wait: Duration,
    ) -> Result<RecordState, ApiError> {
        let status = jobs.cancel(job, wait)?;
        Ok(self.finish(workspace, status))
    }

    /// `status` with its import: done once, under the lock, when the recording first shows
    /// succeeded; answered from what was done afterwards.
    fn finish(&self, workspace: &Workspace, status: JobStatus) -> RecordState {
        let mut finishes = guard(&self.finishes);
        let Some(finish) = finishes.get_mut(&status.job) else {
            return RecordState {
                status,
                imported: Imported::default(),
            };
        };
        if let (Finish::Pending { bundle, at }, JobState::Succeeded, Some(report)) =
            (&*finish, status.state, &status.recording)
        {
            *finish = Finish::Finished(import(workspace, bundle, *at, report));
        }
        let imported = match finish {
            Finish::Finished(imported) => imported.clone(),
            Finish::Pending { .. } => Imported::default(),
        };
        RecordState { status, imported }
    }
}

/// Imports a recording into the project it was made for and places it at `at` on a track
/// free there, as two of the user's edits.
fn import(
    workspace: &Workspace,
    bundle: &Path,
    at: Option<i64>,
    report: &RecordingReport,
) -> Imported {
    let mut imported = Imported::default();
    let result = (|| {
        if workspace
            .snapshot()
            .is_none_or(|(open, _, _)| open != bundle)
        {
            return Err(invalid(format!(
                "{} was recorded for a project that is no longer open",
                report.output
            ))
            .with_hint("Open that project and import the file from its recordings folder."));
        }
        let file = PathBuf::from(&report.output);
        let mut add = Request::single(
            "media.add",
            fosscut_engine::import::media_add_args(&file, None, bundle)?,
        );
        add.reason = Some(format!("Record from the {}", report.what.name()));
        let added = workspace.execute(add)?;
        let asset = added
            .results
            .first()
            .and_then(|result| result["asset"].as_str())
            .map(str::to_string)
            .ok_or_else(|| ApiError::new(codes::INTERNAL, "media.add returned no asset"))?;
        imported.asset = Some(asset.clone());
        let Some(at) = at else {
            return Ok(());
        };
        let mut place = Request::single(
            "clip.add",
            json!({ "asset": asset, "start": { "flicks": at }, "free_track": true }),
        );
        place.reason = Some("Place the recording".into());
        let placed = workspace.execute(place)?;
        imported.clip = placed
            .results
            .first()
            .and_then(|result| result["clip"].as_str())
            .map(str::to_string);
        Ok(())
    })();
    imported.import_error = result.err();
    imported
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::NewProject;
    use serde_json::Value;

    const SECOND: i64 = fosscut_time::FLICKS_PER_SECOND;
    const TONE: &str = "lavfi:sine=frequency=440:sample_rate=48000";
    const PICTURES: &str = "lavfi:testsrc2=size=320x240:rate=30";

    /// A workspace with a new project open, in a short runtime directory (Unix socket paths
    /// are limited to about 100 bytes).
    fn opened(name: &str) -> (Workspace, PathBuf, PathBuf) {
        let base = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let runtime = base.join(format!("fcd-{name}-{}", std::process::id()));
        let work =
            std::env::temp_dir().join(format!("fosscut-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).unwrap();
        let workspace = Workspace::new(runtime.clone(), None);
        (workspace, runtime, work)
    }

    fn open(workspace: &Workspace, work: &Path) -> PathBuf {
        workspace
            .create(&NewProject {
                path: work.join("Rec.fosscut"),
                name: None,
                width: 1280,
                height: 720,
                rate: "30".into(),
            })
            .unwrap()
            .bundle
    }

    fn done(workspace: Workspace, runtime: PathBuf, work: PathBuf) {
        workspace.close();
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
    }

    /// True when ffprobe (for importing) is here; in CI (`FOSSCUT_REQUIRE_FFMPEG`) it must
    /// be. FFmpeg's `lavfi` input, which stands in for a microphone, is in every build the
    /// workspace links.
    fn ffprobe() -> bool {
        let found = std::process::Command::new(
            std::env::var("FOSSCUT_FFPROBE").unwrap_or_else(|_| "ffprobe".into()),
        )
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
        let required =
            std::env::var_os("FOSSCUT_REQUIRE_FFMPEG").is_some_and(|v| !v.is_empty() && v != "0");
        assert!(
            found || !required,
            "FOSSCUT_REQUIRE_FFMPEG is set but ffprobe is missing"
        );
        found
    }

    fn clips(workspace: &Workspace) -> Vec<Value> {
        let view = workspace
            .query("timeline.read", json!({ "detail": "full" }))
            .unwrap();
        view["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["clips"].as_array().unwrap().clone())
            .collect()
    }

    fn choice(what: &str, input: &str) -> RecordChoice {
        RecordChoice {
            what: what.into(),
            input: Some(input.into()),
            ..RecordChoice::default()
        }
    }

    /// Waits for a stopped recording's file, as the window polls for it.
    fn finished(
        recordings: &Recordings,
        workspace: &Workspace,
        jobs: &Jobs,
        job: &str,
    ) -> RecordState {
        let mut state = recordings
            .stop(workspace, jobs, job, Duration::from_secs(20))
            .unwrap();
        for _ in 0..200 {
            if state.status.state != JobState::Running {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
            state = recordings.status(workspace, jobs, job).unwrap();
        }
        state
    }

    #[test]
    fn recordings_go_to_the_preferred_folder_without_replacing_a_file() {
        let (_, runtime, work) = opened("recfolder");
        let default = work.join("Demo.fosscut/recordings/Voiceover 2026.m4a");
        assert_eq!(recording_output(default.clone(), None).unwrap(), default);
        let takes = work.join("Takes");
        let first = recording_output(default.clone(), Some(&takes)).unwrap();
        assert_eq!(first, takes.join("Voiceover 2026.m4a"));
        assert!(takes.is_dir(), "the folder is made");
        std::fs::write(&first, b"taken").unwrap();
        assert_eq!(
            recording_output(default, Some(&takes)).unwrap(),
            takes.join("Voiceover 2026 2.m4a")
        );
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn choices_are_refused_before_anything_records() {
        let (workspace, runtime, work) = opened("recno");
        let (recordings, jobs) = (Recordings::default(), Jobs::new());
        let closed = recordings
            .start(&workspace, &jobs, &choice("microphone", TONE))
            .unwrap_err();
        assert!(closed.message.contains("no project"), "{closed:?}");
        open(&workspace, &work);
        let what = recordings
            .start(&workspace, &jobs, &choice("radio", TONE))
            .unwrap_err();
        assert_eq!(what.code, codes::INVALID_ARGUMENTS);
        assert!(what.message.contains("microphone, camera or screen"));
        let input = recordings
            .start(&workspace, &jobs, &choice("microphone", "sine"))
            .unwrap_err();
        assert!(input.message.contains("FORMAT:INPUT"), "{input:?}");
        let size = recordings
            .start(
                &workspace,
                &jobs,
                &RecordChoice {
                    size: Some("big".into()),
                    ..choice("camera", PICTURES)
                },
            )
            .unwrap_err();
        assert!(size.message.contains("WIDTHxHEIGHT"), "{size:?}");
        let fps = recordings
            .start(
                &workspace,
                &jobs,
                &RecordChoice {
                    fps: Some("fast".into()),
                    ..choice("camera", PICTURES)
                },
            )
            .unwrap_err();
        assert_eq!(fps.code, "fosscut.export.invalid_settings", "{fps:?}");
        let unknown = recordings
            .status(&workspace, &jobs, "job_0099")
            .unwrap_err();
        assert_eq!(unknown.code, "fosscut.job.not_found");
        assert!(jobs.list().is_empty(), "nothing started");
        assert_eq!(picture_size(" 1280 x 720 ").unwrap(), (1280, 720));
        done(workspace, runtime, work);
    }

    #[test]
    fn a_stopped_recording_is_imported_and_placed_at_its_playhead_once() {
        if !ffprobe() {
            eprintln!("skipped: no ffprobe");
            return;
        }
        let (workspace, runtime, work) = opened("rec");
        let bundle = open(&workspace, &work);
        let (recordings, jobs) = (Recordings::default(), Jobs::new());
        let started = recordings
            .start(
                &workspace,
                &jobs,
                &RecordChoice {
                    at: Some(2 * SECOND),
                    ..choice("microphone", TONE)
                },
            )
            .unwrap();
        assert_eq!(started.status.state, JobState::Running);
        assert_eq!(started.status.kind, "recording");
        let output = PathBuf::from(started.status.output.clone().unwrap());
        assert!(
            output.starts_with(bundle.join(record::RECORDINGS)),
            "{output:?}"
        );
        assert_eq!(started.imported, Imported::default());
        std::thread::sleep(Duration::from_millis(800));
        let running = recordings
            .status(&workspace, &jobs, &started.status.job)
            .unwrap();
        assert_eq!(running.status.state, JobState::Running);
        assert!(
            running.imported.asset.is_none(),
            "nothing imported while it records"
        );

        let state = finished(&recordings, &workspace, &jobs, &started.status.job);
        assert_eq!(
            state.status.state,
            JobState::Succeeded,
            "{:?}",
            state.status
        );
        let report = state.status.recording.clone().unwrap();
        assert!(report.stopped);
        assert_eq!(PathBuf::from(&report.output), output);
        assert!(state.imported.import_error.is_none(), "{state:?}");
        let asset = state.imported.asset.clone().unwrap();
        let clip = state.imported.clip.clone().unwrap();
        let shown = clips(&workspace);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0]["id"], clip.as_str());
        assert_eq!(shown[0]["asset"], asset.as_str());
        assert_eq!(shown[0]["kind"], "audio");
        assert_eq!(
            shown[0]["start"]["flicks"],
            2 * SECOND,
            "where the playhead was"
        );
        let history = workspace
            .query("history.list", json!({ "limit": 2 }))
            .unwrap();
        let entries = history["entries"].as_array().unwrap();
        assert!(entries.iter().all(|e| e["actor"] == "human"));
        assert_eq!(entries[1]["reason"], "Record from the microphone");

        // Reading it again, or stopping it again, imports nothing more.
        let again = recordings
            .status(&workspace, &jobs, &started.status.job)
            .unwrap();
        assert_eq!(again.imported, state.imported);
        let stopped = recordings
            .stop(&workspace, &jobs, &started.status.job, Duration::ZERO)
            .unwrap();
        assert_eq!(stopped.imported, state.imported);
        let media = workspace.query("media.list", Value::Null).unwrap();
        assert_eq!(media["assets"].as_array().unwrap().len(), 1);
        assert_eq!(clips(&workspace).len(), 1);
        done(workspace, runtime, work);
    }

    #[test]
    fn a_recording_for_a_closed_project_is_not_imported_into_another() {
        if !ffprobe() {
            eprintln!("skipped: no ffprobe");
            return;
        }
        let (workspace, runtime, work) = opened("recgone");
        open(&workspace, &work);
        let (recordings, jobs) = (Recordings::default(), Jobs::new());
        let started = recordings
            .start(&workspace, &jobs, &choice("microphone", TONE))
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        workspace.close();
        let state = finished(&recordings, &workspace, &jobs, &started.status.job);
        assert_eq!(state.status.state, JobState::Succeeded);
        let error = state.imported.import_error.unwrap();
        assert!(error.message.contains("no longer open"), "{error:?}");
        assert!(state.imported.asset.is_none());
        done(workspace, runtime, work);
    }
}
