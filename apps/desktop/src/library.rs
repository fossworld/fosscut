//! The project browser's data: a summary of each recent project (name, length, size,
//! thumbnail), and the trash, where projects go when deleted from the browser and from
//! which they can be restored. Projects in the trash are whole bundles, moved, never
//! rewritten.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use fosscut_core::{ApiError, codes};
use fosscut_engine::live::WriterLock;
use fosscut_render::Renderer;
use fosscut_time::{Flicks, TimeReport};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Thumbnail width in pixels.
const THUMBNAIL: u32 = 240;

/// One project in the browser.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectSummary {
    /// The bundle.
    pub path: PathBuf,
    /// The project's name.
    pub name: String,
    /// The primary timeline's length.
    pub duration: TimeReport,
    /// How many timelines it has.
    pub timelines: usize,
    /// How many assets its media bin has.
    pub assets: u64,
    /// The bundle's size on disk in bytes (its media files are elsewhere).
    pub bytes: u64,
    /// When it last changed, in milliseconds since 1970.
    pub modified_ms: u64,
    /// A frame of the primary timeline as a PNG data URL, when one could be rendered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
}

/// A project in the trash.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrashEntry {
    /// The entry's folder name in the trash.
    pub id: String,
    /// The project's name.
    pub name: String,
    /// Where it was.
    pub original: PathBuf,
    /// When it was moved to the trash, in milliseconds since 1970.
    pub trashed_ms: u64,
}

/// The browser's store: thumbnails in the cache folder and the trash in the data folder.
pub struct Library {
    trash: PathBuf,
    thumbnails: PathBuf,
    renderer: Mutex<Option<Renderer>>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn io(what: &str, error: std::io::Error) -> ApiError {
    ApiError::new("fosscut.storage.io", format!("{what}: {error}"))
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(codes::INVALID_ARGUMENTS, message)
}

/// The total size of the files under `path`.
fn size_of(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return std::fs::metadata(path).map_or(0, |m| m.len());
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                size_of(&path)
            } else {
                entry.metadata().map_or(0, |m| m.len())
            }
        })
        .sum()
}

/// Moves a folder, copying it when a rename cannot cross file systems.
fn move_dir(from: &Path, to: &Path) -> Result<(), ApiError> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io("cannot create the trash", e))?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_dir(from, to)?;
    std::fs::remove_dir_all(from).map_err(|e| io("cannot finish moving the project", e))
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), ApiError> {
    std::fs::create_dir_all(to).map_err(|e| io("cannot copy the project", e))?;
    for entry in std::fs::read_dir(from).map_err(|e| io("cannot read the project", e))? {
        let entry = entry.map_err(|e| io("cannot read the project", e))?;
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| io("cannot copy the project", e))?;
        }
    }
    Ok(())
}

/// A name for an entry folder: letters, digits, dashes and underscores only.
fn slug(name: &str) -> String {
    let kept: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    kept.chars().take(40).collect()
}

impl Library {
    /// A library keeping the trash in `data` and thumbnails in `cache`.
    pub fn new(data: &Path, cache: &Path) -> Library {
        Library {
            trash: data.join("Trash"),
            thumbnails: cache.join("thumbnails"),
            renderer: Mutex::new(None),
        }
    }

