//! Importing media: probing with ffprobe, content identity hashing and the path hints a
//! project stores, combined into the arguments of the `media.add` command.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path};
use std::process::Command;

use fosscut_core::{ApiError, codes};
use fosscut_model::{
    AssetKind, AudioInfo, ContentIdentity, Id, MediaInfo, PathHints, Project, VideoInfo,
};
use fosscut_time::{Flicks, Rate};
use serde_json::{Value, json};

const SAMPLE: u64 = 1 << 20;

/// Probed media ready for `media.add`.
pub struct Probed {
    /// Video, audio or image.
    pub kind: AssetKind,
    /// Streams, duration and format details.
    pub info: MediaInfo,
}

/// The arguments of a `media.add` command that imports `file` into the project at
/// `bundle`, named `name` or after the file.
pub fn media_add_args(file: &Path, name: Option<&str>, bundle: &Path) -> Result<Value, ApiError> {
    // A folder with an index.html, or an .html file, is a web composition (ADR-0039).
    if crate::web::is_composition(file) {
        let options = crate::web::CompositionOptions::default();
        return crate::web::media_add_args(file, name, bundle, &options);
    }
    // A glTF model is copied into the bundle with its files (ADR-0040).
    if crate::model::is_model(file) {
        return crate::model::media_add_args(file, name, bundle);
    }
    let probed = probe(file)?;
    let name = name.map(str::to_string).unwrap_or_else(|| {
        file.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    Ok(json!({
        "name": name,
        "kind": probed.kind,
        "path": path_hints(file, bundle)?,
        "identity": identity(file)?,
        "info": probed.info,
    }))
}

/// The arguments of a `media.relink` command pointing `asset` at `file`, its new place.
/// Only the content identity is measured; `media.relink` refuses a different file.
pub fn media_relink_args(asset: &Id, file: &Path, bundle: &Path) -> Result<Value, ApiError> {
    Ok(json!({
        "asset": asset,
        "path": path_hints(file, bundle)?,
        "identity": identity(file)?,
    }))
}

/// The assets of `project` whose files cannot be found from `bundle`.
pub fn missing_assets(project: &Project, bundle: &Path) -> Vec<Id> {
    project
        .media
        .assets
        .iter()
        .filter(|a| crate::export::resolve(project, bundle, &a.id).is_err())
        .map(|a| a.id.clone())
        .collect()
}

/// Marks the assets of a `media.list` result whose files cannot be found from `bundle`
/// with `"missing": true`, so front ends can offer to relink them.
pub fn mark_missing(listing: &mut Value, bundle: &Path) {
    let Some(assets) = listing.get_mut("assets").and_then(Value::as_array_mut) else {
        return;
    };
    for asset in assets {
        let hints: PathHints = asset
            .get("path")
            .and_then(|p| serde_json::from_value(p.clone()).ok())
            .unwrap_or_default();
        let relative = hints.relative.as_ref().map(|r| {
            r.split('/')
                .fold(bundle.to_path_buf(), |path, part| path.join(part))
        });
        let found = relative.is_some_and(|p| p.is_file())
            || hints
                .absolute
                .as_ref()
                .is_some_and(|a| Path::new(a).is_file());
        if !found {
            asset["missing"] = json!(true);
        }
    }
}

/// Where the file of `asset` is, for showing it in the file manager.
pub fn asset_file(
    project: &Project,
    bundle: &Path,
    asset: &Id,
) -> Result<std::path::PathBuf, ApiError> {
    crate::export::resolve(project, bundle, asset).map_err(ApiError::from)
}

/// The ffprobe program: `FOSSCUT_FFPROBE` when set, then the one FossCut's releases ship
/// (beside the program, or in `../lib/fosscut` as the Linux package installs it), otherwise
/// `ffprobe` on the path. An app opened from the Finder or a desktop menu does not see the
/// shell's path, so the shipped one matters there.
fn ffprobe_path() -> std::path::PathBuf {
    ffprobe_from(
        std::env::var_os("FOSSCUT_FFPROBE"),
        std::env::current_exe().ok().as_deref(),
    )
}

fn ffprobe_from(
    variable: Option<std::ffi::OsString>,
    program: Option<&Path>,
) -> std::path::PathBuf {
    let name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    if let Some(path) = variable.filter(|v| !v.is_empty()) {
        return path.into();
    }
    program
        .and_then(Path::parent)
        .into_iter()
        .flat_map(|dir| [dir.join(name), dir.join("../lib/fosscut").join(name)])
        .find(|path| path.is_file())
        .unwrap_or_else(|| name.into())
}

/// Probes `file`: `.cube` files are read as 3D LUTs, `.ttf`, `.otf` and `.ttc` files as
/// fonts, `.svg` files as vector drawings, everything else goes to ffprobe.
pub fn probe(file: &Path) -> Result<Probed, ApiError> {
    let extension = |names: &[&str]| {
        file.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| names.iter().any(|n| e.eq_ignore_ascii_case(n)))
    };
    if extension(&["cube"]) {
        return probe_lut(file);
    }
    if extension(&["ttf", "otf", "ttc", "otc"]) {
        return probe_font(file);
    }
    // Vector drawings import as drawings, not as images ffprobe rasterizes (ADR-0036).
    if extension(&["svg"]) {
        return crate::vector::probe(file);
    }
    // 3D models (ADR-0040).
    if extension(&["glb", "gltf"]) {
        return crate::model::probe(file);
    }
    probe_media(file)
}

/// Reads a font file and lists its families (ADR-0027).
fn probe_font(file: &Path) -> Result<Probed, ApiError> {
    let data = std::fs::read(file).map_err(|e| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("cannot read {}: {e}", file.display()),
        )
    })?;
    let (families, faces) = fosscut_render::text::font_families(&data).map_err(|e| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("{} is not a usable font: {e}", file.display()),
        )
        .with_hint("FossCut reads TrueType and OpenType fonts (.ttf, .otf) and collections (.ttc).")
    })?;
    Ok(Probed {
        kind: AssetKind::Font,
        info: MediaInfo {
            font: Some(fosscut_model::FontInfo { families, faces }),
            ..MediaInfo::default()
        },
    })
}

