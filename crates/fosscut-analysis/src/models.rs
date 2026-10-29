//! Models for analyzers that need one (ADR-0019 item 5, ADR-0020 item 2). Manifests are
//! compiled in; a model installs only when asked, downloads with the system `curl` (or is
//! copied from a local file), and is verified by size and SHA-256 before it is used.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// A model FossCut can install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// Stable name, for example `whisper-base.en`.
    pub name: &'static str,
    /// What it does: `speech`.
    pub task: &'static str,
    /// File name in the models folder, and in the download address.
    pub file: &'static str,
    /// Exact size in bytes.
    pub size: u64,
    /// SHA-256 of the file, lowercase hex.
    pub sha256: &'static str,
    /// SPDX license of the weights.
    pub license: &'static str,
    /// `en` for English-only models, `multilingual` otherwise.
    pub languages: &'static str,
}

const WHISPER: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

/// Every model, most capable first.
pub const MODELS: &[Manifest] = &[
    Manifest {
        name: "whisper-large-v3-turbo",
        task: "speech",
        file: "ggml-large-v3-turbo.bin",
        size: 1_624_555_275,
        sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        license: "MIT",
        languages: "multilingual",
    },
    Manifest {
        name: "whisper-small",
        task: "speech",
        file: "ggml-small.bin",
        size: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        license: "MIT",
        languages: "multilingual",
    },
    Manifest {
        name: "whisper-base.en",
        task: "speech",
        file: "ggml-base.en.bin",
        size: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        license: "MIT",
        languages: "en",
    },
    Manifest {
        name: "whisper-tiny.en",
        task: "speech",
        file: "ggml-tiny.en.bin",
        size: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        license: "MIT",
        languages: "en",
    },
];

/// A model as front ends list it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelInfo {
    /// Stable name.
    pub name: &'static str,
    /// What it does.
    pub task: &'static str,
    /// Download size in bytes.
    pub size_bytes: u64,
    /// SPDX license.
    pub license: &'static str,
    /// `en` or `multilingual`.
    pub languages: &'static str,
    /// Where it downloads from.
    pub url: String,
    /// Whether it is installed in the models folder.
    pub installed: bool,
}

/// Why a model could not be found, chosen or installed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelError {
    /// No model has this name.
    #[error("there is no model {name}")]
    Unknown {
        /// The requested name.
        name: String,
    },
    /// The model, or any model for the task and language, is not installed.
    #[error("{what} is not installed")]
    NotInstalled {
        /// The model or kind of model.
        what: String,
    },
    /// The file does not match the manifest.
    #[error("{name} does not match its checksum: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// The model.
        name: String,
        /// The manifest's SHA-256 (or size).
        expected: String,
        /// What the file has.
        actual: String,
    },
    /// The download or copy failed.
    #[error("cannot install {name}: {message}")]
    DownloadFailed {
        /// The model.
        name: String,
        /// What went wrong.
        message: String,
    },
}

/// The manifest called `name`.
pub fn manifest(name: &str) -> Result<&'static Manifest, ModelError> {
    MODELS
        .iter()
        .find(|m| m.name == name)
        .ok_or_else(|| ModelError::Unknown { name: name.into() })
}

/// Where `model` lives in `dir`.
pub fn path(dir: &Path, model: &Manifest) -> PathBuf {
    dir.join(model.file)
}

/// The download address of `model`.
pub fn url(model: &Manifest) -> String {
    format!("{WHISPER}{}", model.file)
}

/// Whether `model` is installed in `dir`: present with the manifest's size. The checksum is
/// verified once, when it is installed.
pub fn is_installed(dir: &Path, model: &Manifest) -> bool {
    std::fs::metadata(path(dir, model)).is_ok_and(|m| m.len() == model.size)
}

/// `model` as front ends list it, with whether it is installed in `dir`.
pub fn info(dir: &Path, model: &Manifest) -> ModelInfo {
    ModelInfo {
        name: model.name,
        task: model.task,
        size_bytes: model.size,
        license: model.license,
        languages: model.languages,
        url: url(model),
        installed: is_installed(dir, model),
    }
}

