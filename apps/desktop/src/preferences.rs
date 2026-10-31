//! The person's preferences (ADR-0031) as the app keeps them: read once when the app
//! starts, changed from the Settings window, saved at once, and written into the requests
//! the app builds, as the CLI does: a new project's frame rate, the loudness target, where
//! new projects, exports and recordings go. Commands never read them (ADR-0031, item 3).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use fosscut_core::ApiError;
use fosscut_engine::preferences::{LANGUAGES, Preferences, SHORTCUT_SETS, UpdatePolicy};
use fosscut_engine::updates::UpdateCheck;
use serde::Serialize;
use serde_json::{Value, json};

/// The preferences file and the preferences it holds.
pub struct PreferenceStore {
    file: PathBuf,
    current: Mutex<Preferences>,
    /// Why the file could not be read when the app started; the defaults apply meanwhile,
    /// and the next change replaces the file.
    problem: Mutex<Option<String>>,
}

/// What `preferences_get` and the calls that change preferences answer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PreferencesView {
    /// The preferences file.
    pub path: PathBuf,
    /// Every preference, with its default where the file sets none.
    pub preferences: Value,
    /// The defaults, for showing which values are changed and resetting them.
    pub defaults: Value,
    /// The languages the app is translated into.
    pub languages: Vec<&'static str>,
    /// The shortcut sets.
    pub shortcut_sets: Vec<&'static str>,
    /// Why the file could not be read, while the defaults stand in for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl PreferenceStore {
    /// The preferences in `file`; the defaults when it does not exist or cannot be read.
    pub fn open(file: PathBuf) -> PreferenceStore {
        let (current, problem) = match Preferences::load(&file) {
            Ok(preferences) => (preferences, None),
            Err(error) => (Preferences::default(), Some(error.to_string())),
        };
        PreferenceStore {
            file,
            current: Mutex::new(current),
            problem: Mutex::new(problem),
        }
    }

    /// The preferences now.
    pub fn current(&self) -> Preferences {
        guard(&self.current).clone()
    }

    /// Every preference, the defaults and the file, as the window shows them.
    pub fn view(&self) -> PreferencesView {
        PreferencesView {
            path: self.file.clone(),
            preferences: json!(self.current()),
            defaults: json!(Preferences::default()),
            languages: LANGUAGES.to_vec(),
            shortcut_sets: SHORTCUT_SETS.to_vec(),
            problem: guard(&self.problem).clone(),
        }
    }

    /// Sets the dotted `key` to `value` (`null` returns it to its default) and saves the
    /// file. A value the engine refuses changes nothing and answers
    /// `fosscut.preferences.invalid` with the reason; so does a file that cannot be
    /// written, with its own code.
    pub fn set(&self, key: &str, value: Value) -> Result<PreferencesView, ApiError> {
        {
            let mut current = guard(&self.current);
            let mut changed = current.clone();
            changed.set_value(key, value).map_err(ApiError::from)?;
            changed.save(&self.file).map_err(ApiError::from)?;
            *current = changed;
        }
        *guard(&self.problem) = None;
        Ok(self.view())
    }

    /// Returns the dotted `key` to its default and saves the file.
    pub fn reset(&self, key: &str) -> Result<PreferencesView, ApiError> {
        self.set(key, Value::Null)
    }
}

/// The frame rate of a new project: the one the window asked for, else the preferred one.
pub fn new_project_rate(preferences: &Preferences, asked: Option<&str>) -> String {
    asked
        .map(str::trim)
        .filter(|rate| !rate.is_empty())
        .map_or_else(|| preferences.timeline.frame_rate.clone(), str::to_string)
}

/// The loudness to level clips to: the one the window asked for, else the preferred one.
pub fn loudness_target(preferences: &Preferences, asked: Option<f64>) -> f64 {
    asked.unwrap_or(preferences.editing.loudness_target)
}

/// Where the dialog for a new project opens: the preferred folder, else the Movies folder
/// when there is one, else home.
pub fn projects_folder(preferences: &Preferences, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(folder) = &preferences.locations.projects {
        return Some(folder.clone());
    }
    let home = home?;
    let movies = home.join(if cfg!(target_os = "macos") {
        "Movies"
    } else {
        "Videos"
    });
    Some(if movies.is_dir() {
        movies
    } else {
        home.to_path_buf()
    })
}