/// Reads a `.cube` file and checks it parses as a 3D LUT.
fn probe_lut(file: &Path) -> Result<Probed, ApiError> {
    let text = std::fs::read_to_string(file).map_err(|e| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("cannot read {}: {e}", file.display()),
        )
    })?;
    let lut = fosscut_render::lut::Lut3d::parse_cube(&text).map_err(|e| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("{} is not a usable 3D LUT: {e}", file.display()),
        )
        .with_hint("FossCut reads Adobe/Resolve .cube files with LUT_3D_SIZE from 2 to 256.")
    })?;
    Ok(Probed {
        kind: AssetKind::Lut,
        info: MediaInfo {
            lut: Some(fosscut_model::LutInfo { size: lut.size }),
            ..MediaInfo::default()
        },
    })
}

/// Runs ffprobe on `file`.
fn probe_media(file: &Path) -> Result<Probed, ApiError> {
    let output = Command::new(ffprobe_path())
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(file)
        .output()
        .map_err(|e| ApiError::new(codes::PROBE_UNAVAILABLE, format!("cannot run ffprobe: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ApiError::new(
            codes::PROBE_FAILED,
            format!("ffprobe could not read {}", file.display()),
        )
        .with_details(json!({ "ffprobe": stderr })));
    }
    let report: Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("unexpected ffprobe output: {e}"),
        )
    })?;
    interpret(&report).ok_or_else(|| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!(
                "{} has no usable audio, video or image stream",
                file.display()
            ),
        )
    })
}

fn interpret(report: &Value) -> Option<Probed> {
    let format = &report["format"];
    let format_name = format["format_name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let streams = report["streams"].as_array()?;
    let is_cover = |s: &Value| s["disposition"]["attached_pic"].as_i64() == Some(1);
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && !is_cover(s));
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    let duration = format["duration"]
        .as_str()
        .and_then(|d| Flicks::parse_seconds(d).ok())
        .filter(|d| d.is_positive());

    let still_formats = [
        "image2",
        "png_pipe",
        "jpeg_pipe",
        "webp_pipe",
        "bmp_pipe",
        "tiff_pipe",
    ];
    let is_still = video.is_some()
        && audio.is_none()
        && (still_formats.iter().any(|f| format_name.contains(f)) || duration.is_none());

    let video_info = video.map(|s| VideoInfo {
        width: s["width"].as_u64().unwrap_or(0) as u32,
        height: s["height"].as_u64().unwrap_or(0) as u32,
        rate: ["r_frame_rate", "avg_frame_rate"]
            .iter()
            .filter_map(|key| s[*key].as_str())
            .find_map(|r| r.parse::<Rate>().ok())
            .filter(|_| !is_still),
        codec: s["codec_name"].as_str().map(str::to_string),
        rotation: rotation(s),
    });
    let audio_info = audio.map(|s| AudioInfo {
        sample_rate: s["sample_rate"]
            .as_str()
            .and_then(|r| r.parse().ok())
            .unwrap_or(0),
        channels: s["channels"].as_u64().unwrap_or(0) as u16,
        codec: s["codec_name"].as_str().map(str::to_string),
    });
    let kind = if is_still {
        AssetKind::Image
    } else if video_info.is_some() {
        AssetKind::Video
    } else if audio_info.is_some() {
        AssetKind::Audio
    } else {
        return None;
    };
    Some(Probed {
        kind,
        info: MediaInfo {
            duration: if kind == AssetKind::Image {
                None
            } else {
                duration
            },
            container: (!format_name.is_empty()).then_some(format_name),
            video: video_info,
            audio: if kind == AssetKind::Image {
                None
            } else {
                audio_info
            },
            lut: None,
            ..Default::default()
        },
    })
}

