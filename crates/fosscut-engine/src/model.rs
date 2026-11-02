//! 3D model assets (ADR-0040): `.glb` files, and `.gltf` files with the buffers and images
//! they name, copied into the bundle's `models/` folder under the hash of their content,
//! probed for what they hold, and loaded for 3D object clips when frames are rendered.
//!
//! A `.gltf` file may only name files in its own folder or below it, by relative URIs (or
//! carry them as `data:` URIs): FossCut copies exactly those files, never reaches outside
//! the folder the user chose, and never fetches anything (ADR-0006, item 10).

use std::io::Read;
use std::path::{Component, Path};
use std::sync::Arc;

use fosscut_core::{ApiError, codes};
use fosscut_model::{AssetKind, ContentIdentity, Id, MediaInfo, ModelContent, PathHints, Project};
use fosscut_render::DecoderFrames;
use fosscut_render::objects::ModelData;
use serde_json::{Value, json};

use crate::EngineError;
use crate::import::Probed;

/// The bundle folder model files are copied into.
pub const MODELS_DIR: &str = "models";
/// The most bytes a model's files may hold together.
const MAX_BYTES: u64 = 1 << 30;

/// True for a `.glb` or `.gltf` file.
pub fn is_model(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("glb") || e.eq_ignore_ascii_case("gltf"))
}

fn failed(file: &Path, message: impl std::fmt::Display) -> ApiError {
    ApiError::new(
        codes::PROBE_FAILED,
        format!("{} is not a usable glTF model: {message}", file.display()),
    )
    .with_hint("FossCut reads glTF 2.0: .glb files, or .gltf files with the buffers and PNG or JPEG images they name beside them.")
}

/// Loads the model at `file` and reports what it holds.
pub fn probe(file: &Path) -> Result<Probed, ApiError> {
    let model = ModelData::load(file).map_err(|e| failed(file, e))?;
    Ok(Probed {
        kind: AssetKind::Model,
        info: MediaInfo {
            model: Some(model.info),
            ..MediaInfo::default()
        },
    })
}