/// Where the dialog for an export opens: the preferred folder, else beside the project.
pub fn exports_folder(preferences: &Preferences, bundle: Option<&Path>) -> Option<PathBuf> {
    preferences
        .locations
        .exports
        .clone()
        .or_else(|| bundle.and_then(Path::parent).map(Path::to_path_buf))
}

/// Where recordings go: the preferred folder, else none (the project's `recordings`).
pub fn recordings_folder(preferences: &Preferences) -> Option<PathBuf> {
    preferences.locations.recordings.clone()
}

/// The home folder, from the environment.
pub fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// What `update_check` answers: whether it asked, and what it found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UpdateAnswer {
    /// The `updates` preference: `off`, `notify` or `check`.
    pub policy: UpdatePolicy,
    /// Whether the release server was asked; never while `updates` is off.
    pub checked: bool,
    /// What the check found, once asked.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub found: Option<UpdateCheck>,
    /// Why nothing was asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Asks `address` (the GitHub releases API when `None`) for the latest release unless
/// `policy` is off, comparing it with `current`. Nothing about the person is sent.
pub fn check_for_updates(
    policy: UpdatePolicy,
    current: &str,
    address: Option<&str>,
) -> Result<UpdateAnswer, ApiError> {
    if policy == UpdatePolicy::Off {
        return Ok(UpdateAnswer {
            policy,
            checked: false,
            found: None,
            message: Some(
                "update checks are off; set updates to notify or check to allow them".into(),
            ),
        });
    }
    let found = fosscut_engine::updates::check(current, address).map_err(ApiError::from)?;
    Ok(UpdateAnswer {
        policy,
        checked: true,
        found: Some(found),
        message: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A preferences file in a folder of its own, never the person's.
    fn file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fosscut-desktop-prefs-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("preferences.json")
    }

    #[test]
    fn preferences_are_read_set_saved_and_reset() {
        let path = file("store");
        let store = PreferenceStore::open(path.clone());
        let view = store.view();
        assert_eq!(view.path, path);
        assert_eq!(view.preferences, view.defaults);
        assert_eq!(view.preferences["editing"]["frame_step"], 10);
        assert_eq!(view.languages, ["en", "hu"]);
        assert_eq!(view.shortcut_sets, ["fosscut", "classic"]);
        assert!(!path.exists(), "reading writes nothing");

        let view = store.set("editing.frame_step", json!(5)).unwrap();
        assert_eq!(view.preferences["editing"]["frame_step"], 5);
        assert_eq!(view.defaults["editing"]["frame_step"], 10);
        store.set("timeline.frame_rate", json!(25)).unwrap();
        store
            .set(
                "shortcuts",
                json!({ "set": "classic", "overrides": { "timeline.split": "Mod+K" } }),
            )
            .unwrap();
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(stored["editing"]["frame_step"], 5);
        assert_eq!(stored["timeline"]["frame_rate"], "25");
        assert_eq!(stored["shortcuts"]["overrides"]["timeline.split"], "Mod+K");
        assert!(stored.get("cache").is_none(), "only changes are written");

        let again = PreferenceStore::open(path.clone());
        assert_eq!(again.current(), store.current(), "read back as saved");
        let view = again.reset("editing.frame_step").unwrap();
        assert_eq!(view.preferences["editing"]["frame_step"], 10);
        assert_eq!(again.current().shortcuts.set, "classic");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn refused_values_change_nothing_and_say_why() {
        let path = file("refused");
        let store = PreferenceStore::open(path.clone());
        for (key, value, says) in [
            (
                "editing.frame_step",
                json!(0),
                "editing.frame_step must be from 1",
            ),
            ("language", json!("xx"), "language xx is not one of en, hu"),
            ("export.sound_name", json!("nothing"), "not a library sound"),
            (
                "locations.exports",
                json!("relative"),
                "must be an absolute path",
            ),
            (
                "editing.missing",
                json!(1),
                "there is no preference editing.missing",
            ),
        ] {
            let refused = store.set(key, value).unwrap_err();
            assert_eq!(refused.code, "fosscut.preferences.invalid", "{key}");
            assert!(refused.message.contains(says), "{key}: {}", refused.message);
        }
        assert_eq!(store.current(), Preferences::default());
        assert!(!path.exists(), "nothing was saved");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_unreadable_file_gives_the_defaults_until_a_change_replaces_it() {
        let path = file("unreadable");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        let store = PreferenceStore::open(path.clone());
        assert_eq!(store.current(), Preferences::default());
        let problem = store.view().problem.unwrap();
        assert!(problem.contains("cannot read"), "{problem}");
        let view = store.set("notifications", json!(false)).unwrap();
        assert_eq!(view.problem, None);
        assert!(!PreferenceStore::open(path.clone()).current().notifications);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn requests_take_the_preferred_values_unless_the_window_asks_for_others() {
        let mut preferences = Preferences::default();
        preferences.set("timeline.frame_rate", "29.97").unwrap();
        preferences.set("editing.loudness_target", "-16").unwrap();
        assert_eq!(new_project_rate(&preferences, None), "29.97");
        assert_eq!(new_project_rate(&preferences, Some("  ")), "29.97");
        assert_eq!(new_project_rate(&preferences, Some("60")), "60");
        assert_eq!(loudness_target(&preferences, None), -16.0);
        assert_eq!(loudness_target(&preferences, Some(-23.0)), -23.0);
        assert_eq!(new_project_rate(&Preferences::default(), None), "30");
        assert_eq!(loudness_target(&Preferences::default(), None), -14.0);
    }

    #[test]
    fn folders_follow_the_locations_or_their_defaults() {
        let home =
            std::env::temp_dir().join(format!("fosscut-desktop-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let defaults = Preferences::default();
        assert_eq!(projects_folder(&defaults, Some(&home)), Some(home.clone()));
        let movies = home.join(if cfg!(target_os = "macos") {
            "Movies"
        } else {
            "Videos"
        });
        std::fs::create_dir_all(&movies).unwrap();
        assert_eq!(projects_folder(&defaults, Some(&home)), Some(movies));
        assert_eq!(projects_folder(&defaults, None), None);
        let bundle = home.join("Work").join("Trip.fosscut");
        assert_eq!(
            exports_folder(&defaults, Some(&bundle)),
            Some(home.join("Work")),
            "beside the project"
        );
        assert_eq!(recordings_folder(&defaults), None);

        let mut chosen = Preferences::default();
        let place = |name: &str| home.join(name).display().to_string();
        chosen
            .set("locations.projects", &place("Projects"))
            .unwrap();
        chosen.set("locations.exports", &place("Exports")).unwrap();
        chosen.set("locations.recordings", &place("Takes")).unwrap();
        assert_eq!(projects_folder(&chosen, None), Some(home.join("Projects")));
        assert_eq!(
            exports_folder(&chosen, Some(&bundle)),
            Some(home.join("Exports"))
        );
        assert_eq!(recordings_folder(&chosen), Some(home.join("Takes")));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn update_checks_ask_only_when_allowed() {
        let off = check_for_updates(UpdatePolicy::Off, "0.1.0", Some("file:///nowhere")).unwrap();
        assert!(!off.checked);
        assert_eq!(off.found, None);
        assert!(off.message.unwrap().contains("off"));
        assert_eq!(
            serde_json::to_value(check_for_updates(UpdatePolicy::Off, "0.1.0", None).unwrap())
                .unwrap()["policy"],
            "off"
        );

        let dir =
            std::env::temp_dir().join(format!("fosscut-desktop-updates-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let release = dir.join("latest.json");
        std::fs::write(
            &release,
            r#"{ "tag_name": "v0.3.0", "html_url": "https://example.org/r/0.3.0" }"#,
        )
        .unwrap();
        let text = release.display().to_string().replace('\\', "/");
        let address = format!("file:///{}", text.trim_start_matches('/'));
        let found = check_for_updates(UpdatePolicy::Notify, "0.1.0", Some(&address)).unwrap();
        assert!(found.checked);
        let check = found.found.clone().unwrap();
        assert!(check.newer);
        assert_eq!(check.latest.as_deref(), Some("0.3.0"));
        let shown = serde_json::to_value(&found).unwrap();
        assert_eq!(shown["policy"], "notify");
        assert_eq!(shown["newer"], true);
        assert_eq!(shown["url"], "https://example.org/r/0.3.0");
        let failed = check_for_updates(
            UpdatePolicy::Check,
            "0.1.0",
            Some(&format!("{address}.missing")),
        )
        .unwrap_err();
        assert_eq!(failed.code, "fosscut.updates.check_failed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