    /// Reads a project's summary; `thumbnails` renders a frame (cached by revision).
    pub fn summary(&self, bundle: &Path, thumbnails: bool) -> Result<ProjectSummary, ApiError> {
        let info = fosscut_engine::project::query(bundle, "project.info", Value::Null)?;
        let primary = info["primary_timeline"].as_str().unwrap_or_default();
        let timelines = info["timelines"].as_array().cloned().unwrap_or_default();
        let shown = timelines.iter().find(|t| t["id"] == primary);
        let duration = shown
            .and_then(|t| t["duration"]["flicks"].as_i64())
            .unwrap_or(0);
        let rate = shown
            .and_then(|t| t["format"]["rate"].as_str())
            .and_then(|r| r.parse().ok())
            .unwrap_or(fosscut_time::Rate::FPS_30);
        let modified_ms = std::fs::metadata(bundle.join(fosscut_model::MANIFEST_PATH))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        let revision = info["revision"].as_u64().unwrap_or(0);
        let thumbnail = if thumbnails && duration > 0 {
            self.thumbnail(bundle, revision, Flicks::new(duration))
        } else {
            None
        };
        Ok(ProjectSummary {
            path: bundle.to_path_buf(),
            name: info["name"].as_str().unwrap_or_default().to_string(),
            duration: TimeReport::new(Flicks::new(duration), rate),
            timelines: timelines.len(),
            assets: info["assets"].as_u64().unwrap_or(0),
            bytes: size_of(bundle),
            modified_ms,
            thumbnail,
        })
    }