/// Decodes `%XX` escapes of a URI path.
fn decoded(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((high * 16 + low) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The files of the model at `file`: its own name first, then the files a `.gltf` names,
/// as relative paths with `/` from its folder.
fn files_of(file: &Path) -> Result<Vec<String>, ApiError> {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| failed(file, "it has no file name"))?;
    let mut files = vec![name];
    let gltf = file
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gltf"));
    if !gltf {
        return Ok(files);
    }
    let text = std::fs::read_to_string(file).map_err(|e| failed(file, e))?;
    let document: Value = serde_json::from_str(&text).map_err(|e| failed(file, e))?;
    let uris = ["buffers", "images"]
        .iter()
        .flat_map(|key| document[key].as_array().into_iter().flatten())
        .filter_map(|entry| entry["uri"].as_str());
    for uri in uris {
        if uri.starts_with("data:") {
            continue;
        }
        if uri.contains("://") || uri.starts_with('/') || uri.contains('\\') {
            return Err(failed(
                file,
                format!("it names {uri}, which is not a file beside it"),
            ));
        }
        let relative = decoded(uri);
        let escapes = Path::new(&relative)
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
        if escapes {
            return Err(failed(
                file,
                format!("it names {uri}, outside its own folder"),
            ));
        }
        if !files.contains(&relative) {
            files.push(relative);
        }
    }
    Ok(files)
}

/// The model's content identity: the total size, and a BLAKE3 hash of every file's
/// relative path, size and bytes, in the order of `files`.
fn identity(root: &Path, files: &[String]) -> Result<ContentIdentity, ApiError> {
    let mut hasher = blake3::Hasher::new();
    let mut total = 0u64;
    let mut buffer = vec![0u8; 1 << 16];
    for file in files {
        let path = root.join(file);
        let mut reader = std::fs::File::open(&path).map_err(|e| failed(&path, e))?;
        let size = reader.metadata().map_err(|e| failed(&path, e))?.len();
        hasher.update(file.as_bytes());
        hasher.update(&[0]);
        hasher.update(&size.to_le_bytes());
        loop {
            let n = reader.read(&mut buffer).map_err(|e| failed(&path, e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        total += size;
        if total > MAX_BYTES {
            return Err(failed(&path, "its files hold more than 1 GiB"));
        }
    }
    Ok(ContentIdentity {
        size: total,
        sample_blake3: hasher.finalize().to_hex().to_string(),
    })
}

/// Copies `files` from `root` into `folder` unless it is there already: into a hidden
/// folder beside it first, renamed into place when complete.
fn copy_into(root: &Path, files: &[String], folder: &Path) -> Result<(), ApiError> {
    if folder.is_dir() {
        return Ok(());
    }
    let parent = folder.parent().unwrap_or(folder);
    let staging = parent.join(format!(
        ".{}.partial",
        folder
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    let result = (|| {
        std::fs::create_dir_all(parent)?;
        let _ = std::fs::remove_dir_all(&staging);
        for file in files {
            let target = file
                .split('/')
                .fold(staging.clone(), |path, part| path.join(part));
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::copy(root.join(file), &target)?;
        }
        std::fs::rename(&staging, folder)
    })();
    match result {
        Ok(()) => Ok(()),
        Err(_) if folder.is_dir() => {
            let _ = std::fs::remove_dir_all(&staging);
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(ApiError::new(
                codes::PROBE_FAILED,
                format!("cannot copy the model into {}: {error}", folder.display()),
            ))
        }
    }
}

/// The arguments of a `media.add` command that imports the model at `file` into the
/// project at `bundle`, named `name` or after the file. The model is read first, then its
/// files are copied into `models/<hash>/` in the bundle, once per content.
pub fn media_add_args(file: &Path, name: Option<&str>, bundle: &Path) -> Result<Value, ApiError> {
    let probed = probe(file)?;
    let root = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let files = files_of(file)?;
    let identity = identity(&root, &files)?;
    let key = &identity.sample_blake3[..16];
    let folder = bundle.join(MODELS_DIR).join(key);
    copy_into(&root, &files, &folder)?;
    let copied = folder.join(&files[0]);
    let mut path = crate::import::path_hints(&copied, bundle)?;
    if path.relative.is_none() {
        path = PathHints {
            relative: Some(format!("{MODELS_DIR}/{key}/{}", files[0])),
            ..path
        };
    }
    let name = name.map(str::to_string).unwrap_or_else(|| files[0].clone());
    Ok(json!({
        "name": name,
        "kind": AssetKind::Model,
        "path": path,
        "identity": identity,
        "info": probed.info,
    }))
}

/// Loads the model of a 3D object into `frames` when it shows a model asset not loaded
/// yet.
pub(crate) fn open_model(
    project: &Project,
    bundle: &Path,
    content: &ModelContent,
    frames: &mut DecoderFrames,
) -> Result<(), EngineError> {
    let Some(asset) = content.model_asset() else {
        return Ok(());
    };
    if frames.has_model(asset) {
        return Ok(());
    }
    let path = crate::export::resolve(project, bundle, asset)?;
    frames.insert_model(asset.clone(), Arc::new(load(asset, &path)?));
    Ok(())
}

/// Reads the model file of `asset` at `path` for drawing.
pub(crate) fn load(asset: &Id, path: &Path) -> Result<ModelData, EngineError> {
    ModelData::load(path).map_err(|message| EngineError::Failed {
        code: codes::OBJECT_UNREADABLE.into(),
        message: format!("the model of {asset} at {}: {message}", path.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_are_decoded_and_kept_inside_the_folder() {
        assert_eq!(decoded("my%20house.bin"), "my house.bin");
        assert_eq!(decoded("100%"), "100%");
        let dir = std::env::temp_dir().join(format!("fosscut-model-uris-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, uri: &str| {
            let file = dir.join(name);
            std::fs::write(
                &file,
                format!(r#"{{"asset": {{"version": "2.0"}}, "buffers": [{{"byteLength": 4, "uri": "{uri}"}}]}}"#),
            )
            .unwrap();
            file
        };
        let inside = write("inside.gltf", "parts/my%20house.bin");
        assert_eq!(
            files_of(&inside).unwrap(),
            vec!["inside.gltf".to_string(), "parts/my house.bin".to_string()]
        );
        for uri in ["../secret.bin", "/etc/passwd", "https://example.com/a.bin"] {
            let outside = write("outside.gltf", uri);
            let error = files_of(&outside).unwrap_err();
            assert_eq!(error.code, codes::PROBE_FAILED, "{uri}");
        }
        let embedded = write(
            "embedded.gltf",
            "data:application/octet-stream;base64,AAAAAA==",
        );
        assert_eq!(
            files_of(&embedded).unwrap(),
            vec!["embedded.gltf".to_string()]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
