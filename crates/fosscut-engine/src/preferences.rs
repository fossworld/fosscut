//! The user's preferences (ADR-0031): one `preferences.json` per user in FossCut's
//! configuration folder, holding only what differs from the defaults.
//!
//! Commands never read preferences. Front ends fill requests from them (a new project's
//! rate, an image clip's length, a loudness target), and the engine applies the ones that
//! change speed and disk use but never results: hardware acceleration, proxies, remuxed
//! copies, the frame cache and cache cleanup (ADR-0032).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::EngineError;

/// The version of the preferences file this build writes.
pub const VERSION: u32 = 1;

/// The preferences file's name in [`config_dir`].
pub const FILE: &str = "preferences.json";

/// The languages the desktop app is translated into.
pub const LANGUAGES: [&str; 2] = ["en", "hu"];

/// FossCut's per-user configuration folder: `FOSSCUT_CONFIG_DIR` when set, otherwise
/// `FossCut` in `~/Library/Application Support` on macOS or in `%APPDATA%` on Windows, and
/// `fosscut` in `$XDG_CONFIG_HOME` (or `~/.config`) elsewhere.
pub fn config_dir() -> PathBuf {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = var("FOSSCUT_CONFIG_DIR") {
        return dir;
    }
    if cfg!(target_os = "macos") {
        var("HOME")
            .map(|home| {
                home.join("Library")
                    .join("Application Support")
                    .join("FossCut")
            })
            .unwrap_or_else(|| std::env::temp_dir().join("FossCut"))
    } else if cfg!(windows) {
        var("APPDATA")
            .map(|dir| dir.join("FossCut"))
            .unwrap_or_else(|| std::env::temp_dir().join("FossCut"))
    } else {
        var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".config")))
            .unwrap_or_else(std::env::temp_dir)
            .join("fosscut")
    }
}

/// The preferences file: [`FILE`] in [`config_dir`].
pub fn path() -> PathBuf {
    config_dir().join(FILE)
}

/// Every preference, with its default when the file does not set it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// The file's version.
    pub version: u32,
    /// Where projects, recordings and exports go.
    pub locations: Locations,
    /// How long caches are kept and how large they may grow.
    pub cache: CachePolicy,
    /// Editing defaults.
    pub editing: Editing,
    /// Timeline defaults.
    pub timeline: TimelineDefaults,
    /// What happens when an export finishes.
    pub export: ExportPreferences,
    /// Speed: hardware acceleration, proxies, the frame cache and remuxing.
    pub performance: Performance,
    /// The desktop app's language, as a code in [`LANGUAGES`].
    pub language: String,
    /// Whether FossCut checks for a newer release.
    pub updates: UpdatePolicy,
    /// Whether the desktop app shows system notifications (exports and recordings
    /// finished).
    pub notifications: bool,
    /// Keyboard shortcuts.
    pub shortcuts: Shortcuts,
}

impl Default for Preferences {
    fn default() -> Preferences {
        Preferences {
            version: VERSION,
            locations: Locations::default(),
            cache: CachePolicy::default(),
            editing: Editing::default(),
            timeline: TimelineDefaults::default(),
            export: ExportPreferences::default(),
            performance: Performance::default(),
            language: "en".into(),
            updates: UpdatePolicy::Notify,
            notifications: true,
            shortcuts: Shortcuts::default(),
        }
    }
}

/// Where things go; each is decided by the front end when not set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Locations {
    /// The folder new projects go in; the Movies folder (or home) when not set.
    pub projects: Option<PathBuf>,
    /// The folder recordings go in; the project's `recordings` folder when not set.
    pub recordings: Option<PathBuf>,
    /// The folder exports go in; beside the project when not set.
    pub exports: Option<PathBuf>,
}

/// How long caches are kept and how large they may grow (ADR-0032 item 5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CachePolicy {
    /// Remove cache entries unused for longer than this many days; 0 keeps them.
    pub max_age_days: u32,
    /// Remove the oldest entries beyond this many gigabytes; 0 sets no limit.
    pub limit_gb: f64,
}

impl Default for CachePolicy {
    fn default() -> CachePolicy {
        CachePolicy {
            max_age_days: 30,
            limit_gb: 20.0,
        }
    }
}

/// Editing defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Editing {
    /// Frames one step moves with Shift and an arrow key.
    pub frame_step: u32,
    /// Pixels one nudge moves a clip on the canvas.
    pub nudge_pixels: u32,
    /// How long a still image is when placed, in seconds.
    pub image_duration: f64,
    /// The loudness `audio normalize` aims for, in LUFS.
    pub loudness_target: f64,
}

