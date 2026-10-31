//! The analysis cache (ADR-0019, item 3): one JSON file per file content, analyzer version
//! and options, under the user's cache directory and never in a project. Deleting it only
//! costs the time to analyze again.
//!
//! Analyzers with per-frame results too large for JSON (pose and mattes,
//! [`Analyzer::has_data`]) also keep a binary data file beside their entry, with the same name
//! and the extension `.bin`. The entry's result names it by size and BLAKE3 hash
//! ([`crate::vision::DataFile`]), and it is written before the entry, so an entry never points
//! at a file that is not there yet; a file that does not match is ignored.

use std::path::{Path, PathBuf};

use fosscut_model::ContentIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Analyzer;
use crate::vision::DataFile;

/// What a cache file holds: the result with everything that produced it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    analyzer: String,
    version: u32,
    options: Value,
    result: Value,
}

/// Where the result of `analyzer` with `options` on the file with `identity` is kept, below
/// `root` (the engine's cache directory).
pub fn entry_path(
    root: &Path,
    identity: &ContentIdentity,
    analyzer: Analyzer,
    options: &Value,
) -> PathBuf {
    // The identity comes from the project's media table; keep only hex digits so an edited
    // table cannot point outside the cache.
    let hash: String = identity
        .sample_blake3
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(64)
        .collect();
    let options_hash = blake3::hash(options.to_string().as_bytes()).to_hex();
    root.join("analysis")
        .join(format!("{hash}-{}", identity.size))
        .join(format!(
            "{}@{}-{}.json",
            analyzer.name(),
            analyzer.version(),
            &options_hash.as_str()[..16]
        ))
}

/// The cached result at `path`, if it was produced by this version of `analyzer` with
/// `options`.
pub fn read(path: &Path, analyzer: Analyzer, options: &Value) -> Option<Value> {
    let entry: Entry = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (entry.analyzer == analyzer.name()
        && entry.version == analyzer.version()
        && entry.options == *options)
        .then_some(entry.result)
}