fn rotation(stream: &Value) -> i32 {
    if let Some(r) = stream["tags"]["rotate"]
        .as_str()
        .and_then(|r| r.parse().ok())
    {
        return r;
    }
    stream["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|d| d["rotation"].as_f64()))
        .map(|r| r.round() as i32)
        .unwrap_or(0)
}

/// Size plus a BLAKE3 hash of the size, the first MiB and the last MiB.
pub fn identity(file: &Path) -> Result<ContentIdentity, ApiError> {
    let error = |e: std::io::Error| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("cannot read {}: {e}", file.display()),
        )
    };
    let mut handle = File::open(file).map_err(error)?;
    let size = handle.metadata().map_err(error)?.len();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    let mut buffer = vec![0u8; SAMPLE.min(size) as usize];
    handle.read_exact(&mut buffer).map_err(error)?;
    hasher.update(&buffer);
    if size > SAMPLE {
        let tail = SAMPLE.min(size - SAMPLE);
        handle.seek(SeekFrom::End(-(tail as i64))).map_err(error)?;
        let mut buffer = vec![0u8; tail as usize];
        handle.read_exact(&mut buffer).map_err(error)?;
        hasher.update(&buffer);
    }
    Ok(ContentIdentity {
        size,
        sample_blake3: hasher.finalize().to_hex().to_string(),
    })
}

/// Absolute path and the path relative to the bundle directory.
pub fn path_hints(file: &Path, bundle: &Path) -> Result<PathHints, ApiError> {
    let absolute = std::fs::canonicalize(file).map_err(|e| {
        ApiError::new(
            codes::PROBE_FAILED,
            format!("cannot find {}: {e}", file.display()),
        )
    })?;
    let relative = std::fs::canonicalize(bundle)
        .ok()
        .and_then(|bundle| relative_path(&absolute, &bundle));
    Ok(PathHints {
        relative,
        absolute: Some(absolute.to_string_lossy().into_owned()),
    })
}

fn relative_path(target: &Path, base: &Path) -> Option<String> {
    let target: Vec<Component> = target.components().collect();
    let base: Vec<Component> = base.components().collect();
    if target.first() != base.first() {
        return None;
    }
    let common = target.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> =
        std::iter::repeat_n("..".to_string(), base.len() - common).collect();
    parts.extend(
        target[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffprobe_is_the_variable_then_the_shipped_one_then_the_path() {
        let name = if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        };
        let base = std::env::temp_dir().join(format!("fosscut-ffprobe-{}", std::process::id()));
        let program = base.join("bin").join("fosscut");
        std::fs::create_dir_all(base.join("bin")).unwrap();
        std::fs::create_dir_all(base.join("lib").join("fosscut")).unwrap();
        // Nothing shipped: the one on the path.
        assert_eq!(ffprobe_from(None, Some(&program)), Path::new(name));
        // The Linux package's place, then beside the program.
        std::fs::write(base.join("lib").join("fosscut").join(name), b"").unwrap();
        assert_eq!(
            ffprobe_from(None, Some(&program)),
            base.join("bin").join("../lib/fosscut").join(name)
        );
        std::fs::write(base.join("bin").join(name), b"").unwrap();
        assert_eq!(
            ffprobe_from(None, Some(&program)),
            base.join("bin").join(name)
        );
        // The variable wins, unless it is empty.
        assert_eq!(
            ffprobe_from(Some("/opt/ff/ffprobe".into()), Some(&program)),
            Path::new("/opt/ff/ffprobe")
        );
        assert_eq!(
            ffprobe_from(Some("".into()), Some(&program)),
            base.join("bin").join(name)
        );
        assert_eq!(ffprobe_from(None, None), Path::new(name));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn interprets_a_video_report() {
        let report = json!({
            "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "8.000000" },
            "streams": [
                { "codec_type": "video", "codec_name": "h264", "width": 1280, "height": 720, "r_frame_rate": "30/1" },
                { "codec_type": "audio", "codec_name": "aac", "sample_rate": "48000", "channels": 1 }
            ]
        });
        let probed = interpret(&report).unwrap();
        assert_eq!(probed.kind, AssetKind::Video);
        assert_eq!(probed.info.duration, Some(Flicks::from_seconds(8).unwrap()));
        assert_eq!(probed.info.video.unwrap().rate, Some(Rate::FPS_30));
    }

    #[test]
    fn interprets_images_and_audio_with_cover_art() {
        let image = json!({
            "format": { "format_name": "png_pipe" },
            "streams": [{ "codec_type": "video", "codec_name": "png", "width": 64, "height": 64, "r_frame_rate": "25/1" }]
        });
        assert_eq!(interpret(&image).unwrap().kind, AssetKind::Image);
        let song = json!({
            "format": { "format_name": "mp3", "duration": "3.5" },
            "streams": [
                { "codec_type": "audio", "codec_name": "mp3", "sample_rate": "44100", "channels": 2 },
                { "codec_type": "video", "codec_name": "mjpeg", "disposition": { "attached_pic": 1 } }
            ]
        });
        let probed = interpret(&song).unwrap();
        assert_eq!(probed.kind, AssetKind::Audio);
        assert!(probed.info.video.is_none());
    }

    #[test]
    fn relative_paths_climb_out_of_the_bundle() {
        let rel = relative_path(
            Path::new("/a/media/clip.mp4"),
            Path::new("/a/projects/Promo.fosscut"),
        );
        assert_eq!(rel.as_deref(), Some("../../media/clip.mp4"));
    }
}