impl Default for Editing {
    fn default() -> Editing {
        Editing {
            frame_step: 10,
            nudge_pixels: 10,
            image_duration: 5.0,
            loudness_target: -14.0,
        }
    }
}

/// How times are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeDisplay {
    /// `HH:MM:SS:FF`.
    #[default]
    Timecode,
    /// Frame numbers.
    Frames,
    /// Seconds.
    Seconds,
}

/// Timeline defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimelineDefaults {
    /// Play sound while scrubbing and skimming.
    pub scrub_audio: bool,
    /// Edit linked clips together (a clip's group follows it).
    pub linked: bool,
    /// The frame rate of new projects, as `fosscut export --fps` takes it.
    pub frame_rate: String,
    /// How the timeline and inspector show times.
    pub time_display: TimeDisplay,
}

impl Default for TimelineDefaults {
    fn default() -> TimelineDefaults {
        TimelineDefaults {
            scrub_audio: true,
            linked: true,
            frame_rate: "30".into(),
            time_display: TimeDisplay::Timecode,
        }
    }
}

/// What happens when an export finishes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportPreferences {
    /// Play a sound.
    pub sound: bool,
    /// The library sound to play.
    pub sound_name: String,
}

impl Default for ExportPreferences {
    fn default() -> ExportPreferences {
        ExportPreferences {
            sound: true,
            sound_name: "ding".into(),
        }
    }
}

/// When proxies are made and used (ADR-0032 item 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyChoice {
    /// Never.
    #[default]
    Off,
    /// For media longer than 1920 pixels on either side.
    Auto,
    /// For every video asset.
    On,
}

/// Hardware decoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecodingChoice {
    /// Hardware where the platform supports the codec, software otherwise.
    #[default]
    Auto,
    /// Always software.
    Off,
}

/// The size of proxies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyScale {
    /// Half the width and height.
    #[default]
    Half,
    /// A quarter of the width and height.
    Quarter,
}

/// Speed: hardware acceleration, proxies, the frame cache and remuxing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Performance {
    /// Hardware decoding.
    pub hardware_decoding: DecodingChoice,
    /// Use hardware encoders; when false, a codec with only hardware encoders is
    /// unavailable rather than replaced.
    pub hardware_encoding: bool,
    /// When proxies are made and used by the preview.
    pub proxies: ProxyChoice,
    /// The size of proxies.
    pub proxy_size: ProxyScale,
    /// Gigabytes of rendered preview frames to keep; 0 turns the frame cache off.
    pub render_cache_gb: f64,
    /// Render frames around the playhead into the cache while the app is idle.
    pub auto_render: bool,
    /// Copy media in hard-to-seek containers into QuickTime for the preview.
    pub auto_remux: bool,
}

impl Default for Performance {
    fn default() -> Performance {
        Performance {
            hardware_decoding: DecodingChoice::Auto,
            hardware_encoding: true,
            proxies: ProxyChoice::Off,
            proxy_size: ProxyScale::Half,
            render_cache_gb: 10.0,
            auto_render: true,
            auto_remux: true,
        }
    }
}

/// Whether FossCut checks for a newer release.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePolicy {
    /// Never ask.
    Off,
    /// Check when the app starts and say when a newer release exists.
    #[default]
    Notify,
    /// Also check daily while the app runs.
    Check,
}

/// Keyboard shortcuts: a set and the person's own changes to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    /// The set the overrides apply to: `fosscut` or `classic`.
    pub set: String,
    /// Keys by action, replacing the set's, for example `"timeline.split": "S"`; an empty
    /// string leaves the action without keys.
    pub overrides: BTreeMap<String, String>,
}

impl Default for Shortcuts {
    fn default() -> Shortcuts {
        Shortcuts {
            set: "fosscut".into(),
            overrides: BTreeMap::new(),
        }
    }
}

/// The shortcut sets the desktop app has.
pub const SHORTCUT_SETS: [&str; 2] = ["fosscut", "classic"];

fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::Failed {
        code: "fosscut.preferences.invalid".into(),
        message: message.into(),
    }
}

fn unreadable(path: &Path, error: impl std::fmt::Display) -> EngineError {
    EngineError::Failed {
        code: "fosscut.preferences.unreadable".into(),
        message: format!("cannot read {}: {error}", path.display()),
    }
}

