//! The analysis cache (ADR-0019, item 3): one JSON file per file content, analyzer version
//! and options, under the user's cache directory and never in a project. Deleting it only
//! costs the time to analyze again.

use std::path::{Path, PathBuf};

use fosscut_model::ContentIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Analyzer;

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
