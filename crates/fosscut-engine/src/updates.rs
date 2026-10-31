//! Checking for a newer release (ADR-0031 item 6): one request for the latest release of
//! FossCut on GitHub, through `curl` as model downloads are, sending nothing about the
//! user. It only reports; installing stays the person's action.

use std::process::Command;

use serde::Serialize;
use serde_json::Value;

use crate::EngineError;

/// Where the latest release is described.
pub const RELEASES: &str = "https://api.github.com/repos/fossworld/fosscut/releases/latest";

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UpdateCheck {
    /// This build's version.
    pub current: String,
    /// The latest release's version, or none when nothing is published yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    /// Whether the latest release is newer than this build.
    pub newer: bool,
    /// The release's page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// When it was published, as GitHub reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
}

fn failed(message: impl Into<String>) -> EngineError {
    EngineError::Failed {
        code: "fosscut.updates.check_failed".into(),
        message: message.into(),
    }
}

/// Asks `address` (the [`RELEASES`] API when `None`) for the latest release and compares
/// it with `current`.
pub fn check(current: &str, address: Option<&str>) -> Result<UpdateCheck, EngineError> {
    let address = address.unwrap_or(RELEASES);
    let output = Command::new("curl")
        .args([
            "--location",
            "--silent",
            "--show-error",
            "--max-time",
            "15",
            "--header",
            "Accept: application/vnd.github+json",
            "--write-out",
            "\n%{http_code}",
        ])
        .arg(address)
        .output()
        .map_err(|e| failed(format!("cannot run curl: {e}")))?;
    if !output.status.success() {
        return Err(failed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, status) = text.rsplit_once('\n').unwrap_or((&text, ""));
    // file:// answers have no HTTP status; GitHub answers 404 before the first release.
    match status.trim() {
        "404" => Ok(UpdateCheck {
            current: current.into(),
            latest: None,
            newer: false,
            url: None,
            published: None,
        }),
        "200" | "000" | "" => parse(current, body),
        other => Err(failed(format!("the release server answered {other}"))),
    }
}

/// Reads a release description as GitHub's API gives it.
pub fn parse(current: &str, body: &str) -> Result<UpdateCheck, EngineError> {
    let release: Value = serde_json::from_str(body)
        .map_err(|e| failed(format!("the release description is not JSON: {e}")))?;
    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| failed("the release description has no tag_name"))?;
    let latest = tag.trim_start_matches('v').to_string();
    Ok(UpdateCheck {
        current: current.into(),
        newer: is_newer(&latest, current),
        latest: Some(latest),
        url: release["html_url"].as_str().map(str::to_string),
        published: release["published_at"].as_str().map(str::to_string),
    })
}

/// Whether version `latest` is newer than `current`, comparing dot-separated numbers; a
/// pre-release (`1.0.0-rc.1`) is older than its release.
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parts(version: &str) -> (Vec<u64>, bool) {
        let version = version.trim().trim_start_matches('v');
        let (core, pre) = match version.split_once('-') {
            Some((core, _)) => (core, true),
            None => (version, false),
        };
        let numbers = core
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect();
        (numbers, pre)
    }
    let (mut new, new_pre) = parts(latest);
    let (mut old, old_pre) = parts(current);
    let width = new.len().max(old.len());
    new.resize(width, 0);
    old.resize(width, 0);
    match new.cmp(&old) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => old_pre && !new_pre,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.3"));
        assert!(is_newer("1.0.0", "1.0.0-rc.2"));
        assert!(!is_newer("1.0.0-rc.2", "1.0.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
    }

    /// A `file://` address for `path` that curl reads on every platform.
    fn file_url(path: &std::path::Path) -> String {
        let text = path.display().to_string().replace('\\', "/");
        format!("file:///{}", text.trim_start_matches('/'))
    }

    #[test]
    fn a_release_description_is_read_from_a_file() {
        let dir = std::env::temp_dir().join(format!("fosscut-updates-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("latest.json");
        std::fs::write(
            &file,
            r#"{ "tag_name": "v0.2.0", "html_url": "https://example.org/r", "published_at": "2026-11-01T00:00:00Z" }"#,
        )
        .unwrap();
        let address = file_url(&file);
        let found = check("0.1.0", Some(&address)).unwrap();
        assert_eq!(found.latest.as_deref(), Some("0.2.0"));
        assert!(found.newer);
        assert_eq!(found.url.as_deref(), Some("https://example.org/r"));
        let same = check("0.2.0", Some(&address)).unwrap();
        assert!(!same.newer);
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(
            check("0.1.0", Some(&address)).unwrap_err().code(),
            "fosscut.updates.check_failed"
        );
        let missing = file_url(&dir.join("none.json"));
        assert_eq!(
            check("0.1.0", Some(&missing)).unwrap_err().code(),
            "fosscut.updates.check_failed"
        );
    }
}
