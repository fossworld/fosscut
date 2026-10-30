//! Discovery records (ADR-0022, item 3): how clients find the host of a bundle. Records
//! live in the user's runtime directory, never in the bundle, and are written and removed
//! only by a process that holds the bundle's writer lock.

use std::io::Write;
use std::path::{Path, PathBuf};

use fosscut_core::ApiError;
use serde::{Deserialize, Serialize};

use super::{UNAVAILABLE, error};

/// The subdirectory of the runtime directory that holds records.
const SESSIONS: &str = "sessions";

/// What a host announces about a project it serves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Wire protocol version the host speaks.
    pub protocol: u32,
    /// Unix socket path or Windows pipe name.
    pub endpoint: String,
    /// The session key both sides prove they know, in hex. Never sent over the wire.
    pub key: String,
    /// The host's process id, for people; never used to decide liveness.
    pub pid: u32,
    /// The host program.
    pub host: HostName,
    /// The bundle's canonical path.
    pub bundle: PathBuf,
    /// The project's id.
    pub project: String,
    /// When hosting started, in milliseconds since the Unix epoch.
    pub started: u64,
}

/// A host program's name and version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostName {
    /// For example `FossCut` or `fosscut host`.
    pub name: String,
    /// Its version.
    pub version: String,
}

/// The user's runtime directory for live sessions: `FOSSCUT_RUNTIME_DIR` when set (for
/// tests), otherwise `~/Library/Application Support/fosscut/run` on macOS,
/// `%LOCALAPPDATA%\fosscut\run` on Windows, and `$XDG_RUNTIME_DIR/fosscut` or
/// `~/.local/state/fosscut/run` elsewhere.
pub fn runtime_dir() -> PathBuf {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = var("FOSSCUT_RUNTIME_DIR") {
        return dir;
    }
    let dir = if cfg!(target_os = "macos") {
        var("HOME").map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("fosscut")
                .join("run")
        })
    } else if cfg!(windows) {
        var("LOCALAPPDATA").map(|base| base.join("fosscut").join("run"))
    } else {
        var("XDG_RUNTIME_DIR")
            .map(|base| base.join("fosscut"))
            .or_else(|| {
                var("HOME").map(|home| {
                    home.join(".local")
                        .join("state")
                        .join("fosscut")
                        .join("run")
                })
            })
    };
    dir.unwrap_or_else(|| std::env::temp_dir().join("fosscut").join("run"))
}

/// Creates `dir` and its record folder, owner-only on Unix, and checks they are private.
pub(crate) fn prepare(dir: &Path) -> Result<(), ApiError> {
    for path in [dir.to_path_buf(), dir.join(SESSIONS)] {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).map_err(|e| {
            error(
                UNAVAILABLE,
                format!(
                    "cannot create the runtime directory {}: {e}",
                    path.display()
                ),
            )
        })?;
        check_private(&path)?;
    }
    Ok(())
}