/// Stores `result` at `path`, through a temporary file so readers never see half of it.
pub fn write(
    path: &Path,
    analyzer: Analyzer,
    options: &Value,
    result: &Value,
) -> std::io::Result<()> {
    let entry = Entry {
        analyzer: analyzer.name().to_string(),
        version: analyzer.version(),
        options: options.clone(),
        result: result.clone(),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("partial");
    std::fs::write(&temporary, serde_json::to_vec(&entry)?)?;
    std::fs::rename(&temporary, path)
}

/// The data file beside the entry at `entry`.
pub fn data_path(entry: &Path) -> PathBuf {
    entry.with_extension("bin")
}

/// Stores `data` as the data file of the entry at `entry`, through a temporary file.
pub fn write_data(entry: &Path, data: &[u8]) -> std::io::Result<()> {
    let path = data_path(entry);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("bin.partial");
    std::fs::write(&temporary, data)?;
    std::fs::rename(&temporary, &path)
}

/// Whether the data file of the entry at `entry` is there with `file`'s size; its contents are
/// checked when read.
pub fn has_data(entry: &Path, file: &DataFile) -> bool {
    std::fs::metadata(data_path(entry)).is_ok_and(|m| m.is_file() && m.len() == file.bytes)
}

/// The data file of the entry at `entry`, if it is the one `file` describes.
pub fn read_data(entry: &Path, file: &DataFile) -> Option<Vec<u8>> {
    let data = std::fs::read(data_path(entry)).ok()?;
    file.matches(&data).then_some(data)
}

/// A cached result of one analyzer, with any options.
#[derive(Clone, Debug, PartialEq)]
pub struct Cached {
    /// The entry's path.
    pub path: PathBuf,
    /// The options it was measured with.
    pub options: Value,
    /// The result.
    pub result: Value,
    /// When it was written.
    pub modified: Option<std::time::SystemTime>,
}

/// Every result of this version of `analyzer` cached for the file with `identity`, whatever
/// its options, newest first: for effects that read an analysis however it was measured.
pub fn entries(root: &Path, identity: &ContentIdentity, analyzer: Analyzer) -> Vec<Cached> {
    let probe = entry_path(root, identity, analyzer, &Value::Null);
    let Some(dir) = probe.parent() else {
        return Vec::new();
    };
    let prefix = format!("{}@{}-", analyzer.name(), analyzer.version());
    let Ok(listing) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<Cached> = listing
        .filter_map(Result::ok)
        .filter(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix) && name.ends_with(".json")
        })
        .filter_map(|item| {
            let path = item.path();
            let entry: Entry = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            (entry.analyzer == analyzer.name() && entry.version == analyzer.version()).then(|| {
                Cached {
                    modified: item.metadata().ok().and_then(|m| m.modified().ok()),
                    path,
                    options: entry.options,
                    result: entry.result,
                }
            })
        })
        .collect();
    found.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    found
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn identity(hash: &str) -> ContentIdentity {
        ContentIdentity {
            size: 1234,
            sample_blake3: hash.into(),
        }
    }

    #[test]
    fn entries_round_trip_and_are_keyed_by_everything_that_matters() {
        let root = std::env::temp_dir().join(format!("fosscut-cache-{}", std::process::id()));
        let id = identity("ab12cd");
        let options = json!({ "threshold_dbfs": -40.0, "min_duration_ms": 300 });
        let path = entry_path(&root, &id, Analyzer::Silences, &options);
        assert!(path.starts_with(root.join("analysis").join("ab12cd-1234")));
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("silences@1-")
        );
        assert_eq!(
            read(&path, Analyzer::Silences, &options),
            None,
            "nothing cached yet"
        );

        let result = json!({ "ranges": [] });
        write(&path, Analyzer::Silences, &options, &result).unwrap();
        assert_eq!(read(&path, Analyzer::Silences, &options), Some(result));
        // Other options, another analyzer or another file use other entries.
        let stricter = json!({ "threshold_dbfs": -50.0, "min_duration_ms": 300 });
        assert_ne!(entry_path(&root, &id, Analyzer::Silences, &stricter), path);
        assert_ne!(
            entry_path(&root, &id, Analyzer::Loudness, &Value::Null),
            path
        );
        assert_ne!(
            entry_path(&root, &identity("ef34"), Analyzer::Silences, &options),
            path
        );
        // An entry that does not match what was asked for is ignored.
        assert_eq!(read(&path, Analyzer::Silences, &stricter), None);
        assert_eq!(read(&path, Analyzer::Scenes, &options), None);
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(read(&path, Analyzer::Silences, &options), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn data_files_sit_beside_entries_and_are_found_with_any_options() {
        let root = std::env::temp_dir().join(format!("fosscut-cache-data-{}", std::process::id()));
        let id = identity("cd34");
        let person = json!({ "model": "person" });
        let subject = json!({ "model": "subject", "every": 5 });
        let first = entry_path(&root, &id, Analyzer::Matte, &person);
        let second = entry_path(&root, &id, Analyzer::Matte, &subject);
        assert_eq!(data_path(&first).extension().unwrap(), "bin");
        let data = b"per-frame mattes";
        let file = DataFile::of(data);
        assert!(!has_data(&first, &file));
        write_data(&first, data).unwrap();
        assert!(has_data(&first, &file));
        assert_eq!(read_data(&first, &file).unwrap(), data);
        assert_eq!(read_data(&first, &DataFile::of(b"other")), None);
        write(&first, Analyzer::Matte, &person, &json!({ "frames": 1 })).unwrap();
        write(&second, Analyzer::Matte, &subject, &json!({ "frames": 2 })).unwrap();
        write(
            &entry_path(&root, &id, Analyzer::Pose, &json!({})),
            Analyzer::Pose,
            &json!({}),
            &json!({}),
        )
        .unwrap();
        let found = entries(&root, &id, Analyzer::Matte);
        assert_eq!(found.len(), 2, "both mattes, not the pose");
        assert!(
            found
                .iter()
                .any(|c| c.options == subject && c.result["frames"] == 2)
        );
        assert!(entries(&root, &identity("ff"), Analyzer::Matte).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn identities_cannot_escape_the_cache() {
        let root = Path::new("/cache");
        // Only the hex digits of "../../etc" survive: "ec".
        let path = entry_path(
            root,
            &identity("../../etc"),
            Analyzer::Loudness,
            &Value::Null,
        );
        assert_eq!(path.parent(), Some(Path::new("/cache/analysis/ec-1234")));
    }
}