impl Preferences {
    /// The preferences in `file`, defaults where it sets nothing; the defaults when the
    /// file does not exist. Fields this build does not know are ignored.
    pub fn load(file: &Path) -> Result<Preferences, EngineError> {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Preferences::default());
            }
            Err(error) => return Err(unreadable(file, error)),
        };
        let stored: Value = serde_json::from_str(&text).map_err(|e| unreadable(file, e))?;
        let mut merged = json!(Preferences::default());
        merge(&mut merged, &stored);
        let preferences: Preferences =
            serde_json::from_value(merged).map_err(|e| unreadable(file, e))?;
        preferences.validate()?;
        Ok(Preferences {
            version: VERSION,
            ..preferences
        })
    }

    /// Writes the values that differ from the defaults to `file`, atomically, creating its
    /// folder.
    pub fn save(&self, file: &Path) -> Result<(), EngineError> {
        self.validate()?;
        let sparse = self.sparse();
        let text = serde_json::to_string_pretty(&sparse).map_err(|e| invalid(e.to_string()))?;
        let write = |error: std::io::Error| EngineError::Failed {
            code: "fosscut.preferences.write_failed".into(),
            message: format!("cannot write {}: {error}", file.display()),
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(write)?;
        }
        let temporary = file.with_extension("json.partial");
        std::fs::write(&temporary, format!("{text}\n")).map_err(write)?;
        std::fs::rename(&temporary, file).map_err(write)
    }

    /// The values that differ from the defaults, with the version.
    pub fn sparse(&self) -> Value {
        let mut value = difference(&json!(self), &json!(Preferences::default()))
            .unwrap_or_else(|| Value::Object(Map::new()));
        value["version"] = json!(VERSION);
        value
    }

    /// The value at a dotted `key` such as `editing.frame_step`, or every preference for
    /// an empty key.
    pub fn get(&self, key: &str) -> Result<Value, EngineError> {
        let all = json!(self);
        if key.is_empty() {
            return Ok(all);
        }
        key.split('.')
            .try_fold(&all, |value, part| value.get(part))
            .cloned()
            .ok_or_else(|| unknown(key))
    }

    /// Sets the dotted `key` to `value`: JSON, or a bare word taken as text (`off`,
    /// `/Users/me/Movies`). `null` returns a key to its default.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), EngineError> {
        let parsed: Value =
            serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.into()));
        self.set_value(key, parsed)
    }

    /// Sets the dotted `key` to a JSON value; `null` returns it to its default.
    pub fn set_value(&mut self, key: &str, value: Value) -> Result<(), EngineError> {
        if key.is_empty() || key == "version" {
            return Err(invalid("name a preference, for example editing.frame_step"));
        }
        // Shortcut overrides are keyed by actions, whose names have dots of their own.
        if let Some(action) = key.strip_prefix("shortcuts.overrides.") {
            let mut updated = self.clone();
            match value {
                Value::Null => {
                    updated.shortcuts.overrides.remove(action);
                }
                Value::String(keys) => {
                    updated.shortcuts.overrides.insert(action.to_string(), keys);
                }
                other => {
                    return Err(invalid(format!(
                        "{key}: keys are text such as \"Shift+S\", not {other}"
                    )));
                }
            }
            updated.validate()?;
            *self = updated;
            return Ok(());
        }
        let defaults = json!(Preferences::default());
        let mut all = json!(self);
        let parts: Vec<&str> = key.split('.').collect();
        let (last, parents) = parts.split_last().ok_or_else(|| unknown(key))?;
        let mut target = &mut all;
        for part in parents {
            target = target.get_mut(*part).ok_or_else(|| unknown(key))?;
        }
        let object = target.as_object_mut().ok_or_else(|| unknown(key))?;
        if !object.contains_key(*last) {
            return Err(unknown(key));
        }
        let value = match value {
            Value::Null => key
                .split('.')
                .try_fold(&defaults, |v, part| v.get(part))
                .cloned()
                .unwrap_or(Value::Null),
            // A text preference given a bare number, such as a frame rate of 29.97.
            Value::Number(number) if object.get(*last).is_some_and(Value::is_string) => {
                Value::String(number.to_string())
            }
            value => value,
        };
        object.insert((*last).to_string(), value);
        let updated: Preferences =
            serde_json::from_value(all).map_err(|e| invalid(format!("{key}: {e}")))?;
        updated.validate()?;
        *self = updated;
        Ok(())
    }

    /// Checks every value is in range, naming the first that is not.
    pub fn validate(&self) -> Result<(), EngineError> {
        let range = |name: &str, value: f64, low: f64, high: f64| {
            if value.is_finite() && (low..=high).contains(&value) {
                Ok(())
            } else {
                Err(invalid(format!(
                    "{name} must be from {low} to {high}, not {value}"
                )))
            }
        };
        range(
            "cache.max_age_days",
            f64::from(self.cache.max_age_days),
            0.0,
            3650.0,
        )?;
        range("cache.limit_gb", self.cache.limit_gb, 0.0, 100_000.0)?;
        range(
            "editing.frame_step",
            f64::from(self.editing.frame_step),
            1.0,
            1000.0,
        )?;
        range(
            "editing.nudge_pixels",
            f64::from(self.editing.nudge_pixels),
            1.0,
            1000.0,
        )?;
        range(
            "editing.image_duration",
            self.editing.image_duration,
            0.1,
            3600.0,
        )?;
        range(
            "editing.loudness_target",
            self.editing.loudness_target,
            -40.0,
            -5.0,
        )?;
        range(
            "performance.render_cache_gb",
            self.performance.render_cache_gb,
            0.0,
            10_000.0,
        )?;
        let rate = crate::frame_rate(&self.timeline.frame_rate)
            .map_err(|e| invalid(format!("timeline.frame_rate: {e}")))?;
        range(
            "timeline.frame_rate",
            f64::from(rate.num()) / f64::from(rate.den().max(1)),
            12.0,
            120.0,
        )?;
        if !LANGUAGES.contains(&self.language.as_str()) {
            return Err(invalid(format!(
                "language {} is not one of {}",
                self.language,
                LANGUAGES.join(", ")
            )));
        }
        if !SHORTCUT_SETS.contains(&self.shortcuts.set.as_str()) {
            return Err(invalid(format!(
                "shortcuts.set {} is not one of {}",
                self.shortcuts.set,
                SHORTCUT_SETS.join(", ")
            )));
        }
        if crate::library::find(&self.export.sound_name, Some("sound")).is_err() {
            return Err(invalid(format!(
                "export.sound_name {} is not a library sound",
                self.export.sound_name
            )));
        }
        for (name, place) in [
            ("locations.projects", &self.locations.projects),
            ("locations.recordings", &self.locations.recordings),
            ("locations.exports", &self.locations.exports),
        ] {
            if place.as_ref().is_some_and(|p| !p.is_absolute()) {
                return Err(invalid(format!("{name} must be an absolute path")));
            }
        }
        Ok(())
    }

    /// Makes this process decode and encode as the performance preferences say: hardware
    /// decoding on or off, and hardware encoders used or left out (ADR-0032 item 6).
    pub fn apply_performance(&self) {
        crate::hardware::set_hardware_decoding(match self.performance.hardware_decoding {
            DecodingChoice::Auto => fosscut_media::HardwareDecoding::Auto,
            DecodingChoice::Off => fosscut_media::HardwareDecoding::Off,
        });
        crate::hardware::set_hardware_encoding(if self.performance.hardware_encoding {
            fosscut_media::HardwareEncoding::Auto
        } else {
            fosscut_media::HardwareEncoding::Off
        });
    }

    /// The cache policy for the cache folder `cache` (ADR-0032 item 5).
    pub fn cache_policy(&self, cache: PathBuf) -> crate::cache::CachePolicy {
        crate::cache::CachePolicy {
            cache,
            max_age_days: self.cache.max_age_days,
            limit_bytes: self.cache_limit_bytes(),
        }
    }

    /// The proxy size the preferences choose.
    pub fn proxy_size(&self) -> crate::proxy::ProxySize {
        match self.performance.proxy_size {
            ProxyScale::Half => crate::proxy::ProxySize::Half,
            ProxyScale::Quarter => crate::proxy::ProxySize::Quarter,
        }
    }

    /// When proxies are made, as the proxy jobs take it.
    pub fn proxy_policy(&self) -> crate::proxy::ProxyPolicy {
        match self.performance.proxies {
            ProxyChoice::Off => crate::proxy::ProxyPolicy::Off,
            ProxyChoice::Auto => crate::proxy::ProxyPolicy::Auto,
            ProxyChoice::On => crate::proxy::ProxyPolicy::On,
        }
    }

    /// The cache limit in bytes, 0 for none.
    pub fn cache_limit_bytes(&self) -> u64 {
        (self.cache.limit_gb * 1e9) as u64
    }

    /// The frame cache's limit in bytes, 0 when it is off.
    pub fn render_cache_bytes(&self) -> u64 {
        (self.performance.render_cache_gb * 1e9) as u64
    }
}