/// On Unix, refuses a directory owned by another user or open to others: whoever controls
/// it could plant a record and receive a client's requests.
pub(crate) fn check_private(dir: &Path) -> Result<(), ApiError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(dir).map_err(|e| {
            error(
                UNAVAILABLE,
                format!("cannot read the runtime directory {}: {e}", dir.display()),
            )
        })?;
        let owner = rustix::process::geteuid().as_raw();
        if !meta.is_dir() || meta.uid() != owner || meta.mode() & 0o077 != 0 {
            return Err(error(
                UNAVAILABLE,
                format!(
                    "the runtime directory {} is not private to this user",
                    dir.display()
                ),
            )
            .with_hint(
                "Remove it, or make it yours with mode 700; FOSSCUT_RUNTIME_DIR chooses another.",
            )
            .with_details(serde_json::json!({ "runtime_dir": dir })));
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// The record key of a bundle: its device and inode on Unix, so every path to it finds
/// the same record, or the hash of its canonical path on Windows.
pub(crate) fn bundle_key(bundle: &Path) -> Result<String, ApiError> {
    let not_found = |e: std::io::Error| {
        ApiError::new(
            "fosscut.project.not_found",
            format!("{} cannot be read: {e}", bundle.display()),
        )
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(bundle).map_err(not_found)?;
        Ok(format!("{:x}-{:x}", meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let canonical = std::fs::canonicalize(bundle).map_err(not_found)?;
        let hash = blake3::hash(canonical.as_os_str().as_encoded_bytes());
        Ok(hash.to_hex()[..32].to_string())
    }
}

/// Where the record for `key` lives.
pub(crate) fn path(runtime: &Path, key: &str) -> PathBuf {
    runtime.join(SESSIONS).join(format!("{key}.json"))
}

/// The record for `key`. `Ok(None)` when there is none or it cannot be parsed; an error
/// when the runtime directory is not private, since its records cannot be trusted.
pub(crate) fn read(runtime: &Path, key: &str) -> Result<Option<Record>, ApiError> {
    let Ok(bytes) = std::fs::read(path(runtime, key)) else {
        return Ok(None);
    };
    check_private(runtime)?;
    check_private(&runtime.join(SESSIONS))?;
    Ok(serde_json::from_slice(&bytes).ok())
}

/// Writes the record for `key`: created new with owner-only permissions, then renamed
/// into place. The caller holds the bundle's writer lock.
pub(crate) fn write(runtime: &Path, key: &str, record: &Record) -> Result<(), ApiError> {
    prepare(runtime)?;
    let target = path(runtime, key);
    let temporary = target.with_extension(format!("json.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&temporary);
    let failed = |e: std::io::Error| {
        error(
            UNAVAILABLE,
            format!("cannot write the session record {}: {e}", target.display()),
        )
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(failed)?;
    let bytes = serde_json::to_vec_pretty(record).unwrap_or_default();
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(failed)?;
    drop(file);
    std::fs::rename(&temporary, &target).map_err(failed)
}

/// Removes the record for `key` and, when it names a socket in the runtime directory, the
/// socket file. The caller holds the bundle's writer lock, so no live host owns them.
pub(crate) fn remove(runtime: &Path, key: &str, record: Option<&Record>) {
    let _ = std::fs::remove_file(path(runtime, key));
    if let Some(record) = record {
        remove_socket(runtime, &record.endpoint);
    }
}

/// Removes a socket file that lies directly in the runtime directory; anything else a
/// record names is left alone.
pub(crate) fn remove_socket(runtime: &Path, endpoint: &str) {
    let socket = Path::new(endpoint);
    if cfg!(unix) && socket.parent() == Some(runtime) {
        let _ = std::fs::remove_file(socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(name: &str) -> PathBuf {
        let dir = super::super::test_runtime(name);
        prepare(&dir).unwrap();
        dir
    }

    fn sample(endpoint: String) -> Record {
        Record {
            protocol: 1,
            endpoint,
            key: "00".repeat(32),
            pid: 7,
            host: HostName {
                name: "test".into(),
                version: "1".into(),
            },
            bundle: PathBuf::from("/x/Demo.fosscut"),
            project: "proj_0001".into(),
            started: 1,
        }
    }

    #[test]
    fn records_round_trip_and_are_removed_with_their_socket() {
        let dir = runtime("round");
        assert_eq!(read(&dir, "k1").unwrap(), None);
        let socket = dir.join("abc.sock");
        std::fs::write(&socket, b"").unwrap();
        let record = sample(socket.display().to_string());
        write(&dir, "k1", &record).unwrap();
        assert_eq!(read(&dir, "k1").unwrap(), Some(record.clone()));
        write(&dir, "k1", &record).unwrap();
        remove(&dir, "k1", Some(&record));
        assert_eq!(read(&dir, "k1").unwrap(), None);
        assert!(!socket.exists() || cfg!(windows));
        std::fs::write(path(&dir, "bad"), b"{not json").unwrap();
        assert_eq!(
            read(&dir, "bad").unwrap(),
            None,
            "a broken record is ignored"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sockets_outside_the_runtime_directory_are_never_removed() {
        let dir = runtime("outside");
        let elsewhere = std::env::temp_dir().join(format!("fosscut-keep-{}", std::process::id()));
        std::fs::write(&elsewhere, b"keep").unwrap();
        remove(&dir, "k", Some(&sample(elsewhere.display().to_string())));
        assert!(elsewhere.exists());
        std::fs::remove_file(&elsewhere).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn records_are_private_and_open_directories_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = runtime("private");
        let record = sample(String::new());
        write(&dir, "k", &record).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&path(&dir, "k")), 0o600);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = read(&dir, "k").unwrap_err();
        assert_eq!(error.code, UNAVAILABLE);
        assert_eq!(error.details["runtime_dir"], serde_json::json!(dir));
        assert!(
            prepare(&dir).is_err(),
            "an open directory is refused, not fixed"
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(read(&dir, "k").unwrap().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_path_to_a_bundle_has_the_same_key() {
        let dir = runtime("key");
        let bundle = dir.join("Demo.fosscut");
        std::fs::create_dir_all(&bundle).unwrap();
        let key = bundle_key(&bundle).unwrap();
        assert_eq!(
            bundle_key(&dir.join(".").join("Demo.fosscut")).unwrap(),
            key
        );
        #[cfg(unix)]
        {
            let link = dir.join("Link.fosscut");
            std::os::unix::fs::symlink(&bundle, &link).unwrap();
            assert_eq!(bundle_key(&link).unwrap(), key);
        }
        let other = dir.join("Other.fosscut");
        std::fs::create_dir_all(&other).unwrap();
        assert_ne!(bundle_key(&other).unwrap(), key);
        let missing = bundle_key(&dir.join("Missing.fosscut")).unwrap_err();
        assert_eq!(missing.code, "fosscut.project.not_found");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_runtime_directory_follows_the_environment_variable_or_the_platform() {
        let dir = runtime_dir();
        match std::env::var_os("FOSSCUT_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            Some(set) => assert_eq!(dir, PathBuf::from(set)),
            None => assert!(
                dir.ends_with("run") || dir.ends_with("fosscut"),
                "{}",
                dir.display()
            ),
        }
    }
}