/// Every model with whether it is installed in `dir`.
pub fn list(dir: &Path) -> Vec<ModelInfo> {
    MODELS.iter().map(|m| info(dir, m)).collect()
}

/// The most capable installed speech model for `language` (any language when `None`);
/// English-only models qualify only for English.
pub fn best_speech_model(dir: &Path, language: Option<&str>) -> Option<&'static Manifest> {
    MODELS.iter().find(|m| {
        m.task == "speech"
            && (m.languages == "multilingual"
                || matches!(language, Some("en"))
                || language.is_none())
            && is_installed(dir, m)
    })
}

/// Where to install from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The manifest's URL, or another URL (`file://` works too), fetched with `curl`.
    Download(Option<String>),
    /// A file already on disk.
    File(PathBuf),
}

/// Installs `model` into `dir` from `source`: written to a temporary file, checked against
/// the manifest's size and SHA-256, then moved into place. Returns the installed path.
pub fn install(dir: &Path, model: &Manifest, source: &Source) -> Result<PathBuf, ModelError> {
    let failed = |message: String| ModelError::DownloadFailed {
        name: model.name.into(),
        message,
    };
    std::fs::create_dir_all(dir)
        .map_err(|e| failed(format!("cannot create {}: {e}", dir.display())))?;
    let target = path(dir, model);
    let partial = dir.join(format!("{}.partial", model.file));
    let _ = std::fs::remove_file(&partial);
    match source {
        Source::Download(address) => {
            let address = address.clone().unwrap_or_else(|| url(model));
            let output = Command::new("curl")
                .args([
                    "--fail",
                    "--location",
                    "--silent",
                    "--show-error",
                    "--output",
                ])
                .arg(&partial)
                .arg(&address)
                .output()
                .map_err(|e| failed(format!("cannot run curl: {e}")))?;
            if !output.status.success() {
                let _ = std::fs::remove_file(&partial);
                return Err(failed(
                    String::from_utf8_lossy(&output.stderr).trim().to_string(),
                ));
            }
        }
        Source::File(file) => {
            std::fs::copy(file, &partial)
                .map_err(|e| failed(format!("cannot copy {}: {e}", file.display())))?;
        }
    }
    if let Err(error) = verify(&partial, model) {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    std::fs::rename(&partial, &target)
        .map_err(|e| failed(format!("cannot move into place: {e}")))?;
    Ok(target)
}

/// Checks `file` against the manifest's size and SHA-256.
fn verify(file: &Path, model: &Manifest) -> Result<(), ModelError> {
    let mismatch = |expected: String, actual: String| ModelError::ChecksumMismatch {
        name: model.name.into(),
        expected,
        actual,
    };
    let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    if size != model.size {
        return Err(mismatch(
            format!("{} bytes", model.size),
            format!("{size} bytes"),
        ));
    }
    let mut reader = std::fs::File::open(file).map_err(|e| ModelError::DownloadFailed {
        name: model.name.into(),
        message: e.to_string(),
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| ModelError::DownloadFailed {
                name: model.name.into(),
                message: e.to_string(),
            })?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != model.sha256 {
        return Err(mismatch(model.sha256.into(), actual));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fosscut-models-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A file of `size` bytes that takes no disk space.
    fn sparse(path: &Path, size: u64) {
        std::fs::File::create(path).unwrap().set_len(size).unwrap();
    }

    /// A small model whose file is the five bytes `hello`.
    const HELLO: Manifest = Manifest {
        name: "hello",
        task: "speech",
        file: "hello.bin",
        size: 5,
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        license: "MIT",
        languages: "en",
    };

    /// `path` as a `file://` address for curl.
    fn file_url(path: &Path) -> String {
        let path = path.display().to_string().replace('\\', "/");
        if path.starts_with('/') {
            format!("file://{path}")
        } else {
            format!("file:///{path}")
        }
    }

    #[test]
    fn manifests_are_complete() {
        let mut names: Vec<&str> = MODELS.iter().map(|m| m.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MODELS.len());
        for model in MODELS {
            assert_eq!(manifest(model.name), Ok(model));
            assert_eq!(model.sha256.len(), 64);
            assert!(
                model
                    .sha256
                    .chars()
                    .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
            );
            assert!(model.size > 50_000_000 && model.license == "MIT");
            assert!(
                url(model).starts_with("https://huggingface.co/ggerganov/whisper.cpp/resolve/")
                    && url(model).ends_with(model.file)
            );
            assert_eq!(model.languages == "en", model.name.ends_with(".en"));
        }
        assert_eq!(
            manifest("whisper-huge"),
            Err(ModelError::Unknown {
                name: "whisper-huge".into()
            })
        );
    }

    #[test]
    fn installed_models_are_found_by_size() {
        let dir = folder("installed");
        assert!(list(&dir).iter().all(|m| !m.installed));
        assert_eq!(best_speech_model(&dir, None), None);

        let tiny = manifest("whisper-tiny.en").unwrap();
        sparse(&path(&dir, tiny), tiny.size - 1);
        assert!(!is_installed(&dir, tiny), "a partial file is not installed");
        sparse(&path(&dir, tiny), tiny.size);
        assert!(is_installed(&dir, tiny));
        let listed = info(&dir, tiny);
        assert!(listed.installed && listed.size_bytes == tiny.size);
        assert_eq!(listed.url, url(tiny));
        assert_eq!(best_speech_model(&dir, None), Some(tiny));
        assert_eq!(best_speech_model(&dir, Some("en")), Some(tiny));
        assert_eq!(
            best_speech_model(&dir, Some("de")),
            None,
            "English-only models are only for English"
        );

        let small = manifest("whisper-small").unwrap();
        sparse(&path(&dir, small), small.size);
        assert_eq!(best_speech_model(&dir, Some("de")), Some(small));
        assert_eq!(
            best_speech_model(&dir, None),
            Some(small),
            "the most capable installed model"
        );
        assert_eq!(list(&dir).iter().filter(|m| m.installed).count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installs_are_verified_before_use() {
        let dir = folder("install");
        let source = dir.join("download.bin");
        std::fs::write(&source, b"hello").unwrap();
        let models = dir.join("models");

        // From a local file, and downloaded (here from a file address).
        let installed = install(&models, &HELLO, &Source::File(source.clone())).unwrap();
        assert_eq!(installed, models.join("hello.bin"));
        assert_eq!(std::fs::read(&installed).unwrap(), b"hello");
        std::fs::remove_file(&installed).unwrap();
        install(&models, &HELLO, &Source::Download(Some(file_url(&source)))).unwrap();
        assert!(is_installed(&models, &HELLO));
        std::fs::remove_file(&installed).unwrap();

        // The right size with the wrong contents, then the wrong size.
        std::fs::write(&source, b"HELLO").unwrap();
        let ModelError::ChecksumMismatch {
            expected, actual, ..
        } = install(&models, &HELLO, &Source::File(source.clone())).unwrap_err()
        else {
            panic!("expected a checksum mismatch");
        };
        assert_eq!((expected.as_str(), actual.len()), (HELLO.sha256, 64));
        std::fs::write(&source, b"hi").unwrap();
        assert_eq!(
            install(&models, &HELLO, &Source::File(source.clone())),
            Err(ModelError::ChecksumMismatch {
                name: "hello".into(),
                expected: "5 bytes".into(),
                actual: "2 bytes".into(),
            })
        );
        assert!(!installed.exists(), "a failed install leaves nothing");
        assert!(!models.join("hello.bin.partial").exists());

        // Sources that cannot be read.
        let missing = dir.join("missing.bin");
        assert!(matches!(
            install(&models, &HELLO, &Source::File(missing.clone())),
            Err(ModelError::DownloadFailed { .. })
        ));
        assert!(matches!(
            install(&models, &HELLO, &Source::Download(Some(file_url(&missing)))),
            Err(ModelError::DownloadFailed { .. })
        ));
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"a file where the folder should be").unwrap();
        assert!(matches!(
            install(&blocked, &HELLO, &Source::File(source)),
            Err(ModelError::DownloadFailed { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