fn unknown(key: &str) -> EngineError {
    invalid(format!(
        "there is no preference {key}; `fosscut preferences` lists them"
    ))
}

/// Writes `stored` over `base`, object by object.
fn merge(base: &mut Value, stored: &Value) {
    match (base, stored) {
        (Value::Object(base), Value::Object(stored)) => {
            for (key, value) in stored {
                match base.get_mut(key) {
                    Some(existing) if existing.is_object() && value.is_object() => {
                        merge(existing, value)
                    }
                    _ => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, stored) => *base = stored.clone(),
    }
}

/// The parts of `value` that differ from `default`; `None` when nothing does.
fn difference(value: &Value, default: &Value) -> Option<Value> {
    match (value, default) {
        (Value::Object(value), Value::Object(default)) => {
            let mut out = Map::new();
            for (key, item) in value {
                let changed = match default.get(key) {
                    Some(base) => difference(item, base),
                    None => Some(item.clone()),
                };
                if let Some(changed) = changed {
                    out.insert(key.clone(), changed);
                }
            }
            (!out.is_empty()).then_some(Value::Object(out))
        }
        (value, default) if value == default => None,
        (value, _) => Some(value.clone()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn temporary(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fosscut-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join(FILE)
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let file = temporary("missing");
        let loaded = Preferences::load(&file).unwrap();
        assert_eq!(loaded, Preferences::default());
        assert_eq!(loaded.editing.frame_step, 10);
        assert_eq!(loaded.cache.max_age_days, 30);
        assert_eq!(loaded.performance.render_cache_gb, 10.0);
        assert_eq!(loaded.sparse(), json!({ "version": 1 }));
    }

    #[test]
    fn only_changes_are_written_and_read_back() {
        let file = temporary("save");
        let mut preferences = Preferences::default();
        preferences.set("editing.frame_step", "5").unwrap();
        preferences.set("performance.proxies", "auto").unwrap();
        preferences.set("timeline.frame_rate", "29.97").unwrap();
        preferences
            .set("shortcuts.overrides.timeline.split", "S")
            .unwrap();
        preferences
            .set("shortcuts.overrides.player.play", "K")
            .unwrap();
        preferences
            .set("shortcuts.overrides.player.play", "null")
            .unwrap();
        preferences.save(&file).unwrap();
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(
            stored,
            json!({
                "version": 1,
                "editing": { "frame_step": 5 },
                "performance": { "proxies": "auto" },
                "timeline": { "frame_rate": "29.97" },
                "shortcuts": { "overrides": { "timeline.split": "S" } },
            })
        );
        assert!(!file.with_extension("json.partial").exists());
        let loaded = Preferences::load(&file).unwrap();
        assert_eq!(loaded, preferences);
        assert_eq!(loaded.get("editing.frame_step").unwrap(), json!(5));
        assert_eq!(loaded.get("performance").unwrap()["proxies"], "auto");
        // null returns a value to its default.
        let mut reset = loaded.clone();
        reset.set("editing.frame_step", "null").unwrap();
        assert_eq!(reset.editing.frame_step, 10);
    }

    #[test]
    fn unknown_fields_are_ignored_and_bad_values_refused() {
        let file = temporary("unknown");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"{ "version": 2, "editing": { "frame_step": 3, "later": true }, "newer": 1 }"#,
        )
        .unwrap();
        let loaded = Preferences::load(&file).unwrap();
        assert_eq!(loaded.editing.frame_step, 3);
        assert_eq!(loaded.version, VERSION);

        let mut preferences = Preferences::default();
        let code = |result: Result<(), EngineError>| result.unwrap_err().code().to_string();
        for (key, value) in [
            ("editing.frame_step", "0"),
            ("editing.loudness_target", "3"),
            ("timeline.frame_rate", "7"),
            ("language", "xx"),
            ("shortcuts.set", "mine"),
            ("export.sound_name", "nothing"),
            ("performance.proxies", "sometimes"),
            ("locations.exports", "relative/folder"),
            ("editing.missing", "1"),
            ("version", "3"),
        ] {
            assert_eq!(
                code(preferences.set(key, value)),
                "fosscut.preferences.invalid",
                "{key}={value}"
            );
        }
        assert_eq!(
            preferences,
            Preferences::default(),
            "refusals change nothing"
        );
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(
            Preferences::load(&file).unwrap_err().code(),
            "fosscut.preferences.unreadable"
        );
    }
}
