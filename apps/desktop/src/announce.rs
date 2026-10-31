//! What the app does when a background job it started finishes, as the preferences allow
//! (ADR-0031, item 6): a finished export plays the library sound chosen for it, and
//! finished exports and recordings show a system notification. Each job is watched from
//! a thread of its own, so this happens whether or not the window asks for the job's
//! status.

use std::sync::Arc;
use std::time::Duration;

use fosscut_engine::jobs::{JobState, JobStatus, Jobs};
use fosscut_engine::preferences::Preferences;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::AppState;
use crate::words::{Word, word};

/// How long one wait for a job lasts before waiting again.
const WAIT: Duration = Duration::from_secs(60);

/// What to do about a finished job.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Announcement {
    /// The library sound to play.
    pub sound: Option<String>,
    /// The system notification to show.
    pub notification: Option<Notice>,
}

/// A system notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// What happened, in the app's language.
    pub title: String,
    /// The file it happened to, or the engine's error (in English).
    pub body: String,
}

/// The file name of a job's output, or its id.
fn file_name(status: &JobStatus) -> String {
    status
        .output
        .as_deref()
        .map(|output| {
            std::path::Path::new(output).file_name().map_or_else(
                || output.to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
        })
        .unwrap_or_else(|| status.job.clone())
}

/// What `preferences` say to do now that the job in `status` finished: an export that
/// succeeded plays `export.sound_name` while `export.sound` is on; exports and recordings
/// that succeed or fail show a notification while `notifications` is on. Cancelled jobs,
/// jobs still running and other kinds of job do nothing.
pub fn announcement(preferences: &Preferences, status: &JobStatus) -> Announcement {
    let title = match (status.kind.as_str(), status.state) {
        ("export", JobState::Succeeded) => Word::ExportFinished,
        ("export", JobState::Failed) => Word::ExportFailed,
        ("recording", JobState::Succeeded) => Word::RecordingFinished,
        ("recording", JobState::Failed) => Word::RecordingFailed,
        _ => return Announcement::default(),
    };
    let body = match (&status.error, status.state) {
        (Some(error), JobState::Failed) => error.message.clone(),
        _ => file_name(status),
    };
    Announcement {
        sound: (status.kind == "export"
            && status.state == JobState::Succeeded
            && preferences.export.sound)
            .then(|| preferences.export.sound_name.clone()),
        notification: preferences.notifications.then(|| Notice {
            title: word(&preferences.language, title).to_string(),
            body,
        }),
    }
}

/// Plays and shows what [`announcement`] says for `status`, with the preferences now.
pub fn announce(app: &AppHandle, status: &JobStatus) {
    let state = app.state::<AppState>();
    let found = announcement(&state.preferences.current(), status);
    if let (Some(sound), Some(speaker)) = (&found.sound, &state.speaker)
        && !speaker.play_sound(sound)
    {
        eprintln!("fosscut-desktop: the library has no sound {sound}");
    }
    if let Some(notice) = found.notification
        && let Err(error) = app
            .notification()
            .builder()
            .title(notice.title)
            .body(notice.body)
            .show()
    {
        eprintln!("fosscut-desktop: notification: {error}");
    }
}

/// Watches job `job` of `jobs` on a thread of its own and announces it once it finishes.
pub fn watch(app: &AppHandle, jobs: &Arc<Jobs>, job: String) {
    let app = app.clone();
    let jobs = Arc::clone(jobs);
    let spawned = std::thread::Builder::new()
        .name("fosscut-job-watch".into())
        .spawn(move || {
            loop {
                match jobs.status(&job, WAIT) {
                    Ok(status) if status.state == JobState::Running => continue,
                    Ok(status) => break announce(&app, &status),
                    Err(_) => break,
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("fosscut-desktop: cannot watch a job: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_engine::Progress;

    /// A job's status as the engine reports it, read from JSON so that fields added to it
    /// later need nothing here.
    fn status(kind: &str, state: JobState) -> JobStatus {
        let mut status = serde_json::json!({
            "job": "job_0001",
            "kind": kind,
            "state": state,
            "progress": Progress::default(),
            "elapsed_ms": 10,
            "output": "/work/Exports/Launch.mp4",
        });
        if state == JobState::Failed {
            status["error"] = serde_json::json!({
                "code": "fosscut.export.missing_media",
                "message": "clip.mp4 is missing",
                "details": null,
            });
        }
        serde_json::from_value(status).unwrap()
    }

    #[test]
    fn a_finished_export_plays_the_chosen_sound_and_notifies() {
        let mut preferences = Preferences::default();
        let done = announcement(&preferences, &status("export", JobState::Succeeded));
        assert_eq!(done.sound.as_deref(), Some("ding"));
        assert_eq!(
            done.notification,
            Some(Notice {
                title: "Export finished".into(),
                body: "Launch.mp4".into()
            })
        );
        preferences.set("export.sound_name", "pop").unwrap();
        preferences.set("language", "hu").unwrap();
        let done = announcement(&preferences, &status("export", JobState::Succeeded));
        assert_eq!(done.sound.as_deref(), Some("pop"));
        assert_eq!(done.notification.unwrap().title, "Az exportálás elkészült");

        preferences.set("export.sound", "false").unwrap();
        preferences.set("notifications", "false").unwrap();
        assert_eq!(
            announcement(&preferences, &status("export", JobState::Succeeded)),
            Announcement::default(),
            "both turned off"
        );
    }

    #[test]
    fn failures_notify_without_the_sound_and_cancellations_do_nothing() {
        let preferences = Preferences::default();
        let failed = announcement(&preferences, &status("export", JobState::Failed));
        assert_eq!(failed.sound, None, "the sound is for a finished export");
        assert_eq!(
            failed.notification,
            Some(Notice {
                title: "Export failed".into(),
                body: "clip.mp4 is missing".into()
            })
        );
        for (kind, state) in [
            ("export", JobState::Cancelled),
            ("export", JobState::Running),
            ("analysis", JobState::Succeeded),
            ("recording", JobState::Cancelled),
        ] {
            assert_eq!(
                announcement(&preferences, &status(kind, state)),
                Announcement::default(),
                "{kind} {state:?}"
            );
        }
    }

    #[test]
    fn recordings_notify_when_they_finish_but_play_no_sound() {
        let mut preferences = Preferences::default();
        let mut recorded = status("recording", JobState::Succeeded);
        recorded.output = Some("/work/Trip.fosscut/recordings/Voiceover.m4a".into());
        let done = announcement(&preferences, &recorded);
        assert_eq!(done.sound, None);
        assert_eq!(
            done.notification,
            Some(Notice {
                title: "Recording finished".into(),
                body: "Voiceover.m4a".into()
            })
        );
        preferences.set("language", "hu").unwrap();
        let failed = announcement(&preferences, &status("recording", JobState::Failed));
        assert_eq!(
            failed.notification.unwrap().title,
            "A felvétel nem sikerült"
        );
        recorded.output = None;
        assert_eq!(
            announcement(&Preferences::default(), &recorded)
                .notification
                .unwrap()
                .body,
            "job_0001",
            "the job when there is no file"
        );
    }
}