    /// A frame from a third of the way in, as a data URL; cached by bundle and revision.
    fn thumbnail(&self, bundle: &Path, revision: u64, duration: Flicks) -> Option<String> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bundle.hash(&mut hasher);
        let file = self
            .thumbnails
            .join(format!("{:016x}-{revision}.png", hasher.finish()));
        let png = match std::fs::read(&file) {
            Ok(png) => png,
            Err(_) => {
                let project = fosscut_engine::project::snapshot(bundle).ok()?;
                let timeline = project.manifest.primary_timeline.clone();
                let mut renderer = self.renderer.lock().ok()?;
                if renderer.is_none() {
                    *renderer = Renderer::headless().ok();
                }
                let at = Flicks::new(duration.get() / 3);
                let still = fosscut_engine::frame::frame(
                    &project,
                    bundle,
                    &timeline,
                    at,
                    Some(THUMBNAIL),
                    renderer.as_mut()?,
                )
                .ok()?;
                let png = fosscut_engine::frame::png(&still.image).ok()?;
                let _ = std::fs::create_dir_all(&self.thumbnails);
                let _ = std::fs::write(&file, &png);
                png
            }
        };
        Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        ))
    }

    /// Moves a project bundle to the trash. Refuses anything but a FossCut project, and a
    /// project another program holds open.
    pub fn trash(&self, bundle: &Path) -> Result<TrashEntry, ApiError> {
        if !bundle.join(fosscut_model::MANIFEST_PATH).is_file() {
            return Err(invalid(format!(
                "{} is not a FossCut project",
                bundle.display()
            )));
        }
        let Some(lock) = WriterLock::try_acquire(bundle)? else {
            return Err(ApiError::new(
                "fosscut.session.already_hosted",
                "another program has this project open",
            )
            .with_hint("Close it there first."));
        };
        let name = bundle
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Project".into());
        let trashed_ms = now_ms();
        let id = format!("{trashed_ms}-{}", slug(&name));
        let entry = TrashEntry {
            id: id.clone(),
            name: name.clone(),
            original: bundle.to_path_buf(),
            trashed_ms,
        };
        let folder = self.trash.join(&id);
        // The lock showed nobody holds the project. Windows cannot move a folder with an
        // open file in it, so it is released just before the move.
        drop(lock);
        move_dir(bundle, &folder.join(format!("{name}.fosscut")))?;
        let record = serde_json::to_vec_pretty(&entry).unwrap_or_default();
        std::fs::write(folder.join("entry.json"), record)
            .map_err(|e| io("cannot record the trashed project", e))?;
        Ok(entry)
    }

    /// Every project in the trash, newest first.
    pub fn trashed(&self) -> Vec<TrashEntry> {
        let mut entries: Vec<TrashEntry> = std::fs::read_dir(&self.trash)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read(e.path().join("entry.json")).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.trashed_ms));
        entries
    }

    fn entry(&self, id: &str) -> Result<TrashEntry, ApiError> {
        self.trashed()
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| invalid(format!("nothing in the trash is called {id}")))
    }

    /// Moves a project back from the trash to where it was, or beside it with
    /// ` (restored)` when that place is taken. Returns where it is now.
    pub fn restore(&self, id: &str) -> Result<PathBuf, ApiError> {
        let entry = self.entry(id)?;
        let folder = self.trash.join(&entry.id);
        let mut target = entry.original.clone();
        let mut n = 1;
        while target.exists() {
            let stem = if n == 1 {
                format!("{} (restored)", entry.name)
            } else {
                format!("{} (restored {n})", entry.name)
            };
            target = entry.original.with_file_name(format!("{stem}.fosscut"));
            n += 1;
        }
        move_dir(&folder.join(format!("{}.fosscut", entry.name)), &target)?;
        let _ = std::fs::remove_dir_all(&folder);
        Ok(target)
    }

    /// Deletes a project in the trash for good.
    pub fn delete(&self, id: &str) -> Result<(), ApiError> {
        let entry = self.entry(id)?;
        std::fs::remove_dir_all(self.trash.join(&entry.id))
            .map_err(|e| io("cannot delete the trashed project", e))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("fosscut-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn project(root: &Path, name: &str) -> PathBuf {
        let bundle = root.join(format!("{name}.fosscut"));
        fosscut_engine::project::create(
            &bundle,
            name,
            fosscut_model::TimelineFormat {
                width: 320,
                height: 180,
                rate: fosscut_time::Rate::FPS_30,
                sample_rate: 48_000,
            },
        )
        .unwrap();
        bundle
    }

    #[test]
    fn summaries_tell_name_length_and_size() {
        let root = scratch("summary");
        let library = Library::new(&root.join("data"), &root.join("cache"));
        let bundle = project(&root, "Trip");
        let summary = library.summary(&bundle, true).unwrap();
        assert_eq!(summary.name, "Trip");
        assert_eq!(summary.duration.flicks, Flicks::ZERO);
        assert_eq!(summary.timelines, 1);
        assert!(summary.bytes > 0 && summary.modified_ms > 0);
        assert!(
            summary.thumbnail.is_none(),
            "an empty timeline has no frame"
        );
        assert!(library.summary(&root.join("none.fosscut"), false).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn projects_go_to_the_trash_and_come_back() {
        let root = scratch("trash");
        let library = Library::new(&root.join("data"), &root.join("cache"));
        let bundle = project(&root, "Old cut");
        let entry = library.trash(&bundle).unwrap();
        assert!(!bundle.exists(), "moved away");
        assert_eq!(library.trashed(), vec![entry.clone()]);
        assert_eq!(entry.name, "Old cut");
        // Its place was taken meanwhile: it comes back beside it.
        let again = project(&root, "Old cut");
        let restored = library.restore(&entry.id).unwrap();
        assert_eq!(restored, root.join("Old cut (restored).fosscut"));
        assert!(restored.join(fosscut_model::MANIFEST_PATH).is_file());
        assert!(library.trashed().is_empty());
        let second = library.trash(&again).unwrap();
        library.delete(&second.id).unwrap();
        assert!(library.trashed().is_empty() && !again.exists());
        // Only FossCut projects.
        let folder = root.join("photos");
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(
            library.trash(&folder).unwrap_err().code,
            codes::INVALID_ARGUMENTS
        );
        assert!(folder.exists());
        assert!(library.restore("missing").is_err() && library.delete("missing").is_err());
        assert_eq!(slug("A b/c"), "A_b_c");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_project_held_open_elsewhere_stays() {
        let root = scratch("held");
        let library = Library::new(&root.join("data"), &root.join("cache"));
        let bundle = project(&root, "Busy");
        let held = WriterLock::acquire(&bundle).unwrap();
        assert_eq!(
            library.trash(&bundle).unwrap_err().code,
            "fosscut.session.already_hosted"
        );
        assert!(bundle.exists());
        drop(held);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
